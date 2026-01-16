
use super::cli_instances::{draw_cli_instances, CLI_INSTANCES_MIN_HEIGHT};
use super::objective::{compute_objective_height, draw_objective, OBJECTIVE_MAX_FRACTION, OBJECTIVE_MIN_HEIGHT};
use super::stats::draw_stats;
use super::util::{compute_wrapped_line_count, parse_markdown_line};
use crate::tui::embedded_terminal::vt100_cell_to_style;
use crate::tui::{FocusedPanel, InputMode, RunTab, Session, SummaryState};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    Frame,
};

pub fn draw_main(frame: &mut Frame, session: &Session, area: Rect) {
    // Check if we're in implementation terminal mode
    let in_implementation_mode = session.input_mode == InputMode::ImplementationTerminal
        && session.implementation_terminal.is_some();

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(area);

    if in_implementation_mode {
        // In implementation mode, the left side is the terminal
        draw_implementation_terminal(frame, session, chunks[0]);
    } else {
        // Normal mode: output and chat panels
        let left_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(chunks[0]);

        // Compute tool panel visibility based on chat area width
        let show_tool_panel = left_chunks[1].width >= 70;

        draw_output(frame, session, left_chunks[0]);
        draw_chat(frame, session, left_chunks[1], show_tool_panel);
    }

    // Split right column into Objective (top), CLI Instances (middle), and Stats (bottom)
    let right_area = chunks[1];
    let objective_text = session
        .workflow_state
        .as_ref()
        .map(|s| s.objective.as_str())
        .unwrap_or("");

    // Compute objective height based on content and available space
    let max_objective_height = ((right_area.height as f32) * OBJECTIVE_MAX_FRACTION) as u16;
    let objective_height = compute_objective_height(objective_text, right_area.width, max_objective_height)
        .max(OBJECTIVE_MIN_HEIGHT);

    // Compute CLI instances panel height: minimum height always, grows with instance count
    let instance_count = session.cli_instances.len();
    // Each instance takes 1 line, plus 2 for borders/title
    let desired_cli_height = (instance_count as u16).saturating_add(2).max(CLI_INSTANCES_MIN_HEIGHT);
    // Allow CLI panel to expand within available right-column space after Objective
    // Stats will take the remaining space (Constraint::Min(0))
    let available_for_cli = right_area.height.saturating_sub(objective_height);
    let cli_instances_height = desired_cli_height.min(available_for_cli);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(objective_height),
            Constraint::Length(cli_instances_height),
            Constraint::Min(0),
        ])
        .split(right_area);

    draw_objective(frame, session, right_chunks[0]);
    draw_cli_instances(frame, session, right_chunks[1]);
    // Show live tools in Stats only when tool panel is NOT visible (narrow terminals)
    draw_stats(frame, session, right_chunks[2], !in_implementation_mode);
}

fn draw_output(frame: &mut Frame, session: &Session, area: Rect) {
    let show_todos = area.width >= 80 && !session.todos.is_empty();

    if show_todos {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
            .split(area);

        draw_output_panel(frame, session, chunks[0]);
        draw_todos(frame, session, chunks[1]);
    } else {
        draw_output_panel(frame, session, area);
    }
}

fn draw_output_panel(frame: &mut Frame, session: &Session, area: Rect) {
    let is_focused = session.focused_panel == FocusedPanel::Output;
    let title = if session.output_follow_mode {
        if is_focused {
            " Output [*] "
        } else {
            " Output "
        }
    } else if is_focused {
        " Output [SCROLLED *] "
    } else {
        " Output [SCROLLED] "
    };

    let border_color = if is_focused { Color::Yellow } else { Color::Blue };

    let output_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color));

    let inner_area = output_block.inner(area);
    let visible_height = inner_area.height as usize;

    let total_lines = session.output_lines.len();
    let max_scroll = total_lines.saturating_sub(visible_height);

    let start = if session.output_follow_mode {
        max_scroll
    } else {
        session.scroll_position.min(max_scroll)
    };

    let end = (start + visible_height).min(total_lines);

    let lines: Vec<Line> = if total_lines == 0 {
        vec![Line::from(Span::styled(
            "Waiting for output...",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        session.output_lines[start..end]
            .iter()
            .map(|line| {
                if line.starts_with("[planning]") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::Cyan)))
                } else if line.starts_with("[claude]") || line.starts_with("[planning-agent]") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::Green)))
                } else if line.contains("error") || line.contains("Error") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::Red)))
                } else {
                    Line::from(line.clone())
                }
            })
            .collect()
    };

    let paragraph = Paragraph::new(lines)
        .block(output_block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);

    if total_lines > visible_height {
        let mut scrollbar_state = ScrollbarState::new(total_lines).position(start);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓")),
            area,
            &mut scrollbar_state,
        );
    }
}

