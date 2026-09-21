use crate::termwindow::box_model::*;
use crate::termwindow::modal::{Modal, MODAL_CHROME_ROW};
use crate::termwindow::{DimensionContext, TermWindow, UIItemType};
use config::i18n::{fill, tr};
use config::keyassignment::{
    ClipboardCopyDestination, ClipboardPasteSource, KeyAssignment, PaneDirection, SpawnCommand,
    SpawnTabDomain, SplitPane, SplitSize,
};
use config::Dimension;
use std::cell::RefCell;
use std::rc::Rc;
use termwiz::cell::unicode_column_width;
use wezterm_term::{KeyCode, KeyModifiers};
use window::color::LinearRgba;
use window::WindowOps;

/// A menu item; a `None` action renders a separator row
pub type MenuItem = (Option<String>, Option<KeyAssignment>);

/// 菜单几何的唯一真源：下面的 `Element` 构造与外框尺寸推导共用这些常量，
/// 于是宽高估算不会和实际 box model 漂移（WZ-04 的 `+ 16.` 魔数由此消失）。
/// 菜单行的左右内边距（单元格）
const ROW_PADDING_H_CELLS: f32 = 0.75;
/// 菜单行的上下内边距（单元格）
const ROW_PADDING_V_CELLS: f32 = 0.1;
/// 菜单外框四边的内边距 / 外边距（单元格）与边框（像素）
const MENU_PADDING_CELLS: f32 = 0.25;
const MENU_MARGIN_CELLS: f32 = 0.25;
const MENU_BORDER_PIXELS: f32 = 1.;
/// 分隔行的字形；宽度同样按显示列数参与估算
const SEPARATOR_ROW: &str = "────────";
/// 宽度下限（单元格），避免只有极短标签时菜单细成一条
const MIN_WIDTH_CELLS: f32 = 10.;

/// 一行的渲染文本：`None` 标签是分隔行
fn row_text(label: Option<&String>) -> String {
    match label {
        Some(label) => label.clone(),
        None => SEPARATOR_ROW.to_string(),
    }
}

/// 菜单内容占用的最大显示列数（纯函数）。
///
/// CJK 一格占两列，按 `chars().count()` 估算会少算一半宽度，box model
/// 随后把中文菜单项拦腰截断（WZ-04），所以一律用显示列宽。
fn content_width_cells(items: &[MenuItem]) -> f32 {
    items.iter().fold(MIN_WIDTH_CELLS, |acc, (label, _)| {
        acc.max(unicode_column_width(&row_text(label.as_ref()), None) as f32)
    })
}

