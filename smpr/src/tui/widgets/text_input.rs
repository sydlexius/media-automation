#![allow(dead_code)]

use crate::tui::app::TextInputState;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;

pub struct TextInput<'a> {
    state: &'a TextInputState,
    style: Style,
    cursor_style: Style,
    placeholder: &'a str,
}

impl<'a> TextInput<'a> {
    pub fn new(state: &'a TextInputState) -> Self {
        Self {
            state,
            style: Style::default().fg(Color::White),
            cursor_style: Style::default().fg(Color::Black).bg(Color::White),
            placeholder: "",
        }
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn placeholder(mut self, placeholder: &'a str) -> Self {
        self.placeholder = placeholder;
        self
    }
}

impl Widget for TextInput<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let text = if self.state.text.is_empty() {
            self.placeholder
        } else {
            &self.state.text
        };
        let is_placeholder = self.state.text.is_empty();
        let style = if is_placeholder {
            Style::default().fg(Color::DarkGray)
        } else {
            self.style
        };
        // cursor is a byte offset — convert to char index for display
        let cursor_char_idx = self.state.text[..self.state.cursor.min(self.state.text.len())]
            .chars()
            .count();
        let width = area.width as usize;
        let scroll = if cursor_char_idx > width.saturating_sub(1) {
            cursor_char_idx - width + 1
        } else {
            0
        };
        let visible: String = text.chars().skip(scroll).take(width).collect();
        buf.set_string(area.x, area.y, &visible, style);
        if !is_placeholder {
            let cursor_x = (cursor_char_idx - scroll) as u16;
            if cursor_x < area.width {
                let cursor_char = text.chars().nth(cursor_char_idx).unwrap_or(' ');
                buf.set_string(
                    area.x + cursor_x,
                    area.y,
                    cursor_char.to_string(),
                    self.cursor_style,
                );
            }
        }
    }
}
