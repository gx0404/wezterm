use crate::termwindow::{PaneInformation, TabInformation, UIItem, UIItemType};
use config::{ConfigHandle, TabBarColors};
use finl_unicode::grapheme_clusters::Graphemes;
use mlua::{FromLua, IntoLua};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use termwiz::cell::{unicode_column_width, Cell, CellAttributes};
use termwiz::color::{AnsiColor, ColorSpec};
use termwiz::escape::csi::Sgr;
use termwiz::escape::parser::Parser;
use termwiz::escape::{Action, ControlCode, CSI};
use termwiz::surface::SEQ_ZERO;
use termwiz_funcs::{format_as_escapes, FormatColor, FormatItem};
use wezterm_term::{Line, Progress};
use window::{IntegratedTitleButton, IntegratedTitleButtonAlignment, IntegratedTitleButtonStyle};

#[derive(Clone, Debug, PartialEq)]
pub struct TabBarState {
    line: Line,
    items: Vec<TabEntry>,
    /// When a tab shows the built-in indeterminate progress spinner, the instant
    /// at which the tab bar should be rebuilt to advance to the next frame;
    /// None when no spinner is visible.
    next_progress_frame_due: Option<Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabBarItem {
    None,
    LeftStatus,
    RightStatus,
    Tab {
        tab_idx: usize,
        active: bool,
    },
    NewTabButton,
    /// fork: the `☰` main-menu button at the right end of the bar
    MenuButton,
    WindowButton(IntegratedTitleButton),
}

/// fork: the main-menu button's label. Keep the text and its display
/// width in one place: the hover zone, the tab-width budget and the
/// rendered line must agree (WZ-13/WZ-14).
const MENU_BUTTON_TEXT: &str = " ☰ ";

/// Display width of `MENU_BUTTON_TEXT` in cells (☰ is 3 bytes and
/// double-width; the two spaces are one cell each, 4 cells total).
fn menu_button_display_cells() -> usize {
    unicode_column_width(MENU_BUTTON_TEXT, None)
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabEntry {
    pub item: TabBarItem,
    pub title: Line,
    x: usize,
    width: usize,
}

#[derive(Clone, Debug)]
struct TitleText {
    items: Vec<FormatItem>,
    len: usize,
    /// True when the built-in title path rendered an indeterminate spinner
    /// frame. A custom format-tab-title callback always leaves this false.
    has_indeterminate: bool,
}

/// fork: Lua arguments shared by every format-tab-title and
/// format-window-title call of one title rebuild (2N+1 calls for N tabs).
/// The tabs/panes sequences and the config table are built once; cloning
/// an mlua Value only takes another reference. Previously every call
/// rebuilt both sequences (O(N^2) per rebuild) and converted the whole
/// Config into a fresh Lua table.
///
/// Handlers therefore all receive the *same* tables: they should treat
/// them as read-only. A handler that mutates one is seen by the later
/// calls of the rebuild, and for the config table by later rebuilds too
/// (see `cached_config_table`). The TabInformation/PaneInformation
/// elements are userdata without setters, so those cannot be modified.
pub struct TitleFormatArgs<'lua> {
    lua: &'lua mlua::Lua,
    tabs: mlua::Table<'lua>,
    panes: mlua::Table<'lua>,
    config: mlua::Value<'lua>,
}

impl<'lua> TitleFormatArgs<'lua> {
    pub fn new(
        lua: &'lua mlua::Lua,
        tab_info: &[TabInformation],
        pane_info: &[PaneInformation],
        config: mlua::Value<'lua>,
    ) -> mlua::Result<Self> {
        Ok(Self {
            lua,
            tabs: lua.create_sequence_from(tab_info.iter().cloned())?,
            panes: lua.create_sequence_from(pane_info.iter().cloned())?,
            config,
        })
    }

    pub fn lua(&self) -> &'lua mlua::Lua {
        self.lua
    }

    pub fn tabs(&self) -> mlua::Table<'lua> {
        self.tabs.clone()
    }

    pub fn panes(&self) -> mlua::Table<'lua> {
        self.panes.clone()
    }

    pub fn config(&self) -> mlua::Value<'lua> {
        self.config.clone()
    }
}

/// fork: the config table handed to the format-* callbacks, kept in the
/// Lua registry so that it is converted once per configuration instead of
/// on each of the 2N+1 calls of every title rebuild.
/// The owner (one per TermWindow, so per-window overrides never share an
/// entry) must drop it from `config_was_reloaded`; the generation check
/// is a second line of defence. A replaced Lua state (config reload) does
/// not own the key any more, which also forces a rebuild.
pub struct TitleConfigTableCache {
    generation: usize,
    key: mlua::RegistryKey,
}

