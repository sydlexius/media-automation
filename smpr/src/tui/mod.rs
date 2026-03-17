// Configure wizard TUI (ratatui)
#![allow(dead_code)]

pub mod app;
pub mod event;
pub mod io;
pub mod keymap;
pub mod render;
pub mod widgets;

#[cfg(test)]
mod app_tests;
#[cfg(test)]
mod io_tests;
#[cfg(test)]
mod keymap_tests;

use app::AppState;
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::config::RawConfig;

#[derive(Debug)]
pub enum TuiError {
    Io(std::io::Error),
    Terminal(String),
}

impl std::fmt::Display for TuiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::Terminal(msg) => write!(f, "terminal error: {msg}"),
        }
    }
}

impl std::error::Error for TuiError {}

impl From<std::io::Error> for TuiError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
    }
}

pub fn run_editor(
    config: RawConfig,
    env_keys: BTreeMap<String, String>,
    config_path: PathBuf,
    env_path: PathBuf,
) -> Result<(), TuiError> {
    let read_only = config_path.is_file()
        && std::fs::metadata(&config_path)
            .map(|m| m.permissions().readonly())
            .unwrap_or(false);

    let mut state = AppState::new(config, env_keys, config_path, env_path);
    state.read_only = read_only;

    enable_raw_mode()?;
    execute!(std::io::stdout(), EnterAlternateScreen)?;
    let _guard = TerminalGuard;

    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;

    loop {
        terminal.draw(|frame| render::render(frame, &state))?;

        let evt = event::poll_event(std::time::Duration::from_millis(100))?;

        match evt {
            event::Event::Key(key) => {
                if state.error_message.is_some() {
                    state.error_message = None;
                    continue;
                }

                if state.quit_requested {
                    match key.code {
                        crossterm::event::KeyCode::Char('y') => {
                            if !state.read_only
                                && let Err(e) = save(&state)
                            {
                                state.error_message = Some(format!("Save failed: {e}"));
                                state.quit_requested = false;
                                continue;
                            }
                            break;
                        }
                        crossterm::event::KeyCode::Char('n') => break,
                        crossterm::event::KeyCode::Esc => {
                            state.quit_requested = false;
                        }
                        _ => {}
                    }
                    continue;
                }

                let action = keymap::map_key(state.mode, state.active_pane, state.section, key);

                if let Some(action) = action {
                    handle_action(&mut state, action);
                }
            }
            event::Event::Resize(_, _) => {}
            event::Event::Tick => {}
        }

        if state.quit_requested && !state.dirty {
            break;
        }
    }

    Ok(())
}

fn handle_action(state: &mut app::AppState, action: keymap::Action) {
    use app::{DetectionCategory, Mode, Section};
    use keymap::Action;

    match action {
        Action::NextSection => state.next_section(),
        Action::PrevSection => state.prev_section(),
        Action::TogglePane => state.toggle_pane(),

        Action::NextItem => {
            if state.section == Section::Detection {
                if state.detection_state.editing {
                    let cat = DetectionCategory::ALL[state.detection_state.selected_category];
                    let count =
                        widgets::detection::get_words(state.config.detection.as_ref(), cat).len();
                    if count > 0 {
                        state.detection_state.word_cursor =
                            (state.detection_state.word_cursor + 1).min(count - 1);
                    }
                } else {
                    state.detection_state.selected_category =
                        (state.detection_state.selected_category + 1)
                            .min(DetectionCategory::ALL.len() - 1);
                }
            }
        }
        Action::PrevItem => {
            if state.section == Section::Detection {
                if state.detection_state.editing {
                    state.detection_state.word_cursor =
                        state.detection_state.word_cursor.saturating_sub(1);
                } else {
                    state.detection_state.selected_category =
                        state.detection_state.selected_category.saturating_sub(1);
                }
            }
        }

        Action::Save => {
            if !state.read_only {
                if let Err(e) = save(state) {
                    state.error_message = Some(format!("Save failed: {e}"));
                } else {
                    state.dirty = false;
                }
            }
        }
        Action::Quit => {
            state.quit_requested = true;
        }

        Action::Edit => match state.section {
            Section::Preferences => {
                state.set_overwrite(!state.preferences_state.overwrite);
            }
            Section::Detection => {
                state.detection_state.editing = true;
                state.detection_state.word_cursor = 0;
                state.mode = Mode::Editing;
            }
            _ => {}
        },

        Action::Confirm => {
            if state.section == Section::Detection {
                if state.detection_state.adding {
                    let word = state.detection_state.text_input.text.trim().to_string();
                    if !word.is_empty() {
                        let cat = DetectionCategory::ALL[state.detection_state.selected_category];
                        let words =
                            widgets::detection::get_words_mut(&mut state.config.detection, cat);
                        words.push(word);
                        state.mark_dirty();
                    }
                    state.detection_state.adding = false;
                    state.detection_state.text_input.clear();
                } else if state.detection_state.editing {
                    state.detection_state.editing = false;
                    state.mode = Mode::Normal;
                }
            }
        }

        Action::Cancel => {
            if state.mode == Mode::Editing {
                if state.detection_state.adding {
                    state.detection_state.adding = false;
                    state.detection_state.text_input.clear();
                } else {
                    state.detection_state.editing = false;
                    state.mode = Mode::Normal;
                }
            }
        }

        Action::Add => {
            if state.section == Section::Detection && state.detection_state.editing {
                state.detection_state.adding = true;
                state.detection_state.text_input.clear();
            }
        }

        Action::Delete => {
            if state.section == Section::Detection && state.detection_state.editing {
                let cat = DetectionCategory::ALL[state.detection_state.selected_category];
                let words = widgets::detection::get_words_mut(&mut state.config.detection, cat);
                if state.detection_state.word_cursor < words.len() {
                    words.remove(state.detection_state.word_cursor);
                    if state.detection_state.word_cursor >= words.len() && !words.is_empty() {
                        state.detection_state.word_cursor = words.len() - 1;
                    }
                    state.mark_dirty();
                }
            }
        }

        Action::Char(c) => {
            if state.detection_state.adding {
                state.detection_state.text_input.insert_char(c);
            }
        }

        Action::Backspace => {
            if state.detection_state.adding {
                state.detection_state.text_input.delete_back();
            }
        }

        // Stubs for unimplemented actions
        Action::Toggle => {}
        Action::NextOption => {}
        Action::PrevOption => {}
        Action::StartFilter => {}
        Action::PageUp => {}
        Action::PageDown => {}
        Action::ExpandCollapse => {}
    }
}

fn save(state: &AppState) -> Result<(), TuiError> {
    io::save_config(&state.config, &state.config_path)?;
    io::save_env(&state.env_keys, &state.initial_labels, &state.env_path)?;
    Ok(())
}
