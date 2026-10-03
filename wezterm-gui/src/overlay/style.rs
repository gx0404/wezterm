//! fork(zh)：文本型浮层（启动器 / InputSelector / 确认框）共享的行样式。
//!
//! 这些浮层跑在 `mux::termwiztermtab::allocate` 造的内存终端里，外观只能
//! 用 termwiz `Change` 序列描述；本模块把「加粗标题行 + 分隔线」与
//! 「选中行底色 + 左侧强调条」收敛成纯函数供各浮层共用，单测锁定序列。
//! 选中行颜色取 `colors.overlay_selected_bg/fg`：两者都未配置时保持
//! 上游的整行反色，默认视觉不变。
//! 边界：只产出 `Change`，不持有状态、不碰终端；行号↔条目映射也在这里，
//! 保证渲染与鼠标命中对「标题占几行」的认知一致。

use config::{ConfigHandle, RgbaColor};
use termwiz::cell::{unicode_column_width, AttributeChange, CellAttributes, Intensity};
use termwiz::color::ColorAttribute;
use termwiz::surface::{Change, Position, SEQ_ZERO};

/// 标题行 + 分隔线占用的行数；列表首项画在这一行（0 起算）
pub const HEADER_ROWS: usize = 2;

/// 选中行左侧强调条
const ACCENT_BAR: &str = "▌";
/// 标题下方分隔线
const SEPARATOR: &str = "─";

/// 选中行的呈现方式
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SelectedStyle {
    /// 未配置 `overlay_selected_*`：沿用整行反色
    Reverse,
    /// 配置了 `overlay_selected_*`：底色 + 左侧强调条
    Highlight {
        bg: ColorAttribute,
        accent: ColorAttribute,
    },
}

impl SelectedStyle {
    /// 只要配了其一就切到高亮样式；缺的那一项回落终端默认色
    pub fn new(bg: Option<RgbaColor>, accent: Option<RgbaColor>) -> Self {
        if bg.is_none() && accent.is_none() {
            return Self::Reverse;
        }
        let to_attr = |c: Option<RgbaColor>| {
            c.map(|c| ColorAttribute::TrueColorWithDefaultFallback(c.into()))
                .unwrap_or(ColorAttribute::Default)
        };
        Self::Highlight {
            bg: to_attr(bg),
            accent: to_attr(accent),
        }
    }
}

/// 列表行的行首标签列
#[derive(Debug, Clone, Copy)]
pub enum RowPrefix<'a> {
    /// 快捷标签，渲染为 ` {label:>width}. `，可带标签专用前景/背景色
    Label {
        label: &'a str,
        width: usize,
        fg: Option<ColorAttribute>,
        bg: Option<ColorAttribute>,
    },
    /// 不显示标签时的等宽空白占位（列数）
    Blank(usize),
}

/// 一行列表项的输入
#[derive(Debug, Clone, Copy)]
pub struct ListRow<'a> {
    pub prefix: RowPrefix<'a>,
    /// 条目文本，可含 `wezterm.format` 产生的转义序列
    pub text: &'a str,
    /// 条目文本最多占用的列数
    pub max_width: usize,
    pub active: bool,
}

/// 文本浮层的行样式；每次渲染从配置现取，配置重载后下一帧即生效
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayStyle {
    pub selected: SelectedStyle,
    /// 强调条显示宽度（东亚歧义宽度按宽处理时为 2）
    bar_width: usize,
    /// 分隔线字符显示宽度
    separator_width: usize,
}

impl OverlayStyle {
    pub fn new(selected: SelectedStyle, bar_width: usize, separator_width: usize) -> Self {
        Self {
            selected,
            bar_width: bar_width.max(1),
            separator_width: separator_width.max(1),
        }
    }

    pub fn from_config(config: &ConfigHandle) -> Self {
        let version = config.unicode_version();
        Self::new(
            SelectedStyle::new(
                config.resolved_palette.overlay_selected_bg,
                config.resolved_palette.overlay_selected_fg,
            ),
            unicode_column_width(ACCENT_BAR, Some(&version)),
            unicode_column_width(SEPARATOR, Some(&version)),
        )
    }

