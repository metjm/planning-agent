//! Approval overlay rendering for the TUI.
//!
//! Handles plan approval, feedback input, and various decision overlays.

use super::super::dropdowns::draw_mention_dropdown;
use super::super::util::{compute_wrapped_line_count, parse_markdown_line, wrap_text_at_width};
use crate::tui::scroll::{ScrollRegion, ScrollableRegions};
use crate::tui::{ApprovalContext, ApprovalMode, FeedbackTarget, Session};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
    },
    Frame,
};

pub fn draw_approval_overlay(
    frame: &mut Frame,
    session: &Session,
    regions: &mut ScrollableRegions,
) {
    let area = frame.area();
    let popup_width = (area.width as f32 * 0.8) as u16;
    let popup_height = (area.height as f32 * 0.8) as u16;
    let popup_x = (area.width - popup_width) / 2;
    let popup_y = (area.height - popup_height) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);
    frame.render_widget(Clear, popup_area);
    match session.approval_mode {
        ApprovalMode::AwaitingChoice => draw_choice_popup(frame, session, popup_area, regions),
        ApprovalMode::EnteringFeedback => draw_feedback_popup(frame, session, popup_area),
        ApprovalMode::EnteringIterations => draw_iterations_input_popup(frame, session, popup_area),
        ApprovalMode::None => {}
    }
}

