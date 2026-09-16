use crate::termwindow::box_model::*;
use crate::termwindow::modal::Modal;
use crate::termwindow::{DimensionContext, TermWindow, UIItemType};
use config::keyassignment::{
    ClipboardCopyDestination, ClipboardPasteSource, KeyAssignment, PaneDirection, SpawnCommand,
    SpawnTabDomain, SplitPane, SplitSize,
};
use config::Dimension;
use std::cell::RefCell;
use std::rc::Rc;
use wezterm_term::{KeyCode, KeyModifiers};
use window::color::LinearRgba;
use window::WindowOps;

/// A menu item; a `None` action renders a separator row
pub type MenuItem = (Option<String>, Option<KeyAssignment>);

/// A lightweight context menu opened at a screen position, mirroring
/// the right-click menus of herdr / tmux (`display-menu`).
/// Mouse hover selects, click activates, Escape / clicking outside
/// dismisses (outside dismissal lives in `mouse_event_impl`).
pub struct ContextMenu {
    items: Vec<MenuItem>,
    selected: RefCell<usize>,
    /// Window-relative pixel position where the menu was opened
    x: f32,
    y: f32,
    element: RefCell<Option<Vec<ComputedElement>>>,
}

impl ContextMenu {
    pub fn new(items: Vec<MenuItem>, x: f32, y: f32) -> Self {
        Self {
            items,
            selected: RefCell::new(0),
            x,
            y,
            element: RefCell::new(None),
        }
    }

    fn row_is_selectable(&self, row: usize) -> bool {
        self.items.get(row).map(|i| i.1.is_some()).unwrap_or(false)
    }

    fn activate(&self, row: usize, term_window: &mut TermWindow) {
        let action = match self.items.get(row) {
            Some((_, Some(action))) => action.clone(),
            _ => return,
        };
        term_window.cancel_modal();
        if let Some(pane) = term_window.get_active_pane_or_overlay() {
            if let Err(err) = term_window.perform_key_assignment(&pane, &action) {
                log::error!("while performing context menu action: {err:#}");
            }
        }
    }

    fn move_selection(&self, delta: isize) {
        let len = self.items.len();
        if len == 0 {
            return;
        }
        let mut selected = self.selected.borrow_mut();
        loop {
            let next = (*selected as isize + delta).rem_euclid(len as isize) as usize;
            *selected = next;
            if self.row_is_selectable(next) {
                break;
            }
        }
    }

    /// The standard pane context menu (right click in the terminal area)
    pub fn pane_menu(x: f32, y: f32) -> Self {
        Self::new(
            vec![
                (
                    Some("Split Pane Right".into()),
                    Some(KeyAssignment::SplitPane(SplitPane {
                        direction: PaneDirection::Right,
                        size: SplitSize::Percent(50),
                        command: SpawnCommand::default(),
                        top_level: false,
                    })),
                ),
                (
                    Some("Split Pane Down".into()),
                    Some(KeyAssignment::SplitPane(SplitPane {
                        direction: PaneDirection::Down,
                        size: SplitSize::Percent(50),
                        command: SpawnCommand::default(),
                        top_level: false,
                    })),
                ),
                (
                    Some("Toggle Pane Zoom".into()),
                    Some(KeyAssignment::TogglePaneZoomState),
                ),
                (None, None),
                (
                    Some("Copy".into()),
                    Some(KeyAssignment::CopyTo(ClipboardCopyDestination::Clipboard)),
                ),
                (
                    Some("Paste".into()),
                    Some(KeyAssignment::PasteFrom(ClipboardPasteSource::Clipboard)),
                ),
                (None, None),
                (
                    Some("Scroll to Top".into()),
                    Some(KeyAssignment::ScrollToTop),
                ),
                (
                    Some("Scroll to Bottom".into()),
                    Some(KeyAssignment::ScrollToBottom),
                ),
                (None, None),
                (
                    Some("Close Pane".into()),
                    Some(KeyAssignment::CloseCurrentPane { confirm: true }),
                ),
            ],
            x,
            y,
        )
    }

