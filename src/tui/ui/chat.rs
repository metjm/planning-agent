use super::theme::Theme;
use super::util::{compute_wrapped_line_count, parse_markdown_line, wrap_text_at_width};
use super::SPINNER_CHARS;
use crate::tui::scroll_regions::{ScrollRegion, ScrollableRegions};
use crate::tui::session::ReviewerStatus;
use crate::tui::{
    FocusedPanel, RunTab, RunTabEntry, Session, SummaryState, ToolResultSummary, ToolTimelineEntry,
};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

pub(super) fn draw_chat_content(
    frame: &mut Frame,
    session: &Session,
    active_tab: Option<&RunTab>,
    area: Rect,
    regions: &mut ScrollableRegions,
) {
    let theme = Theme::for_session(session);
    let is_focused = session.focused_panel == FocusedPanel::Chat;
    let border_color = if is_focused {
        theme.border_focused
    } else {
        theme.success
    };

    let title = if let Some(tab) = active_tab {
        if session.chat_follow_mode {
            if is_focused {
                format!(" {} [*] ", tab.phase)
            } else {
                format!(" {} ", tab.phase)
            }
        } else if is_focused {
            format!(" {} [SCROLLED *] ", tab.phase)
        } else {
            format!(" {} [SCROLLED] ", tab.phase)
        }
    } else {
        " Chat ".to_string()
    };

    let chat_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color));

    let inner_area = chat_block.inner(area);
    let visible_height = inner_area.height as usize;
    let inner_width = inner_area.width;

    let lines: Vec<Line> = if let Some(tab) = active_tab {
        if tab.entries.is_empty() {
            vec![Line::from(Span::styled(
                "Waiting for agent output...",
                Style::default().fg(Color::DarkGray),
            ))]
        } else {
            let max_preview_len = 60;
            let max_summary_len = 60;
            tab.entries
                .iter()
                .map(|entry| match entry {
                    RunTabEntry::Text(msg) => {
                        let agent_color = match msg.agent_name.as_str() {
                            "user" => theme.accent,
                            "claude" => Color::Cyan,
                            "codex" => Color::Magenta,
                            "gemini" => Color::Blue,
                            _ => Color::Yellow,
                        };
                        let badge = Span::styled(
                            format!("[{}] ", msg.agent_name),
                            Style::default()
                                .fg(agent_color)
                                .add_modifier(Modifier::BOLD),
                        );
                        let content =
                            Span::styled(msg.message.clone(), Style::default().fg(theme.text));
                        Line::from(vec![badge, content])
                    }
                    RunTabEntry::Tool(tool_entry) => {
                        let (agent_name, label, icon, style, suffix) = match tool_entry {
                            ToolTimelineEntry::Started {
                                agent_name,
                                display_name,
                                input_preview,
                                ..
                            } => {
                                let label =
                                    format_tool_label(display_name, input_preview, max_preview_len);
                                (
                                    agent_name,
                                    label,
                                    "▶ ",
                                    Style::default().fg(Color::Yellow),
                                    String::new(),
                                )
                            }
                            ToolTimelineEntry::Finished {
                                agent_name,
                                display_name,
                                input_preview,
                                duration_ms,
                                is_error,
                                result_summary,
                                ..
                            } => {
                                let label =
                                    format_tool_label(display_name, input_preview, max_preview_len);
                                let duration = format_duration_secs(*duration_ms);
                                let summary =
                                    format_result_summary(result_summary, max_summary_len);
                                let suffix = if summary.is_empty() {
                                    format!(" ({})", duration)
                                } else {
                                    format!(" ({}) - {}", duration, summary)
                                };
                                let (icon, style) = if *is_error {
                                    ("✗ ", Style::default().fg(Color::Red))
                                } else {
                                    ("✓ ", Style::default().fg(Color::DarkGray))
                                };
                                (agent_name, label, icon, style, suffix)
                            }
                        };

                        let agent_color = match agent_name.as_str() {
                            "claude" => Color::Cyan,
                            "codex" => Color::Magenta,
                            "gemini" => Color::Blue,
                            _ => Color::Yellow,
                        };
                        let badge = Span::styled(
                            format!("[{}] ", agent_name),
                            Style::default()
                                .fg(agent_color)
                                .add_modifier(Modifier::BOLD),
                        );
                        let details = format!("{}{}", label, suffix);
                        Line::from(vec![
                            badge,
                            Span::styled(icon, style),
                            Span::styled(details, style),
                        ])
                    }
                })
                .collect()
        }
    } else {
        vec![Line::from(Span::styled(
            "No active tab",
            Style::default().fg(Color::DarkGray),
        ))]
    };

    let paragraph_for_count = Paragraph::new(lines.clone()).wrap(Wrap { trim: false });
    let wrapped_line_count = paragraph_for_count.line_count(inner_width);

    let max_scroll = wrapped_line_count.saturating_sub(visible_height);

    // Register scrollable region with computed max_scroll
    regions.register(ScrollRegion::ChatContent, inner_area, max_scroll);

    let scroll_offset = if session.chat_follow_mode {
        max_scroll as u16
    } else {
        let tab_scroll = active_tab.map(|t| t.scroll_position).unwrap_or(0);
        (tab_scroll.min(max_scroll)) as u16
    };

    let paragraph = Paragraph::new(lines)
        .block(chat_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_offset, 0));
    frame.render_widget(paragraph, area);

    if wrapped_line_count > visible_height {
        let mut scrollbar_state = ScrollbarState::new(wrapped_line_count)
            .viewport_content_length(visible_height)
            .position(scroll_offset as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓")),
            area,
            &mut scrollbar_state,
        );
    }
}