    /// 标题/说明行（加粗）+ 下一行铺满 `cols` 的分隔线；光标停在列表首行
    pub fn header(&self, text: &str, cols: usize) -> Vec<Change> {
        vec![
            Change::AllAttributes(bold()),
            Change::Text(text.to_string()),
            Change::AllAttributes(CellAttributes::default()),
            Change::Text("\r\n".to_string()),
            Change::AllAttributes(
                CellAttributes::default()
                    .set_intensity(Intensity::Half)
                    .clone(),
            ),
            Change::Text(SEPARATOR.repeat(cols / self.separator_width)),
            Change::AllAttributes(CellAttributes::default()),
            Change::Text("\r\n".to_string()),
        ]
    }

    /// 过滤模式下覆写标题行（同样加粗）；光标留在文本末尾充当输入点
    pub fn prompt_line(&self, text: &str) -> Vec<Change> {
        vec![
            Change::CursorPosition {
                x: Position::Absolute(0),
                y: Position::Absolute(0),
            },
            Change::ClearToEndOfLine(ColorAttribute::Default),
            Change::AllAttributes(bold()),
            Change::Text(text.to_string()),
            Change::AllAttributes(CellAttributes::default()),
        ]
    }

    /// 一行列表项（不含行尾换行）。反色样式与上游逐条一致；高亮样式
    /// 用强调条顶替行首空格，列宽不变，再把整行剩余部分刷成底色。
    pub fn list_row(&self, row: &ListRow) -> Vec<Change> {
        let mut changes = vec![];
        let mut attr = CellAttributes::blank();
        let highlight = match self.selected {
            SelectedStyle::Highlight { bg, accent } if row.active => Some((bg, accent)),
            _ => None,
        };
        let reverse = row.active && highlight.is_none();

        let mut prefix = match row.prefix {
            RowPrefix::Label { label, width, .. } => format!(" {label:>width$}. "),
            RowPrefix::Blank(n) => " ".repeat(n),
        };

        if reverse {
            changes.push(AttributeChange::Reverse(true).into());
            attr.set_reverse(true);
        }
        if let Some((bg, accent)) = highlight {
            attr.set_background(bg);
            changes.push(Change::AllAttributes(
                CellAttributes::blank()
                    .set_foreground(accent)
                    .set_background(bg)
                    .clone(),
            ));
            changes.push(Change::Text(ACCENT_BAR.to_string()));
            changes.push(Change::AllAttributes(attr.clone()));
            prefix = trim_prefix_for_bar(&prefix, self.bar_width);
        }

        match row.prefix {
            RowPrefix::Label { fg, bg, .. } => {
                if let Some(bg) = bg {
                    changes.push(AttributeChange::Background(bg).into());
                }
                if let Some(fg) = fg {
                    changes.push(AttributeChange::Foreground(fg).into());
                }
                changes.push(Change::Text(prefix));
                if highlight.is_some() {
                    if fg.is_some() || bg.is_some() {
                        changes.push(Change::AllAttributes(attr.clone()));
                    }
                } else {
                    if bg.is_some() {
                        changes.push(AttributeChange::Background(ColorAttribute::Default).into());
                    }
                    if fg.is_some() {
                        changes.push(AttributeChange::Foreground(ColorAttribute::Default).into());
                    }
                }
            }
            RowPrefix::Blank(_) => changes.push(Change::Text(prefix)),
        }

        let mut line = crate::tabbar::parse_status_text(row.text, attr.clone());
        if line.len() > row.max_width {
            line.resize(row.max_width, SEQ_ZERO);
        }
        changes.append(&mut line.changes(&attr));
        changes.push(Change::Text(" ".to_string()));
        if let Some((bg, _)) = highlight {
            changes.push(Change::ClearToEndOfLine(bg));
        }
        if reverse {
            changes.push(AttributeChange::Reverse(false).into());
        }
        changes.push(Change::AllAttributes(CellAttributes::default()));
        changes
    }

