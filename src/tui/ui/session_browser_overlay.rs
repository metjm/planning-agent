//! Session browser overlay for viewing and resuming workflow sessions.

use crate::session_daemon::LivenessState;
use crate::tui::session_browser::ConfirmationState;
use crate::tui::TabManager;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

/// Get the color for a liveness state.
fn liveness_color(liveness: &LivenessState) -> Color {
    match liveness {
        LivenessState::Running => Color::Green,
        LivenessState::Unresponsive => Color::Yellow,
        LivenessState::Stopped => Color::DarkGray,
    }
}

/// Get the style for liveness text.
fn liveness_style(liveness: &LivenessState) -> Style {
    Style::default().fg(liveness_color(liveness))
}

/// Draw the session browser overlay for viewing and resuming sessions.
pub fn draw_session_browser_overlay(frame: &mut Frame, tab_manager: &TabManager) {
    let area = frame.area();

    let popup_width = (area.width as f32 * 0.85).min(100.0) as u16;
    let popup_height = (area.height as f32 * 0.8).min(35.0) as u16;
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    // Check if we need to show a confirmation dialog
    if let Some(ref confirmation) = tab_manager.session_browser.confirmation_pending {
        draw_confirmation_dialog(frame, popup_area, confirmation);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Length(1), // Filter/status info
            Constraint::Length(2), // Column headers
            Constraint::Min(0),    // Session list
            Constraint::Length(3), // Instructions
        ])
        .split(popup_area);

    // Title
    let title_text = if tab_manager.session_browser.resuming {
        " Loading session... "
    } else if tab_manager.session_browser.loading {
        " Refreshing... "
    } else {
        " Session Browser "
    };

    let title = Paragraph::new(Line::from(vec![Span::styled(
        title_text,
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    )]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" /sessions "),
    );
    frame.render_widget(title, chunks[0]);

    // Filter/status info
    let entries = tab_manager.session_browser.filtered_entries();
    let total_entries = tab_manager.session_browser.entries.len();
    let live_count = tab_manager
        .session_browser
        .entries
        .iter()
        .filter(|e| e.is_live)
        .count();

    let mut status_spans = Vec::new();

    // Daemon connection status
    if tab_manager.session_browser.daemon_connected {
        status_spans.push(Span::styled("● ", Style::default().fg(Color::Green)));
        status_spans.push(Span::styled(
            "Daemon connected ",
            Style::default().fg(Color::DarkGray),
        ));
    } else {
        status_spans.push(Span::styled("○ ", Style::default().fg(Color::DarkGray)));
        status_spans.push(Span::styled(
            "Daemon offline ",
            Style::default().fg(Color::DarkGray),
        ));
    }

    status_spans.push(Span::styled("│ ", Style::default().fg(Color::DarkGray)));

    // Filter status
    let filter_text = if tab_manager.session_browser.filter_current_dir {
        format!(
            "Showing {} of {} sessions (current dir) ",
            entries.len(),
            total_entries
        )
    } else {
        format!("Showing all {} sessions ", total_entries)
    };
    let filter_style = if tab_manager.session_browser.filter_current_dir {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    status_spans.push(Span::styled(filter_text, filter_style));

    // Live session count
    if live_count > 0 {
        status_spans.push(Span::styled("│ ", Style::default().fg(Color::DarkGray)));
        status_spans.push(Span::styled(
            format!("{} live", live_count),
            Style::default().fg(Color::Green),
        ));
    }

    let filter_line = Paragraph::new(Line::from(status_spans));
    frame.render_widget(filter_line, chunks[1]);

    // Column headers
    let header_line = Paragraph::new(Line::from(vec![
        Span::styled("   ", Style::default()),
        Span::styled(
            format!("{:<18}", "Feature"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:<10}", "Phase"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:<4}", "Iter"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:<10}", "Status"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:<12}", "Liveness"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "Last Seen",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    frame.render_widget(header_line, chunks[2]);

    // Session list
    let list_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue))
        .title(" Sessions (j/k navigate, Enter resume, s force-stop) ");

    let inner_area = list_block.inner(chunks[3]);

    // Check for error
    if let Some(ref error) = tab_manager.session_browser.error {
        let error_para = Paragraph::new(Line::from(vec![
            Span::styled(
                " Error: ",
                Style::default()
                    .fg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(error.clone(), Style::default().fg(Color::Red)),
        ]))
        .block(list_block);
        frame.render_widget(error_para, chunks[3]);
    } else if entries.is_empty() {
        let empty_para = Paragraph::new(Line::from(vec![Span::styled(
            " No sessions found. ",
            Style::default().fg(Color::DarkGray),
        )]))
        .block(list_block);
        frame.render_widget(empty_para, chunks[3]);
    } else {
        // Render the session list
        let visible_height = inner_area.height as usize;
        let scroll_offset = tab_manager.session_browser.scroll_offset;
        let selected_idx = tab_manager.session_browser.selected_idx;

        let mut lines: Vec<Line> = Vec::new();

        for (i, entry) in entries
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(visible_height)
        {
            let is_selected = i == selected_idx;

            let prefix = if is_selected { " > " } else { "   " };
            let dir_indicator = if entry.is_current_dir { "*" } else { " " };

            // Truncate feature name if too long
            let max_name_len = 16;
            let feature_name: String = if entry.feature_name.len() > max_name_len {
                format!("{}...", &entry.feature_name[..max_name_len - 3])
            } else {
                entry.feature_name.clone()
            };

            let style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            // Phase color
            let phase_style = match entry.phase.as_str() {
                "Complete" => Style::default().fg(Color::Green),
                "Planning" => Style::default().fg(Color::Cyan),
                "Implementation" | "Implementing" => Style::default().fg(Color::Blue),
                "Reviewing" => Style::default().fg(Color::Magenta),
                "Revising" => Style::default().fg(Color::Yellow),
                _ => Style::default().fg(Color::White),
            };

            // Workflow status color
            let status_style = match entry.workflow_status.as_str() {
                "Complete" => Style::default().fg(Color::Green),
                "Error" | "Failed" => Style::default().fg(Color::Red),
                "Stopped" => Style::default().fg(Color::DarkGray),
                "Planning" | "Implementing" | "Reviewing" | "Revising" => {
                    Style::default().fg(Color::Cyan)
                }
                _ => Style::default().fg(Color::White),
            };

            // Liveness style with color
            let liveness_str = format!("{}", entry.liveness);
            let live_style = liveness_style(&entry.liveness);

            // Truncate workflow_status and phase
            let phase_display = truncate_str(&entry.phase, 10);
            let status_display = truncate_str(&entry.workflow_status, 10);

            lines.push(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(dir_indicator, Style::default().fg(Color::Magenta)),
                Span::styled(format!("{:<17}", feature_name), style),
                Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{:<10}", phase_display), phase_style),
                Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{:<4}", entry.iteration),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{:<10}", status_display), status_style),
                Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{:<12}", liveness_str), live_style),
                Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    entry.last_seen_relative.clone(),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }

        let list_para = Paragraph::new(lines).block(list_block);
        frame.render_widget(list_para, chunks[3]);

        // Scrollbar if needed
        if entries.len() > visible_height {
            let mut scrollbar_state = ScrollbarState::new(entries.len()).position(scroll_offset);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(Some("↑"))
                    .end_symbol(Some("↓")),
                chunks[3],
                &mut scrollbar_state,
            );
        }
    }

    // Instructions
    let instructions = Paragraph::new(Line::from(vec![
        Span::styled(
            "  [j/k] ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("Navigate "),
        Span::styled(
            " [Enter] ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("Resume "),
        Span::styled(
            " [s] ",
            Style::default()
                .fg(Color::Red)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("Stop "),
        Span::styled(
            " [f] ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("Filter "),
        Span::styled(
            " [r] ",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("Refresh "),
        Span::styled(
            " [Esc/q] ",
            Style::default()
                .fg(Color::Red)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("Close"),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(instructions, chunks[4]);
}

/// Draw a confirmation dialog overlay.
fn draw_confirmation_dialog(frame: &mut Frame, parent_area: Rect, confirmation: &ConfirmationState) {
    // Draw dimmed background
    frame.render_widget(Clear, parent_area);

    let dialog_width = 60u16.min(parent_area.width.saturating_sub(4));
    let dialog_height = 9u16;
    let dialog_x = parent_area.x + (parent_area.width.saturating_sub(dialog_width)) / 2;
    let dialog_y = parent_area.y + (parent_area.height.saturating_sub(dialog_height)) / 2;

    let dialog_area = Rect::new(dialog_x, dialog_y, dialog_width, dialog_height);

    frame.render_widget(Clear, dialog_area);

    let (title, message_lines, warning_color) = match confirmation {
        ConfirmationState::ForceStop { session_id } => {
            let truncated_id = truncate_str(session_id, 30);
            (
                " Force Stop Session ",
                vec![
                    format!("Stop session: {}", truncated_id),
                    String::new(),
                    "This will mark the session as stopped.".to_string(),
                    "The process may continue running.".to_string(),
                ],
                Color::Red,
            )
        }
        ConfirmationState::CrossDirectoryResume {
            session_id,
            target_dir,
        } => {
            let truncated_id = truncate_str(session_id, 25);
            let dir_str = target_dir.display().to_string();
            let truncated_dir = truncate_str(&dir_str, 40);
            (
                " Cross-Directory Resume ",
                vec![
                    format!("Session: {}", truncated_id),
                    format!("Directory: {}", truncated_dir),
                    String::new(),
                    "Open in new terminal window?".to_string(),
                ],
                Color::Yellow,
            )
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Min(0),    // Message
            Constraint::Length(2), // Buttons
        ])
        .split(dialog_area);

    // Title
    let title_widget = Paragraph::new(Line::from(vec![Span::styled(
        title,
        Style::default()
            .fg(warning_color)
            .add_modifier(Modifier::BOLD),
    )]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(warning_color)),
    );
    frame.render_widget(title_widget, chunks[0]);

    // Message
    let message_spans: Vec<Line> = message_lines
        .iter()
        .map(|line| Line::from(Span::styled(format!(" {}", line), Style::default().fg(Color::White))))
        .collect();

    let message_widget = Paragraph::new(message_spans).block(
        Block::default()
            .borders(Borders::LEFT | Borders::RIGHT)
            .border_style(Style::default().fg(warning_color)),
    );
    frame.render_widget(message_widget, chunks[1]);

    // Buttons
    let buttons = Paragraph::new(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(
            "[y] ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Yes  ", Style::default().fg(Color::White)),
        Span::styled(
            "[n] ",
            Style::default()
                .fg(Color::Red)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("No  ", Style::default().fg(Color::White)),
        Span::styled(
            "[Esc] ",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Cancel", Style::default().fg(Color::DarkGray)),
    ]))
    .block(
        Block::default()
            .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
            .border_style(Style::default().fg(warning_color)),
    );
    frame.render_widget(buttons, chunks[2]);
}

/// Truncate a string to max length with ellipsis.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else if max_len > 3 {
        format!("{}...", &s[..max_len - 3])
    } else {
        s[..max_len].to_string()
    }
}