fn draw_choice_popup(
    frame: &mut Frame,
    session: &Session,
    area: Rect,
    regions: &mut ScrollableRegions,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(4),
        ])
        .split(area);

    let (title_text, title_color, border_color, block_title, summary_title) =
        match session.approval_context {
            ApprovalContext::PlanApproval => (
                " ✓ Plan Approved by AI ",
                Color::Green,
                Color::Green,
                " Review Plan ",
                " Plan Summary (j/k to scroll) ",
            ),
            ApprovalContext::ReviewDecision => (
                " ! Reviewer Errors Detected ",
                Color::Yellow,
                Color::Yellow,
                " Review Decision ",
                " Review Failure Details (j/k to scroll) ",
            ),
            ApprovalContext::PlanGenerationFailed => (
                " ✗ Plan Generation Failed ",
                Color::Red,
                Color::Red,
                " Recovery Options ",
                " Error Details (j/k to scroll) ",
            ),
            ApprovalContext::MaxIterationsReached => (
                " ⚠ Max Review Iterations Reached ",
                Color::Yellow,
                Color::Yellow,
                " Workflow Decision ",
                " Status Summary (j/k to scroll) ",
            ),
            ApprovalContext::UserOverrideApproval => (
                " ⚠ Proceeding Without AI Approval ",
                Color::Magenta,
                Color::Magenta,
                " Final Confirmation ",
                " Override Summary (j/k to scroll) ",
            ),
            ApprovalContext::AllReviewersFailed => (
                " ✗ All Reviewers Failed ",
                Color::Red,
                Color::Red,
                " Recovery Options ",
                " Failure Details (j/k to scroll) ",
            ),
            ApprovalContext::WorkflowFailure => (
                " ✗ Workflow Failed ",
                Color::Red,
                Color::Red,
                " Recovery Options ",
                " Failure Details (j/k to scroll) ",
            ),
        };

    let title = Paragraph::new(Line::from(vec![Span::styled(
        title_text,
        Style::default().fg(title_color).bold(),
    )]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(block_title),
    );
    frame.render_widget(title, chunks[0]);

    let summary_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(summary_title);
    let inner_area = summary_block.inner(chunks[1]);
    let (visible_height, inner_width) = (inner_area.height as usize, inner_area.width);

    let summary_lines: Vec<Line> = session
        .plan_summary
        .lines()
        .map(parse_markdown_line)
        .collect();
    let total_lines = compute_wrapped_line_count(&summary_lines, inner_width);
    let max_scroll = total_lines.saturating_sub(visible_height);

    // Register scrollable region with computed max_scroll
    regions.register(ScrollRegion::ApprovalSummary, inner_area, max_scroll);

    let scroll_pos = session.plan_summary_scroll.min(max_scroll);

    let summary = Paragraph::new(summary_lines)
        .block(summary_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_pos as u16, 0));
    frame.render_widget(summary, chunks[1]);

    if total_lines > visible_height {
        let mut scrollbar_state = ScrollbarState::new(total_lines)
            .viewport_content_length(visible_height)
            .position(scroll_pos);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓")),
            chunks[1],
            &mut scrollbar_state,
        );
    }

    let instructions = match session.approval_context {
        ApprovalContext::PlanApproval => Paragraph::new(vec![Line::from(vec![
            Span::styled("  [a] ", Style::default().fg(Color::Green).bold()),
            Span::raw("Accept  "),
            Span::styled("  [i] ", Style::default().fg(Color::Magenta).bold()),
            Span::raw("Implement  "),
            Span::styled("  [d] ", Style::default().fg(Color::Yellow).bold()),
            Span::raw("Decline  "),
            Span::styled("  [j/k] ", Style::default().fg(Color::Cyan).bold()),
            Span::raw("Scroll"),
        ])]),
        ApprovalContext::ReviewDecision => Paragraph::new(vec![Line::from(vec![
            Span::styled("  [c] ", Style::default().fg(Color::Green).bold()),
            Span::raw("Continue  "),
            Span::styled("  [r] ", Style::default().fg(Color::Yellow).bold()),
            Span::raw("Retry Failed  "),
            Span::styled("  [j/k] ", Style::default().fg(Color::Cyan).bold()),
            Span::raw("Scroll"),
        ])]),
        ApprovalContext::PlanGenerationFailed => Paragraph::new(vec![Line::from(vec![
            Span::styled("  [r] ", Style::default().fg(Color::Yellow).bold()),
            Span::raw("Retry  "),
            Span::styled("  [c] ", Style::default().fg(Color::Green).bold()),
            Span::raw("Continue  "),
            Span::styled("  [a] ", Style::default().fg(Color::Red).bold()),
            Span::raw("Abort  "),
            Span::styled("  [j/k] ", Style::default().fg(Color::Cyan).bold()),
            Span::raw("Scroll"),
        ])]),
        ApprovalContext::MaxIterationsReached => Paragraph::new(vec![Line::from(vec![
            Span::styled("  [y] ", Style::default().fg(Color::Green).bold()),
            Span::raw("Proceed  "),
            Span::styled("  [c] ", Style::default().fg(Color::Yellow).bold()),
            Span::raw("Continue Review  "),
            Span::styled("  [d] ", Style::default().fg(Color::Cyan).bold()),
            Span::raw("Decline/Restart  "),
            Span::styled("  [a] ", Style::default().fg(Color::Red).bold()),
            Span::raw("Abort"),
        ])]),
        ApprovalContext::UserOverrideApproval => Paragraph::new(vec![Line::from(vec![
            Span::styled("  [i] ", Style::default().fg(Color::Green).bold()),
            Span::raw("Implement  "),
            Span::styled("  [d] ", Style::default().fg(Color::Yellow).bold()),
            Span::raw("Decline  "),
            Span::styled("  [j/k] ", Style::default().fg(Color::Cyan).bold()),
            Span::raw("Scroll"),
        ])]),
        ApprovalContext::AllReviewersFailed | ApprovalContext::WorkflowFailure => {
            Paragraph::new(vec![Line::from(vec![
                Span::styled("  [r] ", Style::default().fg(Color::Yellow).bold()),
                Span::raw("Retry  "),
                Span::styled("  [s] ", Style::default().fg(Color::Blue).bold()),
                Span::raw("Stop & Save  "),
                Span::styled("  [a] ", Style::default().fg(Color::Red).bold()),
                Span::raw("Abort  "),
                Span::styled("  [j/k] ", Style::default().fg(Color::Cyan).bold()),
                Span::raw("Scroll"),
            ])])
        }
    }
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(instructions, chunks[2]);
}

