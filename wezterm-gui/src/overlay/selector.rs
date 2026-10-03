use crate::overlay::quickselect;
use crate::overlay::style::{self, ListRow, OverlayStyle, RowPrefix};
use crate::scripting::guiwin::GuiWin;
use config::configuration;
use config::keyassignment::{InputSelector, InputSelectorEntry, KeyAssignment};
use mux::termwiztermtab::TermWizTerminal;
use mux_lua::MuxPane;
use nucleo_matcher::pattern::Pattern;
use nucleo_matcher::{Matcher, Utf32Str};
use rayon::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use termwiz::color::ColorAttribute;
use termwiz::input::{InputEvent, KeyCode, KeyEvent, Modifiers, MouseButtons, MouseEvent};
use termwiz::surface::{Change, Position};
use termwiz::terminal::Terminal;
use termwiz_funcs::truncate_right;

// fork: the header is a title plus a separator line; the remaining two
// rows are the inclusive item count and a spare bottom row
const ROW_OVERHEAD: usize = style::HEADER_ROWS + 2;

thread_local! {
    pub static MATCHER: RefCell<Matcher> = RefCell::new(Matcher::new(nucleo_matcher::Config::DEFAULT));
}

pub fn matcher_score(pattern: &Pattern, s: &str) -> Option<u32> {
    MATCHER.with_borrow_mut(|matcher| {
        let mut buf = vec![];
        pattern.score(Utf32Str::new(s, &mut buf), matcher)
    })
}

pub fn matcher_pattern(s: &str) -> Pattern {
    nucleo_matcher::pattern::Pattern::parse(
        s,
        nucleo_matcher::pattern::CaseMatching::Ignore,
        nucleo_matcher::pattern::Normalization::Smart,
    )
}

struct SelectorState {
    active_idx: usize,
    max_items: usize,
    top_row: usize,
    filter_term: String,
    filtered_entries: Vec<InputSelectorEntry>,
    pane: MuxPane,
    window: GuiWin,
    filtering: bool,
    always_fuzzy: bool,
    args: InputSelector,
    event_name: String,
    selection: String,
    labels: Vec<String>,
}

impl SelectorState {
    fn update_filter(&mut self) {
        if self.filter_term.is_empty() {
            self.filtered_entries = self.args.choices.clone();
            return;
        }

        self.filtered_entries.clear();

        struct MatchResult {
            row_idx: usize,
            score: u32,
        }

        let pattern = matcher_pattern(&self.filter_term);

        let mut scores: Vec<MatchResult> = self
            .args
            .choices
            .par_iter()
            .enumerate()
            .filter_map(|(row_idx, entry)| {
                let score = matcher_score(&pattern, &entry.label)?;
                Some(MatchResult { row_idx, score })
            })
            .collect();

        scores.sort_by(|a, b| a.score.cmp(&b.score).reverse());

        for result in scores {
            self.filtered_entries
                .push(self.args.choices[result.row_idx].clone());
        }

        self.active_idx = 0;
        self.top_row = 0;
    }

