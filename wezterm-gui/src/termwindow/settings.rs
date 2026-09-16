//! fork 新增：herdr 式设置浮层（Modal）。
//!
//! 四个分区：语言（中文/English）、外观（内置配色方案，移动即预览、
//! Esc 还原、Enter 应用）、交互（右键菜单/滚动条/响铃/关闭确认）、
//! 字体（字号步进与重置）。
//! 生效链路：预览走 `TermWindow.config_overrides`（每窗口、易失，
//! `config_was_reloaded` 即时刷新）；应用则写入
//! `config::gui_settings::store_key`（gui-settings.json，原子写）后
//! `config::reload()`——全局生效且跨重启持久化，不触碰用户 Lua。
//! 渲染复用命令面板的字体/配色与 box model；交互行经
//! `UIItemType::Modal(row)` 进 hit map，与右键菜单共用鼠标通道。

use crate::termwindow::box_model::*;
use crate::termwindow::modal::{Modal, MODAL_CHROME_ROW};
use crate::termwindow::{DimensionContext, TermWindow, UIItemType};
use config::i18n::{tr, UiLanguage};
use config::keyassignment::KeyAssignment;
use config::{AudibleBell, Dimension, WindowCloseConfirmation};
use std::cell::RefCell;
use std::rc::Rc;
use wezterm_dynamic::{ToDynamic, Value};
use wezterm_term::{KeyCode, KeyModifiers};
use window::color::LinearRgba;
use window::WindowOps;

/// matches `config::config::default_font_size` (the wezterm default)
const DEFAULT_FONT_SIZE: f64 = 12.0;

#[derive(Copy, Clone, PartialEq, Eq)]
enum Section {
    Language,
    Appearance,
    Interaction,
    Font,
}

impl Section {
    const ALL: [Section; 4] = [
        Section::Language,
        Section::Appearance,
        Section::Interaction,
        Section::Font,
    ];

    fn title(self) -> &'static str {
        match self {
            Section::Language => "Language",
            Section::Appearance => "Appearance",
            Section::Interaction => "Interaction",
            Section::Font => "Font",
        }
    }
}

/// One selectable row in the current section
#[derive(Clone)]
enum Item {
    /// Language choice (label is intentionally not translated:
    /// each language is shown in its own writing)
    LanguageChoice(UiLanguage, &'static str),
    /// Builtin color scheme entry
    Scheme(String),
    /// boolean toggle; `key` is the config option name
    BoolToggle {
        label: &'static str,
        key: &'static str,
    },
    /// two-value enum cycler
    EnumChoice {
        label: &'static str,
        key: &'static str,
    },
    /// font size step / reset actions
    FontOp(FontOp),
}

#[derive(Copy, Clone)]
enum FontOp {
    Decrease,
    Increase,
    Reset,
}

pub struct SettingsOverlay {
    section: RefCell<Section>,
    /// selection within the visible (filtered) rows of the section
    selected: RefCell<usize>,
    top_row: RefCell<usize>,
    /// filter text, only used by the Appearance section
    filter: RefCell<String>,
    /// per-window overrides snapshot taken when the overlay was opened;
    /// restored on Esc so cancelled previews don't stick around
    overrides_snapshot: RefCell<Value>,
    element: RefCell<Option<Vec<ComputedElement>>>,
}

fn upsert_override(tw: &mut TermWindow, key: &str, value: Value) {
    let mut obj = match std::mem::take(&mut tw.config_overrides) {
        Value::Object(obj) => obj,
        _ => Default::default(),
    };
    obj.insert(Value::String(key.to_string()), value);
    tw.config_overrides = Value::Object(obj);
    tw.config_was_reloaded();
}

fn restore_overrides(tw: &mut TermWindow, snapshot: &Value) {
    if *snapshot != tw.config_overrides {
        tw.config_overrides = snapshot.clone();
        tw.config_was_reloaded();
    }
}

/// Persist one settings key and reload the configuration so the change
/// applies globally (all windows) and survives restarts.
fn persist_and_reload(key: &str, value: &Value) -> anyhow::Result<()> {
    config::gui_settings::store_key(key, value)?;
    config::reload();
    Ok(())
}

impl SettingsOverlay {
    pub fn new(term_window: &TermWindow) -> Self {
        Self {
            section: RefCell::new(Section::Language),
            selected: RefCell::new(0),
            top_row: RefCell::new(0),
            filter: RefCell::new(String::new()),
            overrides_snapshot: RefCell::new(term_window.config_overrides.clone()),
            element: RefCell::new(None),
        }
    }

