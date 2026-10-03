//! fork（C3）：GPU 上下文丢失后的渲染状态重建。
//!
//! Optimus 笔记本睡眠唤醒、独显掉电重上电、驱动更新与 TDR 都会让 GL
//! 上下文失效。原实现在 `do_paint` 里一检测到就关窗并 forget，用户看到
//! 终端窗口凭空消失。现在 `do_paint` 只投递
//! `TermWindowNotif::RebuildRenderState`（渲染路径内不改状态），主线程
//! 的通知处理分两步：
//!
//! 1. `begin_render_state_rebuild`：丢掉全部 GPU 驻留状态（tab bar /
//!    modal 的 sprite、`RenderState`、GL 或 WebGPU 上下文），再 spawn 一个
//!    主线程 future 经 window 层重新 `enable_opengl()`（或重建
//!    `WebGpuState`）。旧上下文必须先释放干净：EGL surface 与 WGL 像素
//!    格式都绑定在同一个 HWND 上，ANGLE 更要求所有旧上下文销毁后才恢复
//!    丢失的设备。
//! 2. `finish_render_state_rebuild`：新上下文经 `ContextHandoff` 交接
//!    （`Rc<glium::Context>` 不是 Send，不能塞进 `TermWindowNotif::Apply`），
//!    用首次创建的同一路径 `RenderState::new` 重建 atlas / glyph cache /
//!    util sprites，再走与配置重载相同的缓存失效链。
//!
//! 重试计数与退避是纯逻辑（`ContextLossTracker`），连续失败超过
//! `MAX_REBUILD_ATTEMPTS` 才退回原来的关窗路径。

use crate::frontend::front_end;
use crate::renderstate::{RenderContext, RenderState};
use crate::termwindow::webgpu::WebGpuState;
use crate::termwindow::{TermWindow, TermWindowNotif};
use ::window::{Window, WindowOps};
use anyhow::Context;
use config::FrontEndSelection;
use smol::Timer;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

/// 连续重建失败这么多次后放弃并关窗。退避累计约 16 秒，足够独显重新
/// 上电或驱动完成重置；驱动彻底不可用时也不会无限黑窗。
pub(crate) const MAX_REBUILD_ATTEMPTS: u32 = 8;
/// 第一次失败后的退避，之后每次翻倍，封顶 `REBUILD_BACKOFF_MAX`
const REBUILD_BACKOFF_BASE: Duration = Duration::from_millis(250);
const REBUILD_BACKOFF_MAX: Duration = Duration::from_secs(4);

/// 一次重建请求到达主线程时的处置
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RebuildDecision {
    /// 立即重建
    Now,
    /// 等待退避时间后重建
    After(Duration),
    /// 连续失败过多，放弃并退回关窗
    GiveUp,
}

/// 第 `failures` 次连续失败之后的退避时间（failures >= 1）
pub(crate) fn backoff_after(failures: u32) -> Duration {
    let exponent = failures.saturating_sub(1).min(16);
    let delay = REBUILD_BACKOFF_BASE.saturating_mul(1u32 << exponent);
    delay.min(REBUILD_BACKOFF_MAX)
}

/// 根据连续失败次数决定本次重建怎么做
pub(crate) fn rebuild_decision(failures: u32) -> RebuildDecision {
    if failures >= MAX_REBUILD_ATTEMPTS {
        RebuildDecision::GiveUp
    } else if failures == 0 {
        RebuildDecision::Now
    } else {
        RebuildDecision::After(backoff_after(failures))
    }
}

/// 上下文丢失 → 重建 → 出帧 的状态机（纯逻辑，GL 调用都在调用方）
#[derive(Debug, Default)]
pub(crate) struct ContextLossTracker {
    /// 自上次成功出帧以来连续失败的重建次数
    failures: u32,
    /// 一次重建已投递或进行中；期间再检测到丢失不重复投递
    in_progress: bool,
}

impl ContextLossTracker {
    /// `do_paint` 检测到上下文丢失。返回 true 表示调用方要投递一次
    /// `RebuildRenderState`；重建进行中时返回 false。
    pub(crate) fn on_context_lost(&mut self) -> bool {
        if self.in_progress {
            return false;
        }
        self.in_progress = true;
        true
    }

    /// 重建通知到达主线程：按连续失败次数决定立即、退避还是放弃
    pub(crate) fn decide(&self) -> RebuildDecision {
        rebuild_decision(self.failures)
    }

    /// 本次尝试的序号（从 1 起），只用于日志
    pub(crate) fn attempt(&self) -> u32 {
        self.failures + 1
    }

