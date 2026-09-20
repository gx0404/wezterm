//! fork 新增：herdr 式设置浮层（Modal）。
//!
//! 四个分区：语言（中文/English）、外观（内置配色方案，移动即预览、
//! Esc 还原、Enter 应用）、交互（右键菜单/滚动条/响铃/关闭确认）、
//! 字体（字号步进与重置）。
//! 生效链路：预览走 `TermWindow::set_preview_palette`（窗口级临时调色板，
//! 只丢渲染缓存 + 重绘，不重跑 Lua、不重建字体、不改窗口尺寸）；应用则
//! 写入 `config::gui_settings::store_key`（gui-settings.json，原子写）后
//! `config::reload()`——全局生效且跨重启持久化，不触碰用户 Lua。
//! 浮层关闭（Esc / 点外 / 被另一浮层顶掉）统一经 `Modal::on_dismissed`
//! 还原预览，配色不会钉死在随手划过的那套（WZ-02 / WZ-03）。
//! 不变量：本浮层**不写** `TermWindow::config_overrides`——那是每窗口、
//! 优先级高于全局配置且 `ReloadConfiguration` 清不掉的状态，一旦写入就会
//! 把窗口钉死在设置页点过的值上。
//! 不变量：「当前值」一律读 `current_config()`（全局 handle，`config::reload()`
//! 同步换掉），不读 `TermWindow::config`——后者靠 SPAWN_QUEUE 异步回推，
//! 长按确认时会连续读到同一份陈旧值（见 `current_config` 的说明）。
//! 渲染复用命令面板的字体/配色与 box model；交互行经
//! `UIItemType::Modal(row)` 进 hit map，与右键菜单共用鼠标通道。

use crate::termwindow::box_model::*;
use crate::termwindow::modal::{Modal, MODAL_CHROME_ROW, MODAL_SECTION_BASE, MODAL_SECTION_MAX};
use crate::termwindow::{DimensionContext, TermWindow, UIItemType};
use config::i18n::{tr, UiLanguage};
use config::keyassignment::KeyAssignment;
use config::{AudibleBell, Config, ConfigHandle, Dimension, Palette, WindowCloseConfirmation};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use wezterm_dynamic::{ToDynamic, Value};
use wezterm_term::color::ColorPalette;
use wezterm_term::{KeyCode, KeyModifiers};
use window::color::LinearRgba;
use window::WindowOps;

/// matches `config::config::default_font_size` (the wezterm default)
const DEFAULT_FONT_SIZE: f64 = 12.0;

/// 数据行可视区的下限：窗口再矮也留这么多行
const MIN_VISIBLE_ROWS: usize = 4;
/// 浮层占窗口高度的比例（千分之）
const VISIBLE_ROWS_HEIGHT_PERMILLE: usize = 600;
/// 固定 chrome 行：标题、分区 tab、页脚。过滤框与「(no matches)」按需另算，
/// 见 `SettingsOverlay::chrome_rows`
const FIXED_CHROME_ROWS: usize = 3;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
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

// WZ-09: the clickable tab band must be wide enough for every section;
// adding a fifth section without widening MODAL_SECTION_MAX fails here.
const _: () = assert!(Section::ALL.len() <= MODAL_SECTION_MAX);

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
    /// 当前正在预览的配色名。预览是窗口级临时状态，关闭浮层时按它判断
    /// 要不要还原（WZ-03），同名重复预览直接短路
    previewed_scheme: RefCell<Option<String>>,
    /// WZ-19：可见行列表缓存。1001 条配色的名字排序 + 模糊过滤原本在每个
    /// 输入事件里被重建 2-3 次（`compute` / `move_selection` /
    /// `mouse_event` 各调一次 `visible_items`）
    items_cache: RefCell<Option<ItemsCache>>,
    /// 上一次 `compute()` 实际渲染的数据行数上限。滚动窗口必须和渲染用
    /// 同一个数，否则选中行会滑出可视区（WZ-08）——`compute()` 用命令
    /// 面板字体的度量，早先的 `move_selection` 却用终端字体重算一遍。
    visible_rows: RefCell<usize>,
    element: RefCell<Option<Vec<ComputedElement>>>,
}

/// 可见行列表的缓存条目：分区 + 过滤文本一致即可复用（WZ-19）
struct ItemsCache {
    section: Section,
    filter: String,
    items: Rc<Vec<Item>>,
}

/// Persist one settings key and reload the configuration so the change
/// applies globally (all windows) and survives restarts.
fn persist_and_reload(key: &str, value: &Value) -> anyhow::Result<()> {
    config::gui_settings::store_key(key, value)?;
    config::reload();
    Ok(())
}