    fn items_for(&self, section: Section, _term_window: &TermWindow) -> Vec<Item> {
        match section {
            Section::Language => vec![
                Item::LanguageChoice(UiLanguage::ZhCn, "中文"),
                Item::LanguageChoice(UiLanguage::En, "English"),
            ],
            Section::Appearance => {
                let mut names = config::builtin_scheme_names();
                let filter = self.filter.borrow();
                if !filter.is_empty() {
                    let pattern = crate::overlay::selector::matcher_pattern(&filter);
                    names.retain(|name| {
                        crate::overlay::selector::matcher_score(&pattern, name).is_some()
                    });
                }
                names
                    .into_iter()
                    .map(|name| Item::Scheme(name.to_string()))
                    .collect()
            }
            Section::Interaction => vec![
                Item::BoolToggle {
                    label: "Right-click menu",
                    key: "mouse_right_click_menu",
                },
                Item::BoolToggle {
                    label: "Scroll bar",
                    key: "enable_scroll_bar",
                },
                Item::EnumChoice {
                    label: "Audible bell",
                    key: "audible_bell",
                },
                Item::EnumChoice {
                    label: "Close confirmation",
                    key: "window_close_confirmation",
                },
            ],
            Section::Font => vec![
                Item::FontOp(FontOp::Decrease),
                Item::FontOp(FontOp::Increase),
                Item::FontOp(FontOp::Reset),
            ],
        }
    }

    /// The rows visible under the current filter (all sections except
    /// Appearance are unfiltered)
    fn visible_items(&self, term_window: &TermWindow) -> Vec<Item> {
        self.items_for(*self.section.borrow(), term_window)
    }

    fn max_rows_on_screen(
        &self,
        term_window: &TermWindow,
        metrics: &crate::utilsprites::RenderMetrics,
    ) -> usize {
        let mut rows = ((term_window.dimensions.pixel_height * 6 / 10)
            / metrics.cell_size.height as usize)
            .saturating_sub(4);
        rows = rows.max(4);
        rows
    }

    fn move_selection(&self, delta: isize, term_window: &TermWindow) {
        let items = self.visible_items(term_window);
        let len = items.len();
        if len == 0 {
            return;
        }
        let mut selected = self.selected.borrow_mut();
        *selected = (*selected as isize + delta).rem_euclid(len as isize) as usize;

        // keep the selection inside the scroll window
        let max_rows = {
            // metrics-independent variant of max_rows_on_screen: borrow
            // of render metrics requires &TermWindow only
            let mut rows = ((term_window.dimensions.pixel_height * 6 / 10)
                / term_window.render_metrics.cell_size.height as usize)
                .saturating_sub(4);
            rows = rows.max(4);
            rows
        };
        let mut top_row = self.top_row.borrow_mut();
        if *selected < *top_row {
            *top_row = *selected;
        } else if *selected >= *top_row + max_rows {
            *top_row = selected.saturating_sub(max_rows - 1);
        }
    }

    fn switch_section(&self, delta: isize) {
        let all = Section::ALL;
        let idx = all
            .iter()
            .position(|s| *s == *self.section.borrow())
            .unwrap_or(0);
        let next = (idx as isize + delta).rem_euclid(all.len() as isize) as usize;
        self.section.replace(all[next]);
        self.selected.replace(0);
        self.top_row.replace(0);
        // the filter only applies to Appearance
        self.filter.borrow_mut().clear();
    }

    /// Preview a scheme change: volatile per-window override so the
    /// colors update live; cancelled by Esc (restore snapshot)
    fn preview_scheme(&self, term_window: &mut TermWindow, name: &str) {
        upsert_override(term_window, "color_scheme", Value::String(name.to_string()));
    }

