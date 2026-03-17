use crate::tui::app::{AppState, GenrePickerState, Mode};
use crate::wizard::library::DEFAULT_G_GENRES;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, List, ListItem, ListState, Paragraph, StatefulWidget, Widget,
};
use std::collections::HashSet;

/// Initialize genre picker state from current config.
pub fn init_genre_state(state: &mut AppState) {
    let current_genres: Vec<String> = state
        .config
        .detection
        .as_ref()
        .and_then(|d| d.g_genres.as_ref())
        .and_then(|g| g.genres.as_ref())
        .cloned()
        .unwrap_or_default();

    let mut available: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for g in &current_genres {
        if seen.insert(g.to_lowercase()) {
            available.push(g.clone());
        }
    }

    for g in DEFAULT_G_GENRES {
        if seen.insert((*g).to_lowercase()) {
            available.push(g.to_string());
        }
    }

    available.sort_by_key(|a| a.to_lowercase());

    let selected: HashSet<String> = current_genres.into_iter().collect();

    state.genre_state = GenrePickerState {
        available,
        selected: selected.clone(),
        cursor: 0,
        filter: String::new(),
        filter_active: false,
        snapshot: Some(selected),
    };
}

fn is_recommended(genre: &str) -> bool {
    DEFAULT_G_GENRES
        .iter()
        .any(|g: &&str| g.eq_ignore_ascii_case(genre))
}

pub fn filtered_genres(state: &GenrePickerState) -> Vec<(usize, &String)> {
    if state.filter.is_empty() {
        state.available.iter().enumerate().collect()
    } else {
        let filter_lower = state.filter.to_lowercase();
        state
            .available
            .iter()
            .enumerate()
            .filter(|(_, g)| g.to_lowercase().contains(&filter_lower))
            .collect()
    }
}

/// Sync genre selections from GenrePickerState back to RawConfig.
pub fn sync_genres_to_config(state: &mut AppState) {
    let mut genres: Vec<String> = state.genre_state.selected.iter().cloned().collect();
    genres.sort_by_key(|a| a.to_lowercase());
    let det = state.config.detection.get_or_insert_with(Default::default);
    let g = det
        .g_genres
        .get_or_insert(crate::config::RawGenres { genres: None });
    g.genres = Some(genres);
}

pub fn render_genre_picker(state: &AppState, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue))
        .title(" G-Rated Genres ");
    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height < 3 || inner.width < 10 {
        return;
    }

    // In normal mode, show a summary view (not the full picker)
    if state.mode != Mode::FullScreen && state.mode != Mode::Filtering {
        render_genre_summary(state, inner, buf);
        return;
    }

    let filter_height = u16::from(state.genre_state.filter_active);
    let constraints = if filter_height > 0 {
        vec![Constraint::Length(1), Constraint::Fill(1)]
    } else {
        vec![Constraint::Fill(1)]
    };
    let areas = Layout::vertical(constraints).split(inner);

    let (columns_area, filter_area) = if state.genre_state.filter_active {
        (areas[1], Some(areas[0]))
    } else {
        (areas[0], None)
    };

    if let Some(filter_area) = filter_area {
        let line = Line::from(vec![
            Span::styled(" / ", Style::default().fg(Color::Yellow)),
            Span::styled(&state.genre_state.filter, Style::default().fg(Color::White)),
            Span::styled("█", Style::default().fg(Color::White)),
        ]);
        Paragraph::new(line).render(filter_area, buf);
    }

    let [left_area, right_area] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .areas(columns_area);

    render_available_column(state, left_area, buf);
    render_selected_column(state, right_area, buf);
}

