//! fork 新增：herdr 式设置浮层（Modal）。
//!
//! 五个分区：语言（中文/English）、外观（内置配色方案，移动即预览、
//! Esc 还原、Enter 应用）、交互（右键菜单/滚动条/响铃/关闭确认）、
//! 字体（字号步进与重置）、Shell（默认 Shell：列出 launch_menu 里带
//! `GX_SHELL_ID` 标记的条目，选中后写 gui-settings.json 的 `default_shell`，
//! 选 GX Zsh 则删键；重载成功后才发出 `gx-default-shell-changed` 窗口事件，
//! 由 Lua 侧让 herdr 跟随）。入口：`OpenSettings`，以及直达 Shell 分区的
//! `ShowDefaultShellSettings`（主菜单/标签栏菜单的「默认 Shell…」）。
//! 生效链路：预览走 `TermWindow::set_preview_palette`（窗口级临时调色板，
//! 只丢渲染缓存 + 重绘，不重跑 Lua、不重建字体、不改窗口尺寸）。确认
//! （Enter / 单击）先记入 `pending`（同键覆盖）并尽量本地顶班：字号走
//! `TermWindow::adjust_font_scale` 现成缩放通道顶到目标值（Ctrl+= 同款，
//! 不跑 Lua；落地时还原用户先前的缩放），配色以预览调色板顶班（推导与
//! resolve 同序，落地后由持久化配置接管）。持久化 + `config::reload()`
//! 全量重载按确认去抖合并：间隔 <`CONFIRM_DEBOUNCE` 的连续确认（长按
//! Enter 的按键 repeat ~30Hz）只落一次——静置后由尾沿定时器一次
//! `store_key`/`delete_key` + 一次 reload（Timer + `TermWindowNotif::Apply`
//! 回 GUI 线程，票据防旧浮层迟到），切分区与三条关闭路径强制收口
//! （`flush_pending`），Esc 不丢刚确认的改动。全局生效跨重启持久化，不
//! 触碰用户 Lua；写文件或重载失败时在页脚上方显示错误行。开关/枚举/语言
//! 没有便宜的本地通道——`config_overrides` 触发整份 Lua 重载
//! （`config::overridden_config` → `Config::load_with_overrides`）且是
//! `ReloadConfiguration` 清不掉的每窗口钉死状态（见下）——它们的即时反馈
//! 是行内 pending 视图（✓ / On-Off / 字号随每次确认刷新），真正全局生效
//! 在落地时（≤`CONFIRM_DEBOUNCE`）。浮层刚打开时与多击的后几下不应用数据
//! 行（`press_applies`），双击打开它的菜单项不会顺手改掉设置。
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
use crate::termwindow::overlay_style::{
    chrome_px, overlay_layout_context, rows_that_fit, ChromeKind, OverlayStyle,
};
use crate::termwindow::{TermWindow, TermWindowNotif, UIItemType};
use anyhow::Context;
use config::gui_settings::DEFAULT_SHELL_KEY;
use config::i18n::{tr, UiLanguage};
use config::keyassignment::{KeyAssignment, SpawnCommand};
use config::{AudibleBell, Config, ConfigHandle, Dimension, Palette, WindowCloseConfirmation};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
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
/// 固定 chrome 行：标题、分区 tab、页脚。过滤框与空列表提示按需另算，
/// 见 `SettingsOverlay::chrome_rows`
const FIXED_CHROME_ROWS: usize = 3;

/// launch_menu 条目上标记「可选默认 Shell」的环境变量，值是 Shell id
/// （由 GX 的 Lua launch 配置写入；herdr 这类非 Shell 条目不带它）
const SHELL_ID_VAR: &str = "GX_SHELL_ID";
/// `default_shell` 缺省即 GX Zsh：选它时删键而不是写入
const GX_ZSH_SHELL_ID: &str = "gx-zsh";
/// 默认 Shell 落地并重载后发出的窗口事件，Lua 侧据此让 herdr 跟随
const DEFAULT_SHELL_CHANGED_EVENT: &str = "gx-default-shell-changed";
/// 浮层刚打开的这段时间里数据行不响应按下：双击打开浮层的菜单项时，
/// 第二下会落在某一行上。取多击间隔（`wezterm_term::LastMouseClick`
/// 的 500ms）
const OPEN_CLICK_GUARD: Duration = Duration::from_millis(500);

/// 确认去抖窗口：间隔小于它的连续确认（长按 Enter / 快速连点）合并成
/// 一次写盘 + 重载。每次确认都同步 `store_key`（读+解析+临时文件+rename）
/// 加 `config::reload()`（阻塞几十毫秒的全量 Lua 重载），30Hz 的按键
/// repeat 会把整条链连成持续的 UI 卡顿；静置一个窗口后由尾沿定时器
/// （`schedule_flush`）一次性落地
const CONFIRM_DEBOUNCE: Duration = Duration::from_millis(300);

/// 异步回调票据源：尾沿落地定时器回 GUI 线程后比对浮层实例号，旧浮层
/// 身上迟到的定时器直接丢弃（与壁纸浮层 `next_ticket` 同款）
fn next_ticket() -> u64 {
    static TICKET: AtomicU64 = AtomicU64::new(1);
    TICKET.fetch_add(1, Ordering::Relaxed)
}

/// 一条待落地的确认。`value: None` 表示删键（默认 Shell 选回 GX Zsh）；
/// 同键后写覆盖先写（`stage_pending`），flush 时一次性按序落盘
#[derive(Clone, Debug)]
struct PendingWrite {
    key: &'static str,
    value: Option<Value>,
}

/// pending 里某键的落地动作；外层 `None` = 该键没有未落地的确认，
/// 内层 `None` = 待删键
fn pending_of<'a>(pending: &'a [PendingWrite], key: &str) -> Option<&'a Option<Value>> {
    pending.iter().find(|w| w.key == key).map(|w| &w.value)
}

/// pending 里某键待写入的值（不含删键语义）
fn pending_value<'a>(pending: &'a [PendingWrite], key: &str) -> Option<&'a Value> {
    pending_of(pending, key).and_then(|v| v.as_ref())
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Section {
    Language,
    Appearance,
    Interaction,
    Font,
    Shell,
}

impl Section {
    const ALL: [Section; 5] = [
        Section::Language,
        Section::Appearance,
        Section::Interaction,
        Section::Font,
        Section::Shell,
    ];

    fn title(self) -> &'static str {
        match self {
            Section::Language => "Language",
            Section::Appearance => "Appearance",
            Section::Interaction => "Interaction",
            Section::Font => "Font",
            Section::Shell => "Shell",
        }
    }
}

