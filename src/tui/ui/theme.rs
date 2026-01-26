//! Theme module for TUI color palettes.
//!
//! Provides distinct color palettes based on workflow phase:
//! - Planning phase (blue tones): Planning, Reviewing, Revising
//! - Implementation phase (orange/red tones): Implementing, ImplementationReview
//! - Complete phase (green tones): Complete
//!
//! Semantic colors (success=green, error=red) remain consistent across all themes.

use crate::state::{ImplementationPhase, Phase};
use crate::tui::{Session, SessionStatus};
use ratatui::style::Color;

/// Workflow phase for theme selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePhase {
    /// Planning workflow: Planning, Reviewing, Revising
    Planning,
    /// Implementation workflow: Implementing, ImplementationReview
    Implementation,
    /// Workflow complete: Complete
    Complete,
}

/// Theme struct with semantic color roles for the TUI.
///
/// Each color role has a specific purpose across all UI components.
/// All phase themes use the same roles with different colors.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    // === Primary colors ===
    /// Main text color
    pub text: Color,
    /// Muted/secondary text color
    pub muted: Color,
    /// Primary accent color (active elements, highlights)
    pub accent: Color,
    /// Secondary accent color (alternate highlights)
    pub accent_alt: Color,

    // === Border colors ===
    /// Default border color
    pub border: Color,
    /// Focused/active border color
    pub border_focused: Color,

    // === Semantic colors (consistent across themes) ===
    /// Success state color - always green
    pub success: Color,
    /// Warning state color
    pub warning: Color,
    /// Error state color - always red
    pub error: Color,

    // === Phase-specific colors ===
    /// Current phase color
    pub phase_current: Color,
    /// Completed phase color
    pub phase_complete: Color,
    /// Pending/inactive phase color
    pub phase_pending: Color,

    // === Tab bar colors ===
    /// Active tab color
    pub tab_active: Color,
    /// Inactive tab color
    pub tab_inactive: Color,
    /// Tab awaiting approval color
    pub tab_approval: Color,

    // === Output tag colors ===
    /// Planning output tag color ([planning])
    pub tag_planning: Color,
    /// Implementation output tag color ([implementation])
    pub tag_implementation: Color,
    /// Agent output tag color ([claude], etc.)
    pub tag_agent: Color,

    // === Stats panel colors ===
    /// Stats header color
    pub stats_header: Color,
    /// Stats border color
    pub stats_border: Color,
    /// Cost display color
    pub stats_cost: Color,
    /// Token input color
    pub stats_tokens_in: Color,
    /// Token output color
    pub stats_tokens_out: Color,

    // === Todo colors ===
    /// Todo header color
    pub todo_header: Color,
    /// In-progress todo color
    pub todo_in_progress: Color,
    /// Completed todo color
    pub todo_complete: Color,
    /// Pending todo color
    pub todo_pending: Color,

    // === CLI instances colors ===
    /// CLI instances border color
    pub cli_border: Color,
    /// CLI running indicator color
    pub cli_running: Color,
    /// CLI elapsed time color
    pub cli_elapsed: Color,
    /// CLI idle time color
    pub cli_idle: Color,

    // === Objective panel colors ===
    /// Objective panel border color
    pub objective_border: Color,

    // === Phase header background colors ===
    /// Background color for Planning phase header
    pub phase_bg_planning: Color,
    /// Background color for Reviewing phase header
    pub phase_bg_reviewing: Color,
    /// Background color for Revising phase header
    pub phase_bg_revising: Color,
    /// Background color for Complete phase header
    pub phase_bg_complete: Color,
    /// Background color for Waiting/Input phase header
    pub phase_bg_waiting: Color,
    /// Background color for Stopped/Paused phase header
    pub phase_bg_stopped: Color,
    /// Background color for Error phase header
    pub phase_bg_error: Color,
}

