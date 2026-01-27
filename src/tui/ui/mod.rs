mod chat;
mod cli_instances;
mod dropdowns;
mod objective;
mod overlays;
mod panels;
mod stats;
pub mod theme;
pub mod util;

#[cfg(test)]
#[path = "tests/overlays_tests.rs"]
mod overlays_tests;

use crate::tui::scroll_regions::ScrollableRegions;
use crate::tui::{ApprovalMode, InputMode, Session, SessionStatus, TabManager};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// Spinner characters for animated activity indicators.
/// Used throughout the UI for loading/progress animations.
pub const SPINNER_CHARS: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Returns the background color for the header based on session state.
///
/// PRIORITY ORDER (checked top-to-bottom, first match wins):
/// 1. Error state - ALWAYS red, regardless of workflow_view presence
/// 2. Stopped state - muted blue-gray
/// 3. Waiting states (InputPending, AwaitingApproval when not running) - gray
/// 4. Workflow phase colors (if workflow_view exists) - phase-specific
/// 5. Default (no workflow_view and not in above states) - gray (waiting)
fn get_phase_background_color(session: &Session, theme: &theme::Theme) -> ratatui::style::Color {
    use crate::domain::types::{ImplementationPhase, Phase};

    // ============================================================
    // PRIORITY 1: Error state takes precedence over ALL other states
    // This check happens FIRST, before workflow_view is examined.
    // Even if workflow_view is None (e.g., initialization failure),
    // error state will show the correct red background.
    // ============================================================
    if matches!(session.status, SessionStatus::Error) {
        return theme.phase_bg_error;
    }

    // ============================================================
    // PRIORITY 2: Stopped state - distinct from waiting
    // ============================================================
    if matches!(session.status, SessionStatus::Stopped) {
        return theme.phase_bg_stopped;
    }

    // ============================================================
    // PRIORITY 3: Waiting states (when not actively running)
    // InputPending = waiting for user to enter objective
    // AwaitingApproval = waiting for user to approve/reject
    // ============================================================
    if matches!(
        session.status,
        SessionStatus::AwaitingApproval | SessionStatus::InputPending
    ) && !session.running
    {
        return theme.phase_bg_waiting;
    }

    // ============================================================
    // PRIORITY 4+5: Phase-based colors for active workflow
    // Only reached if not Error, Stopped, or Waiting states
    // ============================================================
    match &session.workflow_view {
        Some(view) => {
            // Check implementation phase first
            if let Some(impl_state) = view.implementation_state() {
                if impl_state.phase() != ImplementationPhase::Complete {
                    return match impl_state.phase() {
                        ImplementationPhase::Implementing => theme.phase_bg_planning,
                        ImplementationPhase::ImplementationReview => theme.phase_bg_reviewing,
                        ImplementationPhase::AwaitingDecision => theme.phase_bg_reviewing,
                        ImplementationPhase::Complete => theme.phase_bg_complete,
                    };
                }
            }
            // Planning workflow phases
            match view.planning_phase() {
                Some(Phase::Planning) => theme.phase_bg_planning,
                Some(Phase::Reviewing) => theme.phase_bg_reviewing,
                Some(Phase::Revising) => theme.phase_bg_revising,
                Some(Phase::AwaitingPlanningDecision) => theme.phase_bg_reviewing,
                Some(Phase::Complete) => theme.phase_bg_complete,
                None => theme.phase_bg_waiting,
            }
        }
        // No workflow_view AND not Error/Stopped/Waiting = use waiting color
        // (This is the initial state before workflow starts, or an edge case)
        None => theme.phase_bg_waiting,
    }
}

/// Get the status icon for a session's tab label (existing logic, extracted)
fn get_session_status_icon(session: &Session) -> &'static str {
    match session.status {
        SessionStatus::InputPending => "...",
        SessionStatus::Planning => "",
        SessionStatus::GeneratingSummary => "◐",
        SessionStatus::AwaitingApproval => "?",
        SessionStatus::Stopped => "⏸",
        SessionStatus::Complete => "+",
        SessionStatus::Error => "!",
    }
}

pub fn draw(frame: &mut Frame, tab_manager: &TabManager, scroll_regions: &mut ScrollableRegions) {
    // Clear scroll regions at start of each frame
    scroll_regions.clear();

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
    panels::draw_main(frame, session, chunks[1], scroll_regions);
    overlays::draw_footer(frame, session, tab_manager, chunks[2]);

    let session = tab_manager.active();
    if session.approval_mode != ApprovalMode::None {
        overlays::draw_approval_overlay(frame, session, scroll_regions);
    }
    if session.input_mode == InputMode::NamingTab {
        overlays::draw_tab_input_overlay(frame, session, tab_manager);
    }
    // Render plan modal BEFORE error overlay so errors always take precedence
    let session = tab_manager.active();
    if session.plan_modal_open {
        overlays::draw_plan_modal(frame, session, scroll_regions);
    }
    // Render review modal BEFORE error overlay so errors always take precedence
    let session = tab_manager.active();
    if session.review_modal_open {
        overlays::draw_review_modal(frame, session, scroll_regions);
    }
    // Render session browser overlay
    if tab_manager.session_browser.open {
        overlays::draw_session_browser_overlay(frame, tab_manager);
    }
    // Render workflow browser overlay
    if tab_manager.workflow_browser.open {
        overlays::draw_workflow_browser_overlay(frame, tab_manager);
    }
    // Render implementation success modal after session browser, before error overlay
    let session = tab_manager.active();
    if session.implementation_success_modal.is_some() {
        overlays::draw_implementation_success_overlay(frame, session);
    }
    let session = tab_manager.active();
    if session.error_state.is_some() {
        overlays::draw_error_overlay(frame, session, scroll_regions);
    }
}

