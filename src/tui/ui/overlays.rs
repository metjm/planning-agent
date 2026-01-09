
use super::util::{compute_wrapped_line_count, compute_wrapped_line_count_text, parse_markdown_line, wrap_text_at_width};
use crate::state::Phase;
use crate::tui::mention::MentionState;
use crate::tui::{ApprovalContext, ApprovalMode, FeedbackTarget, Session, TabManager};
use crate::update::UpdateStatus;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

pub fn draw_footer(frame: &mut Frame, session: &Session, tab_manager: &TabManager, area: Rect) {
    let phase = session.workflow_state.as_ref().map(|s| &s.phase);

    let phases = [
        ("Planning", Phase::Planning),
        ("Reviewing", Phase::Reviewing),
        ("Revising", Phase::Revising),
        ("Complete", Phase::Complete),
    ];

    let mut spans: Vec<Span> = Vec::new();

    spans.push(Span::styled(
        format!(" Tab {}/{} ", tab_manager.active_tab + 1, tab_manager.len()),
        Style::default().fg(Color::Cyan),
    ));
    spans.push(Span::styled("│ ", Style::default().fg(Color::DarkGray)));

    for (i, (name, p)) in phases.iter().enumerate() {
        let is_current = phase == Some(p);
        let is_complete = matches!(
            (phase, p),
            (Some(Phase::Complete), _)
                | (Some(Phase::Revising), Phase::Planning)
                | (Some(Phase::Reviewing), Phase::Planning)
        );

        let style = if is_current {
            Style::default().fg(Color::Yellow).bold()
        } else if is_complete {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        spans.push(Span::styled(*name, style));

        if i < phases.len() - 1 {
            spans.push(Span::styled(" → ", Style::default().fg(Color::DarkGray)));
        }
    }

    spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));

    if session.approval_mode != ApprovalMode::None {
        spans.push(Span::styled(
            "[↑/↓] Scroll  [Enter] Select  [Esc] Cancel",
            Style::default().fg(Color::DarkGray),
        ));
    } else if session.running && session.workflow_control_tx.is_some() {
        spans.push(Span::styled(
            "[Esc] Interrupt  ",
            Style::default().fg(Color::Magenta),
        ));
        spans.push(Span::styled(
            "[Ctrl+S] Stop  ",
            Style::default().fg(Color::Yellow),
        ));
        spans.push(Span::styled(
            "[Ctrl+PgUp/Dn] Switch Tabs",
            Style::default().fg(Color::DarkGray),
        ));
    } else {
        spans.push(Span::styled(
            "Tabs: [Ctrl+PgUp/Dn] Switch  [Ctrl+W] Close",
            Style::default().fg(Color::DarkGray),
        ));
    }

    let footer = Paragraph::new(Line::from(spans)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    frame.render_widget(footer, area);
}

pub fn draw_approval_overlay(frame: &mut Frame, session: &Session) {
    let area = frame.area();

    let popup_width = (area.width as f32 * 0.8) as u16;
    let popup_height = (area.height as f32 * 0.8) as u16;
    let popup_x = (area.width - popup_width) / 2;
    let popup_y = (area.height - popup_height) / 2;

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    match session.approval_mode {
        ApprovalMode::AwaitingChoice => {
            draw_choice_popup(frame, session, popup_area);
        }
        ApprovalMode::EnteringFeedback => {
            draw_feedback_popup(frame, session, popup_area);
        }
        ApprovalMode::None => {}
    }
}