fn draw_feedback_popup(frame: &mut Frame, session: &Session, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(area);

    let (title_text, block_title, border_color) = match session.feedback_target {
        FeedbackTarget::ApprovalDecline => {
            (" Enter your feedback ", " Request Changes ", Color::Yellow)
        }
        FeedbackTarget::WorkflowInterrupt => (
            " Interrupt with feedback ",
            " Interrupt Workflow ",
            Color::Magenta,
        ),
    };

    let title = Paragraph::new(Line::from(vec![Span::styled(
        title_text,
        Style::default().fg(border_color).bold(),
    )]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(block_title),
    );
    frame.render_widget(title, chunks[0]);

    let has_content = !session.user_feedback.is_empty() || session.has_feedback_pastes();
    let input_text = if has_content {
        session.get_display_text_feedback()
    } else {
        "Type your changes here...".to_string()
    };
    let input_style = if has_content {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let input_title = match session.feedback_target {
        FeedbackTarget::ApprovalDecline => " Your Feedback ",
        FeedbackTarget::WorkflowInterrupt => " Interrupt Message ",
    };

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(input_title);
    let inner = input_block.inner(chunks[1]);
    let (input_width, input_height) = (inner.width as usize, inner.height as usize);
    if input_width == 0 || input_height == 0 {
        return; // Can't render in zero-size area
    }
    let (cursor_row, cursor_col) = if has_content {
        session.get_feedback_cursor_position(input_width)
    } else {
        (0, 0)
    };

    let scroll = if cursor_row >= session.feedback_scroll + input_height {
        cursor_row.saturating_sub(input_height - 1)
    } else if cursor_row < session.feedback_scroll {
        cursor_row
    } else {
        session.feedback_scroll
    };

    let wrapped_input = wrap_text_at_width(&input_text, input_width);
    let input = Paragraph::new(wrapped_input)
        .style(input_style)
        .block(input_block)
        .scroll((scroll as u16, 0));
    frame.render_widget(input, chunks[1]);

    let (cursor_screen_x, cursor_screen_y) = if has_content {
        let x = inner.x + cursor_col as u16;
        let y = inner.y + (cursor_row - scroll) as u16;
        if y < inner.y + inner.height {
            frame.set_cursor_position((x.min(inner.x + inner.width - 1), y));
        }
        (x, y)
    } else {
        frame.set_cursor_position((inner.x, inner.y));
        (inner.x, inner.y)
    };

    draw_mention_dropdown(
        frame,
        &session.feedback_mention_state,
        cursor_screen_x,
        cursor_screen_y,
        area.width,
    );

    let submit_label = match session.feedback_target {
        FeedbackTarget::ApprovalDecline => "Submit  ",
        FeedbackTarget::WorkflowInterrupt => "Interrupt & Restart  ",
    };
    let instructions = Paragraph::new(Line::from(vec![
        Span::styled("  [Enter] ", Style::default().fg(Color::Green).bold()),
        Span::raw(submit_label),
        Span::styled("  [Esc] ", Style::default().fg(Color::Red).bold()),
        Span::raw("Cancel"),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(instructions, chunks[2]);
}

fn draw_iterations_input_popup(frame: &mut Frame, session: &Session, area: Rect) {
    // Smaller popup for simple number input
    let popup_width = 50.min(area.width);
    let popup_height = 9;
    let popup_x = area.x + (area.width - popup_width) / 2;
    let popup_y = area.y + (area.height - popup_height) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(popup_area);

    // Title
    let title = Paragraph::new(Line::from(vec![Span::styled(
        " Additional Iterations ",
        Style::default().fg(Color::Yellow).bold(),
    )]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(" Continue Review "),
    );
    frame.render_widget(title, chunks[0]);

    // Input field
    let display_text = if session.iterations_input.is_empty() {
        "1".to_string() // Show default
    } else {
        session.iterations_input.clone()
    };
    let input_style = if session.iterations_input.is_empty() {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White)
    };

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Number of additional rounds ");
    let inner = input_block.inner(chunks[1]);

    let input = Paragraph::new(display_text)
        .style(input_style)
        .block(input_block);
    frame.render_widget(input, chunks[1]);

    // Place cursor
    let cursor_x = inner.x + session.iterations_input.len() as u16;
    frame.set_cursor_position((cursor_x.min(inner.x + inner.width - 1), inner.y));

    // Instructions
    let instructions = Paragraph::new(Line::from(vec![
        Span::styled("  [Enter] ", Style::default().fg(Color::Green).bold()),
        Span::raw("Continue  "),
        Span::styled("  [Esc] ", Style::default().fg(Color::Red).bold()),
        Span::raw("Cancel"),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(instructions, chunks[2]);
}
