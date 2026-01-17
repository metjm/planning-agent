//! CLI Instances panel for the TUI.
//!
//! Displays active CLI agent processes with elapsed runtime and idle time.

use super::theme::Theme;
use super::util::format_duration;
use crate::tui::Session;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// Minimum height for the CLI instances panel (title + at least 2 lines of content).
pub const CLI_INSTANCES_MIN_HEIGHT: u16 = 3;

/// Draw the CLI instances panel.
///
/// Displays active CLI agent processes with:
/// - Agent name and PID (or #id fallback)
/// - Elapsed time since start
/// - Idle time since last activity
///
/// Shows "(none)" when no instances are active.
pub fn draw_cli_instances(frame: &mut Frame, session: &Session, area: Rect) {
    let theme = Theme::for_session(session);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" CLI Instances ")
        .border_style(Style::default().fg(theme.cli_border));

    let inner_area = block.inner(area);
    let visible_height = inner_area.height as usize;

    let instances = session.cli_instances_sorted();

    let lines: Vec<Line> = if instances.is_empty() {
        vec![Line::from(Span::styled(
            "(none)",
            Style::default().fg(theme.muted),
        ))]
    } else {
        let mut result: Vec<Line> = Vec::new();
        // Only reserve a line for "+N more" if truncation is actually needed
        let needs_truncation = instances.len() > visible_height;
        let display_capacity = if needs_truncation {
            visible_height.saturating_sub(1)
        } else {
            visible_height
        };

        for instance in instances.iter().take(display_capacity) {
            let elapsed = format_duration(instance.elapsed());
            let idle = format_duration(instance.idle());

            let label = instance.display_label();
            let line = Line::from(vec![
                Span::styled("▶ ", Style::default().fg(theme.cli_running)),
                Span::styled(label, Style::default().fg(theme.text)),
                Span::styled(" | up ", Style::default().fg(theme.muted)),
                Span::styled(elapsed, Style::default().fg(theme.cli_elapsed)),
                Span::styled(" | idle ", Style::default().fg(theme.muted)),
                Span::styled(idle, Style::default().fg(theme.cli_idle)),
            ]);
            result.push(line);
        }

        // Add "+N more" indicator only if truncated
        if needs_truncation {
            let remaining_count = instances.len().saturating_sub(display_capacity);
            result.push(Line::from(Span::styled(
                format!("+{} more", remaining_count),
                Style::default().fg(theme.muted),
            )));
        }

        result
    };

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}