pub(super) fn draw_chat_input(frame: &mut Frame, session: &Session, area: Rect) {
    if area.width < 3 || area.height < 3 {
        return;
    }

    let theme = Theme::for_session(session);
    let is_focused = session.focused_panel == FocusedPanel::ChatInput;
    let border_color = if is_focused {
        theme.border_focused
    } else {
        theme.border
    };
    let title = if session.implementation_interaction.running {
        " Follow-up [running] "
    } else {
        " Follow-up "
    };

    let has_content = !session.tab_input.is_empty() || session.has_tab_input_pastes();
    let placeholder = if session.implementation_interaction.running {
        "Running follow-up... (Esc to cancel)"
    } else {
        "Ask a follow-up about the implementation"
    };
    let input_text = if has_content {
        session.get_display_text_tab()
    } else {
        placeholder.to_string()
    };
    let input_style = if has_content {
        Style::default().fg(theme.text)
    } else {
        Style::default().fg(theme.muted)
    };

    let input_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color));

    let inner = input_block.inner(area);
    let input_width = inner.width as usize;
    let input_height = inner.height as usize;
    if input_width == 0 || input_height == 0 {
        return;
    }

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
        .wrap(Wrap { trim: false })
        .scroll((scroll as u16, 0));
    frame.render_widget(input, area);

    if is_focused {
        let cursor_screen_x = inner.x + visual_col as u16;
        let cursor_screen_y = inner.y + (visual_row.saturating_sub(scroll)) as u16;
        if cursor_screen_y < inner.y + inner.height {
            frame.set_cursor_position((
                cursor_screen_x.min(inner.x + inner.width - 1),
                cursor_screen_y,
            ));
        }
    }
}

