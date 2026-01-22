use super::dropdowns::{draw_mention_dropdown, draw_slash_dropdown};
use super::theme::Theme;
use super::util::{compute_wrapped_line_count, parse_markdown_line, wrap_text_at_width};
use super::SPINNER_CHARS;
use crate::state::{ImplementationPhase, Phase, UiMode};
use crate::tui::{ApprovalMode, FocusedPanel, Session, TabManager};
use crate::update::UpdateStatus;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
    },
    Frame,
};
use unicode_width::UnicodeWidthStr;

fn build_phase_spans(session: &Session, theme: &Theme) -> Vec<Span<'static>> {
    let ui_mode = session.ui_mode();
    let phase = session.workflow_state.as_ref().map(|s| &s.phase);
    let impl_state = session
        .workflow_state
        .as_ref()
        .and_then(|s| s.implementation_state.as_ref());
    let mut spans = Vec::new();

    if ui_mode == UiMode::Implementation {
        let impl_phases = [
            ("Implementing", ImplementationPhase::Implementing),
            ("Reviewing", ImplementationPhase::ImplementationReview),
            ("Complete", ImplementationPhase::Complete),
        ];
        let current = impl_state.map(|s| &s.phase);
        for (i, (name, p)) in impl_phases.iter().enumerate() {
            let is_cur = current == Some(p);
            let is_done = matches!(
                (current, p),
                (Some(ImplementationPhase::Complete), _)
                    | (
                        Some(ImplementationPhase::ImplementationReview),
                        ImplementationPhase::Implementing
                    )
            );
            let style = if is_cur {
                Style::default().fg(theme.phase_current).bold()
            } else if is_done {
                Style::default().fg(theme.phase_complete)
            } else {
                Style::default().fg(theme.phase_pending)
            };
            spans.push(Span::styled(*name, style));
            if i < impl_phases.len() - 1 {
                spans.push(Span::styled(" → ", Style::default().fg(theme.muted)));
            }
        }
    } else {
        let phases = [
            ("Planning", Phase::Planning),
            ("Reviewing", Phase::Reviewing),
            ("Revising", Phase::Revising),
            ("Complete", Phase::Complete),
        ];
        for (i, (name, p)) in phases.iter().enumerate() {
            let is_cur = phase == Some(p);
            let is_done = matches!(
                (phase, p),
                (Some(Phase::Complete), _)
                    | (Some(Phase::Revising), Phase::Planning)
                    | (Some(Phase::Reviewing), Phase::Planning)
            );
            let style = if is_cur {
                Style::default().fg(theme.phase_current).bold()
            } else if is_done {
                Style::default().fg(theme.phase_complete)
            } else {
                Style::default().fg(theme.phase_pending)
            };
            spans.push(Span::styled(*name, style));
            if i < phases.len() - 1 {
                spans.push(Span::styled(" → ", Style::default().fg(theme.muted)));
            }
        }
    }
    spans
}

pub fn draw_footer(frame: &mut Frame, session: &Session, tab_manager: &TabManager, area: Rect) {
    let theme = Theme::for_session(session);
    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::styled(
        format!(" Tab {}/{} ", tab_manager.active_tab + 1, tab_manager.len()),
        Style::default().fg(theme.accent),
    ));
    spans.push(Span::styled("│ ", Style::default().fg(theme.muted)));
    spans.extend(build_phase_spans(session, &theme));
    spans.push(Span::styled(" │ ", Style::default().fg(theme.muted)));

    if session.approval_mode != ApprovalMode::None {
        spans.push(Span::styled(
            "[↑/↓] Scroll  [Enter] Select  [Esc] Cancel",
            Style::default().fg(theme.muted),
        ));
    } else if session.implementation_interaction.running {
        spans.push(Span::styled(
            "[Esc] Cancel Follow-up  ",
            Style::default().fg(theme.warning),
        ));
        spans.push(Span::styled(
            "[Ctrl+PgUp/Dn] Switch Tabs",
            Style::default().fg(theme.muted),
        ));
    } else if session.can_interact_with_implementation() {
        if session.focused_panel == FocusedPanel::ChatInput {
            spans.push(Span::styled(
                "[Enter] Send  [Shift+Enter] Newline  [Esc] Cancel",
                Style::default().fg(theme.muted),
            ));
        } else {
            spans.push(Span::styled(
                "[Tab] Focus Follow-up  ",
                Style::default().fg(theme.accent_alt),
            ));
            spans.push(Span::styled(
                "[Ctrl+PgUp/Dn] Switch Tabs",
                Style::default().fg(theme.muted),
            ));
        }
    } else if session.running && session.workflow_control_tx.is_some() {
        spans.push(Span::styled(
            "[Esc] Interrupt  ",
            Style::default().fg(theme.accent_alt),
        ));
        spans.push(Span::styled(
            "[Ctrl+S] Stop  ",
            Style::default().fg(theme.warning),
        ));
        spans.push(Span::styled(
            "[Ctrl+PgUp/Dn] Switch Tabs",
            Style::default().fg(theme.muted),
        ));
    } else {
        spans.push(Span::styled(
            "Tabs: [Ctrl+PgUp/Dn] Switch  [Ctrl+W] Close",
            Style::default().fg(theme.muted),
        ));
    }

    if session.workflow_state.is_some() {
        spans.push(Span::styled(" │ ", Style::default().fg(theme.muted)));
        spans.push(Span::styled(
            "[p] Plan  [r] Reviews",
            Style::default().fg(theme.border),
        ));
    }

    let daemon_indicator = if tab_manager.daemon_connected {
        Span::styled("● ", Style::default().fg(theme.success))
    } else {
        Span::styled("○ ", Style::default().fg(theme.muted))
    };
    let version_line: Line = if let Some(info) = tab_manager.version_info.as_ref() {
        Line::from(vec![
            daemon_indicator,
            Span::styled(&info.short_sha, Style::default().fg(theme.muted)),
            Span::styled(" ", Style::default()),
            Span::styled(&info.commit_date, Style::default().fg(theme.muted)),
            Span::styled(" ", Style::default()),
        ])
    } else {
        Line::from(vec![daemon_indicator])
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.muted));
    let inner = block.inner(area);
    let left_line = Line::from(spans.clone());
    let (left_width, version_width) = (left_line.width() as u16, version_line.width() as u16);
    frame.render_widget(block, area);

    if inner.width >= left_width.saturating_add(1).saturating_add(version_width) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(version_width)])
            .split(inner);
        frame.render_widget(Paragraph::new(left_line), chunks[0]);
        frame.render_widget(Paragraph::new(version_line), chunks[1]);
    } else {
        frame.render_widget(Paragraph::new(left_line), inner);
    }
}