    fn activate(&self, row: usize, term_window: &mut TermWindow) {
        let items = self.visible_items(term_window);
        let Some(item) = items.get(row).cloned() else {
            return;
        };
        let snapshot = self.overrides_snapshot.borrow().clone();
        let result: anyhow::Result<()> = match item {
            Item::LanguageChoice(lang, _) => persist_and_reload("language", &lang.to_dynamic()),
            Item::Scheme(name) => {
                // drop the preview first so the persisted value is the
                // single source of truth
                restore_overrides(term_window, &snapshot);
                persist_and_reload("color_scheme", &Value::String(name))
            }
            Item::BoolToggle { label: _, key } => {
                let current = effective_bool(term_window, key).unwrap_or_else(|| default_bool(key));
                let next = !current;
                // apply instantly (preview + persist), mirroring herdr's
                // click-to-apply toggles
                upsert_override(term_window, key, Value::Bool(next));
                persist_and_reload(key, &Value::Bool(next))
            }
            Item::EnumChoice { label: _, key } => {
                let next = next_enum_value(term_window, key);
                upsert_override(term_window, key, next.to_dynamic());
                persist_and_reload(key, &next.to_dynamic())
            }
            Item::FontOp(op) => {
                let current = term_window.config.font_size;
                let next = match op {
                    FontOp::Decrease => (current - 0.5).max(6.0),
                    FontOp::Increase => (current + 0.5).min(100.0),
                    FontOp::Reset => DEFAULT_FONT_SIZE,
                };
                let value = Value::F64(ordered_float::OrderedFloat(next));
                upsert_override(term_window, "font_size", value.clone());
                persist_and_reload("font_size", &value)
            }
        };
        if let Err(err) = result {
            log::error!("settings: failed to apply: {err:#}");
        }
    }

    fn row_label(&self, item: &Item, term_window: &TermWindow) -> String {
        match item {
            Item::LanguageChoice(lang, label) => {
                let current = term_window.config.language;
                let marker = if *lang == current { "✓ " } else { "  " };
                format!("{marker}{label}")
            }
            Item::Scheme(name) => {
                let current = term_window.config.color_scheme.as_deref();
                let marker = if current == Some(name.as_str()) {
                    "✓ "
                } else {
                    "  "
                };
                format!("{marker}{name}")
            }
            Item::BoolToggle { label, key } => {
                let value = effective_bool(term_window, key).unwrap_or_else(|| default_bool(key));
                let value = if value { tr("On") } else { tr("Off") };
                format!("  {}: {value}", tr(label))
            }
            Item::EnumChoice { label, key } => {
                let value = enum_display(term_window, key);
                format!("  {}: {value}", tr(label))
            }
            Item::FontOp(op) => {
                let label = match op {
                    FontOp::Decrease => "Decrease font size",
                    FontOp::Increase => "Increase font size",
                    FontOp::Reset => "Reset font size",
                };
                let size = term_window.config.font_size;
                format!("  {}（{}: {size:.1}）", tr(label), tr("Font size"))
            }
        }
    }

