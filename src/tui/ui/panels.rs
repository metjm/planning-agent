use super::chat::{
    draw_chat_content, draw_chat_input, draw_reviewer_history_panel, draw_run_tabs,
    draw_summary_panel,
};
use super::cli_instances::{draw_cli_instances, CLI_INSTANCES_MIN_HEIGHT};
use super::objective::{
    compute_objective_height, draw_objective, OBJECTIVE_MAX_FRACTION, OBJECTIVE_MIN_HEIGHT,
};
use super::stats::draw_stats;
use super::theme::Theme;
use super::util::compute_wrapped_line_count;
use crate::tui::scroll_regions::{ScrollRegion, ScrollableRegions};
use crate::tui::{FocusedPanel, Session, SummaryState};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    Frame,
};

pub fn draw_main(
    frame: &mut Frame,
    session: &Session,
    area: Rect,
    regions: &mut ScrollableRegions,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(area);

    // Normal mode: output and chat panels
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[0]);

    // Compute tool panel visibility based on chat area width
    let show_tool_panel = left_chunks[1].width >= 70;

    draw_output(frame, session, left_chunks[0], regions);
    draw_chat(frame, session, left_chunks[1], show_tool_panel, regions);

    // Split right column into Objective (top), CLI Instances (middle), and Stats (bottom)
    let right_area = chunks[1];
    let objective_text = session
        .workflow_view
        .as_ref()
        .and_then(|v| v.objective())
        .map(|o| o.as_str())
        .unwrap_or("");

    // Compute objective height based on content and available space
    let max_objective_height = ((right_area.height as f32) * OBJECTIVE_MAX_FRACTION) as u16;
    let objective_height =
        compute_objective_height(objective_text, right_area.width, max_objective_height)
            .max(OBJECTIVE_MIN_HEIGHT);

    // Compute CLI instances panel height: minimum height always, grows with instance count
    let instance_count = session.cli_instances.len();
    // Each instance takes 1 line, plus 2 for borders/title
    let desired_cli_height = (instance_count as u16)
        .saturating_add(2)
        .max(CLI_INSTANCES_MIN_HEIGHT);
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
    // Always show live tools in Stats since implementation terminal mode is removed
    draw_stats(frame, session, right_chunks[2], true);
}

fn draw_output(frame: &mut Frame, session: &Session, area: Rect, regions: &mut ScrollableRegions) {
    let show_todos = area.width >= 80 && !session.todos.is_empty();

    if show_todos {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
            .split(area);

        draw_output_panel(frame, session, chunks[0], regions);
        draw_todos(frame, session, chunks[1], regions);
    } else {
        draw_output_panel(frame, session, area, regions);
    }
}

fn draw_output_panel(
    frame: &mut Frame,
    session: &Session,
    area: Rect,
    regions: &mut ScrollableRegions,
) {
    let theme = Theme::for_session(session);
    let is_focused = session.focused_panel == FocusedPanel::Output;
    let title = if session.output_scroll.follow {
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

    let border_color = if is_focused {
        theme.border_focused
    } else {
        theme.border
    };

    let output_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color));

    let inner_area = output_block.inner(area);
    let visible_height = inner_area.height as usize;
    let inner_width = inner_area.width;

    // Build all lines (not sliced) for proper scroll handling with wrapping
    let lines: Vec<Line> = if session.output_lines.is_empty() {
        vec![Line::from(Span::styled(
            "Waiting for output...",
            Style::default().fg(theme.muted),
        ))]
    } else {
        session
            .output_lines
            .iter()
            .map(|line| {
                if line.starts_with("[planning]") {
                    Line::from(Span::styled(
                        line.clone(),
                        Style::default().fg(theme.tag_planning),
                    ))
                } else if line.starts_with("[implementation]") {
                    Line::from(Span::styled(
                        line.clone(),
                        Style::default().fg(theme.tag_implementation),
                    ))
                } else if line.starts_with("[claude]") || line.starts_with("[planning-agent]") {
                    Line::from(Span::styled(
                        line.clone(),
                        Style::default().fg(theme.tag_agent),
                    ))
                } else if line.contains("error") || line.contains("Error") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(theme.error)))
                } else {
                    Line::from(line.clone())
                }
            })
            .collect()
    };

    // Compute wrapped line count for proper scroll bounds
    let total_lines = compute_wrapped_line_count(&lines, inner_width);
    let max_scroll = total_lines.saturating_sub(visible_height);

    // Register scrollable region with computed max_scroll
    regions.register(ScrollRegion::OutputPanel, inner_area, max_scroll);

    let scroll_pos = session.output_scroll.effective_position(max_scroll);

    let paragraph = Paragraph::new(lines)
        .block(output_block)
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

