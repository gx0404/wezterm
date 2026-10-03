//! fork（C4）：`QuitApplication` 的确认判定。
//!
//! 关窗路径（`TermWindow::close_requested`）在 `AlwaysPrompt` 下先问 mux
//! `Window::can_close_without_prompting`：所有窗格都只跑着
//! `skip_close_confirmation_for_processes_named` 里的空闲 shell（或
//! `mux-is-process-stateful` 回调判为无状态）时直接关闭。`QuitApplication`
//! 原先无条件弹「Really Quit」，这里让它复用同一判定：全部 mux 窗口的
//! 全部窗格都空闲才免确认，任一窗格有状态就照常确认。

use config::WindowCloseConfirmation;

/// `QuitApplication` 是否需要弹确认。`windows_can_close` 是每个 mux 窗口
/// `can_close_without_prompting()` 的结果；没有窗口时视为可以直接退出
/// （与 `GuiFrontEnd` 无窗口时的处理一致）。
pub(crate) fn quit_needs_confirmation(
    confirmation: WindowCloseConfirmation,
    windows_can_close: impl IntoIterator<Item = bool>,
) -> bool {
    match confirmation {
        WindowCloseConfirmation::NeverPrompt => false,
        WindowCloseConfirmation::AlwaysPrompt => !windows_can_close.into_iter().all(|can| can),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_prompt_quits_even_with_stateful_panes() {
        assert!(!quit_needs_confirmation(
            WindowCloseConfirmation::NeverPrompt,
            [false, false]
        ));
    }

    #[test]
    fn always_prompt_skips_the_confirmation_when_every_window_is_idle() {
        assert!(!quit_needs_confirmation(
            WindowCloseConfirmation::AlwaysPrompt,
            [true, true, true]
        ));
    }

    #[test]
    fn always_prompt_confirms_when_any_window_is_stateful() {
        assert!(quit_needs_confirmation(
            WindowCloseConfirmation::AlwaysPrompt,
            [true, false, true]
        ));
        assert!(quit_needs_confirmation(
            WindowCloseConfirmation::AlwaysPrompt,
            [false]
        ));
    }

    #[test]
    fn no_windows_means_nothing_to_confirm() {
        assert!(!quit_needs_confirmation(
            WindowCloseConfirmation::AlwaysPrompt,
            std::iter::empty()
        ));
    }
}
