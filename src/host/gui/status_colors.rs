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
/// Uses `impl_phase` when present (implementation workflow),
/// otherwise uses `status` (planning workflow).
pub fn get_status_display(status: &str, impl_phase: Option<&str>) -> (egui::Color32, &'static str) {
    // Check implementation phase first if present
    if let Some(ip) = impl_phase {
        let ip_lower = ip.to_lowercase();
        match ip_lower.as_str() {
            "implementing" => return (IMPLEMENTING, "Implementing"),
            "implementationreview" | "implementation_review" => {
                return (IMPL_REVIEW, "Impl Review")
            }
            "awaitingdecision" | "awaiting_decision" => return (AWAITING, "Awaiting Decision"),
            "complete" => return (COMPLETE, "Complete"),
            "failed" => return (ERROR, "Failed"),
            "cancelled" => return (STOPPED, "Cancelled"),
            _ => {}
        }
    }

    // Use status directly (now distinct from phase)
    let status_lower = status.to_lowercase();
    match status_lower.as_str() {
        // Planning workflow statuses
        "planning" => (PLANNING, "Planning"),
        "reviewing" => (REVIEWING, "Reviewing"),
        "revising" => (REVISING, "Revising"),
        "awaitingplanningdecision" | "awaiting_planning_decision" => {
            (AWAITING, "Awaiting Decision")
        }
        "complete" => (COMPLETE, "Complete"),
        // Terminal states
        "error" => (ERROR, "Error"),
        "stopped" => (STOPPED, "Stopped"),
        // Fallback
        _ => (UNKNOWN, "Unknown"),
    }
}

/// Get color for a workflow phase.
/// After the phase/status separation, phase is only "Planning" or "Implementation".
pub fn get_phase_color(phase: &str) -> egui::Color32 {
    match phase.to_lowercase().as_str() {
        "planning" => PLANNING,
        "implementation" => IMPLEMENTING,
        _ => UNKNOWN,
    }
}