fn draw_choice_popup(frame: &mut Frame, session: &Session, area: Rect) {
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
    let visible_height = inner_area.height as usize;
    let inner_width = inner_area.width;

    let summary_lines: Vec<Line> = session.plan_summary.lines().map(parse_markdown_line).collect();

    // Compute wrapped line count using block-less paragraph
    let total_lines = compute_wrapped_line_count(&summary_lines, inner_width);
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll_pos = session.plan_summary_scroll.min(max_scroll);

    let summary = Paragraph::new(summary_lines)
        .block(summary_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_pos as u16, 0));
    frame.render_widget(summary, chunks[1]);

    if total_lines > visible_height {
        let mut scrollbar_state = ScrollbarState::new(total_lines).position(scroll_pos);
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
            Span::styled("  [p] ", Style::default().fg(Color::Green).bold()),
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

    // Customize title and color based on feedback target
    let (title_text, block_title, border_color) = match session.feedback_target {
        FeedbackTarget::ApprovalDecline => (
            " Enter your feedback ",
            " Request Changes ",
            Color::Yellow,
        ),
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
    let input_width = inner.width as usize;
    let input_height = inner.height as usize;

    // Calculate cursor position for scrolling
    let (cursor_row, cursor_col) = if has_content {
        session.get_feedback_cursor_position(input_width)
    } else {
        (0, 0)
    };

    // Auto-scroll to keep cursor visible (same pattern as tab input)
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

    let cursor_screen_x;
    let cursor_screen_y;
    if has_content {
        cursor_screen_x = inner.x + cursor_col as u16;
        cursor_screen_y = inner.y + (cursor_row - scroll) as u16;
        if cursor_screen_y < inner.y + inner.height {
            frame.set_cursor_position((cursor_screen_x.min(inner.x + inner.width - 1), cursor_screen_y));
        }
    } else {
        cursor_screen_x = inner.x;
        cursor_screen_y = inner.y;
        frame.set_cursor_position((cursor_screen_x, cursor_screen_y));
    }

    // Draw @-mention dropdown if active
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

pub fn draw_tab_input_overlay(frame: &mut Frame, session: &Session, tab_manager: &TabManager) {
    let area = frame.area();

    let has_update_line = matches!(
        &tab_manager.update_status,
        UpdateStatus::UpdateAvailable(_) | UpdateStatus::CheckFailed(_)
    ) || tab_manager.update_error.is_some()
      || tab_manager.update_in_progress
      || tab_manager.update_notice.is_some();

    // Check for command notices/errors (from /config-dangerous, etc.)
    let has_command_line = tab_manager.command_notice.is_some()
        || tab_manager.command_error.is_some()
        || tab_manager.command_in_progress;

    // Calculate the number of lines needed for command notice
    let command_lines = if let Some(ref notice) = tab_manager.command_notice {
        notice.lines().count().max(1) as u16
    } else if tab_manager.command_in_progress || tab_manager.command_error.is_some() {
        1
    } else {
        0
    };

    let popup_width = (area.width as f32 * 0.6).min(80.0) as u16;
    let mut popup_height = 15u16;
    if has_update_line {
        popup_height += 1;
    }
    if has_command_line {
        popup_height += command_lines;
    }
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    // Build constraints dynamically based on what lines we have
    let mut constraints: Vec<Constraint> = vec![Constraint::Length(3)]; // Title
    if has_update_line {
        constraints.push(Constraint::Length(1)); // Update line
    }
    if has_command_line {
        constraints.push(Constraint::Length(command_lines)); // Command notice
    }
    constraints.push(Constraint::Min(5)); // Input
    constraints.push(Constraint::Length(2)); // Instructions

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(popup_area);

    let title = Paragraph::new(Line::from(vec![Span::styled(
        "Enter planning objective:",
        Style::default().fg(Color::Cyan).bold(),
    )]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" New Tab "),
    );
    frame.render_widget(title, chunks[0]);

    let mut chunk_idx = 1;

    if has_update_line {
        let update_line = render_update_line(tab_manager);
        let update_para = Paragraph::new(update_line);
        frame.render_widget(update_para, chunks[chunk_idx]);
        chunk_idx += 1;
    }

    if has_command_line {
        let command_line = render_command_line(tab_manager);
        let command_para = Paragraph::new(command_line);
        frame.render_widget(command_para, chunks[chunk_idx]);
        chunk_idx += 1;
    }

    let input_chunk_idx = chunk_idx;
    let instructions_chunk_idx = chunk_idx + 1;

    let has_content = !session.tab_input.is_empty() || session.has_tab_input_pastes();
    let input_text = if has_content {
        session.get_display_text_tab()
    } else {
        "What do you want to plan?".to_string()
    };

    let input_style = if has_content {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = input_block.inner(chunks[input_chunk_idx]);
    let input_width = inner.width as usize;
    let input_height = inner.height as usize;

    let (cursor_line, cursor_col) = session.get_tab_input_cursor_position();

    let mut visual_row = 0;
    let mut visual_col = cursor_col;

    for (i, line) in session.tab_input.split('\n').enumerate() {
        if i < cursor_line {
            let line_rows = if line.is_empty() {
                1
            } else {
                line.width().div_ceil(input_width)
            };
            visual_row += line_rows;
        } else if i == cursor_line {
            visual_row += cursor_col / input_width;
            visual_col = cursor_col % input_width;
            break;
        }
    }

    let scroll = if visual_row >= session.tab_input_scroll + input_height {
        visual_row.saturating_sub(input_height - 1)
    } else if visual_row < session.tab_input_scroll {
        visual_row
    } else {
        session.tab_input_scroll
    };

    let wrapped_text = wrap_text_at_width(&input_text, input_width);
    let input = Paragraph::new(wrapped_text)
        .style(input_style)
        .block(input_block)
        .scroll((scroll as u16, 0));
    frame.render_widget(input, chunks[input_chunk_idx]);

    let cursor_screen_x;
    let cursor_screen_y;
    if has_content {
        cursor_screen_x = inner.x + visual_col as u16;
        cursor_screen_y = inner.y + (visual_row - scroll) as u16;
        if cursor_screen_y < inner.y + inner.height {
            frame.set_cursor_position((cursor_screen_x.min(inner.x + inner.width - 1), cursor_screen_y));
        }
    } else {
        cursor_screen_x = inner.x;
        cursor_screen_y = inner.y;
    }

    // Draw @-mention dropdown if active
    draw_mention_dropdown(
        frame,
        &session.tab_mention_state,
        cursor_screen_x,
        cursor_screen_y,
        popup_width,
    );

    let help = Paragraph::new(Line::from(vec![
        Span::styled("[Enter]", Style::default().fg(Color::Green)),
        Span::raw(" Start  "),
        Span::styled("[Shift+Enter]", Style::default().fg(Color::Blue)),
        Span::raw(" Newline  "),
        Span::styled("[Esc]", Style::default().fg(Color::Red)),
        Span::raw(" Cancel  "),
        Span::styled("[Ctrl+C/q]", Style::default().fg(Color::Red)),
        Span::raw(" Quit"),
    ]))
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, chunks[instructions_chunk_idx]);
}

fn render_update_line(tab_manager: &TabManager) -> Line<'static> {
    if tab_manager.update_in_progress {
        const SPINNER_CHARS: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let spinner = SPINNER_CHARS[tab_manager.update_spinner_frame as usize % SPINNER_CHARS.len()];
        Line::from(vec![
            Span::styled(format!(" {} ", spinner), Style::default().fg(Color::Yellow).bold()),
            Span::styled("Installing update... ", Style::default().fg(Color::Yellow).bold()),
            Span::styled("(this may take a moment)", Style::default().fg(Color::DarkGray)),
        ])
    } else if let Some(ref notice) = tab_manager.update_notice {
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(Color::Green).bold()),
            Span::styled(notice.clone(), Style::default().fg(Color::Green)),
        ])
    } else if let Some(ref error) = tab_manager.update_error {
        Line::from(vec![
            Span::styled(" Update failed: ", Style::default().fg(Color::Red)),
            Span::styled(error.clone(), Style::default().fg(Color::Red)),
        ])
    } else {
        match &tab_manager.update_status {
            UpdateStatus::UpdateAvailable(info) => {
                Line::from(vec![
                    Span::styled(" Update available ", Style::default().fg(Color::Green).bold()),
                    Span::styled(
                        format!("({}, {}) ", info.short_sha, info.commit_date),
                        Style::default().fg(Color::Green),
                    ),
                    Span::styled("Enter ", Style::default().fg(Color::DarkGray)),
                    Span::styled("/update", Style::default().fg(Color::Yellow)),
                    Span::styled(" to install", Style::default().fg(Color::DarkGray)),
                ])
            }
            UpdateStatus::CheckFailed(err) => {
                Line::from(vec![
                    Span::styled(" Update check failed: ", Style::default().fg(Color::Yellow)),
                    Span::styled(err.clone(), Style::default().fg(Color::DarkGray)),
                ])
            }
            _ => Line::from(""),
        }
    }
}

