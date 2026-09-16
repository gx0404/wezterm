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
}
impl_downcast!(Modal);
