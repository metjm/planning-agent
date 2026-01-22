//! Theme module for TUI color palettes.
//!
//! Provides distinct color palettes for planning and implementation modes.
//! When the UI switches modes, all colors change together to make the
//! mode visually unambiguous.

use crate::state::UiMode;
use crate::tui::Session;
use ratatui::style::Color;

/// Theme struct with semantic color roles for the TUI.
///
/// Each color role has a specific purpose across all UI components.
/// Both planning and implementation modes use the same roles with different colors.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
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

    // === Semantic colors ===
    /// Success state color
    pub success: Color,
    /// Warning state color
    pub warning: Color,
    /// Error state color
    pub error: Color,

    // === Selection colors ===
    /// Selection foreground
    pub selection_fg: Color,
    /// Selection background
    pub selection_bg: Color,

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

    // === Markdown colors ===
    /// Heading level 1 color
    pub md_h1: Color,
    /// Heading level 2 color
    pub md_h2: Color,
    /// Heading level 3 color
    pub md_h3: Color,
    /// Bullet point color
    pub md_bullet: Color,
    /// Bold text color
    pub md_bold: Color,
    /// Inline code color
    pub md_code: Color,

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
}

impl Theme {
    /// Returns the planning mode color palette.
    ///
    /// This is the original/default palette with cyan/magenta/yellow tones.
    pub fn planning() -> Self {
        Self {
            // Primary colors
            text: Color::White,
            muted: Color::DarkGray,
            accent: Color::Cyan,
            accent_alt: Color::Magenta,

            // Border colors
            border: Color::Blue,
            border_focused: Color::Yellow,

            // Semantic colors
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,

            // Selection colors
            selection_fg: Color::Black,
            selection_bg: Color::Cyan,

            // Phase-specific colors
            phase_current: Color::Yellow,
            phase_complete: Color::Green,
            phase_pending: Color::DarkGray,

            // Tab bar colors
            tab_active: Color::Yellow,
            tab_inactive: Color::DarkGray,
            tab_approval: Color::Magenta,

            // Output tag colors
            tag_planning: Color::Cyan,
            tag_implementation: Color::Blue,
            tag_agent: Color::Green,

            // Stats panel colors
            stats_header: Color::Cyan,
            stats_border: Color::Magenta,
            stats_cost: Color::Green,
            stats_tokens_in: Color::Cyan,
            stats_tokens_out: Color::Green,

            // Markdown colors
            md_h1: Color::Magenta,
            md_h2: Color::Blue,
            md_h3: Color::Cyan,
            md_bullet: Color::Yellow,
            md_bold: Color::Yellow,
            md_code: Color::Green,

            // Todo colors
            todo_header: Color::Cyan,
            todo_in_progress: Color::Yellow,
            todo_complete: Color::Green,
            todo_pending: Color::White,

            // CLI instances colors
            cli_border: Color::Blue,
            cli_running: Color::Green,
            cli_elapsed: Color::Cyan,
            cli_idle: Color::Yellow,

            // Objective panel colors
            objective_border: Color::Cyan,
        }
    }