pub(super) fn draw_summary_panel(
    frame: &mut Frame,
    session: &Session,
    active_tab: Option<&RunTab>,
    area: Rect,
    regions: &mut ScrollableRegions,
) {
    let is_focused = session.focused_panel == FocusedPanel::Summary;
    let border_color = if is_focused {
        Color::Yellow
    } else {
        Color::Magenta
    };

    let (title, lines): (String, Vec<Line>) = if let Some(tab) = active_tab {
        match tab.summary_state {
            SummaryState::None => (
                " Summary ".to_string(),
                vec![Line::from(Span::styled(
                    "No summary available",
                    Style::default().fg(Color::DarkGray),
                ))],
            ),
            SummaryState::Generating => {
                let spinner =
                    SPINNER_CHARS[(tab.summary_spinner_frame as usize) % SPINNER_CHARS.len()];
                let title = if is_focused {
                    format!(" {} Summary [*] ", spinner)
                } else {
                    format!(" {} Summary ", spinner)
                };
                (
                    title,
                    vec![
                        Line::from(""),
                        Line::from(vec![
                            Span::styled(
                                format!("  {} ", spinner),
                                Style::default().fg(Color::Yellow),
                            ),
                            Span::styled("Generating summary...", Style::default().fg(Color::Cyan)),
                        ]),
                        Line::from(""),
                        Line::from(Span::styled(
                            "  This may take a moment.",
                            Style::default().fg(Color::DarkGray),
                        )),
                    ],
                )
            }
            SummaryState::Ready => {
                let title = if is_focused {
                    " Summary [*] ".to_string()
                } else {
                    " Summary ".to_string()
                };
                let lines: Vec<Line> = tab.summary_text.lines().map(parse_markdown_line).collect();
                (title, lines)
            }
            SummaryState::Error => {
                let title = if is_focused {
                    " Summary Error [*] ".to_string()
                } else {
                    " Summary Error ".to_string()
                };
                (
                    title,
                    vec![
                        Line::from(""),
                        Line::from(Span::styled(
                            "  Failed to generate summary:",
                            Style::default().fg(Color::Red),
                        )),
                        Line::from(""),
                        Line::from(Span::styled(
                            format!("  {}", tab.summary_text),
                            Style::default().fg(Color::Red),
                        )),
                    ],
                )
            }
        }
    } else {
        (
            " Summary ".to_string(),
            vec![Line::from(Span::styled(
                "No active tab",
                Style::default().fg(Color::DarkGray),
            ))],
        )
    };

    let summary_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color));

    let inner_area = summary_block.inner(area);
    let visible_height = inner_area.height as usize;
    let inner_width = inner_area.width;

    // Compute wrapped line count using block-less paragraph
    let total_lines = compute_wrapped_line_count(&lines, inner_width);
    let max_scroll = total_lines.saturating_sub(visible_height);

    // Register scrollable region with computed max_scroll
    regions.register(ScrollRegion::SummaryPanel, inner_area, max_scroll);

    let scroll_pos = active_tab
        .map(|t| {
            if t.summary_follow_mode {
                max_scroll
            } else {
                t.summary_scroll.min(max_scroll)
            }
        })
        .unwrap_or(0);

    let paragraph = Paragraph::new(lines)
        .block(summary_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_pos as u16, 0));
    frame.render_widget(paragraph, area);

    if total_lines > visible_height {
        let mut scrollbar_state = ScrollbarState::new(total_lines)
            .viewport_content_length(visible_height)
            .position(scroll_pos);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓")),
            area,
            &mut scrollbar_state,
        );
    }
}

