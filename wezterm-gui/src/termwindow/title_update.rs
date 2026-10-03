//! fork（A5）：窗口标题 / tab bar 刷新的过滤与合并逻辑。
//!
//! 一次标题刷新（`TermWindow::update_title_impl`）要重算整条 tab bar、
//! 调 2N+1 次 `format-tab-title` / `format-window-title`，还会触发一次
//! `update-status` Lua 事件；这里放与之相关、可独立单测的纯逻辑。

use std::time::Duration;
use wezterm_term::Alert;

/// fork（A5-c）：标题刷新的合并窗口。窗口内的多次 `update_title` 合成
/// 一次 `update_title_impl`；约两帧，人眼察觉不到，但足以吸收 spinner、
/// 多 pane 同时改标题、按键触发的连发请求。
pub(crate) const TITLE_UPDATE_COALESCE: Duration = Duration::from_millis(40);

/// fork（A5-c）：标题刷新请求的合并状态（纯逻辑，计时器由调用方安排）。
///
/// 用法：`request` 返回 true 时调用方安排一个 `TITLE_UPDATE_COALESCE`
/// 之后回主线程的冲刷，冲刷时 `take` 出请求并执行一次刷新；立即刷新
/// 路径（首帧、焦点变化、切 tab、非 live resize）也先 `take`，在途计时器
/// 到点后发现没有挂起请求就什么都不做。
#[derive(Debug, Default)]
pub(crate) struct TitleUpdateCoalescer {
    /// None：没有挂起请求；Some(with_status)：合并窗口结束时刷新一次，
    /// with_status 为真时顺带发 update-status（其 EventState 1+1 门控不变）
    pending: Option<bool>,
}

impl TitleUpdateCoalescer {
    /// 登记一次刷新请求。返回 true 表示这是本合并窗口的第一次请求，
    /// 调用方要安排一次延迟冲刷；之后的请求只并入挂起状态。
    pub(crate) fn request(&mut self, with_status: bool) -> bool {
        match &mut self.pending {
            Some(pending_status) => {
                *pending_status |= with_status;
                false
            }
            None => {
                self.pending = Some(with_status);
                true
            }
        }
    }

    /// 取出挂起请求；返回 Some(with_status) 时调用方执行一次刷新。
    pub(crate) fn take(&mut self) -> Option<bool> {
        self.pending.take()
    }
}

/// 会让 tab bar 或窗口标题重算的 pane 级 Alert。
///
/// fork（A5-b）：这些 Alert 只影响 pane 所在窗口；调用方必须再按窗口
/// 过滤（`TermWindow::window_contains_pane`），否则一次 OSC 0 会让所有
/// GUI 窗口都重算 tab bar 并发 update-status。
pub(crate) fn alert_refreshes_title(alert: &Alert) -> bool {
    matches!(
        alert,
        Alert::OutputSinceFocusLost
            | Alert::CurrentWorkingDirectoryChanged
            | Alert::WindowTitleChanged(_)
            | Alert::TabTitleChanged(_)
            | Alert::IconTitleChanged(_)
            | Alert::Progress(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use wezterm_term::Progress;

    #[test]
    fn title_and_progress_alerts_refresh_the_title() {
        for alert in [
            Alert::OutputSinceFocusLost,
            Alert::CurrentWorkingDirectoryChanged,
            Alert::WindowTitleChanged("t".to_string()),
            Alert::TabTitleChanged(Some("t".to_string())),
            Alert::IconTitleChanged(None),
            Alert::Progress(Progress::Indeterminate),
        ] {
            assert!(alert_refreshes_title(&alert), "{:?}", alert);
        }
    }

    #[test]
    fn burst_inside_the_window_runs_once() {
        let mut c = TitleUpdateCoalescer::default();
        // 只有第一次请求需要安排计时器
        assert!(c.request(true));
        for _ in 0..10 {
            assert!(!c.request(true));
        }
        // 计时器到点：执行一次
        assert_eq!(c.take(), Some(true));
        // 同一窗口不会再执行第二次
        assert_eq!(c.take(), None);
        // 下一次请求开启新的合并窗口
        assert!(c.request(true));
    }

    #[test]
    fn status_request_is_kept_when_merged() {
        let mut c = TitleUpdateCoalescer::default();
        assert!(c.request(false));
        assert!(!c.request(true));
        assert!(!c.request(false));
        // 窗口内只要有一次要求发 update-status，冲刷就要发
        assert_eq!(c.take(), Some(true));

        assert!(c.request(false));
        assert_eq!(c.take(), Some(false));
    }

    #[test]
    fn immediate_refresh_swallows_the_pending_flush() {
        let mut c = TitleUpdateCoalescer::default();
        assert!(c.request(true));
        // 立即刷新路径先吞掉挂起请求
        assert_eq!(c.take(), Some(true));
        // 在途计时器到点后无事可做
        assert_eq!(c.take(), None);
        // 之后的请求照常开启新窗口
        assert!(c.request(true));
        assert_eq!(c.take(), Some(true));
    }

    #[test]
    fn other_alerts_keep_their_own_handling() {
        // Bell / 用户变量 / 调色板 / 通知各有专门分支，不走标题刷新
        for alert in [
            Alert::Bell,
            Alert::PaletteChanged,
            Alert::SetUserVar {
                name: "n".to_string(),
                value: "v".to_string(),
            },
            Alert::ToastNotification {
                title: None,
                body: "b".to_string(),
                focus: false,
            },
        ] {
            assert!(!alert_refreshes_title(&alert), "{:?}", alert);
        }
    }
}