    /// The context menu for a specific tab (right click on a tab)
    pub fn tab_menu(tab_idx: usize, x: f32, y: f32) -> Self {
        Self::new(
            vec![
                (
                    Some("New Tab".into()),
                    Some(KeyAssignment::SpawnTab(SpawnTabDomain::CurrentPaneDomain)),
                ),
                (
                    Some("Show Tab Navigator".into()),
                    Some(KeyAssignment::ShowTabNavigator),
                ),
                (None, None),
                (
                    Some("Move Tab Left".into()),
                    Some(KeyAssignment::MoveTabRelative(-1)),
                ),
                (
                    Some("Move Tab Right".into()),
                    Some(KeyAssignment::MoveTabRelative(1)),
                ),
                (None, None),
                (
                    Some(format!("Close Tab {tab_idx}")),
                    Some(KeyAssignment::CloseCurrentTab { confirm: true }),
                ),
            ],
            x,
            y,
        )
    }

    /// The context menu for empty areas of the tab bar
    pub fn tab_bar_menu(x: f32, y: f32) -> Self {
        Self::new(
            vec![
                (
                    Some("New Tab".into()),
                    Some(KeyAssignment::SpawnTab(SpawnTabDomain::CurrentPaneDomain)),
                ),
                (
                    Some("Show Launcher".into()),
                    Some(KeyAssignment::ShowLauncher),
                ),
            ],
            x,
            y,
        )
    }

    fn compute(
        term_window: &mut TermWindow,
        items: &[MenuItem],
        selected: usize,
        x: f32,
        y: f32,
    ) -> anyhow::Result<Vec<ComputedElement>> {
        let font = term_window
            .fonts
            .command_palette_font()
            .expect("to resolve command palette font");
        let metrics = crate::utilsprites::RenderMetrics::with_font_metrics(&font.metrics())
            .scale_line_height(term_window.config.command_palette_line_height);

        let bg: InheritableColor = term_window
            .config
            .command_palette_bg_color
            .to_linear()
            .into();
        let fg: InheritableColor = term_window
            .config
            .command_palette_fg_color
            .to_linear()
            .into();

        let mut rows = vec![];
        let mut max_width_cells: f32 = 10.;
        for (idx, (label, action)) in items.iter().enumerate() {
            let (row_bg, row_fg) = if idx == selected && action.is_some() {
                (fg.clone(), bg.clone())
            } else {
                (LinearRgba::TRANSPARENT.into(), fg.clone())
            };
            let text = match label {
                Some(label) => label.clone(),
                None => "────────".to_string(),
            };
            max_width_cells = max_width_cells.max(text.chars().count() as f32);
            rows.push(
                Element::new(&font, ElementContent::Text(text))
                    .colors(ElementColors {
                        border: BorderColor::default(),
                        bg: row_bg,
                        text: row_fg,
                    })
                    .padding(BoxDimension {
                        left: Dimension::Cells(0.75),
                        right: Dimension::Cells(0.75),
                        top: Dimension::Cells(0.1),
                        bottom: Dimension::Cells(0.1),
                    })
                    .min_width(Some(Dimension::Percent(1.)))
                    .display(DisplayType::Block)
                    .item_type(UIItemType::Modal(idx)),
            );
        }

        let element = Element::new(&font, ElementContent::Children(rows))
            .colors(ElementColors {
                border: BorderColor::default(),
                bg: bg.clone(),
                text: fg.clone(),
            })
            .padding(BoxDimension::new(Dimension::Cells(0.25)))
            .border(BoxDimension::new(Dimension::Pixels(1.)))
            .margin(BoxDimension::new(Dimension::Cells(0.25)))
            .display(DisplayType::Block);

        let border = term_window.get_os_border();
        let row_height = metrics.cell_size.height as f32 * 1.2;
        let menu_height = items.len() as f32 * row_height + 8.;
        let menu_width = max_width_cells * metrics.cell_size.width as f32 + 16.;
        // Clamp so that the menu stays inside the window; flip upwards
        // when there is no room below the click position
        let mut menu_y = y;
        if menu_y + menu_height > term_window.dimensions.pixel_height as f32 {
            menu_y = (menu_y - menu_height).max(border.top.get() as f32);
        }
        let menu_x = x
            .min(term_window.dimensions.pixel_width as f32 - menu_width - border.right.get() as f32)
            .max(border.left.get() as f32);

        let (padding_left, padding_top) = term_window.padding_left_top();
        let computed = term_window.compute_element(
            &LayoutContext {
                width: DimensionContext {
                    dpi: term_window.dimensions.dpi as f32,
                    pixel_max: term_window.dimensions.pixel_width as f32,
                    pixel_cell: metrics.cell_size.width as f32,
                },
                height: DimensionContext {
                    dpi: term_window.dimensions.dpi as f32,
                    pixel_max: term_window.dimensions.pixel_height as f32,
                    pixel_cell: metrics.cell_size.height as f32,
                },
                bounds: euclid::rect(
                    padding_left as f32 + menu_x,
                    padding_top as f32 + menu_y,
                    menu_width,
                    menu_height,
                ),
                metrics: &metrics,
                gl_state: term_window.render_state.as_ref().unwrap(),
                zindex: 100,
            },
            &element,
        )?;

        Ok(vec![computed])
    }
}