    fn compute(&self, term_window: &mut TermWindow) -> anyhow::Result<Vec<ComputedElement>> {
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

        let items = self.visible_items(term_window);
        let max_rows = self.max_rows_on_screen(term_window, &metrics);
        let top_row = *self.top_row.borrow();
        let selected = *self.selected.borrow();

        let mut rows = vec![];

        // Title
        rows.push(
            Element::new(&font, ElementContent::Text(tr("Settings").into_owned()))
                .colors(ElementColors {
                    border: BorderColor::default(),
                    bg: LinearRgba::TRANSPARENT.into(),
                    text: fg.clone(),
                })
                .padding(BoxDimension {
                    left: Dimension::Cells(0.5),
                    right: Dimension::Cells(0.5),
                    top: Dimension::Cells(0.1),
                    bottom: Dimension::Cells(0.1),
                })
                .display(DisplayType::Block)
                .item_type(UIItemType::Modal(MODAL_CHROME_ROW)),
        );

        // Section tab row
        let section = *self.section.borrow();
        let mut tab_text = String::new();
        for (idx, s) in Section::ALL.iter().enumerate() {
            if idx > 0 {
                tab_text.push_str("  |  ");
            }
            tab_text.push_str(&tr(s.title()));
        }
        let _ = section;
        rows.push(
            Element::new(&font, ElementContent::Text(tab_text))
                .colors(ElementColors {
                    border: BorderColor::default(),
                    bg: LinearRgba::TRANSPARENT.into(),
                    text: fg.clone(),
                })
                .padding(BoxDimension {
                    left: Dimension::Cells(0.5),
                    right: Dimension::Cells(0.5),
                    top: Dimension::Cells(0.),
                    bottom: Dimension::Cells(0.1),
                })
                .display(DisplayType::Block)
                .item_type(UIItemType::Modal(MODAL_CHROME_ROW)),
        );

        // Filter input line for the Appearance section
        if *self.section.borrow() == Section::Appearance {
            let filter = self.filter.borrow().clone();
            rows.push(
                Element::new(&font, ElementContent::Text(format!("> {filter}_")))
                    .colors(ElementColors {
                        border: BorderColor::default(),
                        bg: LinearRgba::TRANSPARENT.into(),
                        text: fg.clone(),
                    })
                    .padding(BoxDimension {
                        left: Dimension::Cells(0.5),
                        right: Dimension::Cells(0.5),
                        top: Dimension::Cells(0.),
                        bottom: Dimension::Cells(0.1),
                    })
                    .display(DisplayType::Block)
                    .item_type(UIItemType::Modal(MODAL_CHROME_ROW)),
            );
        }

        if items.is_empty() {
            rows.push(
                Element::new(&font, ElementContent::Text(tr("(no matches)").into_owned()))
                    .colors(ElementColors {
                        border: BorderColor::default(),
                        bg: LinearRgba::TRANSPARENT.into(),
                        text: fg.clone(),
                    })
                    .padding(BoxDimension {
                        left: Dimension::Cells(0.5),
                        right: Dimension::Cells(0.5),
                        top: Dimension::Cells(0.),
                        bottom: Dimension::Cells(0.),
                    })
                    .display(DisplayType::Block)
                    .item_type(UIItemType::Modal(MODAL_CHROME_ROW)),
            );
        }

        for (display_idx, item) in items.iter().enumerate().skip(top_row).take(max_rows) {
            let label = self.row_label(item, term_window);
            let is_selected = display_idx == selected;
            let (row_bg, row_fg) = if is_selected {
                (fg.clone(), bg.clone())
            } else {
                (LinearRgba::TRANSPARENT.into(), fg.clone())
            };
            rows.push(
                Element::new(&font, ElementContent::Text(label))
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
                    .item_type(UIItemType::Modal(display_idx)),
            );
        }

        // Footer hints
        rows.push(
            Element::new(
                &font,
                ElementContent::Text(
                    tr("↑↓ select  Tab section  Enter apply  Esc cancel").into_owned(),
                ),
            )
            .colors(ElementColors {
                border: BorderColor::default(),
                bg: LinearRgba::TRANSPARENT.into(),
                text: fg.clone(),
            })
            .padding(BoxDimension {
                left: Dimension::Cells(0.5),
                right: Dimension::Cells(0.5),
                top: Dimension::Cells(0.1),
                bottom: Dimension::Cells(0.1),
            })
            .display(DisplayType::Block)
            .item_type(UIItemType::Modal(MODAL_CHROME_ROW)),
        );

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

        let (padding_left, padding_top) = term_window.padding_left_top();
        let border = term_window.get_os_border();
        let top_bar_height = if term_window.show_tab_bar && !term_window.config.tab_bar_at_bottom {
            term_window.tab_bar_pixel_height().unwrap()
        } else {
            0.
        };

        let desired_width_cells = ((term_window.terminal_size.cols as f32 * 0.6) as usize)
            .max(80)
            .min(term_window.terminal_size.cols);
        let width = desired_width_cells as f32 * term_window.render_metrics.cell_size.width as f32;
        // fork: give the layout the full terminal height (like the command
        // palette) instead of a fixed row budget — a short bounds height
        // made the box model clip the overlay's top rows
        let height = term_window.terminal_size.rows as f32
            * term_window.render_metrics.cell_size.height as f32;

        let x = padding_left
            + ((term_window.dimensions.pixel_width as f32 - padding_left * 2.) - width).max(0.)
                / 2.;
        let y = top_bar_height + padding_top + border.top.get() as f32;

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
                bounds: euclid::rect(x, y, width, height),
                metrics: &metrics,
                gl_state: term_window.render_state.as_ref().unwrap(),
                zindex: 100,
            },
            &element,
        )?;

        Ok(vec![computed])
    }
}

/// Read a bool from the effective (override-aware) config
fn effective_bool(tw: &TermWindow, key: &str) -> Option<bool> {
    match key {
        "mouse_right_click_menu" => Some(tw.config.mouse_right_click_menu),
        "enable_scroll_bar" => Some(tw.config.enable_scroll_bar),
        _ => None,
    }
}

fn default_bool(key: &str) -> bool {
    match key {
        "mouse_right_click_menu" => true,
        "enable_scroll_bar" => false,
        _ => false,
    }
}

/// Enum cycler: each activate moves to the other value of the 2-value set
fn next_enum_value(tw: &TermWindow, key: &str) -> Value {
    match key {
        "audible_bell" => {
            let next = if matches!(tw.config.audible_bell, AudibleBell::SystemBeep) {
                AudibleBell::Disabled
            } else {
                AudibleBell::SystemBeep
            };
            next.to_dynamic()
        }
        "window_close_confirmation" => {
            let next = if matches!(
                tw.config.window_close_confirmation,
                WindowCloseConfirmation::AlwaysPrompt
            ) {
                WindowCloseConfirmation::NeverPrompt
            } else {
                WindowCloseConfirmation::AlwaysPrompt
            };
            next.to_dynamic()
        }
        _ => unreachable!("unknown enum settings key"),
    }
}

