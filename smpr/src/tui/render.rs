#![allow(dead_code)]

use crate::tui::app::{AppState, Mode, Pane};
use crate::tui::widgets::{popup::Popup, sidebar::render_sidebar};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

const MIN_WIDTH: u16 = 40;
const MIN_HEIGHT: u16 = 15;
const SIDEBAR_MIN: u16 = 22;

pub fn render(frame: &mut Frame, state: &AppState) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let msg = Paragraph::new("Terminal too small.\nResize to at least 40x15.")
            .style(Style::default().fg(Color::Red));
        frame.render_widget(msg, area);
        return;
    }

    let [title_area, main_area, status_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);

    render_title_bar(state, title_area, frame.buffer_mut());
    render_status_bar(state, status_area, frame.buffer_mut());

    match state.mode {
        Mode::FullScreen | Mode::Filtering => {
            render_content(state, main_area, frame.buffer_mut());
        }
        _ => {
            if area.width < 60 {
                render_content(state, main_area, frame.buffer_mut());
            } else {
                let [sidebar_area, content_area] =
                    Layout::horizontal([Constraint::Min(SIDEBAR_MIN), Constraint::Fill(1)])
                        .areas(main_area);
                render_sidebar(state, sidebar_area, frame.buffer_mut());
                render_content(state, content_area, frame.buffer_mut());
            }
        }
    }

    if state.server_state.delete_requested {
        Popup::new(
            " Delete Server ",
            "Delete this server?",
            "y=delete  any other key=cancel",
        )
        .render(area, frame.buffer_mut());
    } else if state.quit_requested && state.dirty {
        Popup::new(
            " Unsaved Changes ",
            "Save before quitting?",
            "y=save & quit  n=quit  Esc=cancel",
        )
        .render(area, frame.buffer_mut());
    }

    if let Some(ref msg) = state.error_message {
        Popup::new(" Error ", msg, "Press any key to dismiss").render(area, frame.buffer_mut());
    }
}

fn render_title_bar(state: &AppState, area: Rect, buf: &mut Buffer) {
    let path_str = state.config_path.display().to_string();
    let mut spans = vec![
        Span::styled(
            " smpr configure",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · ", Style::default().fg(Color::DarkGray)),
        Span::styled(path_str, Style::default().fg(Color::Green)),
    ];
    if state.read_only {
        spans.push(Span::styled(
            " [READ-ONLY]",
            Style::default().fg(Color::Yellow),
        ));
    }
    if state.dirty {
        spans.push(Span::styled(" ●", Style::default().fg(Color::Red)));
    }
    Paragraph::new(Line::from(spans))
        .style(Style::default().bg(Color::DarkGray))
        .render(area, buf);
}

fn render_status_bar(state: &AppState, area: Rect, buf: &mut Buffer) {
    let hints = match state.mode {
        Mode::Normal => match state.active_pane {
            Pane::Sidebar => "↑↓ navigate  Tab/Enter content  s save  q quit",
            Pane::Content => "↑↓ navigate  Tab sidebar  Enter edit  s save  q quit",
        },
        Mode::Editing => "Enter confirm  Esc cancel",
        Mode::FullScreen => "↑↓ navigate  Space toggle  / filter  Enter confirm  Esc cancel",
        Mode::Filtering => "Type to filter  Enter confirm  Esc cancel",
    };
    let dirty_text = if state.dirty {
        Span::styled(
            "● MODIFIED",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("no changes", Style::default().fg(Color::DarkGray))
    };
    let line = Line::from(vec![
        Span::styled(format!(" {hints}"), Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
        dirty_text,
    ]);
    Paragraph::new(line)
        .style(Style::default().bg(Color::DarkGray))
        .render(area, buf);
}

fn render_content(state: &AppState, area: Rect, buf: &mut Buffer) {
    use crate::tui::app::Section;
    match state.section {
        Section::Servers => {
            super::widgets::server_list::render_server_list(state, area, buf);
        }
        Section::Preferences => {
            super::widgets::preferences::render_preferences(state, area, buf);
        }
        Section::Detection => {
            super::widgets::detection::render_detection(state, area, buf);
        }
        Section::Genres => {
            super::widgets::genre_picker::render_genre_picker(state, area, buf);
        }
        Section::ForceRatings => {
            super::widgets::force_tree::render_force_tree(state, area, buf);
        }
    }
}