    /// 确认框按钮；高亮样式下强调条顶替标签的首个空格，宽度不变，
    /// 鼠标命中区（按标签显示宽度算）因此无需调整
    pub fn button(&self, label: &str, active: bool) -> Vec<Change> {
        match (active, self.selected) {
            (false, _) => vec![label.into()],
            (true, SelectedStyle::Reverse) => vec![
                AttributeChange::Reverse(true).into(),
                label.into(),
                AttributeChange::Reverse(false).into(),
            ],
            (true, SelectedStyle::Highlight { bg, accent }) => vec![
                Change::AllAttributes(
                    CellAttributes::blank()
                        .set_foreground(accent)
                        .set_background(bg)
                        .clone(),
                ),
                Change::Text(ACCENT_BAR.to_string()),
                Change::AllAttributes(CellAttributes::blank().set_background(bg).clone()),
                Change::Text(trim_prefix_for_bar(label, self.bar_width)),
                Change::AllAttributes(CellAttributes::default()),
            ],
        }
    }
}

/// 屏幕行号 → 条目下标（`top_row` 为首个可见条目）；标题区与越界返回 None
pub fn entry_at_row(y: usize, top_row: usize, num_entries: usize) -> Option<usize> {
    let idx = top_row + y.checked_sub(HEADER_ROWS)?;
    (idx < num_entries).then_some(idx)
}

fn bold() -> CellAttributes {
    CellAttributes::default()
        .set_intensity(Intensity::Bold)
        .clone()
}