fn enum_display(tw: &TermWindow, key: &str) -> std::borrow::Cow<'static, str> {
    match key {
        "audible_bell" => match tw.config.audible_bell {
            AudibleBell::SystemBeep => tr("System beep"),
            AudibleBell::Disabled => tr("Disabled"),
        },
        "window_close_confirmation" => match tw.config.window_close_confirmation {
            WindowCloseConfirmation::AlwaysPrompt => tr("Always prompt"),
            WindowCloseConfirmation::NeverPrompt => tr("Never prompt"),
        },
        _ => unreachable!("unknown enum settings key"),
    }
}

impl Modal for SettingsOverlay {
    fn perform_assignment(
        &self,
        _assignment: &KeyAssignment,
        _term_window: &mut TermWindow,
    ) -> bool {
        false
    }

    fn mouse_event(
        &self,
        event: ::window::MouseEvent,
        row: usize,
        term_window: &mut TermWindow,
    ) -> anyhow::Result<()> {
        use ::window::MouseEventKind as WMEK;
        // chrome rows (title/footer/filter) swallow the event; an
        // out-of-range row must never move the selection
        if row == MODAL_CHROME_ROW {
            return Ok(());
        }
        match event.kind {
            WMEK::Move => {
                if row < self.visible_items(term_window).len() && *self.selected.borrow() != row {
                    self.selected.replace(row);
                    let item = self.visible_items(term_window).get(row).cloned();
                    if let Some(Item::Scheme(name)) = item {
                        self.preview_scheme(term_window, &name);
                    }
                    term_window.invalidate_modal();
                }
            }
            WMEK::Press(::window::MousePress::Left) => {
                self.selected.replace(row);
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
            (KeyCode::Escape, KeyModifiers::NONE) | (KeyCode::Char('g'), KeyModifiers::CTRL) => {
                // cancel: restore any volatile previews
                let snapshot = self.overrides_snapshot.borrow().clone();
                restore_overrides(term_window, &snapshot);
                term_window.cancel_modal();
            }
            (KeyCode::Tab, KeyModifiers::NONE) => {
                self.switch_section(1);
            }
            (KeyCode::Tab, KeyModifiers::SHIFT) => {
                self.switch_section(-1);
            }
            (KeyCode::UpArrow, KeyModifiers::NONE)
            | (KeyCode::Char('p'), KeyModifiers::CTRL)
            | (KeyCode::Char('k'), KeyModifiers::NONE) => {
                self.move_selection(-1, term_window);
                // preview while moving through the scheme list
                let row = *self.selected.borrow();
                let item = self.visible_items(term_window).get(row).cloned();
                if let Some(Item::Scheme(name)) = item {
                    self.preview_scheme(term_window, &name);
                }
            }
            (KeyCode::DownArrow, KeyModifiers::NONE)
            | (KeyCode::Char('n'), KeyModifiers::CTRL)
            | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                self.move_selection(1, term_window);
                let row = *self.selected.borrow();
                let item = self.visible_items(term_window).get(row).cloned();
                if let Some(Item::Scheme(name)) = item {
                    self.preview_scheme(term_window, &name);
                }
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                let row = *self.selected.borrow();
                self.activate(row, term_window);
                return Ok(true);
            }
            (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                if *self.section.borrow() == Section::Appearance {
                    let mut filter = self.filter.borrow_mut();
                    filter.push(c);
                    drop(filter);
                    self.selected.replace(0);
                    self.top_row.replace(0);
                }
            }
            (KeyCode::Backspace, KeyModifiers::NONE) => {
                if *self.section.borrow() == Section::Appearance {
                    let mut filter = self.filter.borrow_mut();
                    filter.pop();
                }
            }
            (KeyCode::Char('u'), KeyModifiers::CTRL) => {
                if *self.section.borrow() == Section::Appearance {
                    self.filter.borrow_mut().clear();
                    self.selected.replace(0);
                    self.top_row.replace(0);
                }
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
            let element = self.compute(term_window)?;
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

/// Convenience: open the settings overlay modally
pub fn open_settings(term_window: &TermWindow) {
    term_window.set_modal(Rc::new(SettingsOverlay::new(term_window)));
    if let Some(window) = term_window.window.as_ref() {
        window.invalidate();
    }
}
