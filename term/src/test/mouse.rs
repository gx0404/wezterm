//! fork(zh): 鼠标上报编码测试，重点覆盖侧键 X1/X2（xterm 按钮 8/9）在
//! SGR、X10、UTF-8(1005) 三种编码下的按下/释放/拖动字节，以及未开启上报、
//! 失焦补发释放和左/中/右/滚轮编码不回归。终端回写经 ThreadedWriter 线程，
//! 测试端用 CaptureWriter 的 flush 信号同步等待。

use super::*;
use crate::input::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use k9::assert_equal as assert_eq;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};
use wezterm_dynamic::{FromDynamic, ToDynamic, Value};

/// Collects what the terminal writes back towards the pty. Writes happen
/// on the ThreadedWriter thread, so every flush is signalled to the test.
struct CaptureWriter {
    buf: Arc<Mutex<Vec<u8>>>,
    flushed: Sender<()>,
}

impl std::io::Write for CaptureWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.flushed.send(()).ok();
        Ok(())
    }
}

struct MouseTerm {
    term: Terminal,
    buf: Arc<Mutex<Vec<u8>>>,
    flushed: Receiver<()>,
    /// Everything the test expects to have been written so far
    expected: Vec<u8>,
}

fn escaped(bytes: &[u8]) -> String {
    bytes
        .iter()
        .flat_map(|&b| std::ascii::escape_default(b))
        .map(char::from)
        .collect()
}

impl MouseTerm {
    fn new() -> Self {
        let buf = Arc::new(Mutex::new(vec![]));
        let (tx, rx) = channel();
        let term = Terminal::new(
            TerminalSize {
                rows: 24,
                cols: 80,
                pixel_width: 80 * 8,
                pixel_height: 24 * 16,
                dpi: 0,
            },
            Arc::new(TestTermConfig { scrollback: 0 }),
            "WezTerm",
            "O_o",
            Box::new(CaptureWriter {
                buf: Arc::clone(&buf),
                flushed: tx,
            }),
        );
        Self {
            term,
            buf,
            flushed: rx,
            expected: vec![],
        }
    }

    /// Enables a DEC private mode, eg: `1000` for click reporting
    fn decset(&mut self, mode: u16) {
        self.term.advance_bytes(format!("\x1b[?{mode}h"));
    }

    fn mouse(
        &mut self,
        kind: MouseEventKind,
        button: MouseButton,
        modifiers: KeyModifiers,
        x: usize,
        y: i64,
    ) {
        self.term
            .mouse_event(MouseEvent {
                kind,
                x,
                y,
                x_pixel_offset: 0,
                y_pixel_offset: 0,
                button,
                modifiers,
            })
            .unwrap();
    }

    fn press(&mut self, button: MouseButton, x: usize, y: i64) {
        self.mouse(MouseEventKind::Press, button, KeyModifiers::NONE, x, y);
    }

    fn release(&mut self, button: MouseButton, x: usize, y: i64) {
        self.mouse(MouseEventKind::Release, button, KeyModifiers::NONE, x, y);
    }

    /// A move as the GUI reports it while only a side button is held:
    /// the terminal falls back to the most recently pressed button.
    fn drag(&mut self, x: usize, y: i64) {
        self.mouse(
            MouseEventKind::Move,
            MouseButton::None,
            KeyModifiers::NONE,
            x,
            y,
        );
    }

    /// Asserts that `chunk` is the next output. The comparison is
    /// cumulative, so unexpected output that arrived earlier fails too.
    fn expect(&mut self, chunk: &[u8]) {
        self.expected.extend_from_slice(chunk);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if self.buf.lock().unwrap().len() >= self.expected.len() {
                break;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() || self.flushed.recv_timeout(remaining).is_err() {
                break;
            }
        }
        let actual = self.buf.lock().unwrap().clone();
        assert_eq!(escaped(&actual), escaped(&self.expected));
    }
}

#[test]
fn sgr_reports_side_buttons_as_xterm_8_and_9() {
    let mut term = MouseTerm::new();
    term.decset(1000);
    term.decset(1006);

    term.press(MouseButton::X1, 4, 2);
    term.expect(b"\x1b[<128;5;3M");
    term.release(MouseButton::X1, 4, 2);
    term.expect(b"\x1b[<128;5;3m");

    term.press(MouseButton::X2, 4, 2);
    term.expect(b"\x1b[<129;5;3M");
    term.release(MouseButton::X2, 4, 2);
    term.expect(b"\x1b[<129;5;3m");
}

#[test]
fn sgr_side_button_with_ctrl() {
    let mut term = MouseTerm::new();
    term.decset(1000);
    term.decset(1006);

    term.mouse(
        MouseEventKind::Press,
        MouseButton::X1,
        KeyModifiers::CTRL,
        4,
        2,
    );
    term.expect(b"\x1b[<144;5;3M");
}