    /// 重建失败；仍视为进行中，由调用方再投递一次通知走退避
    pub(crate) fn on_rebuild_failed(&mut self) {
        self.failures = self.failures.saturating_add(1);
    }

    /// 新上下文已装好；失败计数要等真正出帧成功才清零，驱动仍然不可
    /// 用时新上下文会立刻再次丢失，计数继续累加直到放弃
    pub(crate) fn on_rebuild_succeeded(&mut self) {
        self.in_progress = false;
    }

    /// 一帧成功 present
    pub(crate) fn on_frame_presented(&mut self) {
        self.failures = 0;
    }

    #[cfg(test)]
    fn in_progress(&self) -> bool {
        self.in_progress
    }
}

/// 异步重建出的新上下文交给主线程通知处理的槽位
pub(crate) type ContextHandoff = Rc<RefCell<Option<anyhow::Result<RenderContext>>>>;

impl TermWindow {
    /// `do_paint` / `do_paint_webgpu` 检测到上下文丢失时调用。渲染路径内
    /// 不改渲染状态，只投递通知，重建在 `dispatch_notif` 里进行。
    pub(crate) fn note_context_lost(&mut self, window: &Window) {
        if self.gpu_recovery.on_context_lost() {
            log::warn!("GPU context lost; scheduling a render state rebuild");
            window.notify(TermWindowNotif::RebuildRenderState);
        }
    }

    /// `TermWindowNotif::RebuildRenderState`：释放旧 GPU 状态，异步创建
    /// 新上下文。
    pub(crate) fn begin_render_state_rebuild(&mut self, window: &Window) {
        let delay = match self.gpu_recovery.decide() {
            RebuildDecision::Now => None,
            RebuildDecision::After(delay) => Some(delay),
            RebuildDecision::GiveUp => {
                self.abandon_render_state(window);
                return;
            }
        };
        log::warn!(
            "rebuilding render state after GPU context loss (attempt {}/{}, delay {:?})",
            self.gpu_recovery.attempt(),
            MAX_REBUILD_ATTEMPTS,
            delay.unwrap_or(Duration::ZERO)
        );

        self.release_gpu_state();

        let handoff = Rc::clone(&self.gpu_handoff);
        let window = window.clone();
        let front_end = self.config.front_end;
        let dimensions = self.dimensions;
        let config = self.config.clone();
        promise::spawn::spawn(async move {
            if let Some(delay) = delay {
                Timer::after(delay).await;
            }
            let result = match front_end {
                FrontEndSelection::WebGpu => WebGpuState::new(&window, dimensions, &config)
                    .await
                    .map(|state| RenderContext::WebGpu(Rc::new(state))),
                _ => window.enable_opengl().await.map(RenderContext::Glium),
            };
            handoff.borrow_mut().replace(result);
            window.notify(TermWindowNotif::RenderStateRebuilt);
        })
        .detach();
    }

    /// `TermWindowNotif::RenderStateRebuilt`：装上新上下文并重建
    /// `RenderState`；失败则再投递一次重建走退避/放弃。
    pub(crate) fn finish_render_state_rebuild(&mut self, window: &Window) {
        let result = match self.gpu_handoff.borrow_mut().take() {
            Some(result) => result,
            None => {
                log::error!("RenderStateRebuilt arrived without a pending context");
                return;
            }
        };

        match result.and_then(|ctx| self.install_render_context(ctx)) {
            Ok(()) => {
                self.gpu_recovery.on_rebuild_succeeded();
                log::warn!(
                    "render state rebuilt after GPU context loss: {}",
                    self.opengl_info.as_deref().unwrap_or("?")
                );
                self.invalidate_after_render_state_rebuild();
                window.invalidate();
            }
            Err(err) => {
                log::error!(
                    "failed to rebuild render state (attempt {}/{}): {:#}",
                    self.gpu_recovery.attempt(),
                    MAX_REBUILD_ATTEMPTS,
                    err
                );
                self.gpu_recovery.on_rebuild_failed();
                window.notify(TermWindowNotif::RebuildRenderState);
            }
        }
    }

    /// 丢掉所有引用 GPU 对象的状态。atlas 上的 sprite 还被 fancy tab bar
    /// 与 modal 的 computed element 持有，先让它们失效，否则旧上下文
    /// 释放不掉。
    fn release_gpu_state(&mut self) {
        self.invalidate_fancy_tab_bar();
        self.invalidate_modal();
        // `ShapedInfo::glyph` is an `Rc<CachedGlyph>` whose sprite pins the
        // atlas texture, and through it the context
        self.shape_generation += 1;
        self.shape_cache.borrow_mut().clear();
        self.line_to_ele_shape_cache.borrow_mut().clear();
        self.render_state = None;
        self.gl = None;
        self.webgpu = None;
    }