/// Render slash command status/result line(s).
fn render_command_line(tab_manager: &TabManager) -> ratatui::text::Text<'static> {
    const SPINNER_CHARS: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

    if tab_manager.command_in_progress {
        let spinner = SPINNER_CHARS[tab_manager.update_spinner_frame as usize % SPINNER_CHARS.len()];
        ratatui::text::Text::from(Line::from(vec![
            Span::styled(format!(" {} ", spinner), Style::default().fg(Color::Yellow).bold()),
            Span::styled("Running command...", Style::default().fg(Color::Yellow)),
        ]))
    } else if let Some(ref notice) = tab_manager.command_notice {
        // Multi-line notice - convert each line
        let lines: Vec<Line> = notice
            .lines()
            .map(|line| {
                // Color code status symbols
                if line.contains("✓") {
                    Line::from(Span::styled(format!(" {}", line), Style::default().fg(Color::Green)))
                } else if line.contains("✗") {
                    Line::from(Span::styled(format!(" {}", line), Style::default().fg(Color::Red)))
                } else if line.contains("○") {
                    Line::from(Span::styled(format!(" {}", line), Style::default().fg(Color::DarkGray)))
                } else if line.starts_with("[config") {
                    Line::from(Span::styled(format!(" {}", line), Style::default().fg(Color::Cyan).bold()))
                } else if line.trim().starts_with("Note:") {
                    Line::from(Span::styled(format!(" {}", line), Style::default().fg(Color::Yellow)))
                } else {
                    Line::from(Span::styled(format!(" {}", line), Style::default().fg(Color::White)))
                }
            })
            .collect();
        ratatui::text::Text::from(lines)
    } else if let Some(ref error) = tab_manager.command_error {
        ratatui::text::Text::from(Line::from(vec![
            Span::styled(" Command failed: ", Style::default().fg(Color::Red)),
            Span::styled(error.clone(), Style::default().fg(Color::Red)),
        ]))
    } else {
        ratatui::text::Text::from("")
    }
}