pub fn cached_config_table<'lua>(
    lua: &'lua mlua::Lua,
    cache: &mut Option<TitleConfigTableCache>,
    config: &ConfigHandle,
) -> mlua::Result<mlua::Value<'lua>> {
    if let Some(entry) = cache.as_ref() {
        if entry.generation == config.generation() && lua.owns_registry_value(&entry.key) {
            return lua.registry_value(&entry.key);
        }
    }
    let value = (**config).clone().into_lua(lua)?;
    let key = lua.create_registry_value(value.clone())?;
    *cache = Some(TitleConfigTableCache {
        generation: config.generation(),
        key,
    });
    Ok(value)
}

/// fork: format-window-title over the shared `TitleFormatArgs`; errors
/// are logged and fall back to the built-in title (None), as before.
pub fn call_format_window_title(
    args: &TitleFormatArgs,
    active_tab: &Option<TabInformation>,
    active_pane: &Option<PaneInformation>,
) -> Option<String> {
    let lua = args.lua();
    let result = (|| -> anyhow::Result<Option<String>> {
        let v = config::lua::emit_sync_callback(
            lua,
            (
                "format-window-title".to_string(),
                (
                    active_tab.clone(),
                    active_pane.clone(),
                    args.tabs(),
                    args.panes(),
                    args.config(),
                ),
            ),
        )?;
        match &v {
            mlua::Value::Nil => Ok(None),
            _ => Ok(Some(String::from_lua(v, lua)?)),
        }
    })();
    match result {
        Ok(title) => title,
        Err(err) => {
            log::warn!("format-window-title: {}", err);
            None
        }
    }
}

fn call_format_tab_title(
    tab_idx: usize,
    args: &TitleFormatArgs,
    hover: bool,
    tab_max_width: usize,
) -> Option<TitleText> {
    let lua = args.lua();
    let result = (|| -> anyhow::Result<Option<TitleText>> {
        // The element of the shared tabs sequence is the same userdata a
        // fresh conversion of tab_info[tab_idx] would produce
        let tab: mlua::Value = args.tabs.raw_get(tab_idx + 1)?;
        let v = config::lua::emit_sync_callback(
            lua,
            (
                "format-tab-title".to_string(),
                (
                    tab,
                    args.tabs(),
                    args.panes(),
                    args.config(),
                    hover,
                    tab_max_width,
                ),
            ),
        )?;
        match &v {
            mlua::Value::Nil => Ok(None),
            mlua::Value::Table(_) => {
                let items = <Vec<FormatItem>>::from_lua(v, lua)?;

                let esc = format_as_escapes(items.clone())?;
                let line = parse_status_text(&esc, CellAttributes::default());

                Ok(Some(TitleText {
                    items,
                    len: line.len(),
                    has_indeterminate: false,
                }))
            }
            _ => {
                let s = String::from_lua(v, lua)?;
                let line = parse_status_text(&s, CellAttributes::default());
                Ok(Some(TitleText {
                    len: line.len(),
                    items: vec![FormatItem::Text(s)],
                    has_indeterminate: false,
                }))
            }
        }
    })();
    match result {
        Ok(s) => s,
        Err(err) => {
            log::warn!("format-tab-title: {}", err);
            None
        }
    }
}

/// pct is a percentage in the range 0-100.
/// We want to map it to one of the nerdfonts:
///
/// * `md-checkbox_blank_circle_outline` (0xf0130) for an empty circle
/// * `md_circle_slice_1..=7` (0xf0a9e ..= 0xf0aa4) for a partly filled
///   circle
/// * `md_circle_slice_8` (0xf0aa5) for a filled circle
///
/// We use an empty circle for values close to 0%, a filled circle for values
/// close to 100%, and a partly filled circle for the rest (roughly evenly
/// distributed).
fn pct_to_glyph(pct: u8) -> char {
    match pct {
        0..=5 => '\u{f0130}',    // empty circle
        6..=18 => '\u{f0a9e}',   // centered at 12 (slightly smaller than 12.5)
        19..=31 => '\u{f0a9f}',  // centered at 25
        32..=43 => '\u{f0aa0}',  // centered at 37.5
        44..=56 => '\u{f0aa1}',  // half-filled circle, centered at 50
        57..=68 => '\u{f0aa2}',  // centered at 62.5
        69..=81 => '\u{f0aa3}',  // centered at 75
        82..=94 => '\u{f0aa4}',  // centered at 88 (slightly larger than 87.5)
        95..=100 => '\u{f0aa5}', // filled circle
        // Any other value is mapped to a filled circle.
        _ => '\u{f0aa5}',
    }
}

