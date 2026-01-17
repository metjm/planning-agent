//! Implementation success overlay rendering.

use crate::tui::Session;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub fn draw_implementation_success_overlay(frame: &mut Frame, session: &Session) {
    if let Some(ref modal) = session.implementation_success_modal {
        let area = frame.area();

        // Popup sizing - smaller than error overlay since message is short
        let popup_width = 50u16.min(area.width.saturating_sub(4));
        let popup_height = 7u16;

        let popup_x = (area.width.saturating_sub(popup_width)) / 2;
        let popup_y = (area.height.saturating_sub(popup_height)) / 2;

        let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

        frame.render_widget(Clear, popup_area);

        // Split into content and instructions
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),    // Content
                Constraint::Length(1), // Instructions
            ])
            .split(popup_area);

        let success_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Green))
            .title(" Implementation Complete ");

        // Build the success message
        let iteration_text = if modal.iterations_used == 1 {
            "1 iteration".to_string()
        } else {
            format!("{} iterations", modal.iterations_used)
        };

        let content = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "Implementation approved!",
                Style::default().fg(Color::Green),
            )),
            Line::from(Span::raw(format!("Completed in {}", iteration_text))),
            Line::from(""),
        ])
        .block(success_block)
        .alignment(ratatui::layout::Alignment::Center);

        frame.render_widget(content, chunks[0]);

        // Instructions line
        let instructions = Paragraph::new(Line::from(vec![
            Span::raw("  "),
            Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
            Span::raw(" or "),
            Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
            Span::raw(" to dismiss"),
        ]));
        frame.render_widget(instructions, chunks[1]);
    }
}
