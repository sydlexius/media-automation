#![allow(dead_code)]

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

pub struct Popup<'a> {
    title: &'a str,
    message: &'a str,
    hint: &'a str,
}

impl<'a> Popup<'a> {
    pub fn new(title: &'a str, message: &'a str, hint: &'a str) -> Self {
        Self {
            title,
            message,
            hint,
        }
    }
}

impl Widget for Popup<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let popup_width = 50u16.min(area.width.saturating_sub(4));
        let popup_height = 5u16.min(area.height.saturating_sub(2));

        let [popup_area] = Layout::horizontal([Constraint::Length(popup_width)])
            .flex(Flex::Center)
            .areas(area);
        let [popup_area] = Layout::vertical([Constraint::Length(popup_height)])
            .flex(Flex::Center)
            .areas(popup_area);

        Clear.render(popup_area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(self.title);

        let inner = block.inner(popup_area);
        block.render(popup_area, buf);

        let lines = vec![
            Line::from(self.message),
            Line::from(""),
            Line::styled(self.hint, Style::default().fg(Color::DarkGray)),
        ];
        Paragraph::new(lines).render(inner, buf);
    }
}
