use crate::state::Phase;
use crate::tui::{App, ApprovalMode};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    Frame,
};

pub fn draw(frame: &mut Frame, app: &App) {
    // Main layout: header, content, footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header
            Constraint::Min(0),    // Main content
            Constraint::Length(3), // Footer
        ])
        .split(frame.area());

    draw_header(frame, app, chunks[0]);
    draw_main(frame, app, chunks[1]);
    draw_footer(frame, app, chunks[2]);

    // Draw approval overlay if in approval mode
    if app.approval_mode != ApprovalMode::None {
        draw_approval_overlay(frame, app);
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let title = format!(
        " Planning Agent - {} ",
        app.feature_name()
    );

    let header = Paragraph::new(Line::from(vec![
        Span::styled(title, Style::default().fg(Color::Cyan).bold()),
        Span::raw(" "),
        Span::styled("[q]", Style::default().fg(Color::DarkGray)),
        Span::styled("uit", Style::default().fg(Color::DarkGray)),
    ]))
    .style(Style::default().bg(Color::DarkGray).fg(Color::White));

    frame.render_widget(header, area);
}

fn draw_main(frame: &mut Frame, app: &App, area: Rect) {
    // Split main area: left (70%) | stats (30%)
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(area);

    // Split left side: output (40%) | streaming (60%)
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[0]);

    draw_output(frame, app, left_chunks[0]);
    draw_streaming(frame, app, left_chunks[1]);
    draw_stats(frame, app, chunks[1]);
}

