#![allow(dead_code)]

use super::app::{Mode, Pane, Section};
use crossterm::event::{KeyCode, KeyEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    // Navigation
    NextItem,
    PrevItem,
    NextSection,
    PrevSection,
    TogglePane,

    // Editing
    Edit,
    Confirm,
    Cancel,
    Add,
    Delete,
    Refresh,         // r — scan server libraries
    SetRating(char), // n/g/p/r — direct rating set in force-rate tree

    // Full-screen
    Toggle,
    NextOption,
    PrevOption,
    StartFilter,
    PageUp,
    PageDown,
    ExpandCollapse,

    // Global
    Save,
    Quit,

    // Text input passthrough
    Char(char),
    Backspace,
}

pub fn map_key(mode: Mode, pane: Pane, section: Section, key: KeyEvent) -> Option<Action> {
    match mode {
        Mode::Normal => map_normal(pane, section, key),
        Mode::Editing => map_editing(key),
        Mode::FullScreen => map_fullscreen(section, key),
        Mode::Filtering => map_filtering(key),
    }
}

/// In filtering mode, all chars go to text input. Only Enter/Esc escape.
fn map_filtering(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Esc => Some(Action::Cancel),
        KeyCode::Backspace => Some(Action::Backspace),
        KeyCode::Char(c) => Some(Action::Char(c)),
        _ => None,
    }
}

fn map_normal(pane: Pane, section: Section, key: KeyEvent) -> Option<Action> {
    match pane {
        Pane::Sidebar => match key.code {
            KeyCode::Down | KeyCode::Char('j') => Some(Action::NextSection),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::PrevSection),
            KeyCode::Tab | KeyCode::Enter => Some(Action::TogglePane),
            KeyCode::Char('s') => Some(Action::Save),
            KeyCode::Char('q') => Some(Action::Quit),
            _ => None,
        },
        Pane::Content => match key.code {
            KeyCode::Down | KeyCode::Char('j') => Some(Action::NextItem),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::PrevItem),
            KeyCode::Tab | KeyCode::Esc => Some(Action::TogglePane),
            KeyCode::Enter => Some(Action::Edit),
            KeyCode::Char('s') => Some(Action::Save),
            KeyCode::Char('q') => Some(Action::Quit),
            KeyCode::Char('a') => match section {
                Section::Servers | Section::Detection => Some(Action::Add),
                _ => None,
            },
            KeyCode::Char('d') => match section {
                Section::Servers | Section::Detection | Section::ForceRatings => {
                    Some(Action::Delete)
                }
                _ => None,
            },
            KeyCode::Char('r') => match section {
                Section::Servers => Some(Action::Refresh),
                Section::ForceRatings => Some(Action::SetRating('r')),
                _ => None,
            },
            KeyCode::Char('n') => match section {
                Section::ForceRatings => Some(Action::SetRating('n')),
                _ => None,
            },
            KeyCode::Char('g') => match section {
                Section::ForceRatings => Some(Action::SetRating('g')),
                _ => None,
            },
            KeyCode::Char('p') => match section {
                Section::ForceRatings => Some(Action::SetRating('p')),
                _ => None,
            },
            _ => None,
        },
    }
}

fn map_editing(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Esc => Some(Action::Cancel),
        KeyCode::Backspace => Some(Action::Backspace),
        KeyCode::Char(c) => Some(Action::Char(c)),
        KeyCode::Left => Some(Action::PrevOption),
        KeyCode::Right => Some(Action::NextOption),
        KeyCode::Up => Some(Action::PrevItem),
        KeyCode::Down => Some(Action::NextItem),
        _ => None,
    }
}

fn map_fullscreen(section: Section, key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc => Some(Action::Cancel),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::PrevItem),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::NextItem),
        KeyCode::Char(' ') => Some(Action::Toggle),
        KeyCode::PageUp => Some(Action::PageUp),
        KeyCode::PageDown => Some(Action::PageDown),
        KeyCode::Left | KeyCode::Char('h') => Some(Action::PrevOption),
        KeyCode::Right | KeyCode::Char('l') => Some(Action::NextOption),
        KeyCode::Char('/') => match section {
            Section::Genres => Some(Action::StartFilter),
            _ => None,
        },
        KeyCode::Enter => match section {
            Section::Genres => Some(Action::Confirm),
            _ => Some(Action::Confirm),
        },
        KeyCode::Backspace => Some(Action::Backspace),
        KeyCode::Char(c) => {
            if section == Section::Genres {
                Some(Action::Char(c))
            } else {
                None
            }
        }
        _ => None,
    }
}
