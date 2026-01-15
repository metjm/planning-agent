
use super::dropdowns::{draw_mention_dropdown, draw_slash_dropdown};
use super::util::{compute_wrapped_line_count, parse_markdown_line, wrap_text_at_width};
use crate::state::Phase;
use crate::tui::{ApprovalContext, ApprovalMode, FeedbackTarget, Session, TabManager};
use crate::update::UpdateStatus;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
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

    // Show 'p' Plan hint when workflow is active
    if session.workflow_state.is_some() {
        spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            "[p] Plan",
            Style::default().fg(Color::Blue),
        ));
    }

    // Build version info line for right side
    let version_line: Option<Line> = tab_manager.version_info.as_ref().map(|info| {
        Line::from(vec![
            Span::styled(&info.short_sha, Style::default().fg(Color::DarkGray)),
            Span::styled(" ", Style::default()),
            Span::styled(&info.commit_date, Style::default().fg(Color::DarkGray)),
            Span::styled(" ", Style::default()),
        ])
    });

    // Create the block first
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    // Compute inner area
    let inner = block.inner(area);

    // Calculate widths
    let left_line = Line::from(spans.clone());
    let left_width = left_line.width() as u16;
    let version_width = version_line.as_ref().map(|l| l.width() as u16).unwrap_or(0);

    // Render the block
    frame.render_widget(block, area);

    // Only render right-aligned version if there's enough space
    // Need at least: left_width + 1 (gap) + version_width
    let inner_width = inner.width;
    let min_required = left_width.saturating_add(1).saturating_add(version_width);

    if version_line.is_some() && inner_width >= min_required {
        // Split into left and right
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(version_width),
            ])
            .split(inner);

        // Left content
        let left_para = Paragraph::new(left_line);
        frame.render_widget(left_para, chunks[0]);

        // Right content (version info)
        if let Some(ver_line) = version_line {
            let right_para = Paragraph::new(ver_line);
            frame.render_widget(right_para, chunks[1]);
        }
    } else {
        // Not enough space or no version info, just render left content
        let footer = Paragraph::new(left_line);
        frame.render_widget(footer, inner);
    }
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
        ApprovalContext::AllReviewersFailed => Paragraph::new(vec![Line::from(vec![
            Span::styled("  [r] ", Style::default().fg(Color::Yellow).bold()),
            Span::raw("Retry  "),
            Span::styled("  [s] ", Style::default().fg(Color::Blue).bold()),
            Span::raw("Stop & Save  "),
            Span::styled("  [a] ", Style::default().fg(Color::Red).bold()),
            Span::raw("Abort  "),
            Span::styled("  [j/k] ", Style::default().fg(Color::Cyan).bold()),
            Span::raw("Scroll"),
        ])]),
        ApprovalContext::WorkflowFailure => Paragraph::new(vec![Line::from(vec![
            Span::styled("  [r] ", Style::default().fg(Color::Yellow).bold()),
            Span::raw("Retry  "),
            Span::styled("  [s] ", Style::default().fg(Color::Blue).bold()),
            Span::raw("Stop & Save  "),
            Span::styled("  [a] ", Style::default().fg(Color::Red).bold()),
            Span::raw("Abort  "),
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

    // Draw @-mention dropdown if active (takes priority)
    draw_mention_dropdown(
        frame,
        &session.tab_mention_state,
        cursor_screen_x,
        cursor_screen_y,
        popup_width,
    );

    // Draw slash command dropdown if active (only when mention dropdown not showing)
    if !session.tab_mention_state.active {
        draw_slash_dropdown(
            frame,
            &session.tab_slash_state,
            cursor_screen_x,
            cursor_screen_y,
            popup_width,
        );
    }

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

/// Draw the plan modal overlay showing the full plan file contents.
///
/// The modal is 80% of the terminal size with scrollable content and a scrollbar.
pub fn draw_plan_modal(frame: &mut Frame, session: &Session) {
    let area = frame.area();

    let popup_width = (area.width as f32 * 0.8) as u16;
    let popup_height = (area.height as f32 * 0.8) as u16;
    let popup_x = (area.width - popup_width) / 2;
    let popup_y = (area.height - popup_height) / 2;

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Title
            Constraint::Min(0),     // Content
            Constraint::Length(3),  // Instructions
        ])
        .split(popup_area);

    // Title block
    let plan_path = session
        .workflow_state
        .as_ref()
        .map(|s| s.plan_file.display().to_string())
        .unwrap_or_else(|| "Plan".to_string());

    let title = Paragraph::new(Line::from(vec![Span::styled(
        format!(" {} ", plan_path),
        Style::default().fg(Color::Cyan).bold(),
    )]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" Plan File "),
    );
    frame.render_widget(title, chunks[0]);

    // Content block with scrolling
    let content_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue))
        .title(" Content (j/k to scroll) ");

    let inner_area = content_block.inner(chunks[1]);
    let visible_height = inner_area.height as usize;
    let inner_width = inner_area.width;

    let content_lines: Vec<Line> = session
        .plan_modal_content
        .lines()
        .map(parse_markdown_line)
        .collect();

    let total_lines = compute_wrapped_line_count(&content_lines, inner_width);
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll_pos = session.plan_modal_scroll.min(max_scroll);

    let content = Paragraph::new(content_lines)
        .block(content_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_pos as u16, 0));
    frame.render_widget(content, chunks[1]);

    // Scrollbar
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

    // Instructions
    let instructions = Paragraph::new(Line::from(vec![
        Span::styled("  [j/k] ", Style::default().fg(Color::Cyan).bold()),
        Span::raw("Scroll  "),
        Span::styled("  [g/G] ", Style::default().fg(Color::Cyan).bold()),
        Span::raw("Top/Bottom  "),
        Span::styled("  [PgUp/Dn] ", Style::default().fg(Color::Cyan).bold()),
        Span::raw("Page  "),
        Span::styled("  [Esc/p] ", Style::default().fg(Color::Yellow).bold()),
        Span::raw("Close"),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(instructions, chunks[2]);
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