    fn render(&mut self, term: &mut TermWizTerminal) -> termwiz::Result<()> {
        let size = term.get_screen_size()?;
        let max_width = size.cols.saturating_sub(6);
        let max_items = size.rows.saturating_sub(ROW_OVERHEAD);
        if max_items != self.max_items {
            self.labels = quickselect::compute_labels_for_alphabet_with_preserved_case(
                &self.args.alphabet,
                self.filtered_entries.len().min(max_items + 1),
            );
            self.max_items = max_items;
        }

        let config = configuration();
        let style = OverlayStyle::from_config(&config);

        let mut changes = vec![
            Change::ClearScreen(ColorAttribute::Default),
            Change::CursorPosition {
                x: Position::Absolute(0),
                y: Position::Absolute(0),
            },
        ];
        changes.extend(style.header(
            &truncate_right(&self.args.description, max_width),
            size.cols,
        ));

        let labels = &self.labels;
        let max_label_len = labels.iter().map(|s| s.len()).max().unwrap_or(0);
        let mut labels_iter = labels.into_iter();

        let colors = &config.resolved_palette;
        let input_selector_label_fg = colors.input_selector_label_fg.map(ColorAttribute::from);
        let input_selector_label_bg = colors.input_selector_label_bg.map(ColorAttribute::from);

        for (row_num, (entry_idx, entry)) in self
            .filtered_entries
            .iter()
            .enumerate()
            .skip(self.top_row)
            .enumerate()
        {
            if row_num > max_items {
                break;
            }

            // from above we know that row_num <= max_items
            // show labels as long as we have more labels left
            // and we are not filtering
            let prefix = if !self.filtering {
                match labels_iter.next() {
                    Some(label) => RowPrefix::Label {
                        label,
                        width: max_label_len,
                        fg: input_selector_label_fg,
                        bg: input_selector_label_bg,
                    },
                    None => RowPrefix::Blank(max_label_len + 3),
                }
            } else if !self.always_fuzzy {
                RowPrefix::Blank(max_label_len + 3)
            } else {
                RowPrefix::Blank(4)
            };

            changes.extend(style.list_row(&ListRow {
                prefix,
                text: &entry.label,
                max_width,
                active: entry_idx == self.active_idx,
            }));
            changes.push(Change::Text("\r\n".to_string()));
        }

        if self.filtering || !self.filter_term.is_empty() {
            changes.extend(style.prompt_line(&truncate_right(
                &format!("{}{}", self.args.fuzzy_description, self.filter_term),
                max_width,
            )));
        }

        term.render(&changes)
    }

    fn trigger_event(&self, entry: Option<InputSelectorEntry>) {
        let name = self.event_name.clone();
        let window = self.window.clone();
        let pane = self.pane.clone();

        promise::spawn::spawn_into_main_thread(async move {
            trampoline(name, window, pane, entry);
            anyhow::Result::<()>::Ok(())
        })
        .detach();
    }

    fn launch(&self, active_idx: usize) -> bool {
        if let Some(entry) = self.filtered_entries.get(active_idx).cloned() {
            self.trigger_event(Some(entry));
            true
        } else {
            false
        }
    }

    fn move_up(&mut self) {
        self.active_idx = self.active_idx.saturating_sub(1);
        if self.active_idx < self.top_row {
            self.top_row = self.active_idx;
        }
    }

    fn move_down(&mut self) {
        self.active_idx = (self.active_idx + 1).min(self.filtered_entries.len() - 1);
        if self.active_idx > self.top_row + self.max_items {
            self.top_row = self.active_idx.saturating_sub(self.max_items);
        }
    }

