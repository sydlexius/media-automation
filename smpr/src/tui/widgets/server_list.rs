use crate::tui::app::{AppState, Mode, ServerField};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

pub fn render_server_list(state: &AppState, area: Rect, buf: &mut Buffer) {
    let servers = match &state.config.servers {
        Some(s) if !s.is_empty() => s,
        _ => {
            Paragraph::new("  No servers configured. Press 'a' to add one.")
                .style(Style::default().fg(Color::DarkGray))
                .render(area, buf);
            return;
        }
    };

    let entries: Vec<(&String, &crate::config::RawServerConfig)> = servers.iter().collect();
    let mut y = area.y;

    for (i, (label, server)) in entries.iter().enumerate() {
        let card_height = 5u16;
        if y + card_height > area.y + area.height {
            break;
        }

        let is_selected = i == state.server_state.selected;
        let border_color = if is_selected {
            Color::Blue
        } else {
            Color::DarkGray
        };

        let card_area = Rect {
            x: area.x,
            y,
            width: area.width,
            height: card_height.min(area.y + area.height - y),
        };

        let type_str = server.server_type.as_deref().unwrap_or("unknown");
        let type_badge = match type_str {
            "emby" => Span::styled(
                " EMBY ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            "jellyfin" => Span::styled(
                " JELLYFIN ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            other => Span::styled(format!(" {other} "), Style::default().fg(Color::Gray)),
        };

        let cursor = if is_selected { "▸ " } else { "  " };
        let title = Line::from(vec![
            Span::styled(cursor, Style::default().fg(Color::Blue)),
            Span::styled(
                label.as_str(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            type_badge,
        ]);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(title);

        let inner = block.inner(card_area);
        block.render(card_area, buf);

        let url = server.url.as_deref().unwrap_or("(not set)");
        let api_key = state
            .env_keys
            .get(*label)
            .map(|k| {
                if k.len() > 4 {
                    format!("{}...{}", "•".repeat(8), &k[k.len() - 4..])
                } else {
                    "•".repeat(k.len())
                }
            })
            .unwrap_or_else(|| "(not set)".to_string());

        let libs = server
            .libraries
            .as_ref()
            .map(|l| l.keys().cloned().collect::<Vec<_>>().join(", "))
            .unwrap_or_else(|| "(none)".to_string());

        let is_editing = is_selected && state.mode == Mode::Editing;

        render_field(
            buf,
            inner.x,
            inner.y,
            inner.width,
            "URL",
            url,
            is_editing && state.server_state.editing_field == Some(ServerField::Url),
            &state.server_state.text_input.text,
        );
        if inner.height > 1 {
            render_field(
                buf,
                inner.x,
                inner.y + 1,
                inner.width,
                "Key",
                &api_key,
                is_editing && state.server_state.editing_field == Some(ServerField::ApiKey),
                &state.server_state.text_input.text,
            );
        }
        if inner.height > 2 {
            let lib_line = Line::from(vec![
                Span::styled("  Libs    ", Style::default().fg(Color::DarkGray)),
                Span::styled(libs, Style::default().fg(Color::White)),
            ]);
            buf.set_line(inner.x, inner.y + 2, &lib_line, inner.width);
        }

        y += card_height + 1;
    }
}

#[allow(clippy::too_many_arguments)]
fn render_field(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    label: &str,
    value: &str,
    editing: bool,
    edit_text: &str,
) {
    let label_span = Span::styled(
        format!("  {label:<8}"),
        Style::default().fg(Color::DarkGray),
    );
    if editing {
        let line = Line::from(vec![
            label_span,
            Span::styled(edit_text, Style::default().fg(Color::White)),
            Span::styled("█", Style::default().fg(Color::White)),
        ]);
        buf.set_line(x, y, &line, width);
    } else {
        let val_color = if value.starts_with('•') || value == "(not set)" {
            Color::Yellow
        } else {
            Color::White
        };
        let line = Line::from(vec![
            label_span,
            Span::styled(value, Style::default().fg(val_color)),
        ]);
        buf.set_line(x, y, &line, width);
    }
}
