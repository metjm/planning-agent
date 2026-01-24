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
mod workflow_browser_overlay;

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

/// Status icon shown in header based on workflow state.
/// Covers all possible SessionStatus variants.
enum HeaderStatusIcon {
    /// Spinning animation when agent is actively running
    Spinner(u8), // frame index
    /// Waiting for user input (hourglass)
    Waiting,
    /// Session is stopped/paused (can be resumed)
    Stopped,
    /// Workflow complete (checkmark)
    Complete,
    /// Error state (exclamation)
    Error,
}

impl HeaderStatusIcon {
    fn to_char(&self) -> char {
        match self {
            HeaderStatusIcon::Spinner(frame) => {
                SPINNER_CHARS[(*frame as usize) % SPINNER_CHARS.len()]
            }
            HeaderStatusIcon::Waiting => '⏳',
            HeaderStatusIcon::Stopped => '⏸',
            HeaderStatusIcon::Complete => '✓',
            HeaderStatusIcon::Error => '!',
        }
    }
}

/// Determines the header status icon based on session state.
/// Handles ALL SessionStatus variants explicitly.
fn determine_header_status(session: &Session, spinner_frame: u8) -> HeaderStatusIcon {
    // Completion states - show checkmark
    if matches!(session.status, SessionStatus::Complete) {
        return HeaderStatusIcon::Complete;
    }

    // Error state - show exclamation
    if matches!(session.status, SessionStatus::Error) {
        return HeaderStatusIcon::Error;
    }

    // Stopped state - show pause icon (distinct from waiting)
    if matches!(session.status, SessionStatus::Stopped) {
        return HeaderStatusIcon::Stopped;
    }

    // Running states - show spinner
    // This includes:
    // - session.running flag being true (agent actively working)
    // - SessionStatus::Planning (AI is planning)
    // - SessionStatus::GeneratingSummary (AI generating summary)
    if session.running
        || matches!(
            session.status,
            SessionStatus::Planning | SessionStatus::GeneratingSummary
        )
    {
        return HeaderStatusIcon::Spinner(spinner_frame);
    }

    // Waiting states - show hourglass
    // - SessionStatus::AwaitingApproval (waiting for user approval)
    // - SessionStatus::InputPending (waiting for user input/objective)
    if matches!(
        session.status,
        SessionStatus::AwaitingApproval | SessionStatus::InputPending
    ) {
        return HeaderStatusIcon::Waiting;
    }

    // Fallback - should not reach here if all variants are covered,
    // but default to waiting as the safest option
    HeaderStatusIcon::Waiting
}

/// Returns the background color for the header based on session state.
///
/// PRIORITY ORDER (checked top-to-bottom, first match wins):
/// 1. Error state - ALWAYS red, regardless of workflow_state presence
/// 2. Stopped state - muted blue-gray
/// 3. Waiting states (InputPending, AwaitingApproval when not running) - gray
/// 4. Workflow phase colors (if workflow_state exists) - phase-specific
/// 5. Default (no workflow_state and not in above states) - gray (waiting)
fn get_phase_background_color(session: &Session, theme: &theme::Theme) -> ratatui::style::Color {
    use crate::state::{ImplementationPhase, Phase};

    // ============================================================
    // PRIORITY 1: Error state takes precedence over ALL other states
    // This check happens FIRST, before workflow_state is examined.
    // Even if workflow_state is None (e.g., initialization failure),
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
    match &session.workflow_state {
        Some(state) => {
            // Check implementation phase first
            if let Some(impl_state) = &state.implementation_state {
                if impl_state.phase != ImplementationPhase::Complete {
                    return match impl_state.phase {
                        ImplementationPhase::Implementing => theme.phase_bg_planning,
                        ImplementationPhase::ImplementationReview => theme.phase_bg_reviewing,
                        ImplementationPhase::Complete => theme.phase_bg_complete,
                    };
                }
            }
            // Planning workflow phases
            match state.phase {
                Phase::Planning => theme.phase_bg_planning,
                Phase::Reviewing => theme.phase_bg_reviewing,
                Phase::Revising => theme.phase_bg_revising,
                Phase::Complete => theme.phase_bg_complete,
            }
        }
        // No workflow_state AND not Error/Stopped/Waiting = use waiting color
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
        session_browser_overlay::draw_session_browser_overlay(frame, tab_manager);
    }
    // Render workflow browser overlay
    if tab_manager.workflow_browser.open {
        workflow_browser_overlay::draw_workflow_browser_overlay(frame, tab_manager);
    }
    // Render implementation success modal after session browser, before error overlay
    let session = tab_manager.active();
    if session.implementation_success_modal.is_some() {
        success_overlay::draw_implementation_success_overlay(frame, session);
    }
    let session = tab_manager.active();
    if session.error_state.is_some() {
        error_overlay::draw_error_overlay(frame, session, scroll_regions);
    }
}

fn draw_tab_bar(frame: &mut Frame, tab_manager: &TabManager, area: Rect) {
    use unicode_width::UnicodeWidthStr;

    let active_session = tab_manager.active();
    let theme = theme::Theme::for_session(active_session);

    // Determine phase info from active session
    let phase_name = active_session.phase_name();
    let (iter, max_iter) = active_session.iteration();

    // Determine status icon using session's spinner frame (animated per-session)
    let status_icon = determine_header_status(active_session, active_session.spinner_frame);
    let icon_char = status_icon.to_char();

    // Determine background color based on phase
    let bg_color = get_phase_background_color(active_session, &theme);

    // Build left section: icon + phase name + iteration
    let iter_display = if max_iter > 0 {
        format!(" ({}/{})", iter, max_iter)
    } else {
        String::new()
    };
    let left_section = format!(" {} {}{}", icon_char, phase_name, iter_display);

    // Build right section: path/title
    let right_section = if let Some(ref state) = active_session.workflow_state {
        let plan_path = state.plan_file.display().to_string();
        format!("Planning Agent - {} ", plan_path)
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
    // - left_section: String width via UnicodeWidthStr
    // - tabs: Line::width() gives total display width of all tab_spans
    // - right_section: String width via UnicodeWidthStr
    //
    // Safety: saturating_sub ensures padding >= 0. If terminal is too
    // narrow, padding becomes 0 and content may overlap, but won't panic.
    // The final Line will simply be wider than available space, which
    // ratatui handles gracefully by truncating at terminal edge.
    // ============================================================
    let left_width = left_section.width(); // UnicodeWidthStr on String
    let right_width = right_section.width(); // UnicodeWidthStr on String
    let tabs_line = Line::from(tab_spans.clone());
    let tabs_width = tabs_line.width(); // Line::width() from ratatui
    let available = area.width as usize;
    let padding = available.saturating_sub(left_width + tabs_width + right_width);

    // Build final line with left, tabs, padding, right
    // Note: total_width = left_width + tabs_width + padding + right_width
    // When padding > 0: total_width == available (right-aligned)
    // When padding == 0: total_width > available (content truncated at edge)
    let mut spans: Vec<Span> = Vec::new();
    // Left: phase info with bold theme text on phase background
    spans.push(Span::styled(
        left_section,
        Style::default()
            .fg(theme.text)
            .add_modifier(Modifier::BOLD)
            .bg(bg_color),
    ));
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
