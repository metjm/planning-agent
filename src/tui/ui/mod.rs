mod approval_overlay;
mod chat;
mod cli_instances;
mod dropdowns;
mod error_overlay;
mod objective;
mod overlays;
mod panels;
mod session_browser_overlay;
mod stats;
mod success_overlay;
pub mod theme;
pub mod util;

use crate::tui::{ApprovalMode, InputMode, SessionStatus, TabManager};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

pub fn draw(frame: &mut Frame, tab_manager: &TabManager) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(frame.area());

    draw_tab_bar(frame, tab_manager, chunks[0]);

    let session = tab_manager.active();
    panels::draw_main(frame, session, chunks[1]);
    overlays::draw_footer(frame, session, tab_manager, chunks[2]);

    if session.approval_mode != ApprovalMode::None {
        overlays::draw_approval_overlay(frame, session);
    }
    if session.input_mode == InputMode::NamingTab {
        overlays::draw_tab_input_overlay(frame, session, tab_manager);
    }
    // Render plan modal BEFORE error overlay so errors always take precedence
    if session.plan_modal_open {
        overlays::draw_plan_modal(frame, session);
    }
    // Render session browser overlay
    if tab_manager.session_browser.open {
        session_browser_overlay::draw_session_browser_overlay(frame, tab_manager);
    }
    // Render implementation success modal after session browser, before error overlay
    if session.implementation_success_modal.is_some() {
        success_overlay::draw_implementation_success_overlay(frame, session);
    }
    if session.error_state.is_some() {
        error_overlay::draw_error_overlay(frame, session);
    }
}

fn draw_tab_bar(frame: &mut Frame, tab_manager: &TabManager, area: Rect) {
    let active_session = tab_manager.active();
    let theme = theme::Theme::for_session(active_session);
    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::raw(" "));

    for (i, session) in tab_manager.sessions.iter().enumerate() {
        let is_active = i == tab_manager.active_tab;

        let status_icon = match session.status {
            SessionStatus::InputPending => "...",
            SessionStatus::Planning => "",
            SessionStatus::GeneratingSummary => "◐",
            SessionStatus::AwaitingApproval => "?",
            SessionStatus::Stopped => "⏸",
            SessionStatus::Complete => "+",
            SessionStatus::Error => "!",
            SessionStatus::Verifying => "⚡",
            SessionStatus::Fixing => "🔧",
            SessionStatus::VerificationComplete => "✓",
        };

        let name = if session.name.is_empty() {
            "New Tab"
        } else {
            &session.name
        };
        let display_name: String = if name.len() > 15 {
            format!("{}...", name.get(..12).unwrap_or(name))
        } else {
            name.to_string()
        };

        let label = if status_icon.is_empty() {
            format!("[{}]", display_name)
        } else {
            format!("[{} {}]", display_name, status_icon)
        };

        let style = if is_active {
            Style::default()
                .fg(theme.tab_active)
                .add_modifier(Modifier::BOLD)
        } else if session.approval_mode != ApprovalMode::None {
            Style::default().fg(theme.tab_approval)
        } else {
            Style::default().fg(theme.tab_inactive)
        };

        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
    }

    spans.push(Span::styled(
        "[Ctrl++]",
        Style::default().fg(theme.success).dim(),
    ));

    let title = if let Some(ref state) = active_session.workflow_state {
        let plan_path = state.plan_file.display().to_string();
        format!(" Planning Agent - {} ", plan_path)
    } else {
        " Planning Agent ".to_string()
    };

    let tabs = Paragraph::new(Line::from(spans)).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(theme.muted))
            .title(title)
            .title_alignment(Alignment::Center),
    );

    frame.render_widget(tabs, area);
}