/// 设置页读「当前值」的唯一来源：刚落地的全局配置。
///
/// 不能读 `TermWindow::config`。落地走 `config::reload()`，它在锁内同步换掉
/// 全局 CONFIG，但推给窗口的 `config_was_reloaded` 是经 `Window::notify` →
/// SPAWN_QUEUE 异步投递的，而 X11 主循环先把排队的 X 事件一次排干才轮到
/// SPAWN_QUEUE。于是 `Config::load()` 阻塞的那几十毫秒里堆积的按键重复事件
/// 会连续读到同一份陈旧窗口配置：长按 Enter 步进字号只动一格、连点两下开关
/// 不回弹。读全局 handle 才永远是上一次确认刚写进去的值。
fn current_config() -> ConfigHandle {
    config::configuration()
}

impl SettingsOverlay {
    pub fn new() -> Self {
        Self {
            section: RefCell::new(Section::Language),
            selected: RefCell::new(0),
            top_row: RefCell::new(0),
            filter: RefCell::new(String::new()),
            previewed_scheme: RefCell::new(None),
            items_cache: RefCell::new(None),
            visible_rows: RefCell::new(MIN_VISIBLE_ROWS),
            element: RefCell::new(None),
        }
    }

    fn items_for(&self, section: Section) -> Vec<Item> {
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
    /// Appearance are unfiltered)。
    ///
    /// 结果按「分区 + 过滤文本」缓存：这个函数在一次按键里会被调用多次，
    /// 而外观分区每次都要排序 1001 个名字再跑一遍模糊匹配（WZ-19）。
    fn visible_items(&self) -> Rc<Vec<Item>> {
        let section = *self.section.borrow();
        {
            let cache = self.items_cache.borrow();
            if let Some(cache) = cache.as_ref() {
                if cache.section == section && cache.filter.as_str() == *self.filter.borrow() {
                    return Rc::clone(&cache.items);
                }
            }
        }
        let items = Rc::new(self.items_for(section));
        self.items_cache.replace(Some(ItemsCache {
            section,
            filter: self.filter.borrow().clone(),
            items: Rc::clone(&items),
        }));
        items
    }

    /// `compute()` 实际 push 的 chrome 行数：标题 + 分区 tab + 页脚固定三行，
    /// 外观分区多一行过滤框，列表为空时再多一行「(no matches)」。与
    /// `compute()` 共用同一判据，两处口径不会分叉。
    fn chrome_rows(&self, items_len: usize) -> usize {
        FIXED_CHROME_ROWS + usize::from(self.filter_is_active()) + usize::from(items_len == 0)
    }

    /// 数据行可视区的行数，按传入度量（渲染实际使用的那份）计算
    fn max_rows_on_screen(
        &self,
        term_window: &TermWindow,
        metrics: &crate::utilsprites::RenderMetrics,
        items_len: usize,
    ) -> usize {
        let cell_height = (metrics.cell_size.height as usize).max(1);
        ((term_window.dimensions.pixel_height * VISIBLE_ROWS_HEIGHT_PERMILLE / 1000) / cell_height)
            .saturating_sub(self.chrome_rows(items_len))
            .max(MIN_VISIBLE_ROWS)
    }

    fn move_selection(&self, delta: isize) {
        let items = self.visible_items();
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

    /// 换分区：丢掉本分区的全部易失状态后切过去。
    fn switch_section(&self, delta: isize, term_window: &mut TermWindow) {
        if self.select_section(delta) {
            term_window.set_preview_palette(None);
        }
        // WZ-18: land on the row holding the effective value
        self.locate_current();
    }

    /// 点击分区 tab（WZ-09）：绝对索引版本，与 `switch_section` 共用
    /// 同一段状态收敛。
    fn click_section(&self, target: usize, term_window: &mut TermWindow) {
        if self.select_section_index(target) {
            term_window.set_preview_palette(None);
        }
        self.locate_current();
    }

    /// WZ-18：把选中行定位到当前生效值所在行（语言/配色分区；其它
    /// 分区行数少，定位到首行），并收敛滚动窗口让它可见。
    fn locate_current(&self) {
        let items = self.visible_items();
        let config = current_config();
        let idx = locate_current_in(&items, &config);
        self.selected.replace(idx);
        // 复用滚动窗口收敛逻辑把当前值纳入可视区
        self.move_selection(0);
        // WZ-18：行预算缓存可能来自上一分区（chrome 行数不同，见
        // `visible_rows` 的已知取舍注释），贴末行的当前值会落在下一帧
        // 的渲染窗口之外；往上让一行保它在窗内。
        let max_rows = (*self.visible_rows.borrow()).max(1);
        let budget = max_rows.saturating_sub(1).max(1);
        let mut top_row = self.top_row.borrow_mut();
        if idx > 0 && idx >= *top_row + budget {
            *top_row = idx + 1 - budget;
        }
    }

    /// 换分区 = 丢掉本分区的全部易失状态：过滤文本、选中行、滚动位置，
    /// 以及外观分区可能还挂着的配色预览。预览行在新分区已经不可见，留着
    /// 就会出现「窗口是预览色、界面上却没有任何一行对应它」的脱节，而且在
    /// 新分区按 Enter 时会先闪回原配色再应用（`activate` 里的还原）。
    ///
    /// 返回 true 表示窗口的预览调色板还需要还原。还原要 `TermWindow`
    /// （单测里造不出来），拆出去这条口径才测得到。
    fn select_section(&self, delta: isize) -> bool {
        let all = Section::ALL;
        let idx = all
            .iter()
            .position(|s| *s == *self.section.borrow())
            .unwrap_or(0);
        let next = (idx as isize + delta).rem_euclid(all.len() as isize) as usize;
        self.select_section_index(next)
    }

    /// WZ-09：按绝对索引切分区；点当前分区不重置浏览状态，只兜底丢预览。
    fn select_section_index(&self, target: usize) -> bool {
        if target >= Section::ALL.len() {
            return false;
        }
        if target
            == Section::ALL
                .iter()
                .position(|s| *s == *self.section.borrow())
                .unwrap_or(0)
        {
            return self.take_preview();
        }
        let had_preview = self.take_preview();
        self.section.replace(Section::ALL[target]);
        self.selected.replace(0);
        self.top_row.replace(0);
        // the filter only applies to Appearance
        self.filter.borrow_mut().clear();
        had_preview
    }

    /// 预览一套内置配色（WZ-02）。
    ///
    /// 只替换窗口级预览调色板并重绘，不写 `config_overrides`：旧实现每经过
    /// 一行就走一次 `config_was_reloaded`，等于整份 Lua 重载、全部字体重建
    /// 再加 `apply_dimensions`，后者沿 PTY 把 SIGWINCH 打进 pane 内的程序，
    /// 1001 条配色里长按 ↓ 直接卡死。
    fn preview_scheme(&self, term_window: &mut TermWindow, name: &str) {
        if !self.preview_is_stale(name) {
            return;
        }
        let Some(palette) = preview_palette_for_scheme(
            name,
            &term_window.config.color_schemes,
            term_window.config.colors.as_ref(),
        ) else {
            return;
        };
        self.previewed_scheme.replace(Some(name.to_string()));
        term_window.set_preview_palette(Some(palette));
    }

    /// 当前预览是否已经就是 `name`。鼠标 Move 在同一行上会连发很多次，
    /// 同名重复预览必须在这里短路，否则每次都要克隆一整份调色板并 bump
    /// 两个失效代数。
    fn preview_is_stale(&self, name: &str) -> bool {
        self.previewed_scheme.borrow().as_deref() != Some(name)
    }

    /// 取走预览标记。返回 true 表示确实有预览在生效、窗口调色板需要还原。
    /// 与 `clear_preview` 拆开是为了让这段状态机能脱离 `TermWindow`
    /// （需要 GPU 与窗口句柄，单测里造不出来）被直接测到。
    fn take_preview(&self) -> bool {
        self.previewed_scheme.borrow_mut().take().is_some()
    }

    /// 丢掉预览调色板。Esc / 点浮层外 / 被另一浮层顶掉三条路径统一走
    /// 这里（WZ-03）。
    fn clear_preview(&self, term_window: &mut TermWindow) {
        if self.take_preview() {
            term_window.set_preview_palette(None);
        }
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
        self.move_selection(delta);
        let row = *self.selected.borrow();
        let item = self.visible_items().get(row).cloned();
        if let Some(Item::Scheme(name)) = item {
            self.preview_scheme(term_window, &name);
        }
    }

    /// 应用选中行。
    ///
    /// 落地统一只走一次 `persist_and_reload`：写 gui-settings.json 后
    /// `config::reload()` 会把新配置推回每个窗口（`config_was_reloaded`）。
    /// 旧实现先 `upsert_override` 再 persist，等于连做两次全量重载，而且
    /// 那份每窗口 override 优先级高于全局配置、`ReloadConfiguration` 也清
    /// 不掉，等于把窗口钉在设置页点过的值上（WZ-03 / WZ-20）。
    ///
    /// 下一个值只由 `pending_write` 从 `current_config()` 推出，不经
    /// `TermWindow`——窗口那份 handle 要等异步回推才刷新（见 `current_config`）。
    fn activate(&self, row: usize, term_window: &mut TermWindow) {
        let items = self.visible_items();
        let Some(item) = items.get(row).cloned() else {
            return;
        };
        // 预览是窗口级临时状态，落地前一律丢掉，让持久化后的配置成为唯一真源
        self.clear_preview(term_window);
        let (key, value) = pending_write(&item, &current_config());
        if let Err(err) = persist_and_reload(key, &value) {
            log::error!("settings: failed to apply: {err:#}");
        }
        // 确认后立刻重算浮层：行标签同样读全局配置，勾选标记与「字号: 12.5」
        // 因此不必等异步 `config_was_reloaded` 回推才刷新
        term_window.invalidate_modal();
    }

    /// 行标签里的当前值与 `activate` 推导下一个值读同一份配置
    /// （`current_config()`），显示与落地因此不会各说各话。
    fn row_label(&self, item: &Item, config: &Config) -> String {
        match item {
            Item::LanguageChoice(lang, label) => {
                let current = config.language;
                let marker = if *lang == current { "✓ " } else { "  " };
                format!("{marker}{label}")
            }
            Item::Scheme(name) => {
                let current = config.color_scheme.as_deref();
                let marker = if current == Some(name.as_str()) {
                    "✓ "
                } else {
                    "  "
                };
                format!("{marker}{name}")
            }
            Item::BoolToggle { label, key } => {
                let value = effective_bool(config, key).unwrap_or_else(|| default_bool(key));
                let value = if value { tr("On") } else { tr("Off") };
                format!("  {}: {value}", tr(label))
            }
            Item::EnumChoice { label, key } => {
                let value = enum_display(config, key);
                format!("  {}: {value}", tr(label))
            }
            Item::FontOp(op) => {
                let label = match op {
                    FontOp::Decrease => "Decrease font size",
                    FontOp::Increase => "Increase font size",
                    FontOp::Reset => "Reset font size",
                };
                let size = config.font_size;
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

        let items = self.visible_items();
        // 行里显示的设置值读刚落地的全局配置，与 `activate` 推导下一个值
        // 同源（见 `current_config`）；字体与浮层配色仍用窗口那份
        let settings_config = current_config();
        // 已知取舍：行预算是 paint 派生缓存——`compute()` 在绘制期算出、
        // 键盘处理下一轮才读到。窗口 resize 后若按键先于重绘到达，
        // `move_selection` 用的还是上一帧的预算，滚动位置会跳一下并在下一
        // 帧自愈；首帧之前则按 `MIN_VISIBLE_ROWS` 算。换成现算需要在按键
        // 路径上重取命令面板字体度量，代价大于这一帧的滞后。
        let max_rows = self.max_rows_on_screen(term_window, &metrics, items.len());
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

        // Section tab row (WZ-09): one child element per section so each
        // tab is individually addressable (click targets via the
        // MODAL_SECTION_BASE sentinel band) and the active section is
        // rendered with the same inverted colors as a selected data row.
        let section = *self.section.borrow();
        let mut tab_children = vec![];
        for (idx, s) in Section::ALL.iter().enumerate() {
            if idx > 0 {
                tab_children.push(
                    Element::new(&font, ElementContent::Text("  |  ".to_string())).colors(
                        ElementColors {
                            border: BorderColor::default(),
                            bg: LinearRgba::TRANSPARENT.into(),
                            text: fg.clone(),
                        },
                    ),
                );
            }
            let active = *s == section;
            tab_children.push(
                Element::new(&font, ElementContent::Text(tr(s.title()).into_owned()))
                    .colors(ElementColors {
                        border: BorderColor::default(),
                        bg: if active {
                            fg.clone()
                        } else {
                            LinearRgba::TRANSPARENT.into()
                        },
                        text: if active { bg.clone() } else { fg.clone() },
                    })
                    .padding(BoxDimension {
                        left: Dimension::Cells(0.5),
                        right: Dimension::Cells(0.5),
                        top: Dimension::Cells(0.),
                        bottom: Dimension::Cells(0.),
                    })
                    .item_type(UIItemType::Modal(MODAL_SECTION_BASE + idx)),
            );
        }
        rows.push(
            Element::new(&font, ElementContent::Children(tab_children))
                .colors(ElementColors {
                    border: BorderColor::default(),
                    bg: LinearRgba::TRANSPARENT.into(),
                    text: fg.clone(),
                })
                .padding(BoxDimension {
                    left: Dimension::Cells(0.),
                    right: Dimension::Cells(0.5),
                    top: Dimension::Cells(0.),
                    bottom: Dimension::Cells(0.1),
                })
                .min_width(Some(Dimension::Percent(1.)))
                .display(DisplayType::Block)
                .item_type(UIItemType::Modal(MODAL_CHROME_ROW)),
        );

        // Filter input line for the Appearance section；判据与
        // `classify_key` 的按键路由共用 `filter_is_active()`，画出来的过滤框
        // 与「可打印字符进过滤框」永远同时成立（批 4 审查 minor）
        if self.filter_is_active() {
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
            let label = self.row_label(item, &settings_config);
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

/// 按名取一套配色并叠加用户 `colors = {…}` 覆盖——复刻
/// `Config::resolve_color_scheme` 的查找次序（先 `config.color_schemes`，
/// 即 Lua 里定义的与配色目录里加载的，再回退内置表）与
/// `compute_extra_defaults` 里 `resolved_palette` 的推导顺序（先 scheme，
/// 再 colors 覆盖）。两处一致，预览色才等于确认后真正落地的色。
///
/// 全程只读内存里的表：`config::COLOR_SCHEMES` 是进程内 lazy_static，
/// 1001 套配色只解析一次。不重跑 Lua、不重建字体、不动窗口尺寸。名字两张
/// 表里都没有时返回 `None`，调用方保持当前配色不变。
fn preview_palette_for_scheme(
    name: &str,
    user_schemes: &HashMap<String, Palette>,
    colors: Option<&Palette>,
) -> Option<ColorPalette> {
    let mut palette = user_schemes
        .get(name)
        .or_else(|| config::COLOR_SCHEMES.get(name))
        .cloned()?;
    if let Some(colors) = colors {
        palette = palette.overlay_with(colors);
    }
    Some(palette.into())
}

/// 一行「确认」要写进 gui-settings.json 的键值。
///
/// 只依赖传入的 `config`（`current_config()` 取来的、刚落地的那份），签名里
/// 没有 `TermWindow`——窗口那份 ConfigHandle 因此不可能再被误读成当前值，
/// 连续两次确认必然从上一次刚写下的值继续推（WZ-20 的回归护栏）。
/// WZ-18：在可见行里找「当前生效值」的行号；找不到（被过滤/无对应
/// 值的分区）回 0。纯函数，不依赖 TermWindow。
fn locate_current_in(items: &[Item], config: &Config) -> usize {
    for (idx, item) in items.iter().enumerate() {
        let is_current = match item {
            Item::LanguageChoice(lang, _) => *lang == config.language,
            Item::Scheme(name) => config.color_scheme.as_deref() == Some(name.as_str()),
            _ => false,
        };
        if is_current {
            return idx;
        }
    }
    0
}

fn pending_write(item: &Item, config: &Config) -> (&'static str, Value) {
    match item {
        Item::LanguageChoice(lang, _) => ("language", lang.to_dynamic()),
        Item::Scheme(name) => ("color_scheme", Value::String(name.clone())),
        Item::BoolToggle { label: _, key } => {
            let current = effective_bool(config, key).unwrap_or_else(|| default_bool(key));
            (*key, Value::Bool(!current))
        }
        Item::EnumChoice { label: _, key } => (*key, next_enum_value(config, key)),
        Item::FontOp(op) => (
            "font_size",
            Value::F64(ordered_float::OrderedFloat(next_font_size(
                config.font_size,
                *op,
            ))),
        ),
    }
}

/// 字号步进：上下各留一道夹限，重置回 wezterm 默认值
fn next_font_size(current: f64, op: FontOp) -> f64 {
    match op {
        FontOp::Decrease => (current - 0.5).max(6.0),
        FontOp::Increase => (current + 0.5).min(100.0),
        FontOp::Reset => DEFAULT_FONT_SIZE,
    }
}

/// Read a bool out of the supplied configuration
fn effective_bool(config: &Config, key: &str) -> Option<bool> {
    match key {
        "mouse_right_click_menu" => Some(config.mouse_right_click_menu),
        "enable_scroll_bar" => Some(config.enable_scroll_bar),
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
fn next_enum_value(config: &Config, key: &str) -> Value {
    match key {
        "audible_bell" => {
            let next = if matches!(config.audible_bell, AudibleBell::SystemBeep) {
                AudibleBell::Disabled
            } else {
                AudibleBell::SystemBeep
            };
            next.to_dynamic()
        }
        "window_close_confirmation" => {
            let next = if matches!(
                config.window_close_confirmation,
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

fn enum_display(config: &Config, key: &str) -> std::borrow::Cow<'static, str> {
    match key {
        "audible_bell" => match config.audible_bell {
            AudibleBell::SystemBeep => tr("System beep"),
            AudibleBell::Disabled => tr("Disabled"),
        },
        "window_close_confirmation" => match config.window_close_confirmation {
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
        // WZ-09: section tabs live in the MODAL_SECTION_BASE sentinel band;
        // a left click switches to that section (same state convergence as
        // the Tab key), hovers and other presses are simply swallowed.
        if (MODAL_SECTION_BASE..MODAL_SECTION_BASE + MODAL_SECTION_MAX).contains(&row) {
            if let WMEK::Press(::window::MousePress::Left) = event.kind {
                self.click_section(row - MODAL_SECTION_BASE, term_window);
                term_window.invalidate_modal();
            }
            return Ok(());
        }
        // chrome rows (title/footer/filter) swallow the event; an
        // out-of-range row must never move the selection
        if row == MODAL_CHROME_ROW {
            return Ok(());
        }
        match event.kind {
            WMEK::Move => {
                let items = self.visible_items();
                if row < items.len() && *self.selected.borrow() != row {
                    self.selected.replace(row);
                    if let Some(Item::Scheme(name)) = items.get(row).cloned() {
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
                // 预览还原交给 `on_dismissed`，Esc / 点外 / 被顶掉三条
                // 路径因此走同一段代码（WZ-03）
                term_window.cancel_modal();
            }
            SettingsKey::SwitchSection(delta) => {
                self.switch_section(delta, term_window);
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

    /// WZ-03：浮层关闭时还原预览。点浮层外、被另一浮层顶掉、Esc 三条
    /// 路径都会到这里，配色不会留在随手划过的那套上。
    fn on_dismissed(&self, term_window: &mut TermWindow) {
        self.clear_preview(term_window);
    }
}

/// Convenience: open the settings overlay modally
pub fn open_settings(term_window: &mut TermWindow) {
    let modal = Rc::new(SettingsOverlay::new());
    // WZ-18: open with the selection on the row holding the effective value
    modal.locate_current();
    term_window.set_modal(modal);
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

    fn builtin(name: &str) -> ColorPalette {
        preview_palette_for_scheme(name, &HashMap::new(), None)
            .unwrap_or_else(|| panic!("{} is a builtin scheme", name))
    }

    #[test]
    fn preview_palette_comes_from_the_builtin_scheme_table() {
        // WZ-02：预览必须能只靠内存里的配色表拿到调色板，不经 Lua 重载
        let batman = builtin("Batman");
        let bamboo = builtin("Bamboo");
        assert_ne!(batman.background, bamboo.background);
        assert_ne!(batman.foreground, bamboo.foreground);
    }

    #[test]
    fn preview_palette_is_none_for_unknown_schemes() {
        assert!(
            preview_palette_for_scheme("no such scheme at all", &HashMap::new(), None).is_none()
        );
    }

    #[test]
    fn preview_palette_keeps_user_colors_on_top_of_the_scheme() {
        // 推导顺序必须与 `resolved_palette` 一致：先 scheme，再 colors 覆盖
        let scheme_only = builtin("Batman");
        let colors = Palette {
            background: Some((0x12, 0x34, 0x56).into()),
            ..Default::default()
        };
        let overlaid = preview_palette_for_scheme("Batman", &HashMap::new(), Some(&colors))
            .expect("Batman is a builtin scheme");
        assert_ne!(scheme_only.background, overlaid.background);
        assert_eq!(overlaid.background, (0x12, 0x34, 0x56).into());
        // 未被 colors 覆盖的字段仍来自 scheme
        assert_eq!(overlaid.foreground, scheme_only.foreground);
    }

    #[test]
    fn user_defined_schemes_shadow_the_builtin_table() {
        // 查找次序必须与 `Config::resolve_color_scheme` 一致：同名时用户
        // 自定义的那套优先，否则预览色与确认后落地的色会对不上
        let mut user_schemes = HashMap::new();
        user_schemes.insert(
            "Batman".to_string(),
            Palette {
                background: Some((0x00, 0xff, 0x00).into()),
                ..Default::default()
            },
        );
        let shadowed = preview_palette_for_scheme("Batman", &user_schemes, None)
            .expect("the user scheme is present");
        assert_eq!(shadowed.background, (0x00, 0xff, 0x00).into());
        assert_ne!(shadowed.background, builtin("Batman").background);
    }

    #[test]
    fn preview_bookkeeping_short_circuits_and_restores_exactly_once() {
        // WZ-03：三条关闭路径都汇到 `take_preview`，它必须只在真有预览在
        // 生效时才要求还原，且只要求一次；WZ-02：同一行上的重复预览短路
        let overlay = SettingsOverlay::new();
        assert!(!overlay.take_preview(), "没预览过就不该要求还原");

        assert!(overlay.preview_is_stale("Batman"));
        overlay.previewed_scheme.replace(Some("Batman".to_string()));
        assert!(!overlay.preview_is_stale("Batman"), "同名重复预览必须短路");
        assert!(overlay.preview_is_stale("Bamboo"));

        assert!(overlay.take_preview(), "有预览在生效就要还原");
        assert!(!overlay.take_preview(), "还原过一次之后不该再还原");
        assert!(overlay.preview_is_stale("Batman"));
    }

    fn f64_value(v: f64) -> Value {
        Value::F64(ordered_float::OrderedFloat(v))
    }

    #[test]
    fn consecutive_activations_step_from_the_freshly_persisted_value() {
        // WZ-20 回归护栏：确认后 `config::reload()` 在锁内同步换掉全局
        // 配置，第二次确认必须从刚落地的值再推一格。旧实现从窗口的
        // ConfigHandle 推导，而那份要等 SPAWN_QUEUE 上的
        // `config_was_reloaded` 才刷新——X11 主循环先排干 X 事件队列，于是
        // 长按 Enter 期间堆积的重复按键全部读到 12.0，字号只动一格
        let mut config = Config::default_config();
        config.font_size = 12.0;
        let item = Item::FontOp(FontOp::Increase);

        let (key, first) = pending_write(&item, &config);
        assert_eq!(key, "font_size");
        assert_eq!(first, f64_value(12.5));

        // `persist_and_reload` 之后全局配置就是这个样子
        config.font_size = 12.5;
        let (_, second) = pending_write(&item, &config);
        assert_eq!(second, f64_value(13.0));
        assert_ne!(first, second);
    }

    #[test]
    fn current_config_tracks_the_globally_reloaded_handle() {
        // `activate` 的「当前值」必须落在 `config::reload()` 在锁内同步
        // 换掉的那份全局 handle 上。先读一次让任何快照式实现在这里定格，
        // 再模拟一次落地——第二次读还是旧值就等于窗口那份要等 SPAWN_QUEUE
        // 才刷新的 ConfigHandle，连续确认会读到陈旧值。
        let before = current_config().font_size;
        let mut config = Config::default_config();
        config.font_size = before + 5.0;
        config::use_this_configuration(config);
        assert_eq!(current_config().font_size, before + 5.0);
    }

    #[test]
    fn switching_section_drops_the_appearance_preview() {
        // 换分区必须把预览一并丢掉：预览行在新分区不可见，留着窗口就会
        // 停在一个界面上找不到对应行的配色（审查 nit）
        let overlay = SettingsOverlay::new();
        overlay.select_section(1); // Language -> Appearance
        overlay.previewed_scheme.replace(Some("Batman".to_string()));
        assert!(
            overlay.select_section(1),
            "带预览换分区必须要求还原窗口调色板"
        );
        assert!(overlay.previewed_scheme.borrow().is_none());
        assert!(!overlay.select_section(1), "没预览时不该要求还原");
    }

    #[test]
    fn bool_toggle_flips_on_every_activation() {
        // 连点两下开关必须一开一关，而不是两次都写 true
        let mut config = Config::default_config();
        config.mouse_right_click_menu = false;
        let item = Item::BoolToggle {
            label: "Right click menu",
            key: "mouse_right_click_menu",
        };

        let (key, first) = pending_write(&item, &config);
        assert_eq!(key, "mouse_right_click_menu");
        assert_eq!(first, Value::Bool(true));

        config.mouse_right_click_menu = true;
        let (_, second) = pending_write(&item, &config);
        assert_eq!(second, Value::Bool(false));
    }

    #[test]
    fn enum_choice_cycles_between_both_values() {
        let mut config = Config::default_config();
        config.window_close_confirmation = WindowCloseConfirmation::AlwaysPrompt;
        let item = Item::EnumChoice {
            label: "Close confirmation",
            key: "window_close_confirmation",
        };

        let (key, first) = pending_write(&item, &config);
        assert_eq!(key, "window_close_confirmation");
        assert_eq!(first, WindowCloseConfirmation::NeverPrompt.to_dynamic());
        assert_eq!(
            enum_display(&config, "window_close_confirmation"),
            tr("Always prompt")
        );

        config.window_close_confirmation = WindowCloseConfirmation::NeverPrompt;
        let (_, second) = pending_write(&item, &config);
        assert_eq!(second, WindowCloseConfirmation::AlwaysPrompt.to_dynamic());
        assert_ne!(first, second);
    }

    #[test]
    fn font_size_steps_are_clamped_and_resettable() {
        assert_eq!(next_font_size(6.0, FontOp::Decrease), 6.0);
        assert_eq!(next_font_size(100.0, FontOp::Increase), 100.0);
        assert_eq!(next_font_size(31.5, FontOp::Reset), DEFAULT_FONT_SIZE);
    }

    #[test]
    fn chrome_rows_follow_the_rows_compute_actually_pushes() {
        // 标题 + 分区 tab + 页脚 = 3；外观分区多一行过滤框；列表为空再多一行
        let overlay = SettingsOverlay::new();
        assert_eq!(overlay.chrome_rows(2), 3);
        overlay.select_section(1); // Language -> Appearance
        assert_eq!(overlay.chrome_rows(1001), 4);
        assert_eq!(overlay.chrome_rows(0), 5);
    }

    #[test]
    fn appearance_items_are_reused_until_the_filter_changes() {
        // WZ-19：一次按键里 compute/move_selection/mouse_event 会各要一次
        // 可见行列表，1001 条配色不能被重复排序 + 重复模糊匹配
        let overlay = SettingsOverlay::new();
        overlay.select_section(1); // Language -> Appearance
        let first = overlay.visible_items();
        let second = overlay.visible_items();
        assert!(Rc::ptr_eq(&first, &second));

        overlay.filter.borrow_mut().push_str("jellybeans");
        let filtered = overlay.visible_items();
        assert!(!Rc::ptr_eq(&first, &filtered));
        assert!(filtered.len() < first.len());
        assert!(Rc::ptr_eq(&filtered, &overlay.visible_items()));
        assert!(matches!(filtered.first(), Some(Item::Scheme(_))));
    }

    #[test]
    fn switching_section_swaps_the_cached_items() {
        let overlay = SettingsOverlay::new();
        let language = overlay.visible_items();
        assert_eq!(language.len(), 2);
        overlay.select_section(1);
        let appearance = overlay.visible_items();
        assert!(!Rc::ptr_eq(&language, &appearance));
        assert!(appearance.len() > 100);
    }

    #[test]
    fn selection_wraps_and_drags_the_scroll_window() {
        let overlay = SettingsOverlay::new();
        overlay.select_section(1); // Appearance: 有足够多的行可滚动
        overlay.visible_rows.replace(4);
        for _ in 0..5 {
            overlay.move_selection(1);
        }
        assert_eq!(*overlay.selected.borrow(), 5);
        assert_eq!(*overlay.top_row.borrow(), 2);
        // 反向回到 0，滚动窗口跟着回顶
        for _ in 0..5 {
            overlay.move_selection(-1);
        }
        assert_eq!(*overlay.selected.borrow(), 0);
        assert_eq!(*overlay.top_row.borrow(), 0);
        // 再往上一格环绕到末行
        overlay.move_selection(-1);
        let len = overlay.visible_items().len();
        assert_eq!(*overlay.selected.borrow(), len - 1);
    }

    #[test]
    fn unhandled_keys_are_passed_through() {
        assert_eq!(nav(KeyCode::Home, true), SettingsKey::PassThrough);
        assert_eq!(
            classify_key(KeyCode::Char('q'), KeyModifiers::ALT, false),
            SettingsKey::PassThrough
        );
    }

    #[test]
    fn locate_current_finds_the_effective_value_row() {
        // WZ-18：打开/切分区后选中行落在当前生效值上
        let mut config = Config::default_config();
        config.language = UiLanguage::En;
        let items = vec![
            Item::LanguageChoice(UiLanguage::ZhCn, "中文"),
            Item::LanguageChoice(UiLanguage::En, "English"),
        ];
        assert_eq!(locate_current_in(&items, &config), 1);

        config.color_scheme = Some("Batman".to_string());
        let schemes = vec![
            Item::Scheme("AdventureTime".to_string()),
            Item::Scheme("Batman".to_string()),
            Item::Scheme("Catppuccin Mocha".to_string()),
        ];
        assert_eq!(locate_current_in(&schemes, &config), 1);

        // 未知值/无对应行回 0；布尔与字号分区无所谓定位
        config.color_scheme = Some("No Such Scheme".to_string());
        assert_eq!(locate_current_in(&schemes, &config), 0);
        let toggles = vec![Item::BoolToggle {
            label: "Right-click menu",
            key: "mouse_right_click_menu",
        }];
        assert_eq!(locate_current_in(&toggles, &config), 0);
    }

    #[test]
    fn clicking_the_active_section_tab_keeps_browsing_state() {
        // WZ-09：点当前分区 tab 不重置选中/滚动；点其它分区才收敛
        let overlay = SettingsOverlay::new();
        overlay.select_section(1); // Language -> Appearance
        overlay.selected.replace(3);
        overlay.top_row.replace(2);
        assert!(
            !overlay.select_section_index(1),
            "点当前分区且无预览时不该要求还原"
        );
        assert_eq!(*overlay.selected.borrow(), 3);
        assert_eq!(*overlay.top_row.borrow(), 2);

        assert!(!overlay.select_section_index(2)); // -> Interaction
        assert_eq!(*overlay.section.borrow(), Section::Interaction);
        assert_eq!(*overlay.selected.borrow(), 0);
        assert_eq!(*overlay.top_row.borrow(), 0);

        // 越界索引安全拒绝
        assert!(!overlay.select_section_index(usize::MAX));
        assert!(!overlay.select_section_index(Section::ALL.len()));
    }

    #[test]
    fn section_tabs_fit_the_sentinel_band() {
        // WZ-09：分区数超出哨兵区间会在编译期被 const 断言拦下；
        // 运行时再守一遍鼠标路由使用的区间上界
        assert!(Section::ALL.len() <= MODAL_SECTION_MAX);
        assert!(MODAL_SECTION_BASE + Section::ALL.len() <= MODAL_CHROME_ROW);
    }
}