impl Theme {
    /// Blue-toned theme for planning workflow phases.
    pub fn planning() -> Self {
        Self {
            // Primary colors - blue tones
            text: Color::White,
            muted: Color::DarkGray,
            accent: Color::Rgb(100, 180, 255),     // Sky blue
            accent_alt: Color::Rgb(150, 130, 255), // Periwinkle

            // Border colors - blue tones
            border: Color::Rgb(60, 100, 160),          // Steel blue
            border_focused: Color::Rgb(130, 200, 255), // Light blue

            // Semantic colors (consistent across themes)
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,

            // Phase-specific colors
            phase_current: Color::Rgb(130, 200, 255), // Light blue
            phase_complete: Color::Green,
            phase_pending: Color::DarkGray,

            // Tab bar colors - blue
            tab_active: Color::Rgb(130, 200, 255), // Light blue
            tab_inactive: Color::DarkGray,
            tab_approval: Color::Rgb(150, 130, 255), // Periwinkle

            // Output tag colors
            tag_planning: Color::Rgb(100, 180, 255), // Sky blue
            tag_implementation: Color::Rgb(255, 165, 0), // Orange (distinct)
            tag_agent: Color::Rgb(150, 255, 150),    // Light green

            // Stats panel colors - blue
            stats_header: Color::Rgb(100, 180, 255), // Sky blue
            stats_border: Color::Rgb(60, 100, 160),  // Steel blue
            stats_cost: Color::Green,
            stats_tokens_in: Color::Rgb(130, 200, 255), // Light blue
            stats_tokens_out: Color::Green,

            // Todo colors - blue
            todo_header: Color::Rgb(100, 180, 255), // Sky blue
            todo_in_progress: Color::Rgb(130, 200, 255), // Light blue
            todo_complete: Color::Green,
            todo_pending: Color::White,

            // CLI instances colors - blue
            cli_border: Color::Rgb(60, 100, 160),   // Steel blue
            cli_running: Color::Rgb(150, 255, 150), // Light green
            cli_elapsed: Color::Rgb(130, 200, 255), // Light blue
            cli_idle: Color::Rgb(100, 180, 255),    // Sky blue

            // Objective panel colors - blue
            objective_border: Color::Rgb(100, 180, 255), // Sky blue

            // Phase header backgrounds
            phase_bg_planning: Color::Rgb(20, 60, 120), // Deep blue
            phase_bg_reviewing: Color::Rgb(60, 50, 100), // Deep purple
            phase_bg_revising: Color::Rgb(80, 60, 40),  // Brown (warning tone)
            phase_bg_complete: Color::Rgb(20, 80, 40),  // Deep green
            phase_bg_waiting: Color::Rgb(60, 60, 60),   // Dark gray
            phase_bg_stopped: Color::Rgb(50, 50, 80),   // Muted blue-gray
            phase_bg_error: Color::Rgb(120, 30, 30),    // Dark red
        }
    }

    /// Orange/red-toned theme for implementation workflow phases.
    pub fn implementation() -> Self {
        Self {
            // Primary colors - orange/red tones
            text: Color::White,
            muted: Color::DarkGray,
            accent: Color::Rgb(255, 165, 0),       // Orange
            accent_alt: Color::Rgb(255, 100, 100), // Coral

            // Border colors - warm tones
            border: Color::Rgb(180, 80, 40),           // Burnt orange
            border_focused: Color::Rgb(255, 200, 100), // Gold

            // Semantic colors (consistent across themes)
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,

            // Phase-specific colors
            phase_current: Color::Rgb(255, 200, 100), // Gold
            phase_complete: Color::Green,
            phase_pending: Color::DarkGray,

            // Tab bar colors - warm
            tab_active: Color::Rgb(255, 200, 100), // Gold
            tab_inactive: Color::DarkGray,
            tab_approval: Color::Rgb(255, 100, 100), // Coral

            // Output tag colors
            tag_planning: Color::Rgb(100, 180, 255), // Sky blue (distinct)
            tag_implementation: Color::Rgb(255, 165, 0), // Orange
            tag_agent: Color::Rgb(150, 255, 150),    // Light green

            // Stats panel colors - warm
            stats_header: Color::Rgb(255, 165, 0), // Orange
            stats_border: Color::Rgb(180, 80, 40), // Burnt orange
            stats_cost: Color::Green,
            stats_tokens_in: Color::Rgb(255, 200, 100), // Gold
            stats_tokens_out: Color::Green,

            // Todo colors - warm
            todo_header: Color::Rgb(255, 165, 0), // Orange
            todo_in_progress: Color::Rgb(255, 200, 100), // Gold
            todo_complete: Color::Green,
            todo_pending: Color::White,

            // CLI instances colors - warm
            cli_border: Color::Rgb(180, 80, 40), // Burnt orange
            cli_running: Color::Rgb(150, 255, 150), // Light green
            cli_elapsed: Color::Rgb(255, 200, 100), // Gold
            cli_idle: Color::Rgb(255, 165, 0),   // Orange

            // Objective panel colors - warm
            objective_border: Color::Rgb(255, 165, 0), // Orange

            // Phase header backgrounds
            phase_bg_planning: Color::Rgb(20, 60, 120), // Deep blue
            phase_bg_reviewing: Color::Rgb(100, 50, 30), // Dark orange-brown
            phase_bg_revising: Color::Rgb(120, 60, 0),  // Burnt orange
            phase_bg_complete: Color::Rgb(20, 80, 40),  // Deep green
            phase_bg_waiting: Color::Rgb(60, 60, 60),   // Dark gray
            phase_bg_stopped: Color::Rgb(50, 50, 80),   // Muted blue-gray
            phase_bg_error: Color::Rgb(120, 30, 30),    // Dark red
        }
    }

