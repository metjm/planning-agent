use crate::state::Phase;
use crate::tui::App;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
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
    // Split main area: output (70%) | stats (30%)
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(area);

    draw_output(frame, app, chunks[0]);
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

    let paragraph = Paragraph::new(lines).block(output_block);
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

    let stats_text = vec![
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
        Line::from(vec![
            Span::styled(" Keys", Style::default().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(" j/↓: scroll down"),
        Line::from(" k/↑: scroll up"),
        Line::from(" g: top  G: bottom"),
        Line::from(" q: quit"),
    ];

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