/// 强调条占 `bar_width` 列：先吃掉行首空格，宽字形再吃掉行尾空格，
/// 保证选中行与其它行列对齐（前缀两端恒为空格）
fn trim_prefix_for_bar(prefix: &str, bar_width: usize) -> String {
    let mut s = prefix.strip_prefix(' ').unwrap_or(prefix).to_string();
    for _ in 1..bar_width {
        if s.ends_with(' ') {
            s.pop();
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use termwiz::surface::Surface;

    fn rgb(r: u8, g: u8, b: u8) -> RgbaColor {
        (r, g, b).into()
    }

    fn highlight_style() -> OverlayStyle {
        OverlayStyle::new(
            SelectedStyle::new(Some(rgb(0x31, 0x32, 0x44)), Some(rgb(0x89, 0xb4, 0xfa))),
            1,
            1,
        )
    }

    fn reverse_style() -> OverlayStyle {
        OverlayStyle::new(SelectedStyle::new(None, None), 1, 1)
    }

    fn render(cols: usize, rows: usize, changes: Vec<Change>) -> Surface {
        let mut surface = Surface::new(cols, rows);
        surface.add_changes(changes);
        surface
    }

    fn row(active: bool) -> ListRow<'static> {
        ListRow {
            prefix: RowPrefix::Label {
                label: "a",
                width: 1,
                fg: None,
                bg: None,
            },
            text: "item",
            max_width: 20,
            active,
        }
    }

    #[test]
    fn unset_colors_keep_reverse_video() {
        assert_eq!(SelectedStyle::new(None, None), SelectedStyle::Reverse);
        assert!(matches!(
            SelectedStyle::new(Some(rgb(1, 2, 3)), None),
            SelectedStyle::Highlight {
                accent: ColorAttribute::Default,
                ..
            }
        ));
    }

    #[test]
    fn reverse_row_matches_upstream_sequence() {
        // 上游 selector/launcher 的选中行：Reverse(true) … Reverse(false)
        let changes = reverse_style().list_row(&row(true));
        assert_eq!(changes[0], AttributeChange::Reverse(true).into());
        assert_eq!(changes[1], Change::Text(" a. ".to_string()));
        let n = changes.len();
        assert_eq!(changes[n - 2], AttributeChange::Reverse(false).into());
        assert_eq!(
            changes[n - 1],
            Change::AllAttributes(CellAttributes::default())
        );

        let surface = render(12, 1, changes);
        assert_eq!(surface.screen_chars_to_string(), " a. item    \n");
        let lines = surface.screen_lines();
        assert!(lines[0]
            .visible_cells()
            .take(9)
            .all(|c| c.attrs().reverse()));
        // 未选中行不反色
        let surface = render(12, 1, reverse_style().list_row(&row(false)));
        assert!(surface.screen_lines()[0]
            .visible_cells()
            .all(|c| !c.attrs().reverse()));
    }

    #[test]
    fn highlight_row_draws_accent_bar_and_fills_background() {
        let style = highlight_style();
        let SelectedStyle::Highlight { bg, accent } = style.selected else {
            panic!("expected highlight");
        };
        let surface = render(12, 1, style.list_row(&row(true)));
        // 强调条顶替行首空格，列宽与未选中行一致
        assert_eq!(surface.screen_chars_to_string(), "▌a. item    \n");
        let lines = surface.screen_lines();
        let cells: Vec<_> = lines[0].visible_cells().collect();
        assert_eq!(cells[0].attrs().foreground(), accent);
        assert!(cells.iter().all(|c| c.attrs().background() == bg));
        assert!(cells.iter().all(|c| !c.attrs().reverse()));
    }

    #[test]
    fn highlight_row_restores_label_colors() {
        let label_bg = ColorAttribute::PaletteIndex(4);
        let style = highlight_style();
        let SelectedStyle::Highlight { bg, .. } = style.selected else {
            panic!("expected highlight");
        };
        let mut r = row(true);
        r.prefix = RowPrefix::Label {
            label: "a",
            width: 1,
            fg: None,
            bg: Some(label_bg),
        };
        let surface = render(12, 1, style.list_row(&r));
        let lines = surface.screen_lines();
        let cells: Vec<_> = lines[0].visible_cells().collect();
        // 标签列用标签底色，条目文本回到高亮底色
        assert_eq!(cells[1].attrs().background(), label_bg);
        assert_eq!(cells[4].str(), "i");
        assert_eq!(cells[4].attrs().background(), bg);
    }

    #[test]
    fn wide_accent_bar_keeps_columns_aligned() {
        assert_eq!(trim_prefix_for_bar(" a. ", 1), "a. ");
        assert_eq!(trim_prefix_for_bar(" a. ", 2), "a.");
        assert_eq!(trim_prefix_for_bar("    ", 2), "  ");
        assert_eq!(trim_prefix_for_bar(" [Y]es ", 1), "[Y]es ");
    }

    #[test]
    fn header_is_bold_with_full_width_separator() {
        let surface = render(6, 3, reverse_style().header("Title", 6));
        assert_eq!(surface.screen_chars_to_string(), "Title \n──────\n      \n");
        let lines = surface.screen_lines();
        assert_eq!(
            lines[0].visible_cells().next().unwrap().attrs().intensity(),
            Intensity::Bold
        );
        assert_eq!(
            lines[1].visible_cells().next().unwrap().attrs().intensity(),
            Intensity::Half
        );
        // 分隔线为宽字形时按显示宽度折半，不会折行
        let wide = OverlayStyle::new(SelectedStyle::Reverse, 2, 2);
        let changes = wide.header("T", 6);
        assert_eq!(changes[5], Change::Text("───".to_string()));
    }

    #[test]
    fn button_keeps_label_width() {
        let style = highlight_style();
        let surface = render(7, 1, style.button(" [Y]es ", true));
        assert_eq!(surface.screen_chars_to_string(), "▌[Y]es \n");
        assert_eq!(
            reverse_style().button(" [Y]es ", true),
            vec![
                AttributeChange::Reverse(true).into(),
                " [Y]es ".into(),
                AttributeChange::Reverse(false).into(),
            ]
        );
        assert_eq!(style.button(" [N]o ", false), vec![" [N]o ".into()]);
    }

    #[test]
    fn rows_below_header_map_to_entries() {
        assert_eq!(entry_at_row(0, 0, 5), None);
        assert_eq!(entry_at_row(1, 0, 5), None);
        assert_eq!(entry_at_row(2, 0, 5), Some(0));
        assert_eq!(entry_at_row(3, 4, 6), Some(5));
        // 越过最后一个条目不命中
        assert_eq!(entry_at_row(4, 4, 6), None);
    }
}
