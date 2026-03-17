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
            } else if state.section == Section::Servers {
                if state.mode == Mode::Editing {
                    state.server_state.editing_field = match state.server_state.editing_field {
                        Some(app::ServerField::Url) => Some(app::ServerField::ApiKey),
                        Some(app::ServerField::ApiKey) => Some(app::ServerField::ServerType),
                        Some(app::ServerField::ServerType) => Some(app::ServerField::Url),
                        None => Some(app::ServerField::Url),
                    };
                    if let Some(label) = selected_server_label(state) {
                        load_field_into_input(state, &label);
                    }
                } else {
                    let count = server_count(state);
                    if count > 0 {
                        state.server_state.selected =
                            (state.server_state.selected + 1).min(count - 1);
                    }
                }
            } else if state.section == Section::Genres && state.mode == Mode::FullScreen {
                let filtered = widgets::genre_picker::filtered_genres(&state.genre_state);
                let max = if filtered.is_empty() && !state.genre_state.filter.is_empty() {
                    0 // phantom add entry
                } else {
                    filtered.len().saturating_sub(1)
                };
                state.genre_state.cursor = (state.genre_state.cursor + 1).min(max);
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
            } else if state.section == Section::Servers {
                if state.mode == Mode::Editing {
                    state.server_state.editing_field = match state.server_state.editing_field {
                        Some(app::ServerField::Url) => Some(app::ServerField::ServerType),
                        Some(app::ServerField::ApiKey) => Some(app::ServerField::Url),
                        Some(app::ServerField::ServerType) => Some(app::ServerField::ApiKey),
                        None => Some(app::ServerField::Url),
                    };
                    if let Some(label) = selected_server_label(state) {
                        load_field_into_input(state, &label);
                    }
                } else {
                    state.server_state.selected = state.server_state.selected.saturating_sub(1);
                }
            } else if state.section == Section::Genres && state.mode == Mode::FullScreen {
                state.genre_state.cursor = state.genre_state.cursor.saturating_sub(1);
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
            Section::Servers => {
                if server_count(state) > 0 {
                    state.mode = Mode::Editing;
                    state.server_state.editing_field = Some(app::ServerField::Url);
                    if let Some(label) = selected_server_label(state) {
                        load_field_into_input(state, &label);
                    }
                }
            }
            Section::Genres => {
                widgets::genre_picker::init_genre_state(state);
                state.mode = Mode::FullScreen;
            }
            _ => {}
        },

        Action::Confirm => {
            // Genre picker confirm — must come before server/detection checks
            if state.section == Section::Genres && state.mode == Mode::FullScreen {
                if state.genre_state.filter_active {
                    let filtered = widgets::genre_picker::filtered_genres(&state.genre_state);
                    if filtered.is_empty() && !state.genre_state.filter.is_empty() {
                        // Add custom genre
                        let new_genre = state.genre_state.filter.clone();
                        state.genre_state.available.push(new_genre.clone());
                        state.genre_state.selected.insert(new_genre);
                        widgets::genre_picker::sync_genres_to_config(state);
                        state.mark_dirty();
                    }
                    state.genre_state.filter.clear();
                    state.genre_state.filter_active = false;
                    state.genre_state.cursor = 0;
                } else {
                    // Exit fullscreen, keep selections
                    state.mode = Mode::Normal;
                }
                return;
            }

            // Adding a new server (label input step) — must come before Detection check
            if state.section == Section::Servers
                && state.mode == Mode::Editing
                && state.server_state.editing_field.is_none()
            {
                let label = state.server_state.text_input.text.trim().to_string();
                if let Err(msg) = app::validate_label(&label) {
                    state.error_message = Some(msg.to_string());
                    return;
                }
                if app::is_duplicate_label(&state.config, &label) {
                    state.error_message = Some("Server label already exists".to_string());
                    return;
                }
                let servers = state.config.servers.get_or_insert_with(BTreeMap::new);
                servers.insert(
                    label.clone(),
                    crate::config::RawServerConfig {
                        url: Some(String::new()),
                        server_type: None,
                        libraries: None,
                    },
                );
                state.server_state.selected = servers.keys().position(|k| k == &label).unwrap_or(0);
                state.server_state.editing_field = Some(app::ServerField::Url);
                state.server_state.text_input.clear();
                state.mark_dirty();
                return;
            }

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
            } else if state.section == Section::Servers
                && state.mode == Mode::Editing
                && let Some(label) = selected_server_label(state)
            {
                let text = state.server_state.text_input.text.trim().to_string();
                match state.server_state.editing_field {
                    Some(app::ServerField::Url) => {
                        if let Err(msg) = app::validate_url(&text) {
                            state.error_message = Some(msg.to_string());
                            return;
                        }
                        if let Some(server) = state
                            .config
                            .servers
                            .as_mut()
                            .and_then(|s| s.get_mut(&label))
                        {
                            server.url = Some(text);
                            state.mark_dirty();
                        }
                    }
                    Some(app::ServerField::ApiKey) => {
                        state.env_keys.insert(label.clone(), text);
                        state.mark_dirty();
                    }
                    Some(app::ServerField::ServerType) => {
                        let val = text.to_lowercase();
                        let type_val = match val.as_str() {
                            "emby" | "jellyfin" => Some(val),
                            "" => None,
                            _ => {
                                state.error_message =
                                    Some("Type must be 'emby' or 'jellyfin'".to_string());
                                return;
                            }
                        };
                        if let Some(server) = state
                            .config
                            .servers
                            .as_mut()
                            .and_then(|s| s.get_mut(&label))
                        {
                            server.server_type = type_val;
                            state.mark_dirty();
                        }
                    }
                    None => {} // handled above
                }
                state.mode = Mode::Normal;
                state.server_state.editing_field = None;
            }
        }

        Action::Cancel => {
            if state.section == Section::Genres && state.mode == Mode::FullScreen {
                if let Some(snapshot) = state.genre_state.snapshot.take() {
                    state.genre_state.selected = snapshot;
                    widgets::genre_picker::sync_genres_to_config(state);
                }
                state.genre_state.filter_active = false;
                state.mode = Mode::Normal;
                return;
            }
            if state.mode == Mode::Editing {
                if state.section == Section::Servers {
                    state.mode = Mode::Normal;
                    state.server_state.editing_field = None;
                    state.server_state.text_input.clear();
                } else if state.detection_state.adding {
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
            } else if state.section == Section::Servers && state.mode != Mode::Editing {
                state.mode = Mode::Editing;
                state.server_state.editing_field = None;
                state.server_state.text_input.clear();
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
            } else if state.section == Section::Servers
                && state.mode == Mode::Normal
                && let Some(label) = selected_server_label(state)
            {
                if let Some(servers) = state.config.servers.as_mut() {
                    servers.remove(&label);
                }
                state.env_keys.remove(&label);
                let count = server_count(state);
                if state.server_state.selected >= count && count > 0 {
                    state.server_state.selected = count - 1;
                }
                state.mark_dirty();
            }
        }

        Action::Char(c) => {
            if state.detection_state.adding {
                state.detection_state.text_input.insert_char(c);
            } else if state.section == Section::Servers && state.mode == Mode::Editing {
                state.server_state.text_input.insert_char(c);
            } else if state.section == Section::Genres && state.genre_state.filter_active {
                state.genre_state.filter.push(c);
                state.genre_state.cursor = 0;
            }
        }

        Action::Backspace => {
            if state.detection_state.adding {
                state.detection_state.text_input.delete_back();
            } else if state.section == Section::Servers && state.mode == Mode::Editing {
                state.server_state.text_input.delete_back();
            } else if state.section == Section::Genres && state.genre_state.filter_active {
                state.genre_state.filter.pop();
                state.genre_state.cursor = 0;
            }
        }

        // Stubs for unimplemented actions
        Action::Toggle => {
            if state.section == Section::Genres && state.mode == Mode::FullScreen {
                let filtered = widgets::genre_picker::filtered_genres(&state.genre_state);
                if let Some((_, genre)) = filtered.get(state.genre_state.cursor) {
                    let genre = (*genre).clone();
                    if state.genre_state.selected.contains(&genre) {
                        state.genre_state.selected.remove(&genre);
                    } else {
                        state.genre_state.selected.insert(genre);
                    }
                    widgets::genre_picker::sync_genres_to_config(state);
                    state.mark_dirty();
                }
            }
        }
        Action::NextOption => {}
        Action::PrevOption => {}
        Action::StartFilter => {
            if state.section == Section::Genres && state.mode == Mode::FullScreen {
                state.genre_state.filter_active = true;
                state.genre_state.filter.clear();
            }
        }
        Action::PageUp => {
            if state.section == Section::Genres && state.mode == Mode::FullScreen {
                state.genre_state.cursor = state.genre_state.cursor.saturating_sub(10);
            }
        }
        Action::PageDown => {
            if state.section == Section::Genres && state.mode == Mode::FullScreen {
                let filtered = widgets::genre_picker::filtered_genres(&state.genre_state);
                let max = filtered.len().saturating_sub(1);
                state.genre_state.cursor = (state.genre_state.cursor + 10).min(max);
            }
        }
        Action::ExpandCollapse => {}
    }
}

