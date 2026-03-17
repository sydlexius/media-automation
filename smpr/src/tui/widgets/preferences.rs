use crate::tui::app::AppState;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

pub fn render_preferences(state: &AppState, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue))
        .title(" Preferences ");
    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height < 3 {
        return;
    }

    let [label_area, toggle_area] =
        Layout::vertical([Constraint::Length(2), Constraint::Length(1)]).areas(inner);

    Paragraph::new("  Overwrite existing ratings?")
        .style(Style::default().fg(Color::White))
        .render(label_area, buf);

    let yes_style = if state.preferences_state.overwrite {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let no_style = if !state.preferences_state.overwrite {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let line = Line::from(vec![
        Span::raw("  "),
        Span::styled(
            if state.preferences_state.overwrite {
                "[Yes]"
            } else {
                " Yes "
            },
            yes_style,
        ),
        Span::raw("  "),
        Span::styled(
            if !state.preferences_state.overwrite {
                "[No]"
            } else {
                " No "
            },
            no_style,
        ),
    ]);

    Paragraph::new(line).render(toggle_area, buf);
}
