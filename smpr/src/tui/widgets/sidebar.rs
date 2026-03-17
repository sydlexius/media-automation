#![allow(dead_code)]

use crate::tui::app::{AppState, Pane, Section};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, StatefulWidget, Widget};

pub fn render_sidebar(state: &AppState, area: Rect, buf: &mut Buffer) {
    let is_focused = state.active_pane == Pane::Sidebar;

    let border_style = if is_focused {
        Style::default().fg(Color::Blue)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(" Sections ");

    let inner = block.inner(area);
    block.render(area, buf);

    let items: Vec<ListItem> = Section::ALL
        .iter()
        .map(|&section| {
            let is_active = state.section == section;
            let count = state.section_count(section);
            let icon = section.icon();
            let label = section.label();
            let count_str = match count {
                Some(n) => format!(" [{n}]"),
                None => String::new(),
            };
            let style = if is_active && is_focused {
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD)
            } else if is_active {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::Gray)
            };
            let line = Line::from(vec![
                Span::styled(format!(" {icon} "), style),
                Span::styled(label.to_string(), style),
                Span::styled(count_str, Style::default().fg(Color::DarkGray)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(state.section.index()));

    let highlight_style = if is_focused {
        Style::default().bg(Color::DarkGray)
    } else {
        Style::default()
    };

    let list = List::new(items).highlight_style(highlight_style);
    StatefulWidget::render(list, inner, buf, &mut list_state);
}
