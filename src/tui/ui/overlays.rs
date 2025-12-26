
use super::util::{parse_markdown_line, wrap_text_at_width};
use crate::state::Phase;
use crate::tui::{ApprovalContext, ApprovalMode, Session, TabManager};
use crate::update::UpdateStatus;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style, Stylize},
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

    let summary_lines: Vec<Line> = session.plan_summary.lines().map(parse_markdown_line).collect();

    let total_lines = summary_lines.len();
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll_pos = session.plan_summary_scroll.min(max_scroll);

    let summary = Paragraph::new(summary_lines)
        .block(summary_block)
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

    let title = Paragraph::new(Line::from(vec![Span::styled(
        " Enter your feedback ",
        Style::default().fg(Color::Yellow).bold(),
    )]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(" Request Changes "),
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

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Your Feedback ");

    let inner = input_block.inner(chunks[1]);
    let input_width = inner.width as usize;

    let wrapped_input = wrap_text_at_width(&input_text, input_width);
    let input = Paragraph::new(wrapped_input)
        .style(input_style)
        .block(input_block);
    frame.render_widget(input, chunks[1]);

    if has_content {
        let (cursor_row, cursor_col) = session.get_feedback_cursor_position(input_width);
        let cursor_x = inner.x + cursor_col as u16;
        let cursor_y = inner.y + cursor_row as u16;
        if cursor_y < inner.y + inner.height {
            frame.set_cursor_position((cursor_x.min(inner.x + inner.width - 1), cursor_y));
        }
    } else {
        frame.set_cursor_position((inner.x, inner.y));
    }

    let instructions = Paragraph::new(Line::from(vec![
        Span::styled("  [Enter] ", Style::default().fg(Color::Green).bold()),
        Span::raw("Submit  "),
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

    let popup_width = (area.width as f32 * 0.6).min(80.0) as u16;
    let popup_height = if has_update_line { 17 } else { 15 };
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let chunks = if has_update_line {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(1),
                Constraint::Min(5),
                Constraint::Length(2),
            ])
            .split(popup_area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(5),
                Constraint::Length(2),
            ])
            .split(popup_area)
    };

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

    let (input_chunk_idx, instructions_chunk_idx) = if has_update_line {
        let update_line = render_update_line(tab_manager);
        let update_para = Paragraph::new(update_line);
        frame.render_widget(update_para, chunks[1]);
        (2, 3)
    } else {
        (1, 2)
    };

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

    if has_content {
        let cursor_x = inner.x + visual_col as u16;
        let cursor_y = inner.y + (visual_row - scroll) as u16;
        if cursor_y < inner.y + inner.height {
            frame.set_cursor_position((cursor_x.min(inner.x + inner.width - 1), cursor_y));
        }
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

pub fn draw_error_overlay(frame: &mut Frame, session: &Session) {
    if let Some(ref error) = session.error_state {
        let area = frame.area();

        let popup_width = (area.width as f32 * 0.5).min(60.0) as u16;
        let popup_height = 8;
        let popup_x = (area.width.saturating_sub(popup_width)) / 2;
        let popup_y = (area.height.saturating_sub(popup_height)) / 2;

        let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

        frame.render_widget(Clear, popup_area);

        let error_text = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(error.clone(), Style::default().fg(Color::Red))),
            Line::from(""),
            Line::from(vec![
                Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
                Span::raw(" Close  "),
                Span::styled("[Ctrl+W]", Style::default().fg(Color::Red)),
                Span::raw(" Close Tab"),
            ]),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red))
                .title(" Error "),
        )
        .wrap(Wrap { trim: false });

        frame.render_widget(error_text, popup_area);
    }
}
