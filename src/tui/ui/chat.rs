use super::theme::Theme;
use super::util::{compute_wrapped_line_count, parse_markdown_line, wrap_text_at_width};
use crate::tui::{FocusedPanel, RunTab, RunTabEntry, Session, SummaryState, ToolResultSummary, ToolTimelineEntry};
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
) {
    let theme = Theme::for_session(session);
    let is_focused = session.focused_panel == FocusedPanel::Chat;
    let border_color = if is_focused { theme.border_focused } else { theme.success };

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
                            Style::default().fg(agent_color).add_modifier(Modifier::BOLD),
                        );
                        let content = Span::styled(
                            msg.message.clone(),
                            Style::default().fg(Color::White),
                        );
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
                                let summary = format_result_summary(result_summary, max_summary_len);
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
                            Style::default().fg(agent_color).add_modifier(Modifier::BOLD),
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
        let mut scrollbar_state =
            ScrollbarState::new(wrapped_line_count).position(scroll_offset as usize);
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
    let border_color = if is_focused { theme.border_focused } else { theme.border };
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
) {
    let is_focused = session.focused_panel == FocusedPanel::Summary;
    let border_color = if is_focused { Color::Yellow } else { Color::Magenta };

    let (title, lines): (String, Vec<Line>) = if let Some(tab) = active_tab {
        match tab.summary_state {
            SummaryState::None => {
                (" Summary ".to_string(), vec![Line::from(Span::styled(
                    "No summary available",
                    Style::default().fg(Color::DarkGray),
                ))])
            }
            SummaryState::Generating => {
                let spinner_chars = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
                let spinner = spinner_chars[(tab.summary_spinner_frame as usize) % spinner_chars.len()];
                let title = if is_focused {
                    format!(" {} Summary [*] ", spinner)
                } else {
                    format!(" {} Summary ", spinner)
                };
                (title, vec![
                    Line::from(""),
                    Line::from(vec![
                        Span::styled(format!("  {} ", spinner), Style::default().fg(Color::Yellow)),
                        Span::styled("Generating summary...", Style::default().fg(Color::Cyan)),
                    ]),
                    Line::from(""),
                    Line::from(Span::styled(
                        "  This may take a moment.",
                        Style::default().fg(Color::DarkGray),
                    )),
                ])
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
                (title, vec![
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
                ])
            }
        }
    } else {
        (" Summary ".to_string(), vec![Line::from(Span::styled(
            "No active tab",
            Style::default().fg(Color::DarkGray),
        ))])
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
    let scroll_pos = active_tab.map(|t| t.summary_scroll.min(max_scroll)).unwrap_or(0);

    let paragraph = Paragraph::new(lines)
        .block(summary_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_pos as u16, 0));
    frame.render_widget(paragraph, area);

    if total_lines > visible_height {
        let mut scrollbar_state = ScrollbarState::new(total_lines).position(scroll_pos);
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

        let display_name: String = if tab.phase.len() > 12 {
            format!("{}...", &tab.phase[..9])
        } else {
            tab.phase.clone()
        };

        let style = if is_active {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
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
    if preview.len() <= max_len {
        preview.to_string()
    } else {
        format!("{}...", &preview[..max_len.saturating_sub(3)])
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
    let display_line = if first_line.is_empty() { "..." } else { first_line };
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

pub(super) fn draw_tool_calls_panel(frame: &mut Frame, session: &Session, area: Rect) {
    let tool_block = Block::default()
        .borders(Borders::ALL)
        .title(" Tool Calls ")
        .border_style(Style::default().fg(Color::Yellow));

    let inner_area = tool_block.inner(area);
    let visible_height = inner_area.height as usize;
    let max_preview_len = 40; // Truncate input_preview to ~40 chars

    let has_active = !session.active_tools_by_agent.is_empty();
    let has_completed = !session.completed_tools_by_agent.is_empty();

    let lines: Vec<Line> = if !has_active && !has_completed {
        vec![Line::from(Span::styled(
            "No active tools",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        let mut tool_lines: Vec<Line> = Vec::new();
        let mut lines_remaining = visible_height.saturating_sub(1); // Reserve 1 line for "more" indicator
        let mut total_remaining_tools = 0;

        // Render active tools first (grouped by agent)
        if has_active {
            // Sort agent names alphabetically for stable ordering
            let mut agent_names: Vec<_> = session.active_tools_by_agent.keys().collect();
            agent_names.sort();

            for agent_name in agent_names {
                if lines_remaining == 0 {
                    // Count remaining tools for the "+N more" message
                    if let Some(tools) = session.active_tools_by_agent.get(agent_name) {
                        total_remaining_tools += tools.len();
                    }
                    continue;
                }

                if let Some(tools) = session.active_tools_by_agent.get(agent_name) {
                    if tools.is_empty() {
                        continue;
                    }

                    // Add agent header
                    if lines_remaining > 0 {
                        tool_lines.push(Line::from(Span::styled(
                            format!("{}:", agent_name),
                            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                        )));
                        lines_remaining = lines_remaining.saturating_sub(1);
                    }

                    // Add active tool entries
                    for tool in tools {
                        if lines_remaining == 0 {
                            total_remaining_tools += 1;
                            continue;
                        }
                        let elapsed_secs = tool.started_at.elapsed().as_secs();
                        let label =
                            format_tool_label(&tool.display_name, &tool.input_preview, max_preview_len);
                        tool_lines.push(Line::from(vec![
                            Span::styled("  ▶ ", Style::default().fg(Color::Yellow)),
                            Span::styled(
                                format!("{} ({}s)", label, elapsed_secs),
                                Style::default().fg(Color::Yellow),
                            ),
                        ]));
                        lines_remaining = lines_remaining.saturating_sub(1);
                    }
                }
            }
        }

        // Render completed tools (newest first, grouped view)
        if has_completed && lines_remaining > 0 {
            // Get all completed tools sorted by completion time (newest first)
            let completed_tools = session.all_completed_tools();

            // Group by agent for display
            let mut by_agent: std::collections::HashMap<&str, Vec<&crate::tui::session::CompletedTool>> =
                std::collections::HashMap::new();
            for (agent, tool) in &completed_tools {
                by_agent.entry(*agent).or_default().push(*tool);
            }

            let mut agent_names: Vec<_> = by_agent.keys().collect();
            agent_names.sort();

            for agent_name in agent_names {
                if lines_remaining == 0 {
                    if let Some(tools) = by_agent.get(agent_name) {
                        total_remaining_tools += tools.len();
                    }
                    continue;
                }

                if let Some(tools) = by_agent.get(agent_name) {
                    if tools.is_empty() {
                        continue;
                    }

                    // Check if we already have an active header for this agent
                    let has_active_header = session.active_tools_by_agent.contains_key(*agent_name);

                    // Add agent header only if we don't have an active one
                    if !has_active_header && lines_remaining > 0 {
                        tool_lines.push(Line::from(Span::styled(
                            format!("{}:", agent_name),
                            Style::default().fg(Color::DarkGray),
                        )));
                        lines_remaining = lines_remaining.saturating_sub(1);
                    }

                    // Add completed tool entries
                    for tool in tools {
                        if lines_remaining == 0 {
                            total_remaining_tools += 1;
                            continue;
                        }

                        let label =
                            format_tool_label(&tool.display_name, &tool.input_preview, max_preview_len);
                        let duration_str = format_duration_secs(tool.duration_ms);

                        let (icon, style) = if tool.is_error {
                            ("  ✗ ", Style::default().fg(Color::Red))
                        } else {
                            ("  ✓ ", Style::default().fg(Color::DarkGray))
                        };

                        tool_lines.push(Line::from(vec![
                            Span::styled(icon, style),
                            Span::styled(
                                format!("{} ({})", label, duration_str),
                                style,
                            ),
                        ]));
                        lines_remaining = lines_remaining.saturating_sub(1);
                    }
                }
            }
        }

        // Add "+N more" indicator if needed
        if total_remaining_tools > 0 {
            tool_lines.push(Line::from(Span::styled(
                format!("+{} more", total_remaining_tools),
                Style::default().fg(Color::DarkGray),
            )));
        }

        tool_lines
    };

    let paragraph = Paragraph::new(lines).block(tool_block).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}
