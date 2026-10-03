//! fork（A5）：窗口标题 / tab bar 刷新的过滤与合并逻辑。
//!
//! 一次标题刷新（`TermWindow::update_title_impl`）要重算整条 tab bar、
//! 调 2N+1 次 `format-tab-title` / `format-window-title`，还会触发一次
//! `update-status` Lua 事件；这里放与之相关、可独立单测的纯逻辑。

use wezterm_term::Alert;

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
