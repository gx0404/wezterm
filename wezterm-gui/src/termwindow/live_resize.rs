//! fork（A7-d）：live resize（交互式拖拽窗口边框）期间推迟的工作。
//!
//! 拖拽时每个像素级 WM_SIZE 只要列数变化，原来都会同步 resize 所有 tab
//! 的所有 pane（scrollback 重排、ConPTY IPC、每个 tab 一次 TabResized
//! 引发的标题重算），并各发一次 update-status / window-resized。live 期间
//! 只让活动 tab 跟随窗口尺寸，其余 tab 记在这里，等拖拽静止或被切到前台
//! 时再补 resize；标题刷新与 window-resized 合并到结束时各发一次。
//!
//! 布局真源仍是 `mux::Tab`：这里只记录「哪些 tab 还没拿到最新尺寸」，
//! 尺寸本身始终取 `TermWindow::terminal_size`，不自建第二份布局。
//! 本模块是纯逻辑（时间由调用方传入），计时器与 mux 调用在
//! `termwindow/resize.rs`。

use mux::tab::TabId;
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// 多久没有新的 live resize 事件就视为拖拽结束。不是所有平台都会报告
/// 拖拽结束：X11 把每个 ConfigureNotify 都标成 live；macOS 结束拖拽时
/// 不会再发一次尺寸；Windows 的 WM_EXITSIZEMOVE 虽会以非 live 调一次
/// wm_size，但尺寸与上一次 WM_SIZE 相同时 window 层不会派发事件。
pub(crate) const LIVE_RESIZE_SETTLE: Duration = Duration::from_millis(100);

#[derive(Debug, Default)]
pub(crate) struct LiveResizeDeferral {
    /// 本窗口里没拿到最新 live 尺寸的 tab
    stale_tabs: HashSet<TabId>,
    /// live 期间跳过了标题刷新
    title_pending: bool,
    /// live 期间跳过了 window-resized 事件
    resized_event_pending: bool,
    /// 最近一次 live resize 事件的时间；Some 表示 live resize 进行中
    last_live_event: Option<Instant>,
    /// 有一个静止计时器在途
    timer_armed: bool,
}

/// `LiveResizeDeferral::finish` 交回、需要补做的工作
#[derive(Debug, Default, PartialEq)]
pub(crate) struct LiveResizeFlush {
    /// 需要补 resize 的 tab（升序）
    pub stale_tabs: Vec<TabId>,
    pub update_title: bool,
    pub window_resized: bool,
}

/// 静止计时器到点时的处理结果
#[derive(Debug, PartialEq)]
pub(crate) enum LiveResizeTimer {
    /// 没有推迟中的工作（已被其它路径结束）
    Idle,
    /// 已静止：调用 `finish` 补做
    Flush,
    /// 期间又来了 live 事件：重新等到这个时刻
    Rearm(Instant),
}

impl LiveResizeDeferral {
    pub fn is_active(&self) -> bool {
        self.last_live_event.is_some()
    }

    pub fn has_stale_tabs(&self) -> bool {
        !self.stale_tabs.is_empty()
    }

    /// 记一次 live resize 事件（它的 window-resized 也随之推迟）。
    /// 返回 Some(deadline) 时调用方要安排一个到该时刻的静止计时器；
    /// 已有计时器在途时返回 None。
    pub fn note_live_event(&mut self, now: Instant) -> Option<Instant> {
        self.last_live_event = Some(now);
        self.resized_event_pending = true;
        if self.timer_armed {
            None
        } else {
            self.timer_armed = true;
            Some(now + LIVE_RESIZE_SETTLE)
        }
    }

    /// 计时器没能安排出去（没有可投递的窗口）：清掉在途标记，调用方
    /// 应随即 `finish`
    pub fn timer_dropped(&mut self) {
        self.timer_armed = false;
    }

    /// 静止计时器到点
    pub fn on_timer(&mut self, now: Instant) -> LiveResizeTimer {
        self.timer_armed = false;
        match self.last_live_event {
            None => LiveResizeTimer::Idle,
            Some(last) => {
                let due = last + LIVE_RESIZE_SETTLE;
                if now >= due {
                    LiveResizeTimer::Flush
                } else {
                    self.timer_armed = true;
                    LiveResizeTimer::Rearm(due)
                }
            }
        }
    }

    /// 选出这次要立即 resize 到新尺寸的 tab。live 时只选活动 tab，
    /// 其余记脏；非 live 的 resize 覆盖全部 tab，脏集合随之清空。
    pub fn tabs_to_resize(
        &mut self,
        tabs: &[TabId],
        active: Option<TabId>,
        live: bool,
    ) -> Vec<TabId> {
        if !live {
            self.stale_tabs.clear();
            return tabs.to_vec();
        }
        let mut resize_now = vec![];
        for &tab_id in tabs {
            if Some(tab_id) == active {
                self.stale_tabs.remove(&tab_id);
                resize_now.push(tab_id);
            } else {
                self.stale_tabs.insert(tab_id);
            }
        }
        resize_now
    }