    fn run_loop(&mut self, term: &mut TermWizTerminal) -> anyhow::Result<()> {
        while let Ok(Some(event)) = term.poll_input(None) {
            match event {
                InputEvent::Key(KeyEvent {
                    key: KeyCode::Char(c),
                    modifiers: Modifiers::NONE,
                }) if !self.filtering && self.args.alphabet.contains(c) => {
                    self.selection.push(c);
                    if let Some(pos) = self.labels.iter().position(|x| *x == self.selection) {
                        // since the number of labels is always <= self.max_items
                        // by construction, we have pos as usize <= self.max_items
                        // for free
                        self.active_idx = self.top_row + pos as usize;
                        if self.launch(self.active_idx) {
                            break;
                        }
                    }
                }
                InputEvent::Key(KeyEvent {
                    key: KeyCode::Char('j'),
                    ..
                }) if !self.filtering => {
                    self.move_down();
                }
                InputEvent::Key(KeyEvent {
                    key: KeyCode::Char('k'),
                    ..
                }) if !self.filtering => {
                    self.move_up();
                }
                InputEvent::Key(KeyEvent {
                    key: KeyCode::Char('P' | 'K'),
                    modifiers: Modifiers::CTRL,
                }) => {
                    self.move_up();
                }
                InputEvent::Key(KeyEvent {
                    key: KeyCode::Char('N' | 'J'),
                    modifiers: Modifiers::CTRL,
                }) => {
                    self.move_down();
                }
                InputEvent::Key(KeyEvent {
                    key: KeyCode::Char('/'),
                    ..
                }) if !self.filtering => {
                    self.filtering = true;
                }
                InputEvent::Key(KeyEvent {
                    key: KeyCode::Backspace,
                    ..
                }) => {
                    if !self.filtering {
                        self.selection.pop();
                    } else {
                        if self.filter_term.pop().is_none() && !self.always_fuzzy {
                            self.filtering = false;
                        }
                        self.update_filter();
                    }
                }
                InputEvent::Key(KeyEvent {
                    key: KeyCode::Char('G' | 'C'),
                    modifiers: Modifiers::CTRL,
                })
                | InputEvent::Key(KeyEvent {
                    key: KeyCode::Escape,
                    ..
                }) => {
                    self.trigger_event(None);
                    break;
                }
                InputEvent::Key(KeyEvent {
                    key: KeyCode::Char(c),
                    ..
                }) if self.filtering => {
                    self.filter_term.push(c);
                    self.update_filter();
                }
                InputEvent::Key(KeyEvent {
                    key: KeyCode::UpArrow,
                    ..
                }) => {
                    self.move_up();
                }
                InputEvent::Key(KeyEvent {
                    key: KeyCode::DownArrow,
                    ..
                }) => {
                    self.move_down();
                }
                InputEvent::Mouse(MouseEvent {
                    y, mouse_buttons, ..
                }) if mouse_buttons.contains(MouseButtons::VERT_WHEEL) => {
                    if mouse_buttons.contains(MouseButtons::WHEEL_POSITIVE) {
                        self.top_row = self.top_row.saturating_sub(1);
                    } else {
                        self.top_row += 1;
                        self.top_row = self.top_row.min(
                            self.filtered_entries
                                .len()
                                .saturating_sub(self.max_items)
                                .saturating_sub(1),
                        );
                    }
                    if let Some(idx) =
                        style::entry_at_row(y as usize, self.top_row, self.filtered_entries.len())
                    {
                        self.active_idx = idx;
                    }
                }
                InputEvent::Mouse(MouseEvent {
                    y, mouse_buttons, ..
                }) => {
                    if let Some(idx) =
                        style::entry_at_row(y as usize, self.top_row, self.filtered_entries.len())
                    {
                        self.active_idx = idx;

                        if mouse_buttons == MouseButtons::LEFT {
                            if self.launch(self.active_idx) {
                                break;
                            }
                        }
                    }
                    if mouse_buttons != MouseButtons::NONE {
                        // Treat any other mouse button as cancel
                        self.trigger_event(None);
                        break;
                    }
                }
                InputEvent::Key(KeyEvent {
                    key: KeyCode::Enter,
                    ..
                }) => {
                    if self.launch(self.active_idx) {
                        break;
                    }
                }
                _ => {}
            }
            self.render(term)?;
        }

        Ok(())
    }
}

fn trampoline(name: String, window: GuiWin, pane: MuxPane, entry: Option<InputSelectorEntry>) {
    promise::spawn::spawn(async move {
        config::with_lua_config_on_main_thread(move |lua| do_event(lua, name, window, pane, entry))
            .await
    })
    .detach();
}

async fn do_event(
    lua: Option<Rc<mlua::Lua>>,
    name: String,
    window: GuiWin,
    pane: MuxPane,
    entry: Option<InputSelectorEntry>,
) -> anyhow::Result<()> {
    if let Some(lua) = lua {
        let id = entry.as_ref().map(|entry| entry.id.clone());
        let label = entry.as_ref().map(|entry| entry.label.to_string());

        let args = lua.pack_multi((window, pane, id, label))?;

        if let Err(err) = config::lua::emit_event(&lua, (name.clone(), args)).await {
            log::error!("while processing {} event: {:#}", name, err);
        }
    }

    Ok(())
}

pub fn selector(
    mut term: TermWizTerminal,
    args: InputSelector,
    window: GuiWin,
    pane: MuxPane,
) -> anyhow::Result<()> {
    let event_name = match *args.action {
        KeyAssignment::EmitEvent(ref id) => id.to_string(),
        _ => {
            anyhow::bail!("InputSelector requires action to be defined by wezterm.action_callback")
        }
    };
    let mut state = SelectorState {
        active_idx: 0,
        max_items: 0,
        pane,
        top_row: 0,
        filter_term: String::new(),
        filtered_entries: vec![],
        window,
        filtering: args.fuzzy,
        always_fuzzy: args.fuzzy,
        args,
        event_name,
        selection: String::new(),
        labels: vec![],
    };

    term.set_raw_mode()?;
    term.render(&[Change::Title(state.args.title.to_string())])?;
    state.update_filter();
    state.render(&mut term)?;
    state.run_loop(&mut term)
}
