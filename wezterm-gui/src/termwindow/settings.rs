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

/// 数据行可视区的下限：窗口再矮也留这么多行
const MIN_VISIBLE_ROWS: usize = 4;
/// 浮层占窗口高度的比例（千分之），以及标题/分区/过滤/页脚等 chrome 行数
const VISIBLE_ROWS_HEIGHT_PERMILLE: usize = 600;
const CHROME_ROWS: usize = 4;

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
    /// 上一次 `compute()` 实际渲染的数据行数上限。滚动窗口必须和渲染用
    /// 同一个数，否则选中行会滑出可视区（WZ-08）——`compute()` 用命令
    /// 面板字体的度量，早先的 `move_selection` 却用终端字体重算一遍。
    visible_rows: RefCell<usize>,
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
            visible_rows: RefCell::new(MIN_VISIBLE_ROWS),
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

    /// 数据行可视区的行数，按传入度量（渲染实际使用的那份）计算
    fn max_rows_on_screen(
        &self,
        term_window: &TermWindow,
        metrics: &crate::utilsprites::RenderMetrics,
    ) -> usize {
        let cell_height = (metrics.cell_size.height as usize).max(1);
        ((term_window.dimensions.pixel_height * VISIBLE_ROWS_HEIGHT_PERMILLE / 1000) / cell_height)
            .saturating_sub(CHROME_ROWS)
            .max(MIN_VISIBLE_ROWS)
    }

    fn move_selection(&self, delta: isize, term_window: &TermWindow) {
        let items = self.visible_items(term_window);
        let len = items.len();
        if len == 0 {
            return;
        }
        let mut selected = self.selected.borrow_mut();
        *selected = (*selected as isize + delta).rem_euclid(len as isize) as usize;

        // keep the selection inside the scroll window: 用 `compute()` 缓存
        // 下来的行预算，和渲染出的行数严格一致（WZ-08）
        let max_rows = (*self.visible_rows.borrow()).max(1);
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

    /// 当前分区是否带过滤输入框。带的时候可打印字符一律进过滤框，
    /// 导航只留 ↑↓ 与 Ctrl+p/n（WZ-07：否则 `j`/`k` 永远搜不出
    /// `jellybeans`/`kanagawa`，而过滤是 1001 条配色唯一可用入口）。
    fn filter_is_active(&self) -> bool {
        *self.section.borrow() == Section::Appearance
    }

    /// 过滤文本变化后重置选中行与滚动位置
    fn reset_scroll(&self) {
        self.selected.replace(0);
        self.top_row.replace(0);
    }

    /// 移动选中行，并在配色列表里顺带预览
    fn move_and_preview(&self, delta: isize, term_window: &mut TermWindow) {
        self.move_selection(delta, term_window);
        let row = *self.selected.borrow();
        let item = self.visible_items(term_window).get(row).cloned();
        if let Some(Item::Scheme(name)) = item {
            self.preview_scheme(term_window, &name);
        }
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
        self.visible_rows.replace(max_rows);
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
                .min_width(Some(Dimension::Percent(1.)))
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
                .min_width(Some(Dimension::Percent(1.)))
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
                    .min_width(Some(Dimension::Percent(1.)))
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
                    .min_width(Some(Dimension::Percent(1.)))
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
            .min_width(Some(Dimension::Percent(1.)))
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
            .display(DisplayType::Block)
            // 外框自己也进 hit map：内边距/边框/外边距那一圈不属于任何行，
            // 点在那里会被「点浮层外即关闭」误判成点外面（WZ-06）
            .item_type(UIItemType::Modal(MODAL_CHROME_ROW));

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

/// 设置页对一次按键的处理意图。把按键路由抽成纯函数，`filter_active`
/// 这条分支（WZ-07）才可单测。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsKey {
    Cancel,
    /// 切换分区，`delta` 为 +1 / -1
    SwitchSection(isize),
    /// 移动选中行
    Move(isize),
    Activate,
    FilterPush(char),
    FilterPop,
    FilterClear,
    /// 浮层吞掉但不做事
    Swallow,
    /// 交回上层（键位绑定 / pane）
    PassThrough,
}

/// 按键路由的唯一真源。
///
/// `filter_active` 为真（外观分区有过滤输入框）时，裸 `j`/`k` 属于过滤
/// 输入而不是导航——否则 1001 条配色里永远搜不出 `jellybeans`/`kanagawa`
/// （WZ-07）；↑↓ 与 Ctrl+p/n 在任何分区都是导航。
fn classify_key(key: KeyCode, mods: KeyModifiers, filter_active: bool) -> SettingsKey {
    match (key, mods) {
        (KeyCode::Escape, KeyModifiers::NONE) | (KeyCode::Char('g'), KeyModifiers::CTRL) => {
            SettingsKey::Cancel
        }
        (KeyCode::Tab, KeyModifiers::NONE) => SettingsKey::SwitchSection(1),
        (KeyCode::Tab, KeyModifiers::SHIFT) => SettingsKey::SwitchSection(-1),
        (KeyCode::UpArrow, KeyModifiers::NONE) | (KeyCode::Char('p'), KeyModifiers::CTRL) => {
            SettingsKey::Move(-1)
        }
        (KeyCode::DownArrow, KeyModifiers::NONE) | (KeyCode::Char('n'), KeyModifiers::CTRL) => {
            SettingsKey::Move(1)
        }
        (KeyCode::Char('k'), KeyModifiers::NONE) if !filter_active => SettingsKey::Move(-1),
        (KeyCode::Char('j'), KeyModifiers::NONE) if !filter_active => SettingsKey::Move(1),
        (KeyCode::Enter, KeyModifiers::NONE) => SettingsKey::Activate,
        (KeyCode::Char('u'), KeyModifiers::CTRL) if filter_active => SettingsKey::FilterClear,
        (KeyCode::Char('u'), KeyModifiers::CTRL) => SettingsKey::Swallow,
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            if filter_active {
                SettingsKey::FilterPush(c)
            } else {
                SettingsKey::Swallow
            }
        }
        (KeyCode::Backspace, KeyModifiers::NONE) => {
            if filter_active {
                SettingsKey::FilterPop
            } else {
                SettingsKey::Swallow
            }
        }
        _ => SettingsKey::PassThrough,
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
        match classify_key(key, mods, self.filter_is_active()) {
            SettingsKey::Cancel => {
                // cancel: restore any volatile previews
                let snapshot = self.overrides_snapshot.borrow().clone();
                restore_overrides(term_window, &snapshot);
                term_window.cancel_modal();
            }
            SettingsKey::SwitchSection(delta) => {
                self.switch_section(delta);
            }
            SettingsKey::Move(delta) => {
                // preview while moving through the scheme list
                self.move_and_preview(delta, term_window);
            }
            SettingsKey::Activate => {
                let row = *self.selected.borrow();
                self.activate(row, term_window);
                return Ok(true);
            }
            SettingsKey::FilterPush(c) => {
                self.filter.borrow_mut().push(c);
                self.reset_scroll();
            }
            SettingsKey::FilterPop => {
                self.filter.borrow_mut().pop();
                // 删字符同样换了一组可见行，选中行与滚动位置必须一起回零
                self.reset_scroll();
            }
            SettingsKey::FilterClear => {
                self.filter.borrow_mut().clear();
                self.reset_scroll();
            }
            // 没有过滤框的分区里，可打印字符与 Backspace 照旧被浮层吞掉，
            // 不落进 pane
            SettingsKey::Swallow => {}
            SettingsKey::PassThrough => return Ok(false),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn nav(key: KeyCode, filter_active: bool) -> SettingsKey {
        classify_key(key, KeyModifiers::NONE, filter_active)
    }

    #[test]
    fn lowercase_jk_types_into_the_appearance_filter() {
        // 过滤框是 1001 条配色唯一可用入口，j/k 必须能打进去（WZ-07）
        assert_eq!(nav(KeyCode::Char('j'), true), SettingsKey::FilterPush('j'));
        assert_eq!(nav(KeyCode::Char('k'), true), SettingsKey::FilterPush('k'));
        for c in "jellybeans".chars() {
            assert_eq!(nav(KeyCode::Char(c), true), SettingsKey::FilterPush(c));
        }
        for c in "kanagawa".chars() {
            assert_eq!(nav(KeyCode::Char(c), true), SettingsKey::FilterPush(c));
        }
    }

    #[test]
    fn lowercase_jk_still_navigates_without_a_filter() {
        assert_eq!(nav(KeyCode::Char('j'), false), SettingsKey::Move(1));
        assert_eq!(nav(KeyCode::Char('k'), false), SettingsKey::Move(-1));
    }

    #[test]
    fn arrows_and_ctrl_pn_navigate_in_every_section() {
        for filter_active in [false, true] {
            assert_eq!(nav(KeyCode::UpArrow, filter_active), SettingsKey::Move(-1));
            assert_eq!(nav(KeyCode::DownArrow, filter_active), SettingsKey::Move(1));
            assert_eq!(
                classify_key(KeyCode::Char('p'), KeyModifiers::CTRL, filter_active),
                SettingsKey::Move(-1)
            );
            assert_eq!(
                classify_key(KeyCode::Char('n'), KeyModifiers::CTRL, filter_active),
                SettingsKey::Move(1)
            );
        }
    }

    #[test]
    fn backspace_edits_the_filter_and_is_swallowed_elsewhere() {
        assert_eq!(nav(KeyCode::Backspace, true), SettingsKey::FilterPop);
        assert_eq!(nav(KeyCode::Backspace, false), SettingsKey::Swallow);
    }

    #[test]
    fn plain_characters_never_reach_the_pane() {
        // 无过滤框的分区吞掉字符，不交回键位绑定 / pane
        assert_eq!(nav(KeyCode::Char('z'), false), SettingsKey::Swallow);
        assert_eq!(
            classify_key(KeyCode::Char('Z'), KeyModifiers::SHIFT, false),
            SettingsKey::Swallow
        );
        assert_eq!(
            classify_key(KeyCode::Char('u'), KeyModifiers::CTRL, false),
            SettingsKey::Swallow
        );
        assert_eq!(
            classify_key(KeyCode::Char('u'), KeyModifiers::CTRL, true),
            SettingsKey::FilterClear
        );
    }

    #[test]
    fn section_switch_and_cancel_keep_working() {
        assert_eq!(nav(KeyCode::Tab, true), SettingsKey::SwitchSection(1));
        assert_eq!(
            classify_key(KeyCode::Tab, KeyModifiers::SHIFT, true),
            SettingsKey::SwitchSection(-1)
        );
        assert_eq!(nav(KeyCode::Escape, true), SettingsKey::Cancel);
        assert_eq!(
            classify_key(KeyCode::Char('g'), KeyModifiers::CTRL, true),
            SettingsKey::Cancel
        );
        assert_eq!(nav(KeyCode::Enter, true), SettingsKey::Activate);
    }

    #[test]
    fn unhandled_keys_are_passed_through() {
        assert_eq!(nav(KeyCode::Home, true), SettingsKey::PassThrough);
        assert_eq!(
            classify_key(KeyCode::Char('q'), KeyModifiers::ALT, false),
            SettingsKey::PassThrough
        );
    }
}
