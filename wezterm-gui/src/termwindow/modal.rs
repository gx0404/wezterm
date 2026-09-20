use crate::termwindow::box_model::ComputedElement;
use crate::TermWindow;
use config::keyassignment::KeyAssignment;
use downcast_rs::{impl_downcast, Downcast};
use std::cell::Ref;
use wezterm_term::{KeyCode, KeyModifiers};

/// fork: sentinel row index for modal chrome (titles, footers, input
/// lines) that must swallow clicks instead of letting the generic
/// "press outside the modal closes it" logic dismiss the overlay.
/// Modal implementations must ignore mouse events routed with this row.
pub const MODAL_CHROME_ROW: usize = usize::MAX;

/// fork: sentinel band for clickable section tabs in the settings
/// overlay (WZ-09). Rows in [MODAL_SECTION_BASE, MODAL_SECTION_BASE +
/// MODAL_SECTION_MAX) identify a section tab by index; data rows are
/// small indices and chrome rows use MODAL_CHROME_ROW, so the band must
/// never collide with either.
pub const MODAL_SECTION_BASE: usize = usize::MAX - 64;
pub const MODAL_SECTION_MAX: usize = 16;
const _: () = assert!(MODAL_SECTION_BASE + MODAL_SECTION_MAX <= MODAL_CHROME_ROW);

pub trait Modal: Downcast {
    fn perform_assignment(
        &self,
        _assignment: &KeyAssignment,
        _term_window: &mut TermWindow,
    ) -> bool {
        false
    }
    /// A mouse event that hit one of the modal's rows in the hit map
    /// (see `UIItemType::Modal`); `row` is the visible row index the
    /// event landed on.
    fn mouse_event(
        &self,
        event: ::window::MouseEvent,
        row: usize,
        term_window: &mut TermWindow,
    ) -> anyhow::Result<()>;
    fn key_down(
        &self,
        key: KeyCode,
        mods: KeyModifiers,
        term_window: &mut TermWindow,
    ) -> anyhow::Result<bool>;
    fn computed_element(
        &self,
        term_window: &mut TermWindow,
    ) -> anyhow::Result<Ref<'_, [ComputedElement]>>;
    fn reconfigure(&self, term_window: &mut TermWindow);
    /// fork: called when this modal goes away. All three dismissal paths
    /// reach it: Esc (the modal calls `cancel_modal`), a click outside
    /// (`mouseevent.rs` calls `cancel_modal`) and being replaced by another
    /// modal (`set_modal`). Implementations restore volatile state such as a
    /// colour scheme preview here, otherwise a preview the user merely
    /// scrolled past stays on the window.
    fn on_dismissed(&self, _term_window: &mut TermWindow) {}
}
impl_downcast!(Modal);
