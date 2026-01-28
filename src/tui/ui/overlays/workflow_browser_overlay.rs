//! Workflow browser overlay for viewing and selecting workflow configurations.

use crate::tui::TabManager;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

/// Draw the workflow browser overlay for viewing and selecting workflows.
pub fn draw_workflow_browser_overlay(frame: &mut Frame, tab_manager: &TabManager) {
    let area = frame.area();

    let popup_width = (area.width as f32 * 0.75).min(90.0) as u16;
    let popup_height = (area.height as f32 * 0.70).min(30.0) as u16;
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);
    frame.render_widget(Clear, popup_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Length(2), // Column headers
            Constraint::Min(0),    // Workflow list
            Constraint::Length(4), // Selected workflow details (2 lines)
            Constraint::Length(2), // Instructions
        ])
        .split(popup_area);

    // Title block
    let title = Paragraph::new(Line::from(vec![Span::styled(
        " Select Workflow ",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" /workflow "),
    );
    frame.render_widget(title, chunks[0]);

    // Column headers
    let header = Paragraph::new(Line::from(vec![
        Span::styled("   ", Style::default()),
        Span::styled(
            format!("{:<16}", "Name"),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:<10}", "Planning"),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:<20}", "Reviewing"),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "Source",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    frame.render_widget(header, chunks[1]);

    // Workflow list block
    let list_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue))
        .title(" Workflows (j/k navigate, Enter select) ");

    let inner_area = list_block.inner(chunks[2]);
    let entries = &tab_manager.workflow_browser.entries;

    if entries.is_empty() {
        let empty_para = Paragraph::new(Line::from(vec![Span::styled(
            " No workflows found. ",
            Style::default().fg(Color::DarkGray),
        )]))
        .block(list_block);
        frame.render_widget(empty_para, chunks[2]);
    } else {
        let visible_height = inner_area.height as usize;
        let scroll_offset = tab_manager.workflow_browser.scroll_offset;
        let selected_idx = tab_manager.workflow_browser.selected_idx;

        let mut lines: Vec<Line> = Vec::new();

        for (i, entry) in entries
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(visible_height)
        {
            let is_selected = i == selected_idx;

            // Selection prefix with checkmark for currently active workflow
            let prefix = if is_selected {
                " > ".to_string()
            } else {
                "   ".to_string()
            };
            let active_indicator = if entry.is_selected { "✓" } else { " " };

            // Truncate names to fit columns (use char count to handle UTF-8)
            let name_display: String = if entry.name.chars().count() > 14 {
                let truncated: String = entry.name.chars().take(11).collect();
                format!("{}...", truncated)
            } else {
                entry.name.clone()
            };
            let planning_display: String = if entry.planning_agent.chars().count() > 10 {
                let truncated: String = entry.planning_agent.chars().take(7).collect();
                format!("{}...", truncated)
            } else {
                entry.planning_agent.clone()
            };
            let reviewing_display: String = if entry.reviewing_agents.chars().count() > 20 {
                let truncated: String = entry.reviewing_agents.chars().take(17).collect();
                format!("{}...", truncated)
            } else {
                entry.reviewing_agents.clone()
            };

            let style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let source_style = Style::default().fg(Color::DarkGray);

            lines.push(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(active_indicator, Style::default().fg(Color::Green)),
                Span::styled(format!("{:<14}", name_display), style),
                Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{:<10}", planning_display), style),
                Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{:<20}", reviewing_display), style),
                Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
                Span::styled(&entry.source, source_style),
            ]));
        }

        let list_para = Paragraph::new(lines).block(list_block);
        frame.render_widget(list_para, chunks[2]);

        // Scrollbar if needed
        if entries.len() > visible_height {
            let mut scrollbar_state = ScrollbarState::new(entries.len())
                .viewport_content_length(visible_height)
                .position(scroll_offset);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(Some("↑"))
                    .end_symbol(Some("↓")),
                chunks[2],
                &mut scrollbar_state,
            );
        }
    }

    // Selected workflow details panel
    if let Some(entry) = tab_manager.workflow_browser.selected_entry() {
        let details = Paragraph::new(vec![
            Line::from(vec![
                Span::styled(" Aggregation: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&entry.aggregation, Style::default().fg(Color::Yellow)),
                Span::styled("  Sequential: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    if entry.sequential_review { "Yes" } else { "No" },
                    Style::default().fg(if entry.sequential_review {
                        Color::Green
                    } else {
                        Color::DarkGray
                    }),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Implementing: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&entry.implementing_agent, Style::default().fg(Color::Cyan)),
                Span::styled("  Impl Review: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    &entry.implementation_reviewing_agent,
                    Style::default().fg(Color::Cyan),
                ),
            ]),
        ])
        .block(Block::default().borders(Borders::TOP));
        frame.render_widget(details, chunks[3]);
    }

    // Instructions
    let instructions = Paragraph::new(Line::from(vec![
        Span::styled(
            " [j/k] ",
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
        Span::raw("Select "),
        Span::styled(
            " [Esc] ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw("Cancel"),
    ]))
    .block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(instructions, chunks[4]);
}