fn draw_todos(frame: &mut Frame, session: &Session, area: Rect, regions: &mut ScrollableRegions) {
    let theme = Theme::for_session(session);
    let is_focused = session.focused_panel == FocusedPanel::Todos;
    let border_color = if is_focused {
        theme.border_focused
    } else {
        theme.accent_alt
    };

    let title = if is_focused {
        if session.todo_scroll.follow {
            " Todos [*] "
        } else {
            " Todos [SCROLLED *] "
        }
    } else if !session.todo_scroll.follow {
        " Todos [SCROLLED] "
    } else {
        " Todos "
    };

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
                    Style::default()
                        .fg(theme.todo_header)
                        .add_modifier(Modifier::BOLD),
                ))
            } else if line.contains("[~]") {
                Line::from(vec![
                    Span::styled("  [", Style::default().fg(theme.muted)),
                    Span::styled("~", Style::default().fg(theme.todo_in_progress)),
                    Span::styled("] ", Style::default().fg(theme.muted)),
                    Span::styled(
                        line.trim_start()
                            .strip_prefix("[~] ")
                            .unwrap_or(line)
                            .to_string(),
                        Style::default().fg(theme.todo_in_progress),
                    ),
                ])
            } else if line.contains("[x]") {
                Line::from(vec![
                    Span::styled("  [", Style::default().fg(theme.muted)),
                    Span::styled("x", Style::default().fg(theme.todo_complete)),
                    Span::styled("] ", Style::default().fg(theme.muted)),
                    Span::styled(
                        line.trim_start()
                            .strip_prefix("[x] ")
                            .unwrap_or(line)
                            .to_string(),
                        Style::default()
                            .fg(theme.todo_complete)
                            .add_modifier(Modifier::DIM),
                    ),
                ])
            } else if line.contains("[ ]") {
                Line::from(vec![
                    Span::styled("  [ ] ", Style::default().fg(theme.muted)),
                    Span::styled(
                        line.trim_start()
                            .strip_prefix("[ ] ")
                            .unwrap_or(line)
                            .to_string(),
                        Style::default().fg(theme.todo_pending),
                    ),
                ])
            } else if line.is_empty() {
                Line::from("")
            } else {
                Line::from(Span::styled(line.clone(), Style::default().fg(theme.muted)))
            }
        })
        .collect();

    // Compute wrapped line count using block-less paragraph
    let total_lines = compute_wrapped_line_count(&lines, inner_width);
    let max_scroll = total_lines.saturating_sub(visible_height);

    // Register scrollable region with computed max_scroll
    regions.register(ScrollRegion::TodosPanel, inner_area, max_scroll);

    let scroll_pos = session.todo_scroll.effective_position(max_scroll);

    let paragraph = Paragraph::new(lines)
        .block(todos_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_pos as u16, 0));
    frame.render_widget(paragraph, area);

    // Render scrollbar when content exceeds visible height
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

pub fn draw_streaming(frame: &mut Frame, session: &Session, area: Rect) {
    let theme = Theme::for_session(session);
    let is_focused = session.focused_panel == FocusedPanel::Chat;
    let title = if session.streaming_scroll.follow {
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

    let border_color = if is_focused {
        theme.border_focused
    } else {
        theme.success
    };

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
            Style::default().fg(theme.muted),
        ))]
    } else {
        session
            .streaming_lines
            .iter()
            .map(|line| {
                if line.starts_with("[stderr]") {
                    Line::from(Span::styled(
                        line.clone(),
                        Style::default().fg(theme.accent_alt),
                    ))
                } else if line.contains("error") || line.contains("Error") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(theme.error)))
                } else {
                    Line::from(Span::styled(line.clone(), Style::default().fg(theme.text)))
                }
            })
            .collect()
    };

    let paragraph_for_count = Paragraph::new(lines.clone()).wrap(Wrap { trim: false });
    let wrapped_line_count = paragraph_for_count.line_count(inner_width);

    let max_scroll = wrapped_line_count.saturating_sub(visible_height);
    let scroll_offset = session.streaming_scroll.effective_position(max_scroll) as u16;

    let paragraph = Paragraph::new(lines)
        .block(streaming_block)
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

pub fn draw_chat(
    frame: &mut Frame,
    session: &Session,
    area: Rect,
    show_tool_panel: bool,
    regions: &mut ScrollableRegions,
) {
    let can_interact = session.can_interact_with_implementation();

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
        draw_reviewer_history_panel(frame, session, tool_area, regions);
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

    let (content_area, input_area) = if can_interact {
        let input_height = 4.min(chunks[1].height.max(1));
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(input_height)])
            .split(chunks[1]);
        (split[0], Some(split[1]))
    } else {
        (chunks[1], None)
    };

    let active_tab = session.run_tabs.get(session.active_run_tab);
    let has_summary = active_tab
        .map(|tab| tab.summary_state != SummaryState::None)
        .unwrap_or(false);

    if has_summary {
        let split_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(content_area);

        draw_chat_content(frame, session, active_tab, split_chunks[0], regions);
        draw_summary_panel(frame, session, active_tab, split_chunks[1], regions);
    } else {
        draw_chat_content(frame, session, active_tab, content_area, regions);
    }

    if let Some(input_area) = input_area {
        draw_chat_input(frame, session, input_area);
    }
}
