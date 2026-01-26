//! Helper functions for rendering overlay elements.

use super::super::SPINNER_CHARS;
use crate::tui::TabManager;
use crate::update::UpdateStatus;
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

/// Render update status line for the tab input overlay.
pub fn render_update_line(tab_manager: &TabManager) -> Line<'static> {
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

/// Render slash command status/result line(s).
pub fn render_command_line(tab_manager: &TabManager) -> ratatui::text::Text<'static> {
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
                        Style::default().fg(Color::DarkGray),
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