/// How long each indeterminate progress spinner frame is shown before the next
/// is due.
const INDETERMINATE_SPINNER_INTERVAL: Duration = Duration::from_millis(100);

/// Reference instant for the indeterminate spinner. The displayed frame and its
/// next-due time are both derived from the elapsed time since this instant, so
/// they stay in step no matter when a repaint happens to rebuild the tab bar.
static SPINNER_EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Renders `value` as a braille cell whose lit dots count up in the cell's
/// reading order, down the left column then down the right column. Stepping
/// `value` through 0..=255 reproduces the `dots8Bit` animation of
/// https://github.com/sindresorhus/cli-spinners without a lookup table.
fn braille_counter(value: u8) -> char {
    // Unicode braille dot bit values in reading order: dots 1, 2, 3, 7 fill the
    // left column and dots 4, 5, 6, 8 the right column.
    const DOTS: [u32; 8] = [0x01, 0x02, 0x04, 0x40, 0x08, 0x10, 0x20, 0x80];
    let mut pattern = 0u32;
    for (bit, dot) in DOTS.iter().enumerate() {
        if value & (1 << bit) != 0 {
            pattern |= dot;
        }
    }
    char::from_u32(0x2800 + pattern).expect("braille pattern is a valid codepoint")
}

/// Returns the spinner glyph to show for the current moment, advancing one
/// frame per INDETERMINATE_SPINNER_INTERVAL. `seed` offsets the starting frame
/// so that tabs busy at the same time do not animate in lock step.
fn indeterminate_spinner_glyph(seed: u64) -> char {
    let elapsed = SPINNER_EPOCH.elapsed().as_millis() as u64;
    let interval = INDETERMINATE_SPINNER_INTERVAL.as_millis() as u64;
    // braille_counter wraps at 256, matching the animation's frame count.
    braille_counter((elapsed / interval + seed) as u8)
}

/// Returns the instant at which the spinner next advances a frame, snapped to
/// the frame grid measured from SPINNER_EPOCH. Because the result falls on a
/// grid boundary rather than a fixed offset from now, repeated rebuilds within
/// one frame all return the same instant and the animation advances steadily
/// even when unrelated repaints rebuild the tab bar in between.
fn next_spinner_frame_due() -> Instant {
    let interval = INDETERMINATE_SPINNER_INTERVAL.as_nanos();
    let elapsed = SPINNER_EPOCH.elapsed().as_nanos();
    let next_frame = elapsed / interval + 1;
    *SPINNER_EPOCH + Duration::from_nanos((next_frame * interval) as u64)
}