pub use super::approval_overlay::draw_approval_overlay;

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
            frame.set_cursor_position((
                cursor_screen_x.min(inner.x + inner.width - 1),
                cursor_screen_y,
            ));
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
        let spinner =
            SPINNER_CHARS[tab_manager.update_spinner_frame as usize % SPINNER_CHARS.len()];
        Line::from(vec![
            Span::styled(
                format!(" {} ", spinner),
                Style::default().fg(Color::Yellow).bold(),
            ),
            Span::styled(
                "Installing update... ",
                Style::default().fg(Color::Yellow).bold(),
            ),
            Span::styled(
                "(this may take a moment)",
                Style::default().fg(Color::DarkGray),
            ),
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
            UpdateStatus::UpdateAvailable(info) => Line::from(vec![
                Span::styled(
                    " Update available ",
                    Style::default().fg(Color::Green).bold(),
                ),
                Span::styled(
                    format!("({}, {}) ", info.short_sha, info.commit_date),
                    Style::default().fg(Color::Green),
                ),
                Span::styled("Enter ", Style::default().fg(Color::DarkGray)),
                Span::styled("/update", Style::default().fg(Color::Yellow)),
                Span::styled(" to install", Style::default().fg(Color::DarkGray)),
            ]),
            UpdateStatus::CheckFailed(err) => Line::from(vec![
                Span::styled(" Update check failed: ", Style::default().fg(Color::Yellow)),
                Span::styled(err.clone(), Style::default().fg(Color::DarkGray)),
            ]),
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
            Constraint::Length(3), // Title
            Constraint::Min(0),    // Content
            Constraint::Length(3), // Instructions
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

/// Draw the review modal overlay showing review feedback with tabs.
///
/// The modal is 80% of the terminal size with a tab bar for switching reviews.
pub fn draw_review_modal(frame: &mut Frame, session: &Session) {
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
            Constraint::Length(3), // Title with tabs
            Constraint::Min(0),    // Content
            Constraint::Length(3), // Instructions
        ])
        .split(popup_area);

    // Title block with tab bar
    let tab_spans: Vec<Span> = session
        .review_modal_entries
        .iter()
        .enumerate()
        .flat_map(|(i, entry)| {
            let is_selected = i == session.review_modal_tab;
            let style = if is_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(ratatui::style::Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let bracket_style = if is_selected {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            vec![
                Span::styled("[", bracket_style),
                Span::styled(entry.display_name.clone(), style),
                Span::styled("] ", bracket_style),
            ]
        })
        .collect();

    let title_line = Line::from(tab_spans);
    let title = Paragraph::new(title_line).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" Reviews (Tab/Arrow to switch) "),
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

    let content_text = session.current_review_content();
    let content_lines: Vec<Line> = content_text.lines().map(parse_markdown_line).collect();

    let total_lines = compute_wrapped_line_count(&content_lines, inner_width);
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll_pos = session.review_modal_scroll.min(max_scroll);

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
        Span::styled(
            "  [Tab/Arrow] ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::raw("Switch Review  "),
        Span::styled(
            "  [j/k] ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::raw("Scroll  "),
        Span::styled(
            "  [g/G] ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::raw("Top/Bottom  "),
        Span::styled(
            "  [Esc/r] ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
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
    if tab_manager.command_in_progress {
        let spinner =
            SPINNER_CHARS[tab_manager.update_spinner_frame as usize % SPINNER_CHARS.len()];
        ratatui::text::Text::from(Line::from(vec![
            Span::styled(
                format!(" {} ", spinner),
                Style::default().fg(Color::Yellow).bold(),
            ),
            Span::styled("Running command...", Style::default().fg(Color::Yellow)),
        ]))
    } else if let Some(ref notice) = tab_manager.command_notice {
        // Multi-line notice - convert each line
        let lines: Vec<Line> = notice
            .lines()
            .map(|line| {
                // Color code status symbols
                if line.contains("✓") {
                    Line::from(Span::styled(
                        format!(" {}", line),
                        Style::default().fg(Color::Green),
                    ))
                } else if line.contains("✗") {
                    Line::from(Span::styled(
                        format!(" {}", line),
                        Style::default().fg(Color::Red),
                    ))
                } else if line.contains("○") {
                    Line::from(Span::styled(
                        format!(" {}", line),
                        Style::default().fg(Color::DarkGray),
                    ))
                } else if line.starts_with("[config") {
                    Line::from(Span::styled(
                        format!(" {}", line),
                        Style::default().fg(Color::Cyan).bold(),
                    ))
                } else if line.trim().starts_with("Note:") {
                    Line::from(Span::styled(
                        format!(" {}", line),
                        Style::default().fg(Color::Yellow),
                    ))
                } else {
                    Line::from(Span::styled(
                        format!(" {}", line),
                        Style::default().fg(Color::White),
                    ))
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