/// Summary view shown in normal mode — reads directly from config.
fn render_genre_summary(state: &AppState, area: Rect, buf: &mut Buffer) {
    let genres: Vec<&str> = state
        .config
        .detection
        .as_ref()
        .and_then(|d| d.g_genres.as_ref())
        .and_then(|g| g.genres.as_ref())
        .map(|v| v.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();

    if genres.is_empty() {
        Paragraph::new("  No genres configured. Press Enter to edit.")
            .style(Style::default().fg(Color::DarkGray))
            .render(area, buf);
        return;
    }

    let header = Line::from(vec![Span::styled(
        format!(
            "  {} genre{} configured:",
            genres.len(),
            if genres.len() == 1 { "" } else { "s" }
        ),
        Style::default().fg(Color::White),
    )]);
    buf.set_line(area.x, area.y, &header, area.width);

    let mut y = area.y + 2;
    let tag_bg = Color::Rgb(45, 74, 34);
    let mut x = area.x + 2;

    for genre in &genres {
        let tag = format!(" {genre} ");
        let len = tag.len() as u16;
        if x + len > area.x + area.width {
            y += 1;
            x = area.x + 2;
            if y >= area.y + area.height {
                break;
            }
        }
        buf.set_string(x, y, &tag, Style::default().fg(Color::Green).bg(tag_bg));
        x += len + 1;
    }

    // Hint at bottom
    let hint_y = area.y + area.height.saturating_sub(1);
    if hint_y > y {
        buf.set_line(
            area.x,
            hint_y,
            &Line::styled("  Enter to edit", Style::default().fg(Color::DarkGray)),
            area.width,
        );
    }
}

fn render_available_column(state: &AppState, area: Rect, buf: &mut Buffer) {
    let header = Line::from(vec![
        Span::styled(
            " Available",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {} total", state.genre_state.available.len()),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    buf.set_line(area.x, area.y, &header, area.width);

    let list_area = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(1),
    };

    let filtered = filtered_genres(&state.genre_state);
    let has_no_match = !state.genre_state.filter.is_empty() && filtered.is_empty();

    let mut items: Vec<ListItem> = Vec::new();

    if has_no_match {
        let add_line = Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("(Add: {})", state.genre_state.filter),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]);
        items.push(ListItem::new(add_line));
    }

    for (_, genre) in &filtered {
        let is_selected = state.genre_state.selected.contains(*genre);
        let recommended = is_recommended(genre);

        let checkbox = if is_selected {
            Span::styled("[✓]", Style::default().fg(Color::Green))
        } else {
            Span::styled("[ ]", Style::default().fg(Color::DarkGray))
        };

        let name_color = if is_selected {
            Color::Green
        } else if recommended {
            Color::Cyan
        } else {
            Color::Gray
        };

        let mut spans = vec![
            Span::raw(" "),
            checkbox,
            Span::raw(" "),
            Span::styled(genre.as_str(), Style::default().fg(name_color)),
        ];

        if recommended {
            spans.push(Span::styled(" ★", Style::default().fg(Color::Cyan)));
        }

        items.push(ListItem::new(Line::from(spans)));
    }

    let mut list_state = ListState::default();
    list_state.select(Some(state.genre_state.cursor));

    let list = List::new(items).highlight_style(Style::default().bg(Color::DarkGray));
    StatefulWidget::render(list, list_area, buf, &mut list_state);
}

fn render_selected_column(state: &AppState, area: Rect, buf: &mut Buffer) {
    let count = state.genre_state.selected.len();
    let header = Line::from(vec![Span::styled(
        format!(" {count} Selected"),
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    )]);
    buf.set_line(area.x, area.y, &header, area.width);

    let list_area = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(1),
    };

    let mut selected: Vec<&String> = state.genre_state.selected.iter().collect();
    selected.sort_by_key(|a| a.to_lowercase());

    let items: Vec<ListItem> = selected
        .iter()
        .map(|g| {
            let recommended = is_recommended(g);
            let mut spans = vec![
                Span::styled(" [✓] ", Style::default().fg(Color::Green)),
                Span::styled(g.as_str(), Style::default().fg(Color::Green)),
            ];
            if recommended {
                spans.push(Span::styled(" ★", Style::default().fg(Color::Cyan)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    Widget::render(List::new(items), list_area, buf);
}