    /// live 期间跳过一次标题刷新，结束时补一次
    pub fn defer_title(&mut self) {
        self.title_pending = true;
    }

    /// `tab_id` 错过了 live resize 时返回 true（并移出脏集合），由调用方
    /// 补 resize
    pub fn take_stale(&mut self, tab_id: TabId) -> bool {
        self.stale_tabs.remove(&tab_id)
    }

    /// 结束 live resize，交回全部推迟的工作
    pub fn finish(&mut self) -> LiveResizeFlush {
        self.last_live_event = None;
        let mut stale_tabs: Vec<TabId> = self.stale_tabs.drain().collect();
        stale_tabs.sort_unstable();
        LiveResizeFlush {
            stale_tabs,
            update_title: std::mem::take(&mut self.title_pending),
            window_resized: std::mem::take(&mut self.resized_event_pending),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_resize_only_resizes_the_active_tab() {
        let mut d = LiveResizeDeferral::default();
        assert_eq!(d.tabs_to_resize(&[1, 2, 3], Some(2), true), vec![2]);
        assert!(d.has_stale_tabs());
        // 再拖一步：仍只动活动 tab
        assert_eq!(d.tabs_to_resize(&[1, 2, 3], Some(2), true), vec![2]);
        d.defer_title();
        d.note_live_event(Instant::now());

        let flush = d.finish();
        assert_eq!(
            flush,
            LiveResizeFlush {
                stale_tabs: vec![1, 3],
                update_title: true,
                window_resized: true,
            }
        );
        // 结束后没有残留
        assert!(!d.is_active());
        assert_eq!(d.finish(), LiveResizeFlush::default());
    }

    #[test]
    fn switching_tabs_during_live_resize_catches_the_tab_up() {
        let mut d = LiveResizeDeferral::default();
        d.tabs_to_resize(&[1, 2, 3], Some(1), true);
        // 切到 tab 3：它错过了尺寸，要补一次；只补一次
        assert!(d.take_stale(3));
        assert!(!d.take_stale(3));
        // 活动 tab 从未记脏
        assert!(!d.take_stale(1));
        // 新的活动 tab 在下一次 live 事件里直接 resize 并移出脏集合
        assert_eq!(d.tabs_to_resize(&[1, 2, 3], Some(2), true), vec![2]);
        assert_eq!(d.finish().stale_tabs, vec![1, 3]);
    }

    #[test]
    fn non_live_resize_covers_every_tab() {
        let mut d = LiveResizeDeferral::default();
        d.tabs_to_resize(&[1, 2, 3], Some(1), true);
        assert_eq!(d.tabs_to_resize(&[1, 2, 3], Some(1), false), vec![1, 2, 3]);
        assert!(!d.has_stale_tabs());
        assert!(d.finish().stale_tabs.is_empty());
    }

    #[test]
    fn settle_timer_waits_for_the_drag_to_go_quiet() {
        let mut d = LiveResizeDeferral::default();
        let t0 = Instant::now();
        // 第一次 live 事件安排计时器，之后的事件只更新时间
        assert_eq!(d.note_live_event(t0), Some(t0 + LIVE_RESIZE_SETTLE));
        let t1 = t0 + Duration::from_millis(60);
        assert_eq!(d.note_live_event(t1), None);
        assert!(d.is_active());

        // 计时器按第一次事件到点：拖拽仍在继续，顺延到最后一次事件之后
        assert_eq!(
            d.on_timer(t0 + LIVE_RESIZE_SETTLE),
            LiveResizeTimer::Rearm(t1 + LIVE_RESIZE_SETTLE)
        );
        // 顺延期间的新事件不再另起计时器
        assert_eq!(d.note_live_event(t1), None);
        assert_eq!(d.on_timer(t1 + LIVE_RESIZE_SETTLE), LiveResizeTimer::Flush);

        // 已被其它路径（非 live resize）结束后，在途计时器无事可做
        d.finish();
        assert_eq!(d.on_timer(t1 + LIVE_RESIZE_SETTLE), LiveResizeTimer::Idle);
        // 计时器到点后才开始的新一轮拖拽重新安排计时器
        let t2 = t1 + Duration::from_secs(1);
        assert_eq!(d.note_live_event(t2), Some(t2 + LIVE_RESIZE_SETTLE));
    }

    #[test]
    fn finish_before_the_timer_fires_keeps_one_timer_in_flight() {
        let mut d = LiveResizeDeferral::default();
        let t0 = Instant::now();
        assert!(d.note_live_event(t0).is_some());
        // 非 live resize 提前结束了这一轮
        d.finish();
        // 旧计时器还在途：紧接着的新一轮拖拽复用它，不重复安排
        let t1 = t0 + Duration::from_millis(10);
        assert_eq!(d.note_live_event(t1), None);
        assert_eq!(
            d.on_timer(t0 + LIVE_RESIZE_SETTLE),
            LiveResizeTimer::Rearm(t1 + LIVE_RESIZE_SETTLE)
        );
    }
}
