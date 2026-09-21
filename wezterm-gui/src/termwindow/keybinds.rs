//! fork 新增：快捷键速查浮层（Modal）。
//!
//! 按命令面板的分组顺序列出全部命令与「当前实际生效」的键位
//! （数据源同 `actions_for_palette_and_menubar`，反映用户自定义
//! key_bindings），只读展示：↑↓/j/k 滚动、鼠标悬停高亮、Esc 关闭。
//! 文案与键帽格式化与命令面板共用（`tr` / `commands::format_key_label`）。

use crate::commands::{format_key_label, CommandDef, ExpandedCommand};
use crate::termwindow::box_model::*;
use crate::termwindow::modal::{Modal, MODAL_CHROME_ROW};
use crate::termwindow::{DimensionContext, TermWindow, UIItemType};
use config::i18n::{tr, tr_cow};
use config::keyassignment::KeyAssignment;
use config::Dimension;
use std::cell::{Ref, RefCell};
use std::rc::Rc;
use wezterm_term::{KeyCode, KeyModifiers};
use window::color::LinearRgba;
use window::WindowOps;

pub struct KeybindsOverlay {
    commands: Vec<ExpandedCommand>,
    selected: RefCell<usize>,
    top_row: RefCell<usize>,
    max_rows_on_screen: RefCell<usize>,
    element: RefCell<Option<Vec<ComputedElement>>>,
}

impl KeybindsOverlay {
    pub fn new(term_window: &TermWindow) -> Self {
        let mut commands = CommandDef::actions_for_palette_and_menubar(&term_window.config);
        commands.sort_by(|a, b| match a.menubar.cmp(&b.menubar) {
            std::cmp::Ordering::Equal => a.brief.cmp(&b.brief),
            ordering => ordering,
        });
        Self {
            commands,
            selected: RefCell::new(0),
            top_row: RefCell::new(0),
            max_rows_on_screen: RefCell::new(0),
            element: RefCell::new(None),
        }
    }

    fn move_selection(&self, delta: isize) {
        let len = self.commands.len();
        if len == 0 {
            return;
        }
        let mut selected = self.selected.borrow_mut();
        *selected = (*selected as isize + delta).rem_euclid(len as isize) as usize;
        let max_rows = *self.max_rows_on_screen.borrow();
        let mut top_row = self.top_row.borrow_mut();
        if *selected < *top_row {
            *top_row = *selected;
        } else if max_rows > 0 && *selected >= *top_row + max_rows {
            *top_row = selected.saturating_sub(max_rows - 1);
        }
    }

