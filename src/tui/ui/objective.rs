
use super::util::compute_wrapped_line_count_text;
use crate::tui::Session;
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

/// Minimum height for the objective panel (title bar + at least 1 line of content).
pub const OBJECTIVE_MIN_HEIGHT: u16 = 3;

/// Maximum fraction of the right column that the objective panel can occupy.
pub const OBJECTIVE_MAX_FRACTION: f32 = 0.35;

/// Compute the height for the objective panel based on content and available space.
///
/// Returns a height that:
/// - Shows all wrapped content if it fits
/// - Clamps to at most `max_height` if content is longer
/// - Returns at least `OBJECTIVE_MIN_HEIGHT`
pub fn compute_objective_height(objective: &str, available_width: u16, max_height: u16) -> u16 {
    if objective.is_empty() {
        return OBJECTIVE_MIN_HEIGHT;
    }

    // Account for block borders (1 each side)
    let inner_width = available_width.saturating_sub(2);
    if inner_width == 0 {
        return OBJECTIVE_MIN_HEIGHT;
    }

    let wrapped_lines = compute_wrapped_line_count_text(objective, inner_width);
    // Add 2 for top/bottom borders
    let needed_height = (wrapped_lines as u16).saturating_add(2);

    needed_height.clamp(OBJECTIVE_MIN_HEIGHT, max_height)
}

/// Draw the objective panel.
///
/// Displays the planning objective from `session.workflow_state.objective`.
/// Shows a placeholder when no objective is set.
pub fn draw_objective(frame: &mut Frame, session: &Session, area: Rect) {
    let objective_text = session
        .workflow_state
        .as_ref()
        .map(|s| s.objective.as_str())
        .unwrap_or("");

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Objective ")
        .border_style(Style::default().fg(Color::Cyan));

    let lines: Vec<Line> = if objective_text.is_empty() {
        vec![Line::from(Span::styled(
            "(not set)",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        // Split by newlines to preserve user formatting
        objective_text
            .lines()
            .map(|line| Line::from(Span::styled(line.to_string(), Style::default().fg(Color::White))))
            .collect()
    };

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}