    /// Green-toned theme for complete workflow state.
    pub fn complete() -> Self {
        Self {
            // Primary colors - green tones
            text: Color::White,
            muted: Color::DarkGray,
            accent: Color::Rgb(100, 220, 100),     // Bright green
            accent_alt: Color::Rgb(150, 255, 180), // Mint

            // Border colors - green tones
            border: Color::Rgb(40, 120, 60),           // Forest green
            border_focused: Color::Rgb(150, 255, 150), // Light green

            // Semantic colors (consistent across themes)
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,

            // Phase-specific colors
            phase_current: Color::Rgb(150, 255, 150), // Light green
            phase_complete: Color::Green,
            phase_pending: Color::DarkGray,

            // Tab bar colors - green
            tab_active: Color::Rgb(150, 255, 150), // Light green
            tab_inactive: Color::DarkGray,
            tab_approval: Color::Rgb(150, 255, 180), // Mint

            // Output tag colors
            tag_planning: Color::Rgb(100, 180, 255), // Sky blue (distinct)
            tag_implementation: Color::Rgb(255, 165, 0), // Orange (distinct)
            tag_agent: Color::Rgb(100, 220, 100),    // Bright green

            // Stats panel colors - green
            stats_header: Color::Rgb(100, 220, 100), // Bright green
            stats_border: Color::Rgb(40, 120, 60),   // Forest green
            stats_cost: Color::Green,
            stats_tokens_in: Color::Rgb(150, 255, 150), // Light green
            stats_tokens_out: Color::Green,

            // Todo colors - green
            todo_header: Color::Rgb(100, 220, 100), // Bright green
            todo_in_progress: Color::Rgb(150, 255, 150), // Light green
            todo_complete: Color::Green,
            todo_pending: Color::White,

            // CLI instances colors - green
            cli_border: Color::Rgb(40, 120, 60), // Forest green
            cli_running: Color::Rgb(150, 255, 150), // Light green
            cli_elapsed: Color::Rgb(150, 255, 150), // Light green
            cli_idle: Color::Rgb(100, 220, 100), // Bright green

            // Objective panel colors - green
            objective_border: Color::Rgb(100, 220, 100), // Bright green

            // Phase header backgrounds
            phase_bg_planning: Color::Rgb(20, 60, 120), // Deep blue
            phase_bg_reviewing: Color::Rgb(60, 50, 100), // Deep purple
            phase_bg_revising: Color::Rgb(80, 60, 40),  // Brown
            phase_bg_complete: Color::Rgb(20, 80, 40),  // Deep green
            phase_bg_waiting: Color::Rgb(60, 60, 60),   // Dark gray
            phase_bg_stopped: Color::Rgb(50, 50, 80),   // Muted blue-gray
            phase_bg_error: Color::Rgb(120, 30, 30),    // Dark red
        }
    }

    /// Determines the theme phase from a session's current state.
    pub fn phase_for_session(session: &Session) -> ThemePhase {
        // Check for complete states first
        if matches!(session.status, SessionStatus::Complete) {
            return ThemePhase::Complete;
        }

        // Check workflow state for phase
        if let Some(ref state) = session.workflow_state {
            // Check implementation phase first
            if let Some(ref impl_state) = state.implementation_state {
                if impl_state.phase != ImplementationPhase::Complete {
                    return ThemePhase::Implementation;
                }
            }

            // Check planning workflow phase
            match state.phase {
                Phase::Complete => ThemePhase::Complete,
                Phase::Planning
                | Phase::Reviewing
                | Phase::Revising
                | Phase::AwaitingPlanningDecision => ThemePhase::Planning,
            }
        } else {
            // No workflow state - default to planning
            ThemePhase::Planning
        }
    }

    /// Returns the appropriate theme for a session based on its workflow phase.
    pub fn for_session(session: &Session) -> Self {
        match Self::phase_for_session(session) {
            ThemePhase::Planning => Self::planning(),
            ThemePhase::Implementation => Self::implementation(),
            ThemePhase::Complete => Self::complete(),
        }
    }
}

#[cfg(test)]
#[path = "tests/theme_tests.rs"]
mod tests;