    /// WZ-11：滚轮只滚视口，不改选中
    fn scroll_rows(&self, delta: isize) {
        let len = self.commands.len();
        let max_rows = (*self.max_rows_on_screen.borrow()).max(1);
        let max_top = len.saturating_sub(max_rows);
        let mut top_row = self.top_row.borrow_mut();
        *top_row = (*top_row as isize + delta).clamp(0, max_top as isize) as usize;
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

        let top_bar_height = if term_window.show_tab_bar && !term_window.config.tab_bar_at_bottom {
            term_window.tab_bar_pixel_height_lossy()
        } else {
            0.
        };
        let (padding_left, padding_top) = term_window.padding_left_top();
        let border = term_window.get_os_border();
        let top_pixel_y = top_bar_height + padding_top + border.top.get() as f32;

        let mut max_rows_on_screen = ((term_window.dimensions.pixel_height * 8 / 10)
            / metrics.cell_size.height as usize)
            .saturating_sub(4);
        if let Some(size) = term_window.config.command_palette_rows {
            max_rows_on_screen = max_rows_on_screen.min(size);
        }
        *self.max_rows_on_screen.borrow_mut() = max_rows_on_screen;

        let mut rows =
            vec![
                Element::new(&font, ElementContent::Text(tr("Keybindings").into_owned()))
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
            ];

        let top_row = *self.top_row.borrow();
        let selected = *self.selected.borrow();

        for (display_idx, command) in self
            .commands
            .iter()
            .enumerate()
            .skip(top_row)
            .take(max_rows_on_screen)
        {
            let group = if command.menubar.is_empty() {
                String::new()
            } else {
                format!(
                    "{}: ",
                    command
                        .menubar
                        .iter()
                        .map(|s| tr(s))
                        .collect::<Vec<_>>()
                        .join(" | ")
                )
            };
            let label = format!("{group}{}", tr_cow(command.brief.clone()));

            let (row_bg, row_fg) = if display_idx == selected {
                (fg.clone(), bg.clone())
            } else {
                (LinearRgba::TRANSPARENT.into(), fg.clone())
            };

            let mut row = vec![Element::new(&font, ElementContent::Text(label))
                .min_width(Some(Dimension::Percent(1.)))];

            if !command.keys.is_empty() {
                let key_label = format_key_label(&command.keys, &term_window.config);
                row.push(
                    Element::new(&font, ElementContent::Text(key_label))
                        .float(Float::Right)
                        .padding(BoxDimension {
                            left: Dimension::Cells(1.25),
                            right: Dimension::Cells(0.5),
                            top: Dimension::Cells(0.),
                            bottom: Dimension::Cells(0.),
                        })
                        .zindex(10)
                        .colors(ElementColors {
                            border: BorderColor::default(),
                            bg: if display_idx == selected {
                                bg.clone()
                            } else {
                                fg.clone()
                            },
                            text: if display_idx == selected {
                                fg.clone()
                            } else {
                                bg.clone()
                            },
                        }),
                );
            }

            rows.push(
                Element::new(&font, ElementContent::Children(row))
                    .colors(ElementColors {
                        border: BorderColor::default(),
                        bg: row_bg,
                        text: row_fg,
                    })
                    .padding(BoxDimension {
                        left: Dimension::Cells(0.25),
                        right: Dimension::Cells(0.25),
                        top: Dimension::Cells(0.),
                        bottom: Dimension::Cells(0.),
                    })
                    .min_width(Some(Dimension::Percent(1.)))
                    .display(DisplayType::Block)
                    .item_type(UIItemType::Modal(display_idx)),
            );
        }

        rows.push(
            Element::new(
                &font,
                ElementContent::Text(tr("↑↓ scroll  Esc close").into_owned()),
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

        let dimensions = term_window.dimensions;
        let size = term_window.terminal_size;
        let desired_width = (size.cols / 2).max(100).min(size.cols);
        let avail_pixel_width =
            size.cols as f32 * term_window.render_metrics.cell_size.width as f32;
        let desired_pixel_width =
            desired_width as f32 * term_window.render_metrics.cell_size.width as f32;

        let element = Element::new(&font, ElementContent::Children(rows))
            .colors(ElementColors {
                border: BorderColor::new(
                    term_window
                        .config
                        .command_palette_bg_color
                        .to_linear()
                        .into(),
                ),
                bg: term_window
                    .config
                    .command_palette_bg_color
                    .to_linear()
                    .into(),
                text: term_window
                    .config
                    .command_palette_fg_color
                    .to_linear()
                    .into(),
            })
            .margin(BoxDimension {
                left: Dimension::Cells(0.25),
                right: Dimension::Cells(0.25),
                top: Dimension::Cells(0.25),
                bottom: Dimension::Cells(0.25),
            })
            .padding(BoxDimension::new(Dimension::Cells(0.25)))
            .border(BoxDimension::new(Dimension::Pixels(1.)))
            .min_width(Some(Dimension::Pixels(desired_pixel_width)))
            // 外框自己也进 hit map：内边距/边框/外边距那一圈不属于任何行，
            // 点在那里会被「点浮层外即关闭」误判成点外面（WZ-06）
            .item_type(UIItemType::Modal(MODAL_CHROME_ROW));

        let x_adjust = ((avail_pixel_width - padding_left) - desired_pixel_width) / 2.;

        let computed = term_window.compute_element(
            &LayoutContext {
                height: DimensionContext {
                    dpi: dimensions.dpi as f32,
                    pixel_max: dimensions.pixel_height as f32,
                    pixel_cell: metrics.cell_size.height as f32,
                },
                width: DimensionContext {
                    dpi: dimensions.dpi as f32,
                    pixel_max: dimensions.pixel_width as f32,
                    pixel_cell: metrics.cell_size.width as f32,
                },
                bounds: euclid::rect(
                    padding_left + x_adjust,
                    top_pixel_y,
                    desired_pixel_width,
                    size.rows as f32 * term_window.render_metrics.cell_size.height as f32,
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

impl Modal for KeybindsOverlay {
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
        if row == MODAL_CHROME_ROW {
            return Ok(());
        }
        // WZ-11：滚轮滚视口
        if let WMEK::VertWheel(amount) = event.kind {
            self.scroll_rows(-(amount as isize));
            term_window.invalidate_modal();
            return Ok(());
        }
        if let WMEK::Move = event.kind {
            if row < self.commands.len() && *self.selected.borrow() != row {
                self.selected.replace(row);
                let mut top_row = self.top_row.borrow_mut();
                let max_rows = *self.max_rows_on_screen.borrow();
                if row < *top_row {
                    *top_row = row;
                } else if max_rows > 0 && row >= *top_row + max_rows {
                    *top_row = row.saturating_sub(max_rows - 1);
                }
                term_window.invalidate_modal();
            }
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
            _ => return Ok(false),
        }
        term_window.invalidate_modal();
        Ok(true)
    }

    fn computed_element(
        &self,
        term_window: &mut TermWindow,
    ) -> anyhow::Result<Ref<'_, [ComputedElement]>> {
        if self.element.borrow().is_none() {
            let element = self.compute(term_window)?;
            self.element.borrow_mut().replace(element);
        }
        Ok(Ref::map(self.element.borrow(), |v| {
            v.as_ref().unwrap().as_slice()
        }))
    }

    fn reconfigure(&self, _term_window: &mut TermWindow) {
        self.element.borrow_mut().take();
    }
}

/// Convenience: open the keybinding cheat sheet modally
pub fn open_keybinds(term_window: &mut TermWindow) {
    let modal = Rc::new(KeybindsOverlay::new(term_window));
    term_window.set_modal(modal);
    if let Some(window) = term_window.window.as_ref() {
        window.invalidate();
    }
}
