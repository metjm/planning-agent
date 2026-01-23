//! Error overlay rendering.

use super::util::compute_wrapped_line_count_text;
use crate::tui::scroll_regions::{ScrollRegion, ScrollableRegions};
use crate::tui::Session;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
    },
    Frame,
};

pub fn draw_error_overlay(frame: &mut Frame, session: &Session, regions: &mut ScrollableRegions) {
    if let Some(ref error) = session.error_state {
        let area = frame.area();

        let popup_width = (area.width as f32 * 0.6).min(70.0) as u16;
        // Calculate inner width for wrapping (popup width minus borders)
        let inner_width = popup_width.saturating_sub(2);

        // Compute wrapped line count for the error text
        let wrapped_error_lines = compute_wrapped_line_count_text(error, inner_width);

        // Error layout: border (1) + empty line (1) + error text + empty line (1) + instructions (1) + border (1)
        // = 5 + wrapped_error_lines
        // Cap popup height at 80% of terminal height
        let max_popup_height = (area.height as f32 * 0.8) as u16;
        let min_popup_height = 8u16;
        let ideal_popup_height = (wrapped_error_lines as u16).saturating_add(5);
        let popup_height = ideal_popup_height.clamp(min_popup_height, max_popup_height);

        let popup_x = (area.width.saturating_sub(popup_width)) / 2;
        let popup_y = (area.height.saturating_sub(popup_height)) / 2;

        let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

        frame.render_widget(Clear, popup_area);

        // Split into error content and instructions
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),    // Error content (scrollable)
                Constraint::Length(1), // Instructions
            ])
            .split(popup_area);

        let error_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red))
            .title(" Error (j/k to scroll) ");

        let inner_area = error_block.inner(chunks[0]);
        let visible_height = inner_area.height as usize;

        // Register scrollable region
        regions.register(ScrollRegion::ErrorOverlay, inner_area);

        // Total lines = 1 (empty) + error lines + 1 (empty)
        let total_content_lines = wrapped_error_lines + 2;
        let max_scroll = total_content_lines.saturating_sub(visible_height);
        let scroll_pos = session.error_scroll.min(max_scroll);

        let error_paragraph = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(error.clone(), Style::default().fg(Color::Red))),
            Line::from(""),
        ])
        .block(error_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_pos as u16, 0));
        frame.render_widget(error_paragraph, chunks[0]);

        // Show scrollbar if content exceeds visible area
        if total_content_lines > visible_height {
            let mut scrollbar_state = ScrollbarState::new(total_content_lines).position(scroll_pos);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(Some("↑"))
                    .end_symbol(Some("↓")),
                chunks[0],
                &mut scrollbar_state,
            );
        }

        // Instructions line
        let instructions = Paragraph::new(Line::from(vec![
            Span::styled("  [Esc]", Style::default().fg(Color::Yellow)),
            Span::raw(" Close  "),
            Span::styled("[Ctrl+W]", Style::default().fg(Color::Red)),
            Span::raw(" Close Tab"),
        ]));
        frame.render_widget(instructions, chunks[1]);
    }
}