pub(super) fn draw_run_tabs(frame: &mut Frame, session: &Session, area: Rect) {
    let mut spans: Vec<Span> = Vec::new();

    for (i, tab) in session.run_tabs.iter().enumerate() {
        let is_active = i == session.active_run_tab;

        let display_name: String = if tab.phase.chars().count() > 12 {
            let truncated: String = tab.phase.chars().take(9).collect();
            format!("{}...", truncated)
        } else {
            tab.phase.clone()
        };

        let style = if is_active {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        spans.push(Span::styled(format!(" [{}] ", display_name), style));
    }

    if session.run_tabs.len() > 1 {
        spans.push(Span::styled(
            " ←/→ ",
            Style::default().fg(Color::DarkGray).dim(),
        ));
    }

    let tabs = Paragraph::new(Line::from(spans));
    frame.render_widget(tabs, area);
}

/// Truncate input_preview to a maximum length with ellipsis
fn truncate_input_preview(preview: &str, max_len: usize) -> String {
    if preview.chars().count() <= max_len {
        preview.to_string()
    } else {
        let truncated: String = preview.chars().take(max_len.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }
}

/// Format tool label with display_name and optional input_preview
fn format_tool_label(display_name: &str, input_preview: &str, max_preview_len: usize) -> String {
    if input_preview.is_empty() {
        display_name.to_string()
    } else {
        let truncated = truncate_input_preview(input_preview, max_preview_len);
        format!("{}: {}", display_name, truncated)
    }
}

fn format_result_summary(summary: &ToolResultSummary, max_len: usize) -> String {
    if summary.line_count == 0 {
        return "no output".to_string();
    }

    let first_line = summary.first_line.trim();
    let display_line = if first_line.is_empty() {
        "..."
    } else {
        first_line
    };
    let truncated_line = truncate_input_preview(display_line, max_len);

    if summary.line_count == 1 && !summary.truncated {
        return truncated_line;
    }

    let count_label = if summary.truncated {
        format!("{}+", summary.line_count)
    } else {
        summary.line_count.to_string()
    };
    format!("{} ({} lines)", truncated_line, count_label)
}

/// Format duration for display
fn format_duration_secs(duration_ms: u64) -> String {
    let secs = duration_ms as f64 / 1000.0;
    if secs < 10.0 {
        format!("{:.1}s", secs)
    } else {
        format!("{}s", secs as u64)
    }
}

pub(super) fn draw_reviewer_history_panel(
    frame: &mut Frame,
    session: &Session,
    area: Rect,
    regions: &mut ScrollableRegions,
) {
    let theme = Theme::for_session(session);
    let spinner_idx = (session.review_history_spinner_frame as usize) % SPINNER_CHARS.len();

    let title = if session.has_running_reviewer() {
        format!(" {} Review History ", SPINNER_CHARS[spinner_idx])
    } else {
        " Review History ".to_string()
    };

    let panel_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(theme.accent_alt));

    let inner_area = panel_block.inner(area);
    let visible_height = inner_area.height as usize;
    let inner_width = inner_area.width;

    let mut lines: Vec<Line> = Vec::new();

    if session.review_history.is_empty() {
        lines.push(Line::from(Span::styled(
            "No reviews yet",
            Style::default().fg(theme.muted),
        )));
    } else {
        // Show most recent rounds first
        for round in session.review_history.iter().rev() {
            // Determine round icon based on aggregate verdict or running state
            let has_running = round
                .reviewers
                .iter()
                .any(|r| matches!(r.status, ReviewerStatus::Running));

            let (round_icon, round_color): (String, Color) = match round.aggregate_verdict {
                Some(true) => ("✓".to_string(), theme.success),
                Some(false) => ("✗".to_string(), theme.error),
                None => {
                    if has_running {
                        (SPINNER_CHARS[spinner_idx].to_string(), theme.warning)
                    } else {
                        ("?".to_string(), theme.muted)
                    }
                }
            };

            lines.push(Line::from(vec![
                Span::styled(format!("{} ", round_icon), Style::default().fg(round_color)),
                Span::styled(
                    format!("{} Round {}", round.kind.label(), round.round),
                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                ),
            ]));

            // Each reviewer in the round
            for entry in &round.reviewers {
                let (icon, color, suffix): (String, Color, String) = match &entry.status {
                    ReviewerStatus::Running => (
                        SPINNER_CHARS[spinner_idx].to_string(),
                        theme.warning,
                        String::new(),
                    ),
                    ReviewerStatus::Completed {
                        approved,
                        summary,
                        duration_ms,
                    } => {
                        let duration_display = if *duration_ms < 10_000 {
                            format!("{:.1}s", *duration_ms as f64 / 1000.0)
                        } else {
                            format!("{}s", duration_ms / 1000)
                        };
                        let (icon, color) = if *approved {
                            ("✓".to_string(), theme.success)
                        } else {
                            ("✗".to_string(), theme.error)
                        };
                        let summary_preview = if summary.chars().count() > 30 {
                            let truncated: String = summary.chars().take(27).collect();
                            format!("{}...", truncated)
                        } else {
                            summary.clone()
                        };
                        (
                            icon,
                            color,
                            format!(" ({}) {}", duration_display, summary_preview),
                        )
                    }
                    ReviewerStatus::Failed { error } => {
                        let error_preview = if error.chars().count() > 25 {
                            let truncated: String = error.chars().take(22).collect();
                            format!("{}...", truncated)
                        } else {
                            error.clone()
                        };
                        ("!".to_string(), theme.error, format!(" {}", error_preview))
                    }
                };

                lines.push(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(format!("{} ", icon), Style::default().fg(color)),
                    Span::styled(&entry.display_id, Style::default().fg(color)),
                    Span::styled(suffix, Style::default().fg(theme.muted)),
                ]));
            }

            lines.push(Line::from("")); // Spacing between rounds
        }
    }

    // Calculate max scroll position using wrapped line count for accurate scrolling
    let total_lines = compute_wrapped_line_count(&lines, inner_width);
    let max_scroll = total_lines.saturating_sub(visible_height);

    // Register scrollable region with computed max_scroll
    regions.register(ScrollRegion::ReviewHistory, inner_area, max_scroll);

    let scroll_position = if session.review_history_follow_mode {
        max_scroll
    } else {
        session.review_history_scroll.min(max_scroll)
    };

    let paragraph = Paragraph::new(lines.clone())
        .block(panel_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_position as u16, 0));
    frame.render_widget(paragraph, area);

    // Scrollbar if content exceeds visible height
    if total_lines > visible_height {
        let mut scrollbar_state = ScrollbarState::new(total_lines)
            .viewport_content_length(visible_height)
            .position(scroll_position);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓")),
            area,
            &mut scrollbar_state,
        );
    }
}