    /// 与 `TermWindow::created` 同一路径重建 `RenderState`
    fn install_render_context(&mut self, ctx: RenderContext) -> anyhow::Result<()> {
        let render_info = ctx.renderer_info();
        let render_state = RenderState::new(
            ctx.clone(),
            &self.fonts,
            &self.render_metrics,
            super::ATLAS_SIZE,
        )
        .context("RenderState::new")?;
        match &ctx {
            RenderContext::Glium(gl) => {
                self.gl = Some(Rc::clone(gl));
            }
            RenderContext::WebGpu(state) => {
                self.webgpu = Some(Rc::clone(state));
            }
        }
        self.opengl_info = Some(render_info);
        self.render_state = Some(render_state);
        Ok(())
    }

    /// 新 atlas 的布局与旧的无关：quad 缓存里的纹理坐标、形状缓存、
    /// tab bar 与 modal 的 computed element 全部作废（与
    /// `config_was_reloaded` / `recreate_texture_atlas` 的失效链一致）。
    fn invalidate_after_render_state_rebuild(&mut self) {
        self.quad_generation += 1;
        self.shape_generation += 1;
        self.shape_cache.borrow_mut().clear();
        self.line_to_ele_shape_cache.borrow_mut().clear();
        self.line_quad_cache.borrow_mut().clear();
        self.invalidate_fancy_tab_bar();
        self.invalidate_modal();
    }

    /// 重建连续失败超过上限：退回原来的关窗路径
    fn abandon_render_state(&mut self, window: &Window) {
        log::error!(
            "GPU context could not be rebuilt after {} attempts; closing the window",
            MAX_REBUILD_ATTEMPTS
        );
        window.close();
        front_end().forget_known_window(window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_rebuild_runs_immediately() {
        assert_eq!(rebuild_decision(0), RebuildDecision::Now);
        let tracker = ContextLossTracker::default();
        assert_eq!(tracker.decide(), RebuildDecision::Now);
        assert_eq!(tracker.attempt(), 1);
    }

    #[test]
    fn backoff_doubles_and_is_capped() {
        assert_eq!(backoff_after(1), Duration::from_millis(250));
        assert_eq!(backoff_after(2), Duration::from_millis(500));
        assert_eq!(backoff_after(3), Duration::from_secs(1));
        assert_eq!(backoff_after(4), Duration::from_secs(2));
        assert_eq!(backoff_after(5), REBUILD_BACKOFF_MAX);
        assert_eq!(backoff_after(6), REBUILD_BACKOFF_MAX);
        assert_eq!(backoff_after(u32::MAX), REBUILD_BACKOFF_MAX);
        assert_eq!(
            rebuild_decision(2),
            RebuildDecision::After(Duration::from_millis(500))
        );
    }

    #[test]
    fn gives_up_after_max_attempts() {
        let mut tracker = ContextLossTracker::default();
        assert!(tracker.on_context_lost());
        for n in 0..MAX_REBUILD_ATTEMPTS {
            assert_ne!(
                tracker.decide(),
                RebuildDecision::GiveUp,
                "attempt {} must still try",
                n + 1
            );
            tracker.on_rebuild_failed();
        }
        assert_eq!(tracker.decide(), RebuildDecision::GiveUp);
        assert_eq!(
            rebuild_decision(MAX_REBUILD_ATTEMPTS + 10),
            RebuildDecision::GiveUp
        );
    }

    #[test]
    fn context_lost_is_reported_once_per_rebuild() {
        let mut tracker = ContextLossTracker::default();
        assert!(tracker.on_context_lost());
        assert!(tracker.in_progress());
        assert!(!tracker.on_context_lost());
        tracker.on_rebuild_failed();
        assert!(
            !tracker.on_context_lost(),
            "a failed rebuild stays in progress"
        );
        tracker.on_rebuild_succeeded();
        assert!(!tracker.in_progress());
        assert!(tracker.on_context_lost());
    }

    #[test]
    fn failures_reset_only_once_a_frame_is_presented() {
        let mut tracker = ContextLossTracker::default();
        assert!(tracker.on_context_lost());
        tracker.on_rebuild_failed();
        tracker.on_rebuild_failed();
        tracker.on_rebuild_succeeded();
        assert_eq!(
            tracker.decide(),
            RebuildDecision::After(backoff_after(2)),
            "a rebuilt context that is lost again before painting keeps backing off"
        );
        tracker.on_frame_presented();
        assert_eq!(tracker.decide(), RebuildDecision::Now);
        assert_eq!(tracker.attempt(), 1);
    }
}