/// 由内容列数推导菜单外框的像素尺寸（纯函数）。
///
/// 内容 + 行内边距 + 外框内边距 + 外边距 + 边框，全部取自上面那批与
/// `Element` 构造共用的常量，因此估算不会与实际 box model 漂移。
fn menu_box_size(content_cells: f32, rows: usize, cell_width: f32, cell_height: f32) -> (f32, f32) {
    let chrome_cells = 2. * (MENU_PADDING_CELLS + MENU_MARGIN_CELLS);
    let row_height = cell_height * (1. + 2. * ROW_PADDING_V_CELLS);
    let width = (content_cells + 2. * ROW_PADDING_H_CELLS + chrome_cells) * cell_width
        + 2. * MENU_BORDER_PIXELS;
    let height = rows as f32 * row_height + chrome_cells * cell_height + 2. * MENU_BORDER_PIXELS;
    (width, height)
}

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
        // fork (WZ-17): bail out after a full lap without finding a
        // selectable row — a menu of pure separators used to spin here
        // and freeze the GUI.
        for _ in 0..len {
            let next = (*selected as isize + delta).rem_euclid(len as isize) as usize;
            if self.row_is_selectable(next) {
                *selected = next;
                break;
            }
            // 把游标先挪过去，下一圈才能继续探测后面的行
            *selected = next;
        }
    }

    /// The standard pane context menu (right click in the terminal area)
    pub fn pane_menu(x: f32, y: f32) -> Self {
        Self::new(
            vec![
                (
                    Some(tr("Split Pane Right").into_owned()),
                    Some(KeyAssignment::SplitPane(SplitPane {
                        direction: PaneDirection::Right,
                        size: SplitSize::Percent(50),
                        command: SpawnCommand::default(),
                        top_level: false,
                    })),
                ),
                (
                    Some(tr("Split Pane Down").into_owned()),
                    Some(KeyAssignment::SplitPane(SplitPane {
                        direction: PaneDirection::Down,
                        size: SplitSize::Percent(50),
                        command: SpawnCommand::default(),
                        top_level: false,
                    })),
                ),
                (
                    Some(tr("Toggle Pane Zoom").into_owned()),
                    Some(KeyAssignment::TogglePaneZoomState),
                ),
                (None, None),
                (
                    Some(tr("Copy").into_owned()),
                    Some(KeyAssignment::CopyTo(ClipboardCopyDestination::Clipboard)),
                ),
                (
                    Some(tr("Paste").into_owned()),
                    Some(KeyAssignment::PasteFrom(ClipboardPasteSource::Clipboard)),
                ),
                (None, None),
                (
                    Some(tr("Scroll to Top").into_owned()),
                    Some(KeyAssignment::ScrollToTop),
                ),
                (
                    Some(tr("Scroll to Bottom").into_owned()),
                    Some(KeyAssignment::ScrollToBottom),
                ),
                (None, None),
                (
                    Some(tr("Close Pane").into_owned()),
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
                    Some(tr("New Tab").into_owned()),
                    Some(KeyAssignment::SpawnTab(SpawnTabDomain::CurrentPaneDomain)),
                ),
                (
                    Some(tr("Show Tab Navigator").into_owned()),
                    Some(KeyAssignment::ShowTabNavigator),
                ),
                (None, None),
                (
                    Some(tr("Move Tab Left").into_owned()),
                    Some(KeyAssignment::MoveTabRelative(-1)),
                ),
                (
                    Some(tr("Move Tab Right").into_owned()),
                    Some(KeyAssignment::MoveTabRelative(1)),
                ),
                (None, None),
                (
                    Some(fill(&tr("Close Tab {n}"), &[("n", &tab_idx.to_string())])),
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
                    Some(tr("New Tab").into_owned()),
                    Some(KeyAssignment::SpawnTab(SpawnTabDomain::CurrentPaneDomain)),
                ),
                (
                    Some(tr("Show Launcher").into_owned()),
                    Some(KeyAssignment::ShowLauncher),
                ),
            ],
            x,
            y,
        )
    }

    /// The herdr-style main menu (fork): opened from the `☰` tab bar
    /// button or the `ShowMainMenu` key assignment. Labels intentionally
    /// reuse the command palette briefs so terminology stays consistent.
    pub fn main_menu(x: f32, y: f32) -> Self {
        Self::new(
            vec![
                (
                    Some(tr("Activate Command Palette").into_owned()),
                    Some(KeyAssignment::ActivateCommandPalette),
                ),
                (
                    Some(tr("Show Keybindings").into_owned()),
                    Some(KeyAssignment::ShowKeybinds),
                ),
                (
                    Some(tr("Open Settings").into_owned()),
                    Some(KeyAssignment::OpenSettings),
                ),
                (
                    Some(tr("Manage Wallpapers").into_owned()),
                    Some(KeyAssignment::ShowWallpaperOverlay),
                ),
                (
                    Some(tr("Reload configuration").into_owned()),
                    Some(KeyAssignment::ReloadConfiguration),
                ),
                (None, None),
                (
                    Some(tr("Hide/Minimize Window").into_owned()),
                    Some(KeyAssignment::Hide),
                ),
                (
                    Some(tr("Quit WezTerm").into_owned()),
                    Some(KeyAssignment::QuitApplication),
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
        for (idx, (label, action)) in items.iter().enumerate() {
            let (row_bg, row_fg) = if idx == selected && action.is_some() {
                (fg.clone(), bg.clone())
            } else {
                (LinearRgba::TRANSPARENT.into(), fg.clone())
            };
            let text = row_text(label.as_ref());
            rows.push(
                Element::new(&font, ElementContent::Text(text))
                    .colors(ElementColors {
                        border: BorderColor::default(),
                        bg: row_bg,
                        text: row_fg,
                    })
                    .padding(BoxDimension {
                        left: Dimension::Cells(ROW_PADDING_H_CELLS),
                        right: Dimension::Cells(ROW_PADDING_H_CELLS),
                        top: Dimension::Cells(ROW_PADDING_V_CELLS),
                        bottom: Dimension::Cells(ROW_PADDING_V_CELLS),
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
            .padding(BoxDimension::new(Dimension::Cells(MENU_PADDING_CELLS)))
            .border(BoxDimension::new(Dimension::Pixels(MENU_BORDER_PIXELS)))
            .margin(BoxDimension::new(Dimension::Cells(MENU_MARGIN_CELLS)))
            .display(DisplayType::Block)
            // 外框自己也要进 hit map：内边距/边框/外边距这一圈不属于任何行，
            // 点在那里会被「点浮层外即关闭」当成点外面（WZ-06）
            .item_type(UIItemType::Modal(MODAL_CHROME_ROW));

        let border = term_window.get_os_border();
        let (menu_width, menu_height) = menu_box_size(
            content_width_cells(items),
            items.len(),
            metrics.cell_size.width as f32,
            metrics.cell_size.height as f32,
        );
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
        // 外框（chrome）吞掉事件：既不移动选中行，也不关闭菜单
        if row == MODAL_CHROME_ROW {
            return Ok(());
        }
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
pub fn open_context_menu(term_window: &mut TermWindow, menu: ContextMenu) {
    term_window.set_modal(Rc::new(menu));
    if let Some(window) = term_window.window.as_ref() {
        window.invalidate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str) -> MenuItem {
        (Some(label.to_string()), Some(KeyAssignment::ScrollToTop))
    }

    #[test]
    fn content_width_counts_display_columns_not_chars() {
        // 「拆分窗格（右）」= 7 个字符、14 个显示列
        let items = vec![item("拆分窗格（右）")];
        assert_eq!(content_width_cells(&items), 14.);
        // 短标签走下限，菜单不会细成一条
        assert_eq!(content_width_cells(&[item("Copy")]), MIN_WIDTH_CELLS);
    }

    #[test]
    fn content_width_covers_the_separator_row() {
        let sep: MenuItem = (None, None);
        assert_eq!(
            content_width_cells(std::slice::from_ref(&sep)),
            MIN_WIDTH_CELLS.max(unicode_column_width(SEPARATOR_ROW, None) as f32)
        );
        // 分隔行不得压低正文行算出的宽度
        assert_eq!(content_width_cells(&[item("拆分窗格（右）"), sep]), 14.);
    }

    #[test]
    fn menu_box_leaves_room_for_the_whole_label() {
        let (cell_w, cell_h) = (9., 20.);
        let cells = content_width_cells(&[item("拆分窗格（右）")]);
        let (width, _) = menu_box_size(cells, 1, cell_w, cell_h);
        // box model 给行的可用文本宽度 = 外框宽 - 边框 - 外框内边距 - 行内边距；
        // 它必须严格大于标签宽度，否则最后一个字形被 `break` 掉
        let chrome =
            2. * MENU_BORDER_PIXELS + 2. * (MENU_PADDING_CELLS + ROW_PADDING_H_CELLS) * cell_w;
        assert!(
            width - chrome > cells * cell_w,
            "width={} chrome={} cells={}",
            width,
            chrome,
            cells
        );
        // 旧的 `+ 16.` 魔数连按字符数算出的宽度都兜不住中文
        assert!(width > cells * cell_w + 16.);
    }

    #[test]
    fn menu_box_height_accounts_for_container_chrome() {
        let (_, height) = menu_box_size(MIN_WIDTH_CELLS, 3, 9., 20.);
        let rows = 3. * 20. * (1. + 2. * ROW_PADDING_V_CELLS);
        assert!(height > rows, "height={} rows={}", height, rows);
    }

    #[test]
    fn all_separator_menu_does_not_hang() {
        // WZ-17：全是分隔符的菜单按方向键必须在有限步内返回
        let menu = ContextMenu::new(vec![(None, None), (None, None), (None, None)], 0., 0.);
        menu.move_selection(1);
        menu.move_selection(-1);
        // 部分可选时照旧跳到可选行
        let menu2 = ContextMenu::new(
            vec![
                (None, None),
                (Some("x".to_string()), Some(KeyAssignment::Nop)),
                (None, None),
            ],
            0.,
            0.,
        );
        menu2.move_selection(1);
        assert_eq!(*menu2.selected.borrow(), 1);
    }
}