impl Modal for ContextMenu {
    fn mouse_event(
        &self,
        event: ::window::MouseEvent,
        row: usize,
        term_window: &mut TermWindow,
    ) -> anyhow::Result<()> {
        use ::window::MouseEventKind as WMEK;
        match event.kind {
            WMEK::Move => {
                if self.row_is_selectable(row) && *self.selected.borrow() != row {
                    self.selected.replace(row);
                    term_window.invalidate_modal();
                }
            }
            WMEK::Press(::window::MousePress::Left) => {
                self.activate(row, term_window);
            }
            _ => {}
        }
        Ok(())
    }

    fn key_down(
        &self,
        key: KeyCode,
        mods: KeyModifiers,
        term_window: &mut TermWindow,
    ) -> anyhow::Result<bool> {
        match (key, mods) {
            (KeyCode::Escape, KeyModifiers::NONE)
            | (KeyCode::Char('g'), KeyModifiers::CTRL)
            | (KeyCode::Char('q'), KeyModifiers::NONE) => {
                term_window.cancel_modal();
            }
            (KeyCode::UpArrow, KeyModifiers::NONE)
            | (KeyCode::Char('p'), KeyModifiers::CTRL)
            | (KeyCode::Char('k'), KeyModifiers::NONE) => {
                self.move_selection(-1);
            }
            (KeyCode::DownArrow, KeyModifiers::NONE)
            | (KeyCode::Char('n'), KeyModifiers::CTRL)
            | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                self.move_selection(1);
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                let row = *self.selected.borrow();
                self.activate(row, term_window);
                return Ok(true);
            }
            _ => return Ok(false),
        }
        term_window.invalidate_modal();
        Ok(true)
    }

    fn computed_element(
        &self,
        term_window: &mut TermWindow,
    ) -> anyhow::Result<std::cell::Ref<'_, [ComputedElement]>> {
        if self.element.borrow().is_none() {
            let element = Self::compute(
                term_window,
                &self.items,
                *self.selected.borrow(),
                self.x,
                self.y,
            )?;
            self.element.borrow_mut().replace(element);
        }
        Ok(std::cell::Ref::map(self.element.borrow(), |v| {
            v.as_ref().unwrap().as_slice()
        }))
    }

    fn reconfigure(&self, _term_window: &mut TermWindow) {
        self.element.borrow_mut().take();
    }
}

/// Convenience: open a context menu modally at the given position
pub fn open_context_menu(term_window: &TermWindow, menu: ContextMenu) {
    term_window.set_modal(Rc::new(menu));
    if let Some(window) = term_window.window.as_ref() {
        window.invalidate();
    }
}