/// Scrambles a tab id into a spinner phase offset. Tab ids are usually handed
/// out sequentially, which would leave adjacent tabs only one frame apart; the
/// splitmix64 finalizer avalanches the low bits so their spinners spread across
/// the animation instead.
fn spinner_phase(tab_id: usize) -> u64 {
    let mut z = tab_id as u64;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

fn compute_tab_title(
    tab_idx: usize,
    tab_info: &[TabInformation],
    lua_args: Option<&TitleFormatArgs>,
    config: &ConfigHandle,
    hover: bool,
    tab_max_width: usize,
) -> TitleText {
    let tab = &tab_info[tab_idx];
    let title =
        lua_args.and_then(|args| call_format_tab_title(tab_idx, args, hover, tab_max_width));

    match title {
        Some(title) => title,
        None => {
            let mut items = vec![];
            let mut len = 0;
            let mut has_indeterminate = false;

            if let Some(pane) = &tab.active_pane {
                let mut title = if tab.tab_title.is_empty() {
                    pane.title.clone()
                } else {
                    tab.tab_title.clone()
                };

                let classic_spacing = if config.use_fancy_tab_bar { "" } else { " " };
                if config.show_tab_index_in_tab_bar {
                    let index = format!(
                        "{classic_spacing}{}: ",
                        tab.tab_index
                            + if config.tab_and_split_indices_are_zero_based {
                                0
                            } else {
                                1
                            }
                    );
                    len += unicode_column_width(&index, None);
                    items.push(FormatItem::Text(index));

                    title = format!("{}{classic_spacing}", title);
                }

                match pane.progress {
                    Progress::None => {}
                    Progress::Percentage(pct) | Progress::Error(pct) => {
                        let graphic = format!("{} ", pct_to_glyph(pct));
                        len += unicode_column_width(&graphic, None);
                        let color = if matches!(pane.progress, Progress::Percentage(_)) {
                            FormatItem::Foreground(FormatColor::AnsiColor(AnsiColor::Green))
                        } else {
                            FormatItem::Foreground(FormatColor::AnsiColor(AnsiColor::Red))
                        };
                        items.push(color);
                        items.push(FormatItem::Text(graphic));
                        items.push(FormatItem::Foreground(FormatColor::Default));
                    }
                    Progress::Indeterminate => {
                        has_indeterminate = true;
                        let graphic = format!(
                            "{} ",
                            indeterminate_spinner_glyph(spinner_phase(tab.tab_id))
                        );
                        len += unicode_column_width(&graphic, None);
                        items.push(FormatItem::Foreground(FormatColor::AnsiColor(
                            AnsiColor::Green,
                        )));
                        items.push(FormatItem::Text(graphic));
                        items.push(FormatItem::Foreground(FormatColor::Default));
                    }
                }

                // We have a preferred soft minimum on tab width to make it
                // easier to click on tab titles, but we'll still go below
                // this if there are too many tabs to fit the window at
                // this width.
                if !config.use_fancy_tab_bar {
                    while len + unicode_column_width(&title, None) < 5 {
                        title.push(' ');
                    }
                }

                len += unicode_column_width(&title, None);
                items.push(FormatItem::Text(title));
            } else {
                let title = " no pane ".to_string();
                len += unicode_column_width(&title, None);
                items.push(FormatItem::Text(title));
            };

            TitleText {
                len,
                items,
                has_indeterminate,
            }
        }
    }
}

fn is_tab_hover(mouse_x: Option<usize>, x: usize, tab_title_len: usize) -> bool {
    return mouse_x
        .map(|mouse_x| mouse_x >= x && mouse_x < x + tab_title_len)
        .unwrap_or(false);
}

impl TabBarState {
    pub fn default() -> Self {
        Self {
            line: Line::with_width(1, SEQ_ZERO),
            items: vec![TabEntry {
                item: TabBarItem::None,
                title: Line::from_text(" ", &CellAttributes::blank(), 1, None),
                x: 1,
                width: 1,
            }],
            next_progress_frame_due: None,
        }
    }

    pub fn line(&self) -> &Line {
        &self.line
    }

    pub fn items(&self) -> &[TabEntry] {
        &self.items
    }

    pub fn next_progress_frame_due(&self) -> Option<Instant> {
        self.next_progress_frame_due
    }

    fn integrated_title_buttons(
        mouse_x: Option<usize>,
        x: &mut usize,
        config: &ConfigHandle,
        items: &mut Vec<TabEntry>,
        line: &mut Line,
        colors: &TabBarColors,
    ) {
        let default_cell = if config.use_fancy_tab_bar {
            CellAttributes::default()
        } else {
            colors.new_tab().as_cell_attributes()
        };

        let default_cell_hover = if config.use_fancy_tab_bar {
            CellAttributes::default()
        } else {
            colors.new_tab_hover().as_cell_attributes()
        };

        let window_hide =
            parse_status_text(&config.tab_bar_style.window_hide, default_cell.clone());
        let window_hide_hover = parse_status_text(
            &config.tab_bar_style.window_hide_hover,
            default_cell_hover.clone(),
        );

        let window_maximize =
            parse_status_text(&config.tab_bar_style.window_maximize, default_cell.clone());
        let window_maximize_hover = parse_status_text(
            &config.tab_bar_style.window_maximize_hover,
            default_cell_hover.clone(),
        );

        let window_close =
            parse_status_text(&config.tab_bar_style.window_close, default_cell.clone());
        let window_close_hover = parse_status_text(
            &config.tab_bar_style.window_close_hover,
            default_cell_hover.clone(),
        );

        for button in &config.integrated_title_buttons {
            use IntegratedTitleButton as Button;
            let title = match button {
                Button::Hide => {
                    let hover = is_tab_hover(mouse_x, *x, window_hide_hover.len());

                    if hover {
                        &window_hide_hover
                    } else {
                        &window_hide
                    }
                }
                Button::Maximize => {
                    let hover = is_tab_hover(mouse_x, *x, window_maximize_hover.len());

                    if hover {
                        &window_maximize_hover
                    } else {
                        &window_maximize
                    }
                }
                Button::Close => {
                    let hover = is_tab_hover(mouse_x, *x, window_close_hover.len());

                    if hover {
                        &window_close_hover
                    } else {
                        &window_close
                    }
                }
            };

            line.append_line(title.to_owned(), SEQ_ZERO);

            let width = title.len();
            items.push(TabEntry {
                item: TabBarItem::WindowButton(*button),
                title: title.to_owned(),
                x: *x,
                width,
            });

            *x += width;
        }
    }

    /// Build a new tab bar from the current state
    /// mouse_x is some if the mouse is on the same row as the tab bar.
    /// title_width is the total number of cell columns in the window.
    /// window allows access to the tabs associated with the window.
    /// fork: `lua_args` carries the shared Lua arguments for the
    /// format-tab-title calls; None means there is no Lua config and
    /// the built-in titles are used.
    pub fn new(
        title_width: usize,
        mouse_x: Option<usize>,
        tab_info: &[TabInformation],
        lua_args: Option<&TitleFormatArgs>,
        colors: Option<&TabBarColors>,
        config: &ConfigHandle,
        left_status: &str,
        right_status: &str,
    ) -> Self {
        let colors = colors.cloned().unwrap_or_else(TabBarColors::default);

        let active_cell_attrs = colors.active_tab().as_cell_attributes();
        let inactive_hover_attrs = colors.inactive_tab_hover().as_cell_attributes();
        let inactive_cell_attrs = colors.inactive_tab().as_cell_attributes();
        let new_tab_hover_attrs = colors.new_tab_hover().as_cell_attributes();
        let new_tab_attrs = colors.new_tab().as_cell_attributes();

        let new_tab = parse_status_text(
            &config.tab_bar_style.new_tab,
            if config.use_fancy_tab_bar {
                CellAttributes::default()
            } else {
                new_tab_attrs.clone()
            },
        );
        let new_tab_hover = parse_status_text(
            &config.tab_bar_style.new_tab_hover,
            if config.use_fancy_tab_bar {
                CellAttributes::default()
            } else {
                new_tab_hover_attrs.clone()
            },
        );

        let use_integrated_title_buttons = config
            .window_decorations
            .contains(window::WindowDecorations::INTEGRATED_BUTTONS);

        // We ultimately want to produce a line looking like this:
        // ` | tab1-title x | tab2-title x |  +      . - X `
        // Where the `+` sign will spawn a new tab (or show a context
        // menu with tab creation options) and the other three chars
        // are symbols representing minimize, maximize and close.

        let mut active_tab_no = 0;

        let tab_titles: Vec<TitleText> = if config.show_tabs_in_tab_bar {
            tab_info
                .iter()
                .enumerate()
                .map(|(tab_idx, tab)| {
                    if tab.is_active {
                        active_tab_no = tab.tab_index;
                    }
                    compute_tab_title(
                        tab_idx,
                        tab_info,
                        lua_args,
                        config,
                        false,
                        config.tab_max_width,
                    )
                })
                .collect()
        } else {
            vec![]
        };
        let titles_len: usize = tab_titles.iter().map(|s| s.len).sum();
        let number_of_tabs = tab_titles.len();

        // fork (WZ-14): the tab width budget must also reserve the ☰ menu
        // button, otherwise a crowded bar pushes it (and the right
        // status area) past the right edge of the line.
        let menu_button_cells = if config.show_menu_button_in_tab_bar {
            menu_button_display_cells()
        } else {
            0
        };
        let available_cells = title_width
            .saturating_sub(number_of_tabs.saturating_sub(1) + new_tab.len() + menu_button_cells);
        let tab_width_max = if config.use_fancy_tab_bar || available_cells >= titles_len {
            // We can render each title with its full width
            usize::max_value()
        } else {
            // We need to clamp the length to balance them out
            available_cells / number_of_tabs
        }
        .min(config.tab_max_width);

        let mut line = Line::with_width(0, SEQ_ZERO);

        let mut x = 0;
        let mut items = vec![];
        let mut has_indeterminate_progress = false;

        let black_cell = Cell::blank_with_attrs(
            CellAttributes::default()
                .set_background(ColorSpec::TrueColor(*colors.background()))
                .clone(),
        );

        if use_integrated_title_buttons
            && config.integrated_title_button_style == IntegratedTitleButtonStyle::MacOsNative
            && config.use_fancy_tab_bar == false
            && config.tab_bar_at_bottom == false
        {
            for _ in 0..10 as usize {
                line.insert_cell(0, black_cell.clone(), title_width, SEQ_ZERO);
                x += 1;
            }
        }

        if use_integrated_title_buttons
            && config.integrated_title_button_style != IntegratedTitleButtonStyle::MacOsNative
            && config.integrated_title_button_alignment == IntegratedTitleButtonAlignment::Left
        {
            Self::integrated_title_buttons(mouse_x, &mut x, config, &mut items, &mut line, &colors);
        }

        let left_status_line = parse_status_text(left_status, black_cell.attrs().clone());
        if left_status_line.len() > 0 {
            items.push(TabEntry {
                item: TabBarItem::LeftStatus,
                title: left_status_line.clone(),
                x,
                width: left_status_line.len(),
            });
            x += left_status_line.len();
            line.append_line(left_status_line, SEQ_ZERO);
        }

        for (tab_idx, tab_title) in tab_titles.iter().enumerate() {
            let tab_title_len = tab_title.len.min(tab_width_max);
            let active = tab_idx == active_tab_no;
            let hover = !active && is_tab_hover(mouse_x, x, tab_title_len);

            // Recompute the title so that it factors in both the hover state
            // and the adjusted maximum tab width based on available space.
            let tab_title =
                compute_tab_title(tab_idx, tab_info, lua_args, config, hover, tab_title_len);

            let cell_attrs = if active {
                &active_cell_attrs
            } else if hover {
                &inactive_hover_attrs
            } else {
                &inactive_cell_attrs
            };

            let tab_start_idx = x;

            has_indeterminate_progress |= tab_title.has_indeterminate;

            let esc = format_as_escapes(tab_title.items.clone()).expect("already parsed ok above");
            let mut tab_line = parse_status_text(
                &esc,
                if config.use_fancy_tab_bar {
                    CellAttributes::default()
                } else {
                    cell_attrs.clone()
                },
            );

            let title = tab_line.clone();
            if tab_line.len() > tab_width_max {
                tab_line.resize(tab_width_max, SEQ_ZERO);
            }

            let width = tab_line.len();

            items.push(TabEntry {
                item: TabBarItem::Tab { tab_idx, active },
                title,
                x: tab_start_idx,
                width,
            });

            line.append_line(tab_line, SEQ_ZERO);
            x += width;
        }

        // New tab button
        if config.show_new_tab_button_in_tab_bar {
            let hover = is_tab_hover(mouse_x, x, new_tab_hover.len());

            let new_tab_button = if hover { &new_tab_hover } else { &new_tab };

            let button_start = x;
            let width = new_tab_button.len();

            line.append_line(new_tab_button.clone(), SEQ_ZERO);

            items.push(TabEntry {
                item: TabBarItem::NewTabButton,
                title: new_tab_button.clone(),
                x: button_start,
                width,
            });

            x += width;
        }

        // Main menu button (fork): opens the herdr-style main menu
        if config.show_menu_button_in_tab_bar {
            // fork (WZ-13): the hover zone must use the rendered cell width;
            // " ☰ " is 5 bytes but only MENU_BUTTON_DISPLAY_CELLS wide.
            let hover = is_tab_hover(mouse_x, x, menu_button_display_cells());
            // fork: hover like the `+` button (new_tab_hover) instead of
            // reverse video; the fancy bar styles the button itself
            let attrs = if config.use_fancy_tab_bar {
                CellAttributes::default()
            } else if hover {
                new_tab_hover_attrs.clone()
            } else {
                new_tab_attrs.clone()
            };
            let menu_button = parse_status_text(MENU_BUTTON_TEXT, attrs);
            let button_start = x;
            let width = menu_button.len();

            line.append_line(menu_button.clone(), SEQ_ZERO);

            items.push(TabEntry {
                item: TabBarItem::MenuButton,
                title: menu_button,
                x: button_start,
                width,
            });

            x += width;
        }

        // Reserve place for integrated title buttons
        let title_width = if use_integrated_title_buttons
            && config.integrated_title_button_style != IntegratedTitleButtonStyle::MacOsNative
            && config.integrated_title_button_alignment == IntegratedTitleButtonAlignment::Right
        {
            let window_hide =
                parse_status_text(&config.tab_bar_style.window_hide, CellAttributes::default());
            let window_hide_hover = parse_status_text(
                &config.tab_bar_style.window_hide_hover,
                CellAttributes::default(),
            );

            let window_maximize = parse_status_text(
                &config.tab_bar_style.window_maximize,
                CellAttributes::default(),
            );
            let window_maximize_hover = parse_status_text(
                &config.tab_bar_style.window_maximize_hover,
                CellAttributes::default(),
            );
            let window_close = parse_status_text(
                &config.tab_bar_style.window_close,
                CellAttributes::default(),
            );
            let window_close_hover = parse_status_text(
                &config.tab_bar_style.window_close_hover,
                CellAttributes::default(),
            );

            let hide_len = window_hide.len().max(window_hide_hover.len());
            let maximize_len = window_maximize.len().max(window_maximize_hover.len());
            let close_len = window_close.len().max(window_close_hover.len());

            let mut width_to_reserve = 0;
            for button in &config.integrated_title_buttons {
                use IntegratedTitleButton as Button;
                let button_len = match button {
                    Button::Hide => hide_len,
                    Button::Maximize => maximize_len,
                    Button::Close => close_len,
                };
                width_to_reserve += button_len;
            }

            title_width.saturating_sub(width_to_reserve)
        } else {
            title_width
        };

        let status_space_available = title_width.saturating_sub(x);

        let mut right_status_line = parse_status_text(right_status, black_cell.attrs().clone());
        items.push(TabEntry {
            item: TabBarItem::RightStatus,
            title: right_status_line.clone(),
            x,
            width: status_space_available,
        });

        while right_status_line.len() > status_space_available {
            right_status_line.remove_cell(0, SEQ_ZERO);
        }

        line.append_line(right_status_line, SEQ_ZERO);
        while line.len() < title_width {
            line.insert_cell(x, black_cell.clone(), title_width, SEQ_ZERO);
        }

        if use_integrated_title_buttons
            && config.integrated_title_button_style != IntegratedTitleButtonStyle::MacOsNative
            && config.integrated_title_button_alignment == IntegratedTitleButtonAlignment::Right
        {
            x = title_width;
            Self::integrated_title_buttons(mouse_x, &mut x, config, &mut items, &mut line, &colors);
        }

        Self {
            line,
            items,
            next_progress_frame_due: has_indeterminate_progress.then(next_spinner_frame_due),
        }
    }

    pub fn compute_ui_items(&self, y: usize, cell_height: usize, cell_width: usize) -> Vec<UIItem> {
        let mut items = vec![];

        for entry in self.items.iter() {
            items.push(UIItem {
                x: entry.x * cell_width,
                width: entry.width * cell_width,
                y,
                height: cell_height,
                item_type: UIItemType::TabBar(entry.item),
            });
        }

        items
    }
}

pub fn parse_status_text(text: &str, default_cell: CellAttributes) -> Line {
    let mut pen = default_cell.clone();
    let mut cells = vec![];
    let mut ignoring = false;
    let mut print_buffer = String::new();

    fn flush_print(buf: &mut String, cells: &mut Vec<Cell>, pen: &CellAttributes) {
        for g in Graphemes::new(buf.as_str()) {
            let cell = Cell::new_grapheme(g, pen.clone(), None);
            let width = cell.width();
            cells.push(cell);
            for _ in 1..width {
                // Line/Screen expect double wide graphemes to be followed by a blank in
                // the next column position, otherwise we'll render incorrectly
                cells.push(Cell::blank_with_attrs(pen.clone()));
            }
        }
        buf.clear();
    }

    let mut parser = Parser::new();
    parser.parse(text.as_bytes(), |action| {
        if ignoring {
            return;
        }
        match action {
            Action::Print(c) => print_buffer.push(c),
            Action::PrintString(s) => print_buffer.push_str(&s),
            Action::Control(c) => {
                flush_print(&mut print_buffer, &mut cells, &pen);
                match c {
                    ControlCode::CarriageReturn | ControlCode::LineFeed => {
                        ignoring = true;
                    }
                    _ => {}
                }
            }
            Action::CSI(csi) => {
                flush_print(&mut print_buffer, &mut cells, &pen);
                match csi {
                    CSI::Sgr(sgr) => match sgr {
                        Sgr::Reset => pen = default_cell.clone(),
                        Sgr::Intensity(i) => {
                            pen.set_intensity(i);
                        }
                        Sgr::Underline(u) => {
                            pen.set_underline(u);
                        }
                        Sgr::Overline(o) => {
                            pen.set_overline(o);
                        }
                        Sgr::VerticalAlign(o) => {
                            pen.set_vertical_align(o);
                        }
                        Sgr::Blink(b) => {
                            pen.set_blink(b);
                        }
                        Sgr::Italic(i) => {
                            pen.set_italic(i);
                        }
                        Sgr::Inverse(inverse) => {
                            pen.set_reverse(inverse);
                        }
                        Sgr::Invisible(invis) => {
                            pen.set_invisible(invis);
                        }
                        Sgr::StrikeThrough(strike) => {
                            pen.set_strikethrough(strike);
                        }
                        Sgr::Foreground(col) => {
                            if let ColorSpec::Default = col {
                                pen.set_foreground(default_cell.foreground());
                            } else {
                                pen.set_foreground(col);
                            }
                        }
                        Sgr::Background(col) => {
                            if let ColorSpec::Default = col {
                                pen.set_background(default_cell.background());
                            } else {
                                pen.set_background(col);
                            }
                        }
                        Sgr::UnderlineColor(col) => {
                            pen.set_underline_color(col);
                        }
                        Sgr::Font(_) => {}
                    },
                    _ => {}
                }
            }
            Action::OperatingSystemCommand(_)
            | Action::DeviceControl(_)
            | Action::Esc(_)
            | Action::KittyImage(_)
            | Action::XtGetTcap(_)
            | Action::Sixel(_) => {
                flush_print(&mut print_buffer, &mut cells, &pen);
            }
        }
    });
    flush_print(&mut print_buffer, &mut cells, &pen);
    Line::from_cells(cells, SEQ_ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_button_width_is_cells_not_bytes() {
        // fork (WZ-13): " ☰ " is 5 bytes but 4 cells (☰ is double-width); the
        // hover zone and the tab-width budget must agree on the cell
        // width, not the byte length
        assert_eq!(MENU_BUTTON_TEXT.len(), 5);
        assert_eq!(menu_button_display_cells(), 4);
        // hover zone spans exactly the rendered cells
        assert!(is_tab_hover(Some(3), 0, menu_button_display_cells()));
        assert!(!is_tab_hover(Some(4), 0, menu_button_display_cells()));
    }

    fn pane_info(pane_id: usize, title: &str) -> PaneInformation {
        PaneInformation {
            pane_id,
            pane_index: 0,
            is_active: true,
            is_zoomed: false,
            has_unseen_output: false,
            left: 0,
            top: 0,
            width: 80,
            height: 24,
            pixel_width: 640,
            pixel_height: 384,
            title: title.to_string(),
            user_vars: Default::default(),
            progress: Progress::None,
        }
    }

    fn tab_info(tab_index: usize) -> TabInformation {
        TabInformation {
            tab_id: tab_index,
            tab_index,
            is_active: tab_index == 0,
            is_last_active: false,
            active_pane: Some(pane_info(tab_index, "shell")),
            window_id: 0,
            tab_title: String::new(),
        }
    }

    #[test]
    fn format_tab_title_calls_share_one_set_of_lua_values() {
        // fork: every format-tab-title call of one rebuild must see the
        // very same tabs/panes/config tables instead of fresh copies
        let lua = mlua::Lua::new();
        let handler: mlua::Function = lua
            .load(
                r#"
                calls = 0
                same = true
                return function(tab, tabs, panes, config, hover, max_width)
                  calls = calls + 1
                  if first_tabs == nil then
                    first_tabs, first_panes, first_config = tabs, panes, config
                  elseif not (rawequal(tabs, first_tabs)
                      and rawequal(panes, first_panes)
                      and rawequal(config, first_config)) then
                    same = false
                  end
                  -- the tab argument is the matching element of `tabs`
                  if not rawequal(tab, tabs[tab.tab_index + 1]) then
                    same = false
                  end
                  return "t" .. tab.tab_index
                end
                "#,
            )
            .eval()
            .unwrap();
        config::lua::register_event(&lua, ("format-tab-title".to_string(), handler)).unwrap();

        let config = ConfigHandle::default_config();
        let tabs: Vec<TabInformation> = (0..3).map(tab_info).collect();
        let panes = vec![pane_info(0, "shell")];
        let mut cache = None;
        let config_value = cached_config_table(&lua, &mut cache, &config).unwrap();
        let args = TitleFormatArgs::new(&lua, &tabs, &panes, config_value).unwrap();

        let bar = TabBarState::new(80, None, &tabs, Some(&args), None, &config, "", "");

        let globals = lua.globals();
        // two passes over three tabs: the measuring pass and the final one
        assert_eq!(globals.get::<_, i64>("calls").unwrap(), 6);
        assert!(globals.get::<_, bool>("same").unwrap());
        let titles: Vec<String> = bar
            .items()
            .iter()
            .filter(|entry| matches!(entry.item, TabBarItem::Tab { .. }))
            .map(|entry| entry.title.as_str().trim().to_string())
            .collect();
        assert_eq!(titles, vec!["t0", "t1", "t2"]);
    }

    #[test]
    fn config_table_is_reused_until_the_cache_is_dropped() {
        let lua = mlua::Lua::new();
        let config = ConfigHandle::default_config();
        let mut cache = None;

        let first = cached_config_table(&lua, &mut cache, &config).unwrap();
        let second = cached_config_table(&lua, &mut cache, &config).unwrap();
        assert!(matches!(first, mlua::Value::Table(_)));
        assert_eq!(first.to_pointer(), second.to_pointer());

        // config_was_reloaded drops the cache: the next rebuild converts
        // the (possibly overridden) config again
        cache = None;
        let third = cached_config_table(&lua, &mut cache, &config).unwrap();
        assert_ne!(first.to_pointer(), third.to_pointer());

        // a replaced Lua state (config reload) does not own the old key
        let other_lua = mlua::Lua::new();
        let fresh = cached_config_table(&other_lua, &mut cache, &config).unwrap();
        assert!(matches!(fresh, mlua::Value::Table(_)));
        let again = cached_config_table(&other_lua, &mut cache, &config).unwrap();
        assert_eq!(fresh.to_pointer(), again.to_pointer());
    }
}