fn selected_server_label(state: &AppState) -> Option<String> {
    state
        .config
        .servers
        .as_ref()
        .and_then(|s| s.keys().nth(state.server_state.selected).cloned())
}

fn server_count(state: &AppState) -> usize {
    state.config.servers.as_ref().map_or(0, |s| s.len())
}

fn load_field_into_input(state: &mut AppState, label: &str) {
    let text = match state.server_state.editing_field {
        Some(app::ServerField::Url) => state
            .config
            .servers
            .as_ref()
            .and_then(|s| s.get(label))
            .and_then(|s| s.url.as_deref())
            .unwrap_or("")
            .to_string(),
        Some(app::ServerField::ApiKey) => state.env_keys.get(label).cloned().unwrap_or_default(),
        Some(app::ServerField::ServerType) => state
            .config
            .servers
            .as_ref()
            .and_then(|s| s.get(label))
            .and_then(|s| s.server_type.as_deref())
            .unwrap_or("")
            .to_string(),
        None => String::new(),
    };
    state.server_state.text_input.set(&text);
}

fn save(state: &AppState) -> Result<(), TuiError> {
    io::save_config(&state.config, &state.config_path)?;
    io::save_env(&state.env_keys, &state.initial_labels, &state.env_path)?;
    Ok(())
}