#[test]
fn sgr_button_event_drag_with_side_buttons() {
    let mut term = MouseTerm::new();
    term.decset(1002);
    term.decset(1006);

    term.press(MouseButton::X1, 4, 2);
    term.expect(b"\x1b[<128;5;3M");
    term.drag(5, 2);
    term.expect(b"\x1b[<160;6;3M");
    term.release(MouseButton::X1, 5, 2);
    term.expect(b"\x1b[<128;6;3m");

    term.press(MouseButton::X2, 5, 2);
    term.expect(b"\x1b[<129;6;3M");
    term.drag(6, 2);
    term.expect(b"\x1b[<161;7;3M");
    term.release(MouseButton::X2, 6, 2);
    term.expect(b"\x1b[<129;7;3m");
}

#[test]
fn x10_side_button_bytes() {
    let mut term = MouseTerm::new();
    term.decset(1002);

    // 32 + 128 = 0xA0; coordinates are 1-based plus 32
    term.press(MouseButton::X1, 4, 2);
    term.expect(b"\x1b[M\xA0%#");
    // 32 + 128 + 32 = 0xC0
    term.drag(5, 2);
    term.expect(b"\x1b[M\xC0&#");
    // X10 has no per-button release, so it reports button 3
    term.release(MouseButton::X1, 5, 2);
    term.expect(b"\x1b[M#&#");

    // Regression: the left button keeps its single byte encoding
    term.press(MouseButton::Left, 4, 2);
    term.expect(b"\x1b[M %#");
    term.release(MouseButton::Left, 4, 2);
    term.expect(b"\x1b[M#%#");
}

#[test]
fn utf8_side_button_bytes() {
    let mut term = MouseTerm::new();
    term.decset(1002);
    term.decset(1005);

    // U+00A0 and U+00C0 encoded as UTF-8, as xterm does
    term.press(MouseButton::X1, 4, 2);
    term.expect(b"\x1b[M\xC2\xA0%#");
    term.drag(5, 2);
    term.expect(b"\x1b[M\xC3\x80&#");
    term.release(MouseButton::X1, 5, 2);
    term.expect(b"\x1b[M#&#");
}

#[test]
fn side_buttons_are_not_reported_without_mouse_tracking() {
    let mut term = MouseTerm::new();

    term.press(MouseButton::X1, 4, 2);
    term.release(MouseButton::X1, 4, 2);
    term.press(MouseButton::X2, 4, 2);
    term.release(MouseButton::X2, 4, 2);

    // Output is ordered, so once the left press shows up anything the
    // side buttons might have written would already be there too.
    term.decset(1000);
    term.decset(1006);
    term.press(MouseButton::Left, 4, 2);
    term.expect(b"\x1b[<0;5;3M");
}

#[test]
fn losing_focus_releases_a_held_side_button() {
    let mut term = MouseTerm::new();
    term.decset(1000);
    term.decset(1006);

    term.press(MouseButton::X1, 4, 2);
    term.expect(b"\x1b[<128;5;3M");
    term.term.focus_changed(false);
    term.expect(b"\x1b[<128;1;1m");
}

#[test]
fn sgr_regular_buttons_and_wheel_are_unchanged() {
    let mut term = MouseTerm::new();
    term.decset(1002);
    term.decset(1006);

    for (button, code) in [
        (MouseButton::Left, 0),
        (MouseButton::Middle, 1),
        (MouseButton::Right, 2),
    ] {
        term.press(button, 4, 2);
        term.expect(format!("\x1b[<{code};5;3M").as_bytes());
        term.release(button, 4, 2);
        term.expect(format!("\x1b[<{code};5;3m").as_bytes());
    }

    term.press(MouseButton::WheelUp(1), 4, 2);
    term.expect(b"\x1b[<64;5;3M");
    term.press(MouseButton::WheelDown(1), 4, 2);
    term.expect(b"\x1b[<65;5;3M");

    term.press(MouseButton::Left, 4, 2);
    term.expect(b"\x1b[<0;5;3M");
    term.mouse(
        MouseEventKind::Move,
        MouseButton::Left,
        KeyModifiers::NONE,
        5,
        2,
    );
    term.expect(b"\x1b[<32;6;3M");
    term.release(MouseButton::Left, 5, 2);
    term.expect(b"\x1b[<0;6;3m");
}

#[test]
fn side_buttons_round_trip_through_dynamic() {
    for (button, name) in [(MouseButton::X1, "X1"), (MouseButton::X2, "X2")] {
        let value = button.to_dynamic();
        assert_eq!(value, Value::String(name.to_string()));
        assert_eq!(
            MouseButton::from_dynamic(&value, Default::default()).unwrap(),
            button
        );
    }
}
