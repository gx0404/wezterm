//! fork（A5-a）：OSC 0/1/2 标题未变时不重复发 Alert。
//! 每条标题 Alert 在 GUI 侧都会触发 tab bar 重算与 Lua 回调，
//! shell 提示符/spinner 反复设置同一标题时不能放大成重算风暴。

use super::*;
use crate::terminal::{Alert, AlertHandler};
use k9::assert_equal as assert_eq;

struct RecordingAlerts(Arc<Mutex<Vec<Alert>>>);

impl AlertHandler for RecordingAlerts {
    fn alert(&mut self, alert: Alert) {
        self.0.lock().unwrap().push(alert);
    }
}

/// 建一个记录全部 Alert 的终端
fn recording_term() -> (TestTerm, Arc<Mutex<Vec<Alert>>>) {
    let mut term = TestTerm::new(2, 10, 0);
    let alerts = Arc::new(Mutex::new(vec![]));
    term.term
        .set_notification_handler(Box::new(RecordingAlerts(Arc::clone(&alerts))));
    (term, alerts)
}

/// 取出并清空目前记录的标题类 Alert
fn take_title_alerts(alerts: &Mutex<Vec<Alert>>) -> Vec<Alert> {
    alerts
        .lock()
        .unwrap()
        .drain(..)
        .filter(|alert| {
            matches!(
                alert,
                Alert::WindowTitleChanged(_) | Alert::IconTitleChanged(_)
            )
        })
        .collect()
}

#[test]
fn repeated_identical_osc0_alerts_once() {
    let (mut term, alerts) = recording_term();

    term.print("\x1b]0;hello\x07");
    term.print("\x1b]0;hello\x07");
    term.print("\x1b]0;hello\x07");
    assert_eq!(
        take_title_alerts(&alerts),
        vec![
            Alert::WindowTitleChanged("hello".to_string()),
            Alert::IconTitleChanged(Some("hello".to_string())),
        ]
    );

    // 标题真的变了仍然要通知
    term.print("\x1b]0;world\x07");
    assert_eq!(
        take_title_alerts(&alerts),
        vec![
            Alert::WindowTitleChanged("world".to_string()),
            Alert::IconTitleChanged(Some("world".to_string())),
        ]
    );
    assert_eq!(term.get_title(), "world");
}

#[test]
fn repeated_identical_osc2_alerts_once() {
    let (mut term, alerts) = recording_term();

    term.print("\x1b]2;spin\x07");
    term.print("\x1b]2;spin\x07");
    assert_eq!(
        take_title_alerts(&alerts),
        vec![Alert::WindowTitleChanged("spin".to_string())]
    );
}

#[test]
fn repeated_identical_osc1_alerts_once() {
    let (mut term, alerts) = recording_term();

    term.print("\x1b]1;icon\x07");
    term.print("\x1b]1;icon\x07");
    assert_eq!(
        take_title_alerts(&alerts),
        vec![Alert::IconTitleChanged(Some("icon".to_string()))]
    );

    // 清空图标标题是一次变化；再清一次不是
    term.print("\x1b]1;\x07");
    term.print("\x1b]1;\x07");
    assert_eq!(
        take_title_alerts(&alerts),
        vec![Alert::IconTitleChanged(None)]
    );
}

#[test]
fn osc0_with_same_title_still_alerts_when_it_clears_icon_title() {
    let (mut term, alerts) = recording_term();

    term.print("\x1b]2;same\x07");
    term.print("\x1b]1;icon\x07");
    take_title_alerts(&alerts);
    assert_eq!(term.get_title(), "icon");

    // 窗口标题没变，但 OSC 0 会清掉图标标题，get_title 的结果随之改变
    term.print("\x1b]0;same\x07");
    assert_eq!(
        take_title_alerts(&alerts),
        vec![
            Alert::WindowTitleChanged("same".to_string()),
            Alert::IconTitleChanged(Some("same".to_string())),
        ]
    );
    assert_eq!(term.get_title(), "same");
}