    /// Returns the implementation mode color palette.
    ///
    /// Uses a distinctly different color scheme with orange/red/warm tones
    /// to make it visually unambiguous that implementation is active.
    pub fn implementation() -> Self {
        Self {
            // Primary colors - warmer tones
            text: Color::White,
            muted: Color::DarkGray,
            accent: Color::Rgb(255, 165, 0),       // Orange
            accent_alt: Color::Rgb(255, 100, 100), // Coral/light red

            // Border colors - warm tones
            border: Color::Rgb(200, 100, 50), // Burnt orange
            border_focused: Color::Rgb(255, 200, 100), // Gold

            // Semantic colors (keep consistent for usability)
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,

            // Selection colors - orange theme
            selection_fg: Color::Black,
            selection_bg: Color::Rgb(255, 165, 0), // Orange

            // Phase-specific colors - warm
            phase_current: Color::Rgb(255, 200, 100), // Gold
            phase_complete: Color::Green,
            phase_pending: Color::DarkGray,

            // Tab bar colors - warm
            tab_active: Color::Rgb(255, 200, 100), // Gold
            tab_inactive: Color::DarkGray,
            tab_approval: Color::Rgb(255, 100, 100), // Coral

            // Output tag colors - distinct for implementation
            tag_planning: Color::Cyan,
            tag_implementation: Color::Rgb(255, 165, 0), // Orange
            tag_agent: Color::Rgb(150, 255, 150),        // Light green

            // Stats panel colors - warm
            stats_header: Color::Rgb(255, 165, 0),  // Orange
            stats_border: Color::Rgb(200, 100, 50), // Burnt orange
            stats_cost: Color::Green,
            stats_tokens_in: Color::Rgb(255, 200, 100), // Gold
            stats_tokens_out: Color::Green,

            // Markdown colors - warm tones
            md_h1: Color::Rgb(255, 100, 100),     // Coral
            md_h2: Color::Rgb(255, 165, 0),       // Orange
            md_h3: Color::Rgb(255, 200, 100),     // Gold
            md_bullet: Color::Rgb(255, 200, 100), // Gold
            md_bold: Color::Rgb(255, 200, 100),   // Gold
            md_code: Color::Rgb(150, 255, 150),   // Light green

            // Todo colors - warm
            todo_header: Color::Rgb(255, 165, 0), // Orange
            todo_in_progress: Color::Rgb(255, 200, 100), // Gold
            todo_complete: Color::Green,
            todo_pending: Color::White,

            // CLI instances colors - warm
            cli_border: Color::Rgb(200, 100, 50), // Burnt orange
            cli_running: Color::Rgb(150, 255, 150), // Light green
            cli_elapsed: Color::Rgb(255, 200, 100), // Gold
            cli_idle: Color::Rgb(255, 165, 0),    // Orange

            // Objective panel colors - warm
            objective_border: Color::Rgb(255, 165, 0), // Orange
        }
    }

    /// Returns the appropriate theme for a session based on its UI mode.
    pub fn for_session(session: &Session) -> Self {
        match session.ui_mode() {
            UiMode::Planning => Self::planning(),
            UiMode::Implementation => Self::implementation(),
        }
    }

    /// Returns the appropriate theme for a given UI mode.
    #[allow(dead_code)]
    pub fn for_mode(mode: UiMode) -> Self {
        match mode {
            UiMode::Planning => Self::planning(),
            UiMode::Implementation => Self::implementation(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_planning_theme_has_distinct_colors() {
        let theme = Theme::planning();
        // Verify key colors are set correctly
        assert_eq!(theme.accent, Color::Cyan);
        assert_eq!(theme.border_focused, Color::Yellow);
        assert_eq!(theme.stats_border, Color::Magenta);
    }

    #[test]
    fn test_implementation_theme_has_distinct_colors() {
        let theme = Theme::implementation();
        // Verify implementation uses warm/orange tones
        assert_eq!(theme.accent, Color::Rgb(255, 165, 0));
        assert_eq!(theme.border_focused, Color::Rgb(255, 200, 100));
        assert_eq!(theme.stats_border, Color::Rgb(200, 100, 50));
    }

    #[test]
    fn test_themes_are_visually_distinct() {
        let planning = Theme::planning();
        let implementation = Theme::implementation();

        // Key colors should be different
        assert_ne!(planning.accent, implementation.accent);
        assert_ne!(planning.border, implementation.border);
        assert_ne!(planning.stats_header, implementation.stats_header);
        assert_ne!(
            planning.tag_implementation,
            implementation.tag_implementation
        );
    }

    #[test]
    fn test_theme_for_mode() {
        let planning = Theme::for_mode(UiMode::Planning);
        let implementation = Theme::for_mode(UiMode::Implementation);

        assert_eq!(planning.accent, Color::Cyan);
        assert_eq!(implementation.accent, Color::Rgb(255, 165, 0));
    }
}