pub fn draw_error_overlay(frame: &mut Frame, session: &Session) {
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
                Constraint::Min(0),     // Error content (scrollable)
                Constraint::Length(1),  // Instructions
            ])
            .split(popup_area);

        let error_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red))
            .title(" Error (j/k to scroll) ");

        let inner_area = error_block.inner(chunks[0]);
        let visible_height = inner_area.height as usize;

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

/// Draw the @-mention dropdown below the given cursor position.
/// The dropdown shows matching files and allows selection.
fn draw_mention_dropdown(
    frame: &mut Frame,
    mention_state: &MentionState,
    cursor_x: u16,
    cursor_y: u16,
    max_width: u16,
) {
    if !mention_state.active || mention_state.matches.is_empty() {
        return;
    }

    let screen_area = frame.area();
    let matches = &mention_state.matches;
    let selected_idx = mention_state.selected_idx;

    // Dropdown dimensions
    let dropdown_height = (matches.len() as u16).min(10) + 2; // +2 for borders
    let max_path_width = matches
        .iter()
        .map(|m| m.path.width())
        .max()
        .unwrap_or(20) as u16;
    let dropdown_width = (max_path_width + 4).min(max_width).max(30); // +4 for padding and selection indicator

    // Position dropdown below cursor, but flip above if not enough space below
    let space_below = screen_area.height.saturating_sub(cursor_y + 1);
    let space_above = cursor_y;

    let (dropdown_y, fits_below) = if space_below >= dropdown_height {
        (cursor_y + 1, true)
    } else if space_above >= dropdown_height {
        (cursor_y.saturating_sub(dropdown_height), false)
    } else {
        // Not enough space either way, prefer below with truncation
        (cursor_y + 1, true)
    };

    let actual_height = if fits_below {
        dropdown_height.min(space_below)
    } else {
        dropdown_height.min(space_above)
    };

    // Adjust x position if dropdown would extend past screen edge
    let dropdown_x = if cursor_x + dropdown_width > screen_area.width {
        screen_area.width.saturating_sub(dropdown_width)
    } else {
        cursor_x
    };

    let dropdown_area = Rect::new(dropdown_x, dropdown_y, dropdown_width, actual_height);

    // Clear background
    frame.render_widget(Clear, dropdown_area);

    // Build dropdown content
    let visible_matches = (actual_height.saturating_sub(2)) as usize;
    let scroll_offset = if selected_idx >= visible_matches {
        selected_idx.saturating_sub(visible_matches - 1)
    } else {
        0
    };

    let items: Vec<Line> = matches
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_matches)
        .map(|(i, m)| {
            let is_selected = i == selected_idx;
            let prefix = if is_selected { "› " } else { "  " };
            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            Line::from(Span::styled(format!("{}{}", prefix, m.path), style))
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Files & Folders (Tab to select) ");

    let dropdown = Paragraph::new(items).block(block);
    frame.render_widget(dropdown, dropdown_area);
}
