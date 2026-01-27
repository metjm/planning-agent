//! Status color rendering for the host GUI.
//!
//! Provides color-coded status display for workflow phases.

use eframe::egui;

// Color palette for workflow status display.
// Each phase has a distinct, meaningful color for quick visual identification.

// Planning workflow phases - Blue tones (creative/thinking work)
pub const PLANNING: egui::Color32 = egui::Color32::from_rgb(33, 150, 243); // Blue
pub const REVIEWING: egui::Color32 = egui::Color32::from_rgb(156, 39, 176); // Purple
pub const REVISING: egui::Color32 = egui::Color32::from_rgb(255, 152, 0); // Orange

// Implementation phases - Green tones (building work)
pub const IMPLEMENTING: egui::Color32 = egui::Color32::from_rgb(76, 175, 80); // Green
pub const IMPL_REVIEW: egui::Color32 = egui::Color32::from_rgb(0, 150, 136); // Teal

// Terminal states
pub const COMPLETE: egui::Color32 = egui::Color32::from_rgb(139, 195, 74); // Light green
pub const ERROR: egui::Color32 = egui::Color32::from_rgb(244, 67, 54); // Red
pub const STOPPED: egui::Color32 = egui::Color32::from_rgb(117, 117, 117); // Gray

// Waiting states
pub const AWAITING: egui::Color32 = egui::Color32::from_rgb(255, 193, 7); // Amber
pub const INPUT_PENDING: egui::Color32 = egui::Color32::from_rgb(3, 169, 244); // Light blue

// Unknown
pub const UNKNOWN: egui::Color32 = egui::Color32::from_rgb(158, 158, 158); // Light gray

// Stale/warning indicator color (amber/orange)
pub const STALE: egui::Color32 = egui::Color32::from_rgb(255, 183, 77);

/// Get the color and display text for a workflow status.
/// Uses both `phase` (workflow phase) and `status` (session status) for context.
pub fn get_status_display(phase: &str, status: &str) -> (egui::Color32, &'static str) {
    let phase_lower = phase.to_lowercase();
    let status_lower = status.to_lowercase();

    match (phase_lower.as_str(), status_lower.as_str()) {
        // Planning workflow phases
        (_, "planning") | ("planning", _) => (PLANNING, "Planning"),
        ("reviewing", _) => (REVIEWING, "Reviewing"),
        ("revising", _) => (REVISING, "Revising"),

        // Implementation workflow phases
        ("implementing", _) | (_, "implementing") => (IMPLEMENTING, "Implementing"),
        ("implementationreview", _) | ("implementation_review", _) => (IMPL_REVIEW, "Impl Review"),

        // Waiting states
        (_, "awaitingapproval") | (_, "awaiting_approval") => (AWAITING, "Awaiting"),
        (_, "inputpending") | (_, "input_pending") => (INPUT_PENDING, "Input Pending"),
        (_, "generatingsummary") | (_, "generating_summary") => (PLANNING, "Summarizing"),

        // Terminal states (check status first, as phase may still show old value)
        (_, "complete") | ("complete", _) => (COMPLETE, "Complete"),
        (_, "error") => (ERROR, "Error"),
        (_, "stopped") => (STOPPED, "Stopped"),

        // Fallback
        _ => (UNKNOWN, "Unknown"),
    }
}

/// Get color for a workflow phase.
pub fn get_phase_color(phase: &str) -> egui::Color32 {
    match phase.to_lowercase().as_str() {
        "planning" => PLANNING,
        "reviewing" => REVIEWING,
        "revising" => REVISING,
        "implementing" => IMPLEMENTING,
        "implementationreview" | "implementation_review" => IMPL_REVIEW,
        "complete" => COMPLETE,
        "awaitingplanningdecision" | "awaiting_planning_decision" => AWAITING,
        "awaitingdecision" | "awaiting_decision" => AWAITING,
        _ => UNKNOWN,
    }
}
