
use super::stats::draw_stats;
use super::util::parse_markdown_line;
use crate::tui::{FocusedPanel, RunTab, Session, SummaryState};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    Frame,
};

pub fn draw_main(frame: &mut Frame, session: &Session, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(area);

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[0]);

    // Compute tool panel visibility based on chat area width
    let show_tool_panel = left_chunks[1].width >= 70;

    draw_output(frame, session, left_chunks[0]);
    draw_chat(frame, session, left_chunks[1], show_tool_panel);
    // Show live tools in Stats only when tool panel is NOT visible (narrow terminals)
    draw_stats(frame, session, chunks[1], !show_tool_panel);
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
    let todos_block = Block::default()
        .borders(Borders::ALL)
        .title(" Todos ")
        .border_style(Style::default().fg(Color::Magenta));

    let inner_area = todos_block.inner(area);
    let visible_height = inner_area.height as usize;

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

    let total_lines = lines.len();
    let display_lines: Vec<Line> = lines.into_iter().take(visible_height).collect();

    let paragraph = Paragraph::new(display_lines).block(todos_block);
    frame.render_widget(paragraph, area);

    if total_lines > visible_height {
        let more_count = total_lines - visible_height;
        let indicator = format!("+{} more", more_count);
        let indicator_area = Rect::new(
            area.x + area.width - indicator.len() as u16 - 2,
            area.y + area.height - 1,
            indicator.len() as u16 + 1,
            1,
        );
        let indicator_widget = Paragraph::new(Span::styled(
            indicator,
            Style::default().fg(Color::DarkGray),
        ));
        frame.render_widget(indicator_widget, indicator_area);
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

    let total_lines = lines.len();
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll_pos = active_tab.map(|t| t.summary_scroll.min(max_scroll)).unwrap_or(0);

    let paragraph = Paragraph::new(lines)
        .block(summary_block)
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

fn draw_tool_calls_panel(frame: &mut Frame, session: &Session, area: Rect) {
    let tool_block = Block::default()
        .borders(Borders::ALL)
        .title(" Tool Calls ")
        .border_style(Style::default().fg(Color::Yellow));

    let inner_area = tool_block.inner(area);
    let visible_height = inner_area.height as usize;

    let lines: Vec<Line> = if session.active_tools.is_empty() {
        vec![Line::from(Span::styled(
            "No active tools",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        let mut tool_lines: Vec<Line> = session
            .active_tools
            .iter()
            .take(visible_height.saturating_sub(1))
            .map(|(name, start_time)| {
                let elapsed = start_time.elapsed().as_secs();
                Line::from(vec![
                    Span::styled("▶ ", Style::default().fg(Color::Yellow)),
                    Span::styled(
                        format!("{} ({}s)", name, elapsed),
                        Style::default().fg(Color::Yellow),
                    ),
                ])
            })
            .collect();

        if session.active_tools.len() > visible_height.saturating_sub(1) {
            let more_count = session.active_tools.len() - (visible_height.saturating_sub(1));
            tool_lines.push(Line::from(Span::styled(
                format!("+{} more", more_count),
                Style::default().fg(Color::DarkGray),
            )));
        }

        tool_lines
    };

    let paragraph = Paragraph::new(lines).block(tool_block);
    frame.render_widget(paragraph, area);
}