fn draw_output(frame: &mut Frame, app: &App, area: Rect) {
    let output_block = Block::default()
        .borders(Borders::ALL)
        .title(" Output ")
        .border_style(Style::default().fg(Color::Blue));

    let inner_area = output_block.inner(area);
    let visible_height = inner_area.height as usize;

    // Calculate which lines to show
    let total_lines = app.output_lines.len();
    let start = app.scroll_position;
    let end = (start + visible_height).min(total_lines);

    let lines: Vec<Line> = app.output_lines[start..end]
        .iter()
        .map(|line| {
            // Color different prefixes
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
        .collect();

    let paragraph = Paragraph::new(lines)
        .block(output_block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);

    // Scrollbar
    if total_lines > visible_height {
        let mut scrollbar_state = ScrollbarState::new(total_lines).position(app.scroll_position);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓")),
            area,
            &mut scrollbar_state,
        );
    }
}

fn draw_streaming(frame: &mut Frame, app: &App, area: Rect) {
    let streaming_block = Block::default()
        .borders(Borders::ALL)
        .title(" Claude Streaming ")
        .border_style(Style::default().fg(Color::Green));

    let inner_area = streaming_block.inner(area);
    let visible_height = inner_area.height as usize;

    let lines: Vec<Line> = if app.streaming_lines.is_empty() {
        vec![Line::from(Span::styled(
            "Waiting for Claude output...",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.streaming_lines
            .iter()
            .map(|line| {
                // Color based on content type
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

    // Calculate wrapped line count WITHOUT block (line_count adds block padding which we don't want)
    let paragraph_for_count = Paragraph::new(lines.clone()).wrap(Wrap { trim: false });
    let wrapped_line_count = paragraph_for_count.line_count(inner_area.width);

    // Scroll to show the latest content at the bottom
    let scroll_offset = if wrapped_line_count > visible_height {
        (wrapped_line_count - visible_height) as u16
    } else {
        0
    };

    let paragraph = Paragraph::new(lines)
        .block(streaming_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_offset, 0));
    frame.render_widget(paragraph, area);

    // Scrollbar
    if wrapped_line_count > visible_height {
        let mut scrollbar_state = ScrollbarState::new(wrapped_line_count).position(scroll_offset as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓")),
            area,
            &mut scrollbar_state,
        );
    }
}

fn draw_stats(frame: &mut Frame, app: &App, area: Rect) {
    let elapsed = app.elapsed();
    let minutes = elapsed.as_secs() / 60;
    let seconds = elapsed.as_secs() % 60;

    let (iter, max_iter) = app.iteration();

    let phase_color = match app.workflow_state.as_ref().map(|s| &s.phase) {
        Some(Phase::Planning) => Color::Yellow,
        Some(Phase::Reviewing) => Color::Blue,
        Some(Phase::Revising) => Color::Magenta,
        Some(Phase::Complete) => Color::Green,
        None => Color::Gray,
    };

    let mut stats_text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(" Status", Style::default().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::raw(" Phase: "),
            Span::styled(app.phase_name(), Style::default().fg(phase_color).bold()),
        ]),
        Line::from(format!(" Iteration: {}/{}", iter, max_iter)),
        Line::from(""),
        Line::from(vec![
            Span::styled(" Stats", Style::default().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(format!(" Elapsed: {}m {:02}s", minutes, seconds)),
        Line::from(format!(" Cost: ${:.4}", app.total_cost)),
        Line::from(""),
    ];

    // Add active tools section
    stats_text.push(Line::from(vec![
        Span::styled(" Active Tools", Style::default().add_modifier(Modifier::BOLD)),
    ]));

    if app.active_tools.is_empty() {
        stats_text.push(Line::from(Span::styled(
            " (none)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (name, start_time) in &app.active_tools {
            let elapsed = start_time.elapsed().as_secs();
            let tool_line = format!(" {} ({}s)", name, elapsed);
            stats_text.push(Line::from(Span::styled(
                tool_line,
                Style::default().fg(Color::Yellow),
            )));
        }
    }

    stats_text.push(Line::from(""));
    stats_text.push(Line::from(vec![
        Span::styled(" Keys", Style::default().add_modifier(Modifier::BOLD)),
    ]));
    stats_text.push(Line::from(" j/↓: scroll down"));
    stats_text.push(Line::from(" k/↑: scroll up"));
    stats_text.push(Line::from(" g: top  G: bottom"));
    stats_text.push(Line::from(" q: quit"));

    let stats = Paragraph::new(stats_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Info ")
            .border_style(Style::default().fg(Color::Magenta)),
    );

    frame.render_widget(stats, area);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let phase = app.workflow_state.as_ref().map(|s| &s.phase);

    let phases = vec![
        ("Planning", Phase::Planning),
        ("Reviewing", Phase::Reviewing),
        ("Revising", Phase::Revising),
        ("Complete", Phase::Complete),
    ];

    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::raw(" "));

    for (i, (name, p)) in phases.iter().enumerate() {
        let is_current = phase == Some(p);
        let is_complete = match (phase, p) {
            (Some(Phase::Complete), _) => true,
            (Some(Phase::Revising), Phase::Planning) => true,
            (Some(Phase::Reviewing), Phase::Planning) => true,
            _ => false,
        };

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

    let footer = Paragraph::new(Line::from(spans)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    frame.render_widget(footer, area);
}

fn draw_approval_overlay(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Create a centered popup
    let popup_width = (area.width as f32 * 0.8) as u16;
    let popup_height = (area.height as f32 * 0.8) as u16;
    let popup_x = (area.width - popup_width) / 2;
    let popup_y = (area.height - popup_height) / 2;

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the background
    frame.render_widget(Clear, popup_area);

    match app.approval_mode {
        ApprovalMode::AwaitingChoice => {
            draw_choice_popup(frame, app, popup_area);
        }
        ApprovalMode::EnteringFeedback => {
            draw_feedback_popup(frame, app, popup_area);
        }
        ApprovalMode::None => {}
    }
}

fn draw_choice_popup(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Title
            Constraint::Min(0),     // Summary content
            Constraint::Length(5),  // Instructions
        ])
        .split(area);

    // Title block
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " ✓ Plan Approved by AI ",
            Style::default().fg(Color::Green).bold(),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Green))
            .title(" Review Plan "),
    );
    frame.render_widget(title, chunks[0]);

    // Summary content
    let summary_lines: Vec<Line> = app
        .plan_summary
        .lines()
        .map(|line| Line::from(line.to_string()))
        .collect();

    let summary = Paragraph::new(summary_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Plan Summary "),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(summary, chunks[1]);

    // Instructions
    let instructions = Paragraph::new(vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  [a] ", Style::default().fg(Color::Green).bold()),
            Span::raw("Accept - Start implementation"),
        ]),
        Line::from(vec![
            Span::styled("  [d] ", Style::default().fg(Color::Yellow).bold()),
            Span::raw("Decline - Request changes"),
        ]),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Magenta)),
    );
    frame.render_widget(instructions, chunks[2]);
}

fn draw_feedback_popup(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Title
            Constraint::Min(0),     // Input area
            Constraint::Length(3),  // Instructions
        ])
        .split(area);

    // Title
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " Enter your feedback ",
            Style::default().fg(Color::Yellow).bold(),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(" Request Changes "),
    );
    frame.render_widget(title, chunks[0]);

    // Input area with cursor
    let input_text = if app.user_feedback.is_empty() {
        "Type your changes here...".to_string()
    } else {
        app.user_feedback.clone()
    };

    let input_style = if app.user_feedback.is_empty() {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White)
    };

    let input = Paragraph::new(input_text)
        .style(input_style)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Your Feedback "),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(input, chunks[1]);

    // Show cursor position
    let inner = chunks[1].inner(ratatui::layout::Margin::new(1, 1));
    let cursor_x = inner.x + (app.cursor_position as u16 % inner.width);
    let cursor_y = inner.y + (app.cursor_position as u16 / inner.width);
    frame.set_cursor_position((cursor_x, cursor_y));

    // Instructions
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