fn draw_todos(frame: &mut Frame, session: &Session, area: Rect) {
    let is_focused = session.focused_panel == FocusedPanel::Todos;
    let border_color = if is_focused { Color::Yellow } else { Color::Magenta };

    let title = if is_focused { " Todos [*] " } else { " Todos " };

    let todos_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color));

    let inner_area = todos_block.inner(area);
    let visible_height = inner_area.height as usize;
    let inner_width = inner_area.width;

    let todo_lines = session.get_todos_display();

    let lines: Vec<Line> = todo_lines
        .iter()
        .map(|line| {
            if line.ends_with(':') && !line.starts_with(' ') {
                Line::from(Span::styled(
                    line.clone(),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ))
            } else if line.contains("[~]") {
                Line::from(vec![
                    Span::styled("  [", Style::default().fg(Color::DarkGray)),
                    Span::styled("~", Style::default().fg(Color::Yellow)),
                    Span::styled("] ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        line.trim_start().strip_prefix("[~] ").unwrap_or(line).to_string(),
                        Style::default().fg(Color::Yellow),
                    ),
                ])
            } else if line.contains("[x]") {
                Line::from(vec![
                    Span::styled("  [", Style::default().fg(Color::DarkGray)),
                    Span::styled("x", Style::default().fg(Color::Green)),
                    Span::styled("] ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        line.trim_start().strip_prefix("[x] ").unwrap_or(line).to_string(),
                        Style::default().fg(Color::Green).add_modifier(Modifier::DIM),
                    ),
                ])
            } else if line.contains("[ ]") {
                Line::from(vec![
                    Span::styled("  [ ] ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        line.trim_start().strip_prefix("[ ] ").unwrap_or(line).to_string(),
                        Style::default().fg(Color::White),
                    ),
                ])
            } else if line.is_empty() {
                Line::from("")
            } else {
                Line::from(Span::styled(line.clone(), Style::default().fg(Color::DarkGray)))
            }
        })
        .collect();

    // Compute wrapped line count using block-less paragraph
    let total_lines = compute_wrapped_line_count(&lines, inner_width);
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll_pos = session.todo_scroll_position.min(max_scroll);

    let paragraph = Paragraph::new(lines)
        .block(todos_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_pos as u16, 0));
    frame.render_widget(paragraph, area);

    // Render scrollbar when content exceeds visible height
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

pub fn draw_streaming(frame: &mut Frame, session: &Session, area: Rect) {
    let is_focused = session.focused_panel == FocusedPanel::Chat;
    let title = if session.streaming_follow_mode {
        if is_focused {
            " Agent Output [*] "
        } else {
            " Agent Output "
        }
    } else if is_focused {
        " Agent Output [SCROLLED *] "
    } else {
        " Agent Output [SCROLLED] "
    };

    let border_color = if is_focused { Color::Yellow } else { Color::Green };

    let streaming_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color));

    let inner_area = streaming_block.inner(area);
    let visible_height = inner_area.height as usize;
    let inner_width = inner_area.width;

    let lines: Vec<Line> = if session.streaming_lines.is_empty() {
        vec![Line::from(Span::styled(
            "Waiting for Claude output...",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        session
            .streaming_lines
            .iter()
            .map(|line| {
                if line.starts_with("[Tool:") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::Yellow)))
                } else if line.starts_with("[Result]") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::Cyan)))
                } else if line.starts_with("[stderr]") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::Magenta)))
                } else if line.contains("error") || line.contains("Error") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::Red)))
                } else {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::White)))
                }
            })
            .collect()
    };

    let paragraph_for_count = Paragraph::new(lines.clone()).wrap(Wrap { trim: false });
    let wrapped_line_count = paragraph_for_count.line_count(inner_width);

    let max_scroll = wrapped_line_count.saturating_sub(visible_height);
    let scroll_offset = if session.streaming_follow_mode {
        max_scroll as u16
    } else {
        (session.streaming_scroll_position.min(max_scroll)) as u16
    };

    let paragraph = Paragraph::new(lines)
        .block(streaming_block)
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