// WZ-09: the clickable tab band must be wide enough for every section;
// outgrowing MODAL_SECTION_MAX without widening it fails here.
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
    /// launch_menu entry tagged with a shell id; `current` marks the
    /// effective default shell
    ShellChoice {
        id: String,
        label: String,
        current: bool,
    },
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
    /// 浮层打开的时刻，见 `OPEN_CLICK_GUARD`
    opened_at: Instant,
    /// 上一次应用失败的原因（写文件失败或重载失败），在页脚上方显示；
    /// 下一次成功应用或换分区时清掉
    error: RefCell<Option<String>>,
    /// 去抖积压：已确认、尚未写盘的键值（同键覆盖）。下一个值/行标签
    /// 都从这个视图推导，落地前连续确认不依赖全局重载推进
    pending: RefCell<Vec<PendingWrite>>,
    /// 最近一次确认的时刻；去抖窗口的计时原点
    last_confirm: RefCell<Option<Instant>>,
    /// 本地字号顶班前的窗口缩放（用户 Ctrl+= 的缩放不能被吃掉），
    /// 落地时还原；`None` = 本波确认没动过字号
    prior_font_scale: Cell<Option<f64>>,
    /// 浮层实例号：迟到的落地定时器只作用于自己这个实例
    instance: u64,
    element: RefCell<Option<Vec<ComputedElement>>>,
}

/// 可见行列表的缓存条目：分区 + 过滤文本（Shell 分区再加配置代数）一致
/// 即可复用（WZ-19）。Shell 分区的行来自 launch_menu 与已存的选择，重载后
/// 必须重建；其它分区的行与配置无关
struct ItemsCache {
    section: Section,
    filter: String,
    generation: usize,
    items: Rc<Vec<Item>>,
}

/// Reload the configuration after a settings write. An error when the
/// reloaded configuration failed to load: the old one stays in effect (and
/// the configuration error window reports why).
fn reload_config() -> anyhow::Result<()> {
    let before = config::configuration().generation();
    config::reload();
    anyhow::ensure!(
        config::configuration().generation() != before,
        "{}",
        tr("Saved, but the configuration failed to reload")
    );
    Ok(())
}

/// 去抖是否已到落地时点（纯函数便于单测）：有积压，且距最近一次确认
/// 已满一个去抖窗口。无积压或确认后仍在窗口内都不落地。
fn flush_is_due(pending_len: usize, last_confirm: Option<Instant>) -> bool {
    pending_len > 0 && last_confirm.map_or(false, |at| at.elapsed() >= CONFIRM_DEBOUNCE)
}

/// 数据行上的一次左键按下是否应用该行：双击的第二下（多击 streak > 1）
/// 与浮层刚打开时的按下都不算——双击打开浮层的菜单项会让第二下落在某一
/// 行上，静默改掉设置
fn press_applies(streak: usize, since_open: Duration) -> bool {
    streak <= 1 && since_open >= OPEN_CLICK_GUARD
}