fn draw_tab_bar(frame: &mut Frame, tab_manager: &TabManager, area: Rect) {
    use overlays::{build_phase_spans, PhaseDisplayMode};
    use unicode_width::UnicodeWidthStr;

    let active_session = tab_manager.active();
    let theme = theme::Theme::for_session(active_session);

    let (iter, max_iter) = active_session.iteration();

    // Determine background color based on phase
    let bg_color = get_phase_background_color(active_session, &theme);

    // Build left section: phase chips + iteration
    let iter_display = if max_iter > 0 {
        format!(" ({}/{})", iter, max_iter)
    } else {
        String::new()
    };

    // Use chip-mode phase spans for compact display with animated spinner
    let phase_spans = if active_session.workflow_view.is_some() {
        build_phase_spans(
            active_session,
            &theme,
            PhaseDisplayMode::Chips {
                spinner_frame: active_session.spinner_frame,
            },
        )
    } else {
        vec![Span::styled(
            "Initializing",
            Style::default().fg(theme.muted),
        )]
    };

    // Build left section spans with background color
    let mut left_spans: Vec<Span> = Vec::new();
    left_spans.push(Span::styled(" ", Style::default().bg(bg_color)));
    for span in &phase_spans {
        left_spans.push(Span::styled(
            span.content.to_string(),
            span.style.bg(bg_color),
        ));
    }
    left_spans.push(Span::styled(
        iter_display.clone(),
        Style::default().fg(theme.muted).bg(bg_color),
    ));
    left_spans.push(Span::styled(" ", Style::default().bg(bg_color)));

    // Calculate left section width for layout
    let left_section_width: usize = left_spans.iter().map(|s| s.content.width()).sum();

    // Build right section: path/title
    let right_section = if let Some(ref view) = active_session.workflow_view {
        if let Some(plan_path) = view.plan_path() {
            format!("Planning Agent - {} ", plan_path.as_path().display())
        } else {
            "Planning Agent ".to_string()
        }
    } else {
        "Planning Agent ".to_string()
    };

    // Build middle section: session tabs (refactored from existing logic)
    let mut tab_spans: Vec<Span> = Vec::new();
    for (i, session) in tab_manager.sessions.iter().enumerate() {
        let is_active = i == tab_manager.active_tab;
        let status_icon_str = get_session_status_icon(session);
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
        let label = if status_icon_str.is_empty() {
            format!("[{}]", display_name)
        } else {
            format!("[{} {}]", display_name, status_icon_str)
        };
        let style = if is_active {
            Style::default()
                .fg(theme.tab_active)
                .add_modifier(Modifier::BOLD)
                .bg(bg_color)
        } else if session.approval_mode != ApprovalMode::None {
            Style::default().fg(theme.tab_approval).bg(bg_color)
        } else {
            Style::default().fg(theme.tab_inactive).bg(bg_color)
        };
        tab_spans.push(Span::styled(label, style));
        tab_spans.push(Span::styled(" ", Style::default().bg(bg_color)));
    }

    // ============================================================
    // Calculate spacing for right alignment
    //
    // Width calculation strategy:
    // - left_section_width: Sum of span widths via UnicodeWidthStr
    // - tabs: Line::width() gives total display width of all tab_spans
    // - right_section: String width via UnicodeWidthStr
    //
    // Safety: saturating_sub ensures padding >= 0. If terminal is too
    // narrow, padding becomes 0 and content may overlap, but won't panic.
    // The final Line will simply be wider than available space, which
    // ratatui handles gracefully by truncating at terminal edge.
    // ============================================================
    let right_width = right_section.width(); // UnicodeWidthStr on String
    let tabs_line = Line::from(tab_spans.clone());
    let tabs_width = tabs_line.width(); // Line::width() from ratatui
    let available = area.width as usize;
    let padding = available.saturating_sub(left_section_width + tabs_width + right_width);

    // Build final line with left (phase chips), tabs, padding, right
    // Note: total_width = left_section_width + tabs_width + padding + right_width
    // When padding > 0: total_width == available (right-aligned)
    // When padding == 0: total_width > available (content truncated at edge)
    let mut spans: Vec<Span> = Vec::new();
    // Left: phase chips with their own colors on phase background
    spans.extend(left_spans);
    // Tabs
    spans.extend(tab_spans);
    // Padding to push right section to edge
    if padding > 0 {
        spans.push(Span::styled(
            " ".repeat(padding),
            Style::default().bg(bg_color),
        ));
    }
    // Right: path info
    spans.push(Span::styled(
        right_section,
        Style::default().fg(theme.muted).bg(bg_color),
    ));

    let header = Paragraph::new(Line::from(spans)).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(theme.muted))
            .style(Style::default().bg(bg_color)),
    );

    frame.render_widget(header, area);
}