pub fn draw_chat(frame: &mut Frame, session: &Session, area: Rect, show_tool_panel: bool) {
    // First, apply the tool-calls split at the outermost level
    let (content_area, tool_area) = if show_tool_panel {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    if let Some(tool_area) = tool_area {
        draw_tool_calls_panel(frame, session, tool_area);
    }

    // Now render the content in content_area
    if session.run_tabs.is_empty() {
        draw_streaming(frame, session, content_area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(content_area);

    draw_run_tabs(frame, session, chunks[0]);

    let active_tab = session.run_tabs.get(session.active_run_tab);
    let has_summary = active_tab
        .map(|tab| tab.summary_state != SummaryState::None)
        .unwrap_or(false);

    if has_summary {
        let split_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(chunks[1]);

        draw_chat_content(frame, session, active_tab, split_chunks[0]);
        draw_summary_panel(frame, session, active_tab, split_chunks[1]);
    } else {
        draw_chat_content(frame, session, active_tab, chunks[1]);
    }
}

fn draw_chat_content(
    frame: &mut Frame,
    session: &Session,
    active_tab: Option<&RunTab>,
    area: Rect,
) {
    let is_focused = session.focused_panel == FocusedPanel::Chat;
    let border_color = if is_focused { Color::Yellow } else { Color::Green };

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
        if tab.messages.is_empty() {
            vec![Line::from(Span::styled(
                "Waiting for agent output...",
                Style::default().fg(Color::DarkGray),
            ))]
        } else {
            tab.messages
                .iter()
                .flat_map(|msg| {
                    let agent_color = match msg.agent_name.as_str() {
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
                    vec![Line::from(vec![badge, content])]
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

fn draw_summary_panel(
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

fn draw_run_tabs(frame: &mut Frame, session: &Session, area: Rect) {
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

/// Format duration for display
fn format_duration_secs(duration_ms: u64) -> String {
    let secs = duration_ms as f64 / 1000.0;
    if secs < 10.0 {
        format!("{:.1}s", secs)
    } else {
        format!("{}s", secs as u64)
    }
}

fn draw_tool_calls_panel(frame: &mut Frame, session: &Session, area: Rect) {
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
                            Style::default().fg(Color::Cyan).add_modifier(ratatui::style::Modifier::BOLD),
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
                        let label = format_tool_label(&tool.display_name, &tool.input_preview, max_preview_len);
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

                        let label = format_tool_label(&tool.display_name, &tool.input_preview, max_preview_len);
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

        // Add "+N more" indicator if there are truncated tools
        if total_remaining_tools > 0 {
            tool_lines.push(Line::from(Span::styled(
                format!("+{} more", total_remaining_tools),
                Style::default().fg(Color::DarkGray),
            )));
        }

        tool_lines
    };

    let paragraph = Paragraph::new(lines).block(tool_block);
    frame.render_widget(paragraph, area);
}

/// Draw the embedded implementation terminal
fn draw_implementation_terminal(frame: &mut Frame, session: &Session, area: Rect) {
    let is_focused = session.focused_panel == FocusedPanel::Implementation;
    let border_color = if is_focused { Color::Yellow } else { Color::Cyan };

    let follow_indicator = if session.implementation_terminal
        .as_ref()
        .map(|t| t.follow_mode)
        .unwrap_or(true)
    {
        ""
    } else {
        "[SCROLLED] "
    };

    let title = format!(" {}Implementation Terminal [Ctrl+\\ to exit] ", follow_indicator);

    let terminal_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color));

    let inner_area = terminal_block.inner(area);
    frame.render_widget(terminal_block, area);

    // Render the vt100 screen content
    if let Some(ref impl_term) = session.implementation_terminal {
        let screen = impl_term.screen();
        let screen_size = screen.size();
        let screen_rows = screen_size.0 as usize;
        let screen_cols = screen_size.1 as usize;
        let visible_rows = inner_area.height as usize;
        let visible_cols = inner_area.width as usize;

        // Get scrollback info for scrollbar
        let scrollback_len = screen.scrollback();
        let total_content_rows = screen_rows + scrollback_len;
        // Note: scroll_offset is tracked but scrollback viewing isn't implemented
        // due to vt100 API limitations (no cell-level access to scrollback)
        let _scroll_offset = impl_term.scroll_offset;

        // Render each row of the visible screen
        // Note: vt100's cell() API only provides access to the current visible screen,
        // not the scrollback buffer. For full scrollback rendering, we'd need to use
        // contents_formatted() and parse ANSI codes, which is complex.
        // For now, we render the current screen with full styling.
        let rows_to_render = visible_rows.min(screen_rows);
        for y in 0..rows_to_render {
            for x in 0..visible_cols.min(screen_cols) {
                if let Some(cell) = screen.cell(y as u16, x as u16) {
                    // Skip wide character continuation cells
                    if cell.is_wide_continuation() {
                        continue;
                    }

                    let contents = cell.contents();
                    let style = vt100_cell_to_style(cell);

                    // Write to the buffer
                    let buf_x = inner_area.x + x as u16;
                    let buf_y = inner_area.y + y as u16;

                    if buf_x < inner_area.x + inner_area.width && buf_y < inner_area.y + inner_area.height {
                        let buf = frame.buffer_mut();
                        if let Some(buf_cell) = buf.cell_mut((buf_x, buf_y)) {
                            buf_cell.set_symbol(if contents.is_empty() { " " } else { contents });
                            buf_cell.set_style(style);
                        }
                    }
                }
            }
        }

        // Show scrollbar indicator if there's scrollback history
        // This shows that content exists above even though we can't scroll to it
        if total_content_rows > visible_rows || scrollback_len > 0 {
            let current_pos = total_content_rows.saturating_sub(screen_rows);
            let mut scrollbar_state = ScrollbarState::new(total_content_rows).position(current_pos);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(Some("↑"))
                    .end_symbol(Some("↓")),
                area,
                &mut scrollbar_state,
            );
        }
    } else {
        // No terminal, show placeholder
        let placeholder = Paragraph::new(Line::from(Span::styled(
            "No implementation terminal active",
            Style::default().fg(Color::DarkGray),
        )));
        frame.render_widget(placeholder, inner_area);
    }
}