/// 设置页读「当前值」的唯一来源：刚落地的全局配置。
///
/// 不能读 `TermWindow::config`。落地走 `config::reload()`，它在锁内同步换掉
/// 全局 CONFIG，但推给窗口的 `config_was_reloaded` 是经 `Window::notify` →
/// SPAWN_QUEUE 异步投递的，而 X11 主循环先把排队的 X 事件一次排干才轮到
/// SPAWN_QUEUE。窗口那份 handle 因此要晚一拍；去抖窗口内的连续确认从
/// `pending` 视图推进（见 `pending_write`），无积压时才回落到这里，两处
/// 合起来保证长按 Enter 期间每一步都从上一次确认的值继续推。
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
            opened_at: Instant::now(),
            error: RefCell::new(None),
            pending: RefCell::new(vec![]),
            last_confirm: RefCell::new(None),
            prior_font_scale: Cell::new(None),
            instance: next_ticket(),
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
            Section::Shell => {
                // 「当前」跟 pending 走：确认后 ✓ 立即搬家，不必等落地
                let saved = effective_default_shell(&self.pending.borrow());
                shell_items(&current_config().launch_menu, saved.as_deref())
            }
        }
    }

    /// The rows visible under the current filter (all sections except
    /// Appearance are unfiltered)。
    ///
    /// 结果按「分区 + 过滤文本」（Shell 分区再加配置代数）缓存：这个函数在
    /// 一次按键里会被调用多次，而外观分区每次都要排序 1001 个名字再跑一遍
    /// 模糊匹配（WZ-19）。
    fn visible_items(&self) -> Rc<Vec<Item>> {
        let section = *self.section.borrow();
        let generation = match section {
            Section::Shell => current_config().generation(),
            _ => 0,
        };
        {
            let cache = self.items_cache.borrow();
            if let Some(cache) = cache.as_ref() {
                if cache.section == section
                    && cache.filter.as_str() == *self.filter.borrow()
                    && cache.generation == generation
                {
                    return Rc::clone(&cache.items);
                }
            }
        }
        let items = Rc::new(self.items_for(section));
        self.items_cache.replace(Some(ItemsCache {
            section,
            filter: self.filter.borrow().clone(),
            generation,
            items: Rc::clone(&items),
        }));
        items
    }

    /// `compute()` 实际 push 的 chrome 行数：标题 + 分区 tab + 页脚固定三行，
    /// 外观分区多一行过滤框，列表为空时再多一行提示（`empty_hint`），应用
    /// 失败时再多一行错误。与 `compute()` 共用同一判据，两处口径不会分叉。
    fn chrome_rows(&self, items_len: usize) -> usize {
        FIXED_CHROME_ROWS
            + usize::from(self.filter_is_active())
            + usize::from(items_len == 0)
            + usize::from(self.error.borrow().is_some())
    }

    /// 数据行可视区的行数，按传入度量（渲染实际使用的那份）计算。按渲染
    /// 出的行高（`overlay_style::row_px`，含行内边距）折算，外框一圈先扣
    /// 掉；chrome 行不比数据行高，各按一行算
    fn max_rows_on_screen(
        &self,
        term_window: &TermWindow,
        metrics: &crate::utilsprites::RenderMetrics,
        items_len: usize,
    ) -> usize {
        let (_, chrome_height) = chrome_px(
            metrics.cell_size.width as f32,
            metrics.cell_size.height as f32,
        );
        let avail = (term_window.dimensions.pixel_height * VISIBLE_ROWS_HEIGHT_PERMILLE / 1000)
            as f32
            - chrome_height;
        rows_that_fit(avail, metrics)
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

    /// 换分区：丢掉本分区的全部易失状态后切过去。切分区是本波确认的
    /// 天然收口——先把积压落地，pending 不跨分区存活。
    fn switch_section(&self, delta: isize, term_window: &mut TermWindow) {
        self.flush_pending(term_window);
        if self.select_section(delta) {
            term_window.set_preview_palette(None);
        }
        // WZ-18: land on the row holding the effective value
        self.locate_current();
    }

    /// 点击分区 tab（WZ-09）：绝对索引版本，与 `switch_section` 共用
    /// 同一段状态收敛。
    fn click_section(&self, target: usize, term_window: &mut TermWindow) {
        self.flush_pending(term_window);
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
        let pending = self.pending.borrow();
        let idx = locate_current_in(&items, &config, &pending);
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
        self.error.replace(None);
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

    fn scroll_rows(&self, delta: isize) {
        let items_len = self.visible_items().len();
        let max_rows = (*self.visible_rows.borrow()).max(1);
        let max_top = items_len.saturating_sub(max_rows);
        let mut top_row = self.top_row.borrow_mut();
        // WZ-11：滚轮只滚视口，不改键盘选中（herdr C-20 同款教训）
        *top_row = (*top_row as isize + delta).clamp(0, max_top as isize) as usize;
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

    /// 应用选中行（确认动作）。
    ///
    /// 确认不再同步落地：写盘 + `config::reload()` 是「读+解析+临时文件+
    /// rename + 全量 Lua 重载 + 各窗口缓存重建」的整条链，长按 Enter 的
    /// 按键 repeat 会以 ~30Hz 连续触发。现在确认只做三件便宜事：记入
    /// `pending`（同键覆盖）、本地顶班（字号 `adjust_font_scale` / 配色
    /// 预览调色板）、刷新行标签（读 pending 视图）；落地合并到去抖窗口
    /// 静置后的 `flush_pending`。旧实现先 `upsert_override` 再 persist 的
    /// 每窗口钉死问题不变地规避着：仍不写 `config_overrides`（WZ-03 /
    /// WZ-20）。
    ///
    /// 下一个值由 `pending_write` 从「全局配置 + pending」推出，不经
    /// `TermWindow`——窗口那份 handle 要等异步回推才刷新（见
    /// `current_config`），去抖窗口内的连击也不能指望全局重载推进。
    fn activate(&self, row: usize, term_window: &mut TermWindow) {
        // 尾沿定时器没能触发的兜底：隔了一个去抖窗口的旧积压先落地，
        // 新一波确认从干净状态起步
        self.flush_if_due(term_window);
        let items = self.visible_items();
        let Some(item) = items.get(row).cloned() else {
            return;
        };
        if let Item::ShellChoice { id, .. } = &item {
            // 默认 Shell 的落地分写键/删键两种（选 GX Zsh = 删键），同样
            // 走去抖；✓ 立即跟 pending 走，行缓存先丢掉
            self.stage_pending(PendingWrite {
                key: DEFAULT_SHELL_KEY,
                value: shell_setting(id),
            });
            self.items_cache.replace(None);
            self.schedule_flush(term_window);
            term_window.invalidate_modal();
            return;
        }
        let pending = self.pending.borrow().clone();
        let (key, value) = pending_write(&item, &current_config(), &pending);
        // WZ-21 守卫：未知枚举 key 的 Null 哨兵不落盘（正常写入路径
        // 永远不会写 Null）
        if matches!(value, Value::Null) {
            log::warn!("settings: skip persist for unknown enum key {key:?}");
            return;
        }
        self.apply_local_preview(&item, &value, term_window);
        self.stage_pending(PendingWrite {
            key,
            value: Some(value),
        });
        self.schedule_flush(term_window);
        // 确认后立刻重算浮层：行标签读 pending 视图，勾选标记与「字号: 12.5」
        // 不必等落地刷新
        term_window.invalidate_modal();
    }

    /// 数值/配色类确认的本地顶班（写盘前的即时视觉），只用 TermWindow
    /// 现成的便宜通道，不碰 `config_overrides`（每窗口钉死 + 触发整份
    /// Lua 重载，见模块头不变量）：
    /// - 字号：把窗口缩放顶到 目标值/渲染基准（`fonts.config()` 的
    ///   font_size，每次确认现读）——`adjust_font_scale` 是 Ctrl+= 同款
    ///   路径，只重建字体与纹理、不跑 Lua；用户先前的缩放记到
    ///   `prior_font_scale`，落地时还原。已知小瞬态：落地重载经
    ///   SPAWN_QUEUE 异步换掉窗口的字体基准，若它恰好落在下一次确认
    ///   之后，旧基准算出的缩放会短暂顶过头，下一次确认即按新基准
    ///   自愈。
    /// - 配色：预览调色板就是最终色（推导与 resolve 同序），落地时由
    ///   持久化配置接管。
    /// - 开关/枚举/语言没有便宜的本地通道，即时反馈只有行内 pending 视图。
    fn apply_local_preview(&self, item: &Item, value: &Value, term_window: &mut TermWindow) {
        match item {
            Item::Scheme(name) => self.preview_scheme(term_window, name),
            Item::FontOp(_) => {
                let Value::F64(target) = value else {
                    return;
                };
                let target = target.into_inner();
                let base = term_window.fonts.config().font_size;
                if base <= 0.0 {
                    return;
                }
                let Some(window) = term_window.window.as_ref().map(|w| w.clone()) else {
                    return;
                };
                if self.prior_font_scale.get().is_none() {
                    self.prior_font_scale
                        .set(Some(term_window.fonts.get_font_scale()));
                }
                term_window.adjust_font_scale(target / base, &window);
            }
            _ => {}
        }
    }

    /// 记一条待落地确认（同键覆盖，保持首次确认的次序），并把去抖窗口
    /// 的计时原点拨到现在
    fn stage_pending(&self, write: PendingWrite) {
        let mut pending = self.pending.borrow_mut();
        pending.retain(|w| w.key != write.key);
        pending.push(write);
        drop(pending);
        self.last_confirm.replace(Some(Instant::now()));
    }

    /// 去抖尾沿：每次确认后安排一个 `CONFIRM_DEBOUNCE` 的定时器，静置
    /// 满一个窗口就把这一波一次性落地。回 GUI 线程复用 copy 浮层的
    /// Timer + `TermWindowNotif::Apply` 模式；期间又有确认时旧的定时器
    /// 会在 `flush_if_due` 的时距判定下自然空转，由最新的那个落地。
    fn schedule_flush(&self, term_window: &TermWindow) {
        let Some(window) = term_window.window.as_ref().map(|w| w.clone()) else {
            // 拿不到窗口句柄（单测环境）就没有尾沿触发，退化为切分区 /
            // 关闭 / 下一次确认时隔到期落地
            return;
        };
        let instance = self.instance;
        promise::spawn::spawn(async move {
            smol::Timer::after(CONFIRM_DEBOUNCE).await;
            window.notify(TermWindowNotif::Apply(Box::new(move |tw| {
                let Some(modal) = tw.get_modal() else {
                    return;
                };
                let Some(overlay) = modal.downcast_ref::<SettingsOverlay>() else {
                    return;
                };
                if overlay.instance == instance {
                    overlay.flush_if_due(tw);
                }
            })));
            anyhow::Result::<()>::Ok(())
        })
        .detach();
    }

    /// 去抖是否已到落地时点：有积压且距最近一次确认满一个去抖窗口
    fn flush_if_due(&self, term_window: &mut TermWindow) {
        if flush_is_due(self.pending.borrow().len(), *self.last_confirm.borrow()) {
            self.flush_pending(term_window);
        }
    }

    /// 把去抖积压一次性落地：按序 `store_key`/`delete_key`（同键只会有
    /// 最后确认的那条），然后只做一次 `config::reload()`——写盘 + 重载
    /// 是整条贵链，合并正是去抖的目的。落地的默认 Shell 照旧在成功后
    /// 发 `gx-default-shell-changed`（Lua launch 配置据此重算 default_prog，
    /// herdr 跟随）；写/重载失败时不发事件，丢掉 Shell 行缓存让 ✓ 按文件
    /// 里的选择重画，错误行说明它尚未生效。
    fn flush_pending(&self, term_window: &mut TermWindow) {
        let pending = std::mem::take(&mut *self.pending.borrow_mut());
        if pending.is_empty() {
            return;
        }
        self.last_confirm.replace(None);
        // 预览是窗口级临时状态，落地前一律丢掉，让持久化后的配置成为
        // 唯一真源（与旧确认路径同款次序）
        self.clear_preview(term_window);
        let touches_shell = pending.iter().any(|w| w.key == DEFAULT_SHELL_KEY);
        let mut applied = Ok(());
        for write in &pending {
            let result = match &write.value {
                Some(value) => config::gui_settings::store_key(write.key, value),
                None => config::gui_settings::delete_key(write.key),
            };
            if applied.is_ok() {
                applied = result.context(tr("failed to write settings"));
            }
        }
        if applied.is_ok() {
            applied = reload_config();
        }
        // 字号顶班到此为止：还原用户先前的缩放，持久化后的 font_size
        // 接管（`config_changed` 不动 scale，重载落地即 目标值×先前缩放，
        // 与逐次同步落地的终态一致）
        if let Some(scale) = self.prior_font_scale.take() {
            if let Some(window) = term_window.window.as_ref().map(|w| w.clone()) {
                term_window.adjust_font_scale(scale, &window);
            }
        }
        if touches_shell {
            if applied.is_err() {
                self.items_cache.replace(None);
            } else {
                term_window.emit_window_event(DEFAULT_SHELL_CHANGED_EVENT, None);
            }
        }
        self.report(applied);
        term_window.invalidate_modal();
        if let Some(window) = term_window.window.as_ref() {
            window.invalidate();
        }
    }

    /// 应用结果落到错误行（成功时清掉），返回是否成功
    fn report(&self, result: anyhow::Result<()>) -> bool {
        match result {
            Ok(()) => {
                self.error.replace(None);
                true
            }
            Err(err) => {
                log::error!("settings: failed to apply: {err:#}");
                self.error.replace(Some(format!("{err:#}")));
                false
            }
        }
    }

    /// 行标签里的当前值与 `activate` 推导下一个值读同一份视图
    /// （全局配置 + pending），显示与落地因此不会各说各话。
    fn row_label(&self, item: &Item, config: &Config) -> String {
        let pending = self.pending.borrow();
        match item {
            Item::LanguageChoice(lang, label) => {
                let current = match pending_value(&pending, "language") {
                    Some(value) => lang.to_dynamic() == *value,
                    None => *lang == config.language,
                };
                let marker = if current { "✓ " } else { "  " };
                format!("{marker}{label}")
            }
            Item::Scheme(name) => {
                let pending_scheme = match pending_value(&pending, "color_scheme") {
                    Some(Value::String(s)) => Some(s.as_str()),
                    _ => None,
                };
                let current = pending_scheme.or_else(|| config.color_scheme.as_deref());
                let marker = if current == Some(name.as_str()) {
                    "✓ "
                } else {
                    "  "
                };
                format!("{marker}{name}")
            }
            Item::BoolToggle { label, key } => {
                let value = current_bool(&pending, config, key);
                let value = if value { tr("On") } else { tr("Off") };
                format!("  {}: {value}", tr(label))
            }
            Item::EnumChoice { label, key } => {
                let value = enum_display(&pending, config, key);
                format!("  {}: {value}", tr(label))
            }
            Item::FontOp(op) => {
                let label = match op {
                    FontOp::Decrease => "Decrease font size",
                    FontOp::Increase => "Increase font size",
                    FontOp::Reset => "Reset font size",
                };
                let size = current_font_size(&pending, config);
                format!("  {}（{}: {size:.1}）", tr(label), tr("Font size"))
            }
            Item::ShellChoice { label, current, .. } => {
                let marker = if *current { "✓ " } else { "  " };
                format!("{marker}{label}")
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
        // 浮层外观（圆角描边外框、选中行强调条、分区 tab 下划线）走共享
        // 样式，配色读窗口那份配置与（可能处于预览中的）窗口调色板
        let window_config = term_window.config.clone();
        let style = OverlayStyle::from_config(&window_config, term_window.palette());

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

        let mut rows = vec![style.chrome(&font, tr("Settings").into_owned(), ChromeKind::Title)];

        // Section tab row (WZ-09): one child element per section so each
        // tab is individually addressable (click targets via the
        // MODAL_SECTION_BASE sentinel band); the active section is marked
        // by an accent underline.
        let section = *self.section.borrow();
        let tab_children = Section::ALL
            .iter()
            .enumerate()
            .map(|(idx, s)| {
                style
                    .section_tab(&font, tr(s.title()).into_owned(), *s == section)
                    .item_type(UIItemType::Modal(MODAL_SECTION_BASE + idx))
            })
            .collect();
        rows.push(
            Element::new(&font, ElementContent::Children(tab_children))
                .colors(ElementColors {
                    border: BorderColor::default(),
                    bg: LinearRgba::TRANSPARENT.into(),
                    text: style.text.into(),
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
            rows.push(style.chrome(&font, format!("> {filter}_"), ChromeKind::Input));
        }

        if items.is_empty() {
            let hint = tr(empty_hint(section)).into_owned();
            rows.push(style.chrome(&font, hint, ChromeKind::Hint));
        }

        for (display_idx, item) in items.iter().enumerate().skip(top_row).take(max_rows) {
            let label = self.row_label(item, &settings_config);
            rows.push(style.row(
                &font,
                ElementContent::Text(label),
                display_idx == selected,
                display_idx,
            ));
        }

        // 应用失败的原因，与壁纸浮层的行内错误同一形态（不关浮层）
        if let Some(err) = self.error.borrow().as_ref() {
            rows.push(style.chrome(&font, format!("! {err}"), ChromeKind::Error));
        }

        // Footer hints
        rows.push(style.chrome(
            &font,
            tr("↑↓ select  Tab section  Enter apply  Esc cancel").into_owned(),
            ChromeKind::Hint,
        ));

        // 外框自己也进 hit map（`container` 挂 MODAL_CHROME_ROW）：内边距/
        // 边框/外边距那一圈不属于任何行，点在那里会被「点浮层外即关闭」
        // 误判成点外面（WZ-06）
        let element = style.container(&font, rows);

        let (padding_left, padding_top) = term_window.padding_left_top();
        let border = term_window.get_os_border();
        let top_bar_height = if term_window.show_tab_bar && !term_window.config.tab_bar_at_bottom {
            term_window.tab_bar_pixel_height_lossy()
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
            &overlay_layout_context(term_window, &metrics, euclid::rect(x, y, width, height)),
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

/// WZ-18：在可见行里找「当前生效值」的行号；找不到（被过滤/无对应
/// 值的分区）回 0。当前值 = pending 优先、`config` 兜底。纯函数，
/// 不依赖 TermWindow。
fn locate_current_in(items: &[Item], config: &Config, pending: &[PendingWrite]) -> usize {
    for (idx, item) in items.iter().enumerate() {
        let is_current = match item {
            Item::LanguageChoice(lang, _) => match pending_value(pending, "language") {
                Some(value) => lang.to_dynamic() == *value,
                None => *lang == config.language,
            },
            Item::Scheme(name) => match pending_value(pending, "color_scheme") {
                Some(Value::String(s)) => s == name,
                _ => config.color_scheme.as_deref() == Some(name.as_str()),
            },
            Item::ShellChoice { current, .. } => *current,
            _ => false,
        };
        if is_current {
            return idx;
        }
    }
    0
}

/// 一行「确认」要写进 gui-settings.json 的键值。
///
/// 只依赖传入的 `config`（`current_config()` 取来的那份）与 pending 视图，
/// 签名里没有 `TermWindow`——窗口那份 ConfigHandle 因此不可能再被误读成
/// 当前值。去抖窗口内的连续确认从 pending 推进（WZ-20 的回归护栏）：
/// 落地前全局配置还没换，字号步进/开关翻转只能沿积压值继续。
fn pending_write(item: &Item, config: &Config, pending: &[PendingWrite]) -> (&'static str, Value) {
    match item {
        Item::LanguageChoice(lang, _) => ("language", lang.to_dynamic()),
        Item::Scheme(name) => ("color_scheme", Value::String(name.clone())),
        Item::BoolToggle { label: _, key } => {
            (*key, Value::Bool(!current_bool(pending, config, key)))
        }
        Item::EnumChoice { label: _, key } => (
            *key,
            next_enum_value(key, &current_enum_value(pending, config, key)),
        ),
        Item::FontOp(op) => (
            "font_size",
            Value::F64(ordered_float::OrderedFloat(next_font_size(
                current_font_size(pending, config),
                *op,
            ))),
        ),
        // Shell 行不走这里：`activate` 交给待落地确认（选 GX Zsh 时删键
        // 而不是写入，见 `shell_setting`）
        Item::ShellChoice { id, .. } => (DEFAULT_SHELL_KEY, Value::String(id.clone())),
    }
}

/// Shell 分区的「当前选择」：pending 待落地值优先（✓ 不必等落地就搬家），
/// 否则读 gui-settings.json 里已存的选择
fn effective_default_shell(pending: &[PendingWrite]) -> Option<String> {
    match pending_of(pending, DEFAULT_SHELL_KEY) {
        Some(Some(Value::String(id))) => Some(id.clone()),
        // 待删键 = 选回 GX Zsh；异常值同样按缺省处理
        Some(_) => None,
        None => config::gui_settings::default_shell(),
    }
}

/// 当前字号：pending 优先，配置兜底
fn current_font_size(pending: &[PendingWrite], config: &Config) -> f64 {
    match pending_value(pending, "font_size") {
        Some(Value::F64(size)) => size.into_inner(),
        _ => config.font_size,
    }
}

/// 当前布尔值：pending 优先，配置兜底
fn current_bool(pending: &[PendingWrite], config: &Config, key: &str) -> bool {
    match pending_value(pending, key) {
        Some(Value::Bool(value)) => *value,
        _ => effective_bool(config, key).unwrap_or_else(|| default_bool(key)),
    }
}

/// 当前枚举值（动态值表示）：pending 优先，配置兜底
fn current_enum_value(pending: &[PendingWrite], config: &Config, key: &str) -> Value {
    if let Some(value) = pending_value(pending, key) {
        return value.clone();
    }
    match key {
        "audible_bell" => config.audible_bell.to_dynamic(),
        "window_close_confirmation" => config.window_close_confirmation.to_dynamic(),
        _ => Value::Null,
    }
}

/// Shell 分区的行：launch_menu 里带 `GX_SHELL_ID` 的条目，按菜单顺序、同一
/// id 只取第一条。✓（`current`）落在仍然可选的已存选择上，否则落在
/// GX Zsh 上，再否则落在第一行。纯函数：已存选择由调用方读好传入。
fn shell_items(launch_menu: &[SpawnCommand], saved: Option<&str>) -> Vec<Item> {
    let mut shells: Vec<(&str, &SpawnCommand)> = vec![];
    for entry in launch_menu {
        match entry.set_environment_variables.get(SHELL_ID_VAR) {
            Some(id) if !id.is_empty() && !shells.iter().any(|(seen, _)| seen == id) => {
                shells.push((id.as_str(), entry));
            }
            _ => {}
        }
    }
    let offered = |id: &str| shells.iter().any(|(seen, _)| *seen == id);
    let current = match saved {
        Some(id) if offered(id) => Some(id),
        _ if offered(GX_ZSH_SHELL_ID) => Some(GX_ZSH_SHELL_ID),
        _ => shells.first().map(|(id, _)| *id),
    };
    shells
        .into_iter()
        .map(|(id, entry)| Item::ShellChoice {
            id: id.to_string(),
            label: entry.label.clone().unwrap_or_else(|| id.to_string()),
            current: current == Some(id),
        })
        .collect()
}

/// 选中某个 Shell 要写进 gui-settings.json 的值；`None` 表示删键——
/// `default_shell` 缺省就是 GX Zsh
fn shell_setting(id: &str) -> Option<Value> {
    (id != GX_ZSH_SHELL_ID).then(|| Value::String(id.to_string()))
}

/// 列表为空时那一行提示：Shell 分区为空说明 launch_menu 没有任何条目带
/// `GX_SHELL_ID`（例如用户自己的 launch.lua），而不是过滤没有命中
fn empty_hint(section: Section) -> &'static str {
    match section {
        Section::Shell => "No shells to choose: no launch_menu entry is tagged with GX_SHELL_ID",
        _ => "(no matches)",
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

/// Enum cycler: each activate moves to the other value of the 2-value set.
/// 当前值以动态值比较——pending 里待落地的值与配置里的值同一种表示
fn next_enum_value(key: &str, current: &Value) -> Value {
    match key {
        "audible_bell" => {
            let next = if *current == AudibleBell::SystemBeep.to_dynamic() {
                AudibleBell::Disabled
            } else {
                AudibleBell::SystemBeep
            };
            next.to_dynamic()
        }
        "window_close_confirmation" => {
            let next = if *current == WindowCloseConfirmation::AlwaysPrompt.to_dynamic() {
                WindowCloseConfirmation::NeverPrompt
            } else {
                WindowCloseConfirmation::AlwaysPrompt
            };
            next.to_dynamic()
        }
        // fork (WZ-21): 新增枚举 key 漏加分支时告警并回退 Null（调用方
        // 守卫不写），不 panic 整个 GUI 进程
        _ => {
            log::warn!("settings: unknown enum settings key {key:?}");
            Value::Null
        }
    }
}

fn enum_display(
    pending: &[PendingWrite],
    config: &Config,
    key: &str,
) -> std::borrow::Cow<'static, str> {
    let current = current_enum_value(pending, config, key);
    match key {
        "audible_bell" => {
            if current == AudibleBell::SystemBeep.to_dynamic() {
                tr("System beep")
            } else {
                tr("Disabled")
            }
        }
        "window_close_confirmation" => {
            if current == WindowCloseConfirmation::AlwaysPrompt.to_dynamic() {
                tr("Always prompt")
            } else {
                tr("Never prompt")
            }
        }
        // fork (WZ-21): 同 next_enum_value，告警并显示占位而不 panic
        _ => {
            log::warn!("settings: unknown enum settings key {key:?}");
            "?".into()
        }
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
            // WZ-11：滚轮滚视口（不改键盘选中）
            WMEK::VertWheel(amount) => {
                self.scroll_rows(-amount as isize);
                term_window.invalidate_modal();
            }
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
                let streak = term_window
                    .last_mouse_click
                    .as_ref()
                    .map_or(1, |click| click.streak);
                if press_applies(streak, self.opened_at.elapsed()) {
                    self.selected.replace(row);
                    self.activate(row, term_window);
                }
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
    /// 路径都会到这里，配色不会留在随手划过的那套上。关闭前先把去抖
    /// 积压一次性落盘（`flush_pending`）——关闭是确认的强制收口，Esc
    /// 不能丢掉用户刚确认的改动；落地本身会清预览，之后 `clear_preview`
    /// 对已还原的状态是无操作。
    fn on_dismissed(&self, term_window: &mut TermWindow) {
        self.flush_pending(term_window);
        self.clear_preview(term_window);
    }
}

/// Convenience: open the settings overlay modally
pub fn open_settings(term_window: &mut TermWindow) {
    open_settings_on(term_window, Section::Language);
}

/// 「默认 Shell…」菜单项：打开设置浮层并直接落在 Shell 分区
pub fn open_shell_settings(term_window: &mut TermWindow) {
    open_settings_on(term_window, Section::Shell);
}

fn open_settings_on(term_window: &mut TermWindow, section: Section) {
    let modal = Rc::new(SettingsOverlay::new());
    modal.section.replace(section);
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
        // WZ-20 回归护栏：落地后 `config::reload()` 在锁内同步换掉全局
        // 配置，无积压时下一次确认从刚落地的值再推一格
        let mut config = Config::default_config();
        config.font_size = 12.0;
        let item = Item::FontOp(FontOp::Increase);

        let (key, first) = pending_write(&item, &config, &[]);
        assert_eq!(key, "font_size");
        assert_eq!(first, f64_value(12.5));

        // 落地之后全局配置就是这个样子
        config.font_size = 12.5;
        let (_, second) = pending_write(&item, &config, &[]);
        assert_eq!(second, f64_value(13.0));
        assert_ne!(first, second);
    }

    #[test]
    fn consecutive_activations_step_from_the_pending_view() {
        // 去抖护栏：落地前（去抖窗口内的连击）全局配置还没换，下一步只能
        // 从 pending 推进——长按 Enter 步进字号每一下都要动，而不是全部
        // 从同一个陈旧值推
        let mut config = Config::default_config();
        config.font_size = 12.0;
        let item = Item::FontOp(FontOp::Increase);

        let (_, first) = pending_write(&item, &config, &[]);
        assert_eq!(first, f64_value(12.5));

        let pending = vec![PendingWrite {
            key: "font_size",
            value: Some(f64_value(12.5)),
        }];
        let (_, second) = pending_write(&item, &config, &pending);
        assert_eq!(second, f64_value(13.0));

        let pending = vec![PendingWrite {
            key: "font_size",
            value: Some(f64_value(13.0)),
        }];
        let (_, third) = pending_write(&item, &config, &pending);
        assert_eq!(third, f64_value(13.5));
    }

    #[test]
    fn flush_is_due_only_after_the_quiet_window() {
        // 去抖判定：无积压不落地；有积压但距最近一次确认不足一个窗口
        // 不落地；满一个窗口才落地
        let now = Instant::now();
        assert!(!flush_is_due(0, Some(now - CONFIRM_DEBOUNCE)), "无积压");
        assert!(
            !flush_is_due(1, None),
            "积压与确认时刻由 stage_pending 一并记录，不存在有积压而无时点"
        );
        assert!(!flush_is_due(1, Some(now)), "确认刚发生，还在去抖窗口内");
        assert!(
            flush_is_due(1, Some(now - CONFIRM_DEBOUNCE)),
            "静置满一个窗口就要落地"
        );
    }

    #[test]
    fn staged_writes_overwrite_the_same_key_and_keep_their_order() {
        // 同键覆盖（只落最后确认的值）、异键并存
        let overlay = SettingsOverlay::new();
        overlay.stage_pending(PendingWrite {
            key: "font_size",
            value: Some(f64_value(13.0)),
        });
        overlay.stage_pending(PendingWrite {
            key: "enable_scroll_bar",
            value: Some(Value::Bool(true)),
        });
        overlay.stage_pending(PendingWrite {
            key: "font_size",
            value: Some(f64_value(14.0)),
        });
        let pending = overlay.pending.borrow();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].key, "enable_scroll_bar");
        assert_eq!(pending[1].key, "font_size");
        assert_eq!(pending[1].value, Some(f64_value(14.0)));
        assert!(overlay.last_confirm.borrow().is_some(), "计时原点已拨动");
    }

    #[test]
    fn pending_shell_choice_moves_the_current_mark() {
        // ✓ 跟 pending 走：确认 PowerShell 后当前值是 pwsh，确认 GX Zsh
        // （删键）回到缺省。不覆盖「无 pending 读文件」分支——那要碰
        // 真实的 gui-settings.json
        let shell = |id: &str| {
            vec![PendingWrite {
                key: DEFAULT_SHELL_KEY,
                value: Some(Value::String(id.to_string())),
            }]
        };
        assert_eq!(
            effective_default_shell(&shell("pwsh")).as_deref(),
            Some("pwsh")
        );
        assert_eq!(
            effective_default_shell(&vec![PendingWrite {
                key: DEFAULT_SHELL_KEY,
                value: None,
            }]),
            None
        );
    }

    #[test]
    fn row_labels_read_the_pending_view() {
        // 行标签是去抖期间的即时反馈：✓ / On-Off / 字号都跟 pending 走
        let overlay = SettingsOverlay::new();
        let config = Config::default_config();

        let toggle = Item::BoolToggle {
            label: "Scroll bar",
            key: "enable_scroll_bar",
        };
        assert!(overlay
            .row_label(&toggle, &config)
            .contains(tr("Off").as_ref()));
        overlay.stage_pending(PendingWrite {
            key: "enable_scroll_bar",
            value: Some(Value::Bool(true)),
        });
        assert!(overlay
            .row_label(&toggle, &config)
            .contains(tr("On").as_ref()));

        let reset = Item::FontOp(FontOp::Reset);
        let initial = overlay.row_label(&reset, &config);
        assert!(initial.contains(&format!("{:.1}", config.font_size)));
        overlay.stage_pending(PendingWrite {
            key: "font_size",
            value: Some(f64_value(14.5)),
        });
        assert!(overlay.row_label(&reset, &config).contains("14.5"));

        let zh = Item::LanguageChoice(UiLanguage::ZhCn, "中文");
        let en = Item::LanguageChoice(UiLanguage::En, "English");
        overlay.stage_pending(PendingWrite {
            key: "language",
            value: Some(UiLanguage::En.to_dynamic()),
        });
        assert!(overlay.row_label(&en, &config).starts_with("✓"));
        assert!(!overlay.row_label(&zh, &config).starts_with("✓"));
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
        // 连点两下开关必须一开一关，而不是两次都写 true：无积压从配置推，
        // 有积压沿 pending 翻转
        let mut config = Config::default_config();
        config.mouse_right_click_menu = false;
        let item = Item::BoolToggle {
            label: "Right click menu",
            key: "mouse_right_click_menu",
        };

        let (key, first) = pending_write(&item, &config, &[]);
        assert_eq!(key, "mouse_right_click_menu");
        assert_eq!(first, Value::Bool(true));

        let pending = vec![PendingWrite {
            key: "mouse_right_click_menu",
            value: Some(Value::Bool(true)),
        }];
        let (_, second) = pending_write(&item, &config, &pending);
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

        let (key, first) = pending_write(&item, &config, &[]);
        assert_eq!(key, "window_close_confirmation");
        assert_eq!(first, WindowCloseConfirmation::NeverPrompt.to_dynamic());
        assert_eq!(
            enum_display(&[], &config, "window_close_confirmation"),
            tr("Always prompt")
        );

        // 去抖窗口内的下一次确认沿 pending 翻回
        let pending = vec![PendingWrite {
            key: "window_close_confirmation",
            value: Some(WindowCloseConfirmation::NeverPrompt.to_dynamic()),
        }];
        let (_, second) = pending_write(&item, &config, &pending);
        assert_eq!(second, WindowCloseConfirmation::AlwaysPrompt.to_dynamic());
        assert_eq!(
            enum_display(&pending, &config, "window_close_confirmation"),
            tr("Never prompt")
        );
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
        // WZ-18：打开/切分区后选中行落在当前生效值上；去抖积压优先于
        // 配置里的值
        let mut config = Config::default_config();
        config.language = UiLanguage::En;
        let items = vec![
            Item::LanguageChoice(UiLanguage::ZhCn, "中文"),
            Item::LanguageChoice(UiLanguage::En, "English"),
        ];
        assert_eq!(locate_current_in(&items, &config, &[]), 1);
        // 待落地的语言选择优先于配置
        let pending = vec![PendingWrite {
            key: "language",
            value: Some(UiLanguage::ZhCn.to_dynamic()),
        }];
        assert_eq!(locate_current_in(&items, &config, &pending), 0);

        config.color_scheme = Some("Batman".to_string());
        let schemes = vec![
            Item::Scheme("AdventureTime".to_string()),
            Item::Scheme("Batman".to_string()),
            Item::Scheme("Catppuccin Mocha".to_string()),
        ];
        assert_eq!(locate_current_in(&schemes, &config, &[]), 1);
        let pending = vec![PendingWrite {
            key: "color_scheme",
            value: Some(Value::String("Catppuccin Mocha".to_string())),
        }];
        assert_eq!(locate_current_in(&schemes, &config, &pending), 2);

        // 未知值/无对应行回 0；布尔与字号分区无所谓定位
        config.color_scheme = Some("No Such Scheme".to_string());
        assert_eq!(locate_current_in(&schemes, &config, &[]), 0);
        let toggles = vec![Item::BoolToggle {
            label: "Right-click menu",
            key: "mouse_right_click_menu",
        }];
        assert_eq!(locate_current_in(&toggles, &config, &[]), 0);
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

    fn shell_entry(label: Option<&str>, id: Option<&str>) -> SpawnCommand {
        let mut entry = SpawnCommand {
            label: label.map(str::to_string),
            ..Default::default()
        };
        if let Some(id) = id {
            entry
                .set_environment_variables
                .insert(SHELL_ID_VAR.to_string(), id.to_string());
        }
        entry
    }

    /// (id, label, current) of every row; the rows must all be shells
    fn shell_rows(items: &[Item]) -> Vec<(String, String, bool)> {
        items
            .iter()
            .map(|item| match item {
                Item::ShellChoice { id, label, current } => (id.clone(), label.clone(), *current),
                _ => panic!("not a shell row"),
            })
            .collect()
    }

    fn current_shells(items: &[Item]) -> Vec<String> {
        shell_rows(items)
            .into_iter()
            .filter(|(_, _, current)| *current)
            .map(|(id, _, _)| id)
            .collect()
    }

    #[test]
    fn shell_rows_come_from_tagged_launch_menu_entries() {
        // herdr 不是 Shell（不带标记）；重复 id 只取第一条；没有 label 用 id
        let menu = vec![
            shell_entry(Some("GX Zsh"), Some("gx-zsh")),
            shell_entry(Some("herdr"), None),
            shell_entry(Some("PowerShell 7"), Some("pwsh")),
            shell_entry(Some("PowerShell 7 again"), Some("pwsh")),
            shell_entry(None, Some("cmd")),
            shell_entry(Some("empty tag"), Some("")),
        ];
        assert_eq!(
            shell_rows(&shell_items(&menu, None)),
            vec![
                ("gx-zsh".to_string(), "GX Zsh".to_string(), true),
                ("pwsh".to_string(), "PowerShell 7".to_string(), false),
                ("cmd".to_string(), "cmd".to_string(), false),
            ]
        );
        assert!(shell_items(&[shell_entry(Some("herdr"), None)], None).is_empty());
    }

    #[test]
    fn current_shell_is_the_saved_choice_then_gx_zsh_then_the_first_row() {
        let menu = vec![
            shell_entry(Some("GX Zsh"), Some("gx-zsh")),
            shell_entry(Some("PowerShell 7"), Some("pwsh")),
            shell_entry(Some("Ubuntu"), Some("wsl:Ubuntu")),
        ];
        assert_eq!(
            current_shells(&shell_items(&menu, Some("wsl:Ubuntu"))),
            vec!["wsl:Ubuntu"]
        );
        assert_eq!(current_shells(&shell_items(&menu, None)), vec!["gx-zsh"]);
        // 已存的选择不再可选（卸载了、换了 launch.lua）时退回 GX Zsh
        assert_eq!(
            current_shells(&shell_items(&menu, Some("nu"))),
            vec!["gx-zsh"]
        );
        // 没有 GX Zsh（未检测到 GX 包）时第一行就是默认
        assert_eq!(
            current_shells(&shell_items(&menu[1..], Some("nu"))),
            vec!["pwsh"]
        );
    }

    #[test]
    fn choosing_gx_zsh_removes_the_saved_choice() {
        assert_eq!(shell_setting(GX_ZSH_SHELL_ID), None);
        assert_eq!(
            shell_setting("pwsh"),
            Some(Value::String("pwsh".to_string()))
        );
        assert_eq!(
            shell_setting("wsl:Ubuntu"),
            Some(Value::String("wsl:Ubuntu".to_string()))
        );
    }

    #[test]
    fn shell_section_marks_and_locates_the_current_shell() {
        let menu = vec![
            shell_entry(Some("GX Zsh"), Some("gx-zsh")),
            shell_entry(Some("PowerShell 7"), Some("pwsh")),
        ];
        let items = shell_items(&menu, Some("pwsh"));
        let config = Config::default_config();
        assert_eq!(locate_current_in(&items, &config, &[]), 1);
        let overlay = SettingsOverlay::new();
        assert_eq!(overlay.row_label(&items[0], &config), "  GX Zsh");
        assert_eq!(overlay.row_label(&items[1], &config), "✓ PowerShell 7");
    }

    #[test]
    fn double_clicking_the_opening_menu_entry_changes_nothing() {
        // 双击「默认 Shell…」：第二下既是多击（streak 2），又在打开后的
        // 保护期内，落在哪一行都不应用
        assert!(!press_applies(2, Duration::from_millis(120)));
        assert!(!press_applies(1, Duration::from_millis(120)));
        assert!(!press_applies(2, Duration::from_secs(3)));
        // 打开一会儿之后的普通单击照常应用
        assert!(press_applies(1, OPEN_CLICK_GUARD));
        assert!(press_applies(1, Duration::from_secs(3)));
    }

    #[test]
    fn failed_apply_shows_an_error_row_until_the_next_success() {
        let overlay = SettingsOverlay::new();
        assert!(!overlay.report(Err(anyhow::anyhow!("disk full"))));
        assert_eq!(overlay.error.borrow().as_deref(), Some("disk full"));
        // 错误行和其它 chrome 行一样占用行预算
        assert_eq!(overlay.chrome_rows(2), FIXED_CHROME_ROWS + 1);
        assert!(overlay.report(Ok(())));
        assert!(overlay.error.borrow().is_none());
        assert_eq!(overlay.chrome_rows(2), FIXED_CHROME_ROWS);
        // 换分区也清掉
        overlay.report(Err(anyhow::anyhow!("again")));
        overlay.select_section(1);
        assert!(overlay.error.borrow().is_none());
    }

    #[test]
    fn empty_shell_section_explains_the_missing_tags() {
        // 自定义 launch.lua 没打标记时要说明原因，而不是显示「无匹配项」
        assert!(empty_hint(Section::Shell).contains(SHELL_ID_VAR));
        assert_eq!(empty_hint(Section::Appearance), "(no matches)");
        let overlay = SettingsOverlay::new();
        overlay.select_section_index(Section::ALL.len() - 1);
        assert_eq!(*overlay.section.borrow(), Section::Shell);
        // 提示与「(no matches)」一样占一行 chrome
        assert_eq!(overlay.chrome_rows(0), FIXED_CHROME_ROWS + 1);
    }
}
