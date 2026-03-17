// Configure wizard TUI (ratatui)
#![allow(dead_code)]

pub mod app;
pub mod event;
pub mod io;
pub mod keymap;
pub mod render;
pub mod widgets;

#[cfg(test)]
mod action_tests;
#[cfg(test)]
mod app_tests;
#[cfg(test)]
mod force_tree_tests;
#[cfg(test)]
mod io_tests;
#[cfg(test)]
mod keymap_tests;
#[cfg(test)]
mod render_tests;

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

    // Initialize force tree from config at startup
    widgets::force_tree::init_force_state(&mut state);

    enable_raw_mode()?;
    let _guard = TerminalGuard; // Drop restores terminal even if EnterAlternateScreen fails
    execute!(std::io::stdout(), EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;

    loop {
        // Update view_height from terminal size (minus title, status, borders = 4 rows)
        let term_height = terminal.size()?.height as usize;
        state.force_state.view_height = term_height.saturating_sub(4);

        terminal.draw(|frame| render::render(frame, &state))?;

        let evt = event::poll_event(std::time::Duration::from_millis(100))?;

        match evt {
            event::Event::Key(key) => {
                if state.error_message.is_some() {
                    state.error_message = None;
                    continue;
                }

                if state.info_message.is_some() {
                    state.info_message = None;
                    continue;
                }

                if state.quit_requested {
                    match key.code {
                        crossterm::event::KeyCode::Char('y') => {
                            if state.read_only {
                                state.error_message = Some(
                                    "Cannot save: config file is read-only. Press 'n' to quit without saving.".to_string(),
                                );
                                state.quit_requested = false;
                                continue;
                            }
                            if let Err(e) = save(&mut state) {
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

                // Handle server delete confirmation
                if state.server_state.delete_requested {
                    match key.code {
                        crossterm::event::KeyCode::Char('y') => {
                            if let Some(label) = selected_server_label(&state) {
                                if let Some(servers) = state.config.servers.as_mut() {
                                    servers.remove(&label);
                                }
                                state.env_keys.remove(&label);
                                let count = server_count(&state);
                                if state.server_state.selected >= count && count > 0 {
                                    state.server_state.selected = count - 1;
                                }
                                state.mark_dirty();
                                widgets::force_tree::init_force_state(&mut state);
                            }
                            state.server_state.delete_requested = false;
                        }
                        _ => {
                            state.server_state.delete_requested = false;
                        }
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
            } else if state.section == Section::ForceRatings {
                let len = state.force_state.nodes.len();
                let mut next = state.force_state.cursor + 1;
                while next < len {
                    let node = &state.force_state.nodes[next];
                    if node.depth == 0 {
                        next += 1;
                        continue;
                    }
                    if !widgets::force_tree::is_node_visible(&state.force_state, next) {
                        next += 1;
                        continue;
                    }
                    break;
                }
                if next < len {
                    state.force_state.cursor = next;
                    adjust_force_scroll(state);
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
            } else if state.section == Section::ForceRatings {
                let mut prev = state.force_state.cursor.saturating_sub(1);
                loop {
                    if state.force_state.nodes.is_empty() {
                        break;
                    }
                    let node = &state.force_state.nodes[prev];
                    if node.depth == 0 && prev > 0 {
                        prev -= 1;
                        continue;
                    }
                    if node.depth == 0 {
                        break;
                    }
                    if !widgets::force_tree::is_node_visible(&state.force_state, prev) {
                        if prev == 0 {
                            break;
                        }
                        prev -= 1;
                        continue;
                    }
                    break;
                }
                if !state.force_state.nodes.is_empty() && state.force_state.nodes[prev].depth > 0 {
                    state.force_state.cursor = prev;
                    adjust_force_scroll(state);
                }
            }
        }

        Action::Save => {
            if state.read_only {
                state.error_message = Some("Cannot save: config file is read-only".to_string());
            } else if let Err(e) = save(state) {
                state.error_message = Some(format!("Save failed: {e}"));
            } else {
                state.dirty = false;
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
                scan_server_genres(state);
                state.mode = Mode::FullScreen;
            }
            Section::ForceRatings => {
                // Enter on a library node = expand/collapse
                let cursor = state.force_state.cursor;
                if let Some(node) = state.force_state.nodes.get(cursor)
                    && node.is_library
                {
                    if state.force_state.expanded.contains(&cursor) {
                        state.force_state.expanded.remove(&cursor);
                    } else {
                        state.force_state.expanded.insert(cursor);
                    }
                    adjust_force_scroll(state);
                }
            }
        },

        Action::Confirm => {
            // Genre filter confirm (Mode::Filtering) — add custom genre or close filter
            if state.section == Section::Genres && state.mode == Mode::Filtering {
                let filtered = widgets::genre_picker::filtered_genres(&state.genre_state);
                if filtered.is_empty() && !state.genre_state.filter.is_empty() {
                    let new_genre = state.genre_state.filter.clone();
                    state.genre_state.available.push(new_genre.clone());
                    state.genre_state.selected.insert(new_genre);
                    widgets::genre_picker::sync_genres_to_config(state);
                    state.mark_dirty();
                }
                state.genre_state.filter.clear();
                state.genre_state.filter_active = false;
                state.genre_state.cursor = 0;
                state.mode = Mode::FullScreen;
                return;
            }

            // Genre picker confirm (FullScreen, filter not active) — exit and keep selections
            if state.section == Section::Genres && state.mode == Mode::FullScreen {
                state.mode = Mode::Normal;
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
                        url: None,
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
            // Esc while filtering → close filter, stay in genre fullscreen
            if state.section == Section::Genres && state.mode == Mode::Filtering {
                state.genre_state.filter.clear();
                state.genre_state.filter_active = false;
                state.genre_state.cursor = 0;
                state.mode = Mode::FullScreen;
                return;
            }
            // Esc in genre fullscreen → revert selections, exit
            if state.section == Section::Genres && state.mode == Mode::FullScreen {
                if let Some(snapshot) = state.genre_state.snapshot.take() {
                    state.genre_state.selected = snapshot;
                    widgets::genre_picker::sync_genres_to_config(state);
                }
                state.genre_state.filter_active = false;
                state.mode = Mode::Normal;
                return;
            } else if state.section == Section::ForceRatings && state.mode == Mode::FullScreen {
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
                && selected_server_label(state).is_some()
            {
                // Request confirmation before deleting
                state.server_state.delete_requested = true;
            } else if state.section == Section::ForceRatings {
                delete_force_tree_node(state);
            }
        }

        Action::Refresh => {
            if state.section == Section::Servers && state.mode == Mode::Normal {
                scan_server_libraries(state);
                // Rebuild force tree since libraries changed
                widgets::force_tree::init_force_state(state);
            }
        }

        Action::Char(c) => {
            if state.detection_state.adding {
                state.detection_state.text_input.insert_char(c);
            } else if state.section == Section::Servers && state.mode == Mode::Editing {
                state.server_state.text_input.insert_char(c);
            } else if state.section == Section::Genres && state.mode == Mode::FullScreen {
                // Auto-activate filter on keypress
                state.genre_state.filter_active = true;
                state.genre_state.filter.clear();
                state.genre_state.filter.push(c);
                state.genre_state.cursor = 0;
                state.mode = Mode::Filtering;
            } else if state.section == Section::Genres && state.mode == Mode::Filtering {
                state.genre_state.filter.push(c);
                state.genre_state.cursor = 0;
            }
        }

        Action::Backspace => {
            if state.detection_state.adding {
                state.detection_state.text_input.delete_back();
            } else if state.section == Section::Servers && state.mode == Mode::Editing {
                state.server_state.text_input.delete_back();
            } else if state.section == Section::Genres && state.mode == Mode::Filtering {
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

        Action::SetRating(ch) => {
            if state.section == Section::ForceRatings {
                state.force_state.radio_cursor = match ch {
                    'n' => 0,
                    'g' => 1,
                    'p' => 2,
                    'r' => 3,
                    _ => 0,
                };
                widgets::force_tree::apply_force_rating(state);
            }
        }
        Action::StartFilter => {
            if state.section == Section::Genres && state.mode == Mode::FullScreen {
                state.genre_state.filter_active = true;
                state.genre_state.filter.clear();
                state.mode = Mode::Filtering;
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

/// Adjust scroll_offset so the cursor stays visible in the force-rate tree.
/// Uses `view_height` stored from the last terminal size query.
fn adjust_force_scroll(state: &mut AppState) {
    // Count total visible nodes and cursor's position among them
    let mut visible_pos: usize = 0;
    let mut total_visible: usize = 0;
    let mut found_cursor = false;
    for (i, _node) in state.force_state.nodes.iter().enumerate() {
        if !widgets::force_tree::is_node_visible(&state.force_state, i) {
            continue;
        }
        if i == state.force_state.cursor && !found_cursor {
            found_cursor = true;
            visible_pos = total_visible;
        }
        total_visible += 1;
    }

    // Use stored view_height, or fall back to a conservative default
    let page_size = if state.force_state.view_height > 0 {
        state.force_state.view_height
    } else {
        15
    };

    // Clamp scroll_offset to valid range
    let max_scroll = total_visible.saturating_sub(page_size);
    if state.force_state.scroll_offset > max_scroll {
        state.force_state.scroll_offset = max_scroll;
    }

    // Ensure cursor is within the visible window
    if visible_pos < state.force_state.scroll_offset {
        state.force_state.scroll_offset = visible_pos;
    } else if visible_pos >= state.force_state.scroll_offset + page_size {
        state.force_state.scroll_offset = visible_pos - page_size + 1;
    }
}

fn save(state: &mut AppState) -> Result<(), TuiError> {
    // Save .env first — if this fails, the config file remains untouched.
    // Both use atomic writes (write to tmp, then rename).
    io::save_env(&state.env_keys, &state.initial_labels, &state.env_path)?;
    io::save_config(&state.config, &state.config_path)?;
    // Update initial_labels to track current servers for .env deletion tracking
    state.initial_labels = state.env_keys.keys().cloned().collect();
    Ok(())
}

/// Delete a library or location from the force-rate tree and config.
fn delete_force_tree_node(state: &mut AppState) {
    let cursor = state.force_state.cursor;
    let node = match state.force_state.nodes.get(cursor) {
        Some(n) if n.depth > 0 => n.clone(),
        _ => return, // can't delete server headers
    };

    if let Some(servers) = state.config.servers.as_mut()
        && let Some(server) = servers.get_mut(&node.server_label)
        && let Some(ref lib_name) = node.library_label
    {
        if let Some(ref loc_name) = node.location_label {
            // Delete a location
            if let Some(libraries) = server.libraries.as_mut()
                && let Some(lib) = libraries.get_mut(lib_name)
                && let Some(locations) = lib.locations.as_mut()
            {
                locations.remove(loc_name);
            }
        } else {
            // Delete an entire library
            if let Some(libraries) = server.libraries.as_mut() {
                libraries.remove(lib_name);
            }
        }
    }

    // Rebuild the tree and adjust cursor
    widgets::force_tree::init_force_state(state);
    state.mark_dirty();
}

/// Scan the selected server for music libraries and populate config entries.
/// Preserves existing force_rating values; adds new libraries/locations.
fn scan_server_libraries(state: &mut AppState) {
    let label = match selected_server_label(state) {
        Some(l) => l,
        None => return,
    };

    let server = match state.config.servers.as_ref().and_then(|s| s.get(&label)) {
        Some(s) => s,
        None => return,
    };

    let url = match &server.url {
        Some(u) if !u.is_empty() => u.clone(),
        _ => {
            state.error_message = Some("Server URL is not set".to_string());
            return;
        }
    };

    let api_key = match state.env_keys.get(&label) {
        Some(k) if !k.is_empty() => k.clone(),
        _ => {
            state.error_message = Some("API key is not set for this server".to_string());
            return;
        }
    };

    // Resolve server type — use config value or auto-detect
    let server_type = match server.server_type.as_deref() {
        Some("emby") => crate::config::ServerType::Emby,
        Some("jellyfin") => crate::config::ServerType::Jellyfin,
        _ => match crate::server::detect_server_type(&url) {
            Ok(t) => t,
            Err(e) => {
                state.error_message = Some(format!("Could not detect server type: {e}"));
                return;
            }
        },
    };

    let client = crate::server::MediaServerClient::new(url, api_key, server_type);

    let libraries = match client.discover_libraries() {
        Ok(libs) => libs,
        Err(e) => {
            state.error_message = Some(format!("Library scan failed: {e}"));
            return;
        }
    };

    if libraries.is_empty() {
        state.error_message = Some("No music libraries found on this server".to_string());
        return;
    }

    // Merge discovered libraries into config, preserving existing force_rating values
    let servers = state.config.servers.as_mut().unwrap();
    let server = servers.get_mut(&label).unwrap();
    let existing_libs = server.libraries.get_or_insert_with(BTreeMap::new);

    for lib in &libraries {
        let lib_entry = existing_libs.entry(lib.name.clone()).or_insert_with(|| {
            crate::config::RawLibraryConfig {
                force_rating: None,
                locations: None,
            }
        });

        // Add location entries from server (preserving existing force_ratings)
        if !lib.locations.is_empty() {
            let existing_locs = lib_entry.locations.get_or_insert_with(BTreeMap::new);
            for loc_path in &lib.locations {
                // Use the last path component as the location label
                let base_name = crate::util::location_leaf(loc_path).to_string();
                let loc_name = if existing_locs.contains_key(&base_name) {
                    loc_path.clone()
                } else {
                    base_name
                };
                existing_locs
                    .entry(loc_name)
                    .or_insert(crate::config::RawLocationConfig { force_rating: None });
            }
        }
    }

    let count = libraries.len();
    state.mark_dirty();
    state.info_message = Some(format!(
        "Found {count} music {}",
        if count == 1 { "library" } else { "libraries" }
    ));
}

/// Scan all configured servers for genres and merge into the genre picker's available list.
fn scan_server_genres(state: &mut AppState) {
    let servers: Vec<(String, String, Option<String>)> = state
        .config
        .servers
        .as_ref()
        .map(|s| {
            s.iter()
                .filter_map(|(label, srv)| {
                    let url = srv.url.as_ref()?.clone();
                    let api_key = state.env_keys.get(label)?.clone();
                    Some((url, api_key, srv.server_type.clone()))
                })
                .collect()
        })
        .unwrap_or_default();

    let mut seen: std::collections::HashSet<String> = state
        .genre_state
        .available
        .iter()
        .map(|g| g.to_lowercase())
        .collect();

    let mut errors: Vec<String> = Vec::new();

    for (url, api_key, type_str) in &servers {
        let server_type = match type_str.as_deref() {
            Some("emby") => crate::config::ServerType::Emby,
            Some("jellyfin") => crate::config::ServerType::Jellyfin,
            _ => match crate::server::detect_server_type(url) {
                Ok(t) => t,
                Err(e) => {
                    errors.push(format!("{url}: {e}"));
                    continue;
                }
            },
        };

        let client =
            crate::server::MediaServerClient::new(url.clone(), api_key.clone(), server_type);

        match client.list_genres() {
            Ok(genres) => {
                for genre in genres {
                    if seen.insert(genre.to_lowercase()) {
                        state.genre_state.available.push(genre);
                    }
                }
            }
            Err(e) => {
                errors.push(format!("{url}: {e}"));
            }
        }
    }

    state
        .genre_state
        .available
        .sort_by_key(|g| g.to_lowercase());

    if !errors.is_empty() {
        state.error_message = Some(format!("Genre scan errors: {}", errors.join("; ")));
    }
}
