//! Session table rendering for the host GUI with click detection and container grouping.

use crate::session_daemon::LivenessState;
use egui_extras::{Column, TableBuilder};
use std::collections::BTreeMap;

/// Check if a session requires user interaction based on its phase and liveness.
pub fn session_needs_interaction(
    phase: &str,
    impl_phase: Option<&str>,
    liveness: LivenessDisplay,
) -> bool {
    if matches!(liveness, LivenessDisplay::Stopped) {
        return false;
    }

    // Check implementation phase for decision state
    if let Some(ip) = impl_phase {
        let ip_lower = ip.to_lowercase();
        if ip_lower == "awaitingdecision" || ip_lower == "awaiting_decision" {
            return true;
        }
    }

    let phase_lower = phase.to_lowercase();
    matches!(
        phase_lower.as_str(),
        "complete" | "awaitingplanningdecision"
    )
}

/// Check if a session has reached a terminal failure state based on its status
/// and implementation phase.
///
/// Returns true for:
/// - implementation_phase == "failed" (implementation workflow failure)
/// - implementation_phase == "cancelled" (implementation workflow cancelled)
/// - workflow_status == "error" (planning workflow failure)
///
/// Returns false for stopped sessions (matching `session_needs_interaction()` pattern).
///
/// This function is co-located with `session_needs_interaction()` because both
/// interpret session state for host-gui decision logic.
pub fn session_has_failed(
    status: &str,
    impl_phase: Option<&str>,
    liveness: LivenessDisplay,
) -> bool {
    // Stopped sessions don't need notifications - match session_needs_interaction() pattern
    if matches!(liveness, LivenessDisplay::Stopped) {
        return false;
    }

    // Check implementation phase for terminal failure states
    if let Some(ip) = impl_phase {
        let ip_lower = ip.to_lowercase();
        if ip_lower == "failed" || ip_lower == "cancelled" {
            return true;
        }
    }

    // Check workflow status for error state
    let status_lower = status.to_lowercase();
    status_lower == "error"
}

/// Check if a session was cancelled (user-initiated termination).
/// Used to distinguish cancellation from unexpected failures for urgency level.
///
/// Returns false for stopped sessions for consistency with other detection functions.
pub fn session_was_cancelled(impl_phase: Option<&str>, liveness: LivenessDisplay) -> bool {
    if matches!(liveness, LivenessDisplay::Stopped) {
        return false;
    }
    impl_phase
        .map(|ip| ip.to_lowercase() == "cancelled")
        .unwrap_or(false)
}

/// Display row for a session.
#[derive(Clone)]
pub struct DisplaySessionRow {
    pub session_id: String,
    pub container_name: String,
    pub feature_name: String,
    pub phase: String,
    pub iteration: u32,
    pub status: String,
    pub liveness: LivenessDisplay,
    pub pid: u32,
    pub updated_ago: String,
    /// Implementation phase if in implementation workflow
    pub implementation_phase: Option<String>,
}

#[derive(Clone, Copy, Default, Debug)]
pub enum LivenessDisplay {
    Running,
    Unresponsive,
    #[default]
    Stopped,
}

impl From<LivenessState> for LivenessDisplay {
    fn from(state: LivenessState) -> Self {
        match state {
            LivenessState::Running => LivenessDisplay::Running,
            LivenessState::Unresponsive => LivenessDisplay::Unresponsive,
            LivenessState::Stopped => LivenessDisplay::Stopped,
        }
    }
}

/// Group sessions by container name.
fn group_by_container<'a>(
    sessions: &[&'a DisplaySessionRow],
) -> Vec<(&'a str, Vec<&'a DisplaySessionRow>)> {
    let mut groups: BTreeMap<&str, Vec<&DisplaySessionRow>> = BTreeMap::new();
    for session in sessions {
        groups
            .entry(&session.container_name)
            .or_default()
            .push(*session);
    }
    groups.into_iter().collect()
}

/// Render the session table with click detection and container sub-grouping.
/// Returns the session_id if a row was clicked.
pub fn render_session_table(
    ui: &mut eframe::egui::Ui,
    sessions: &[DisplaySessionRow],
    selected_session_id: &Option<String>,
) -> Option<String> {
    if sessions.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label("No active sessions");
        });
        return None;
    }

    let mut clicked_session: Option<String> = None;

    // Partition into live and disconnected (preserve existing UX)
    let (live_sessions, disconnected_sessions): (Vec<_>, Vec<_>) = sessions.iter().partition(|s| {
        matches!(
            s.liveness,
            LivenessDisplay::Running | LivenessDisplay::Unresponsive
        )
    });

    // Constants for height calculation
    const HEADER_HEIGHT: f32 = 24.0; // Header + separator height per section
    const SECTION_SPACING: f32 = 16.0; // Spacing between sections

    // Calculate available height and proportional allocation
    let available_height = ui.available_height();
    let live_count = live_sessions.len();
    let disconnected_count = disconnected_sessions.len();

    // Calculate overhead (headers, spacing)
    let live_overhead = if live_count > 0 { HEADER_HEIGHT } else { 0.0 };
    let disconnected_overhead = if disconnected_count > 0 {
        HEADER_HEIGHT
    } else {
        0.0
    };
    let spacing_overhead = if live_count > 0 && disconnected_count > 0 {
        SECTION_SPACING
    } else {
        0.0
    };
    let total_overhead = live_overhead + disconnected_overhead + spacing_overhead;
    let content_height = (available_height - total_overhead).max(0.0);

    // Minimum heights when both sections are present
    const MIN_LIVE_HEIGHT: f32 = 150.0;
    const MIN_DISCONNECTED_HEIGHT: f32 = 80.0;
    const LIVE_REMAINDER_RATIO: f32 = 0.6;

    // Height allocation: prioritize live sessions
    let (live_height, disconnected_height) = if live_count > 0 && disconnected_count > 0 {
        let guaranteed = MIN_LIVE_HEIGHT + MIN_DISCONNECTED_HEIGHT;
        if content_height <= guaranteed {
            // Constrained space: use fixed ratio favoring live
            (
                content_height * LIVE_REMAINDER_RATIO,
                content_height * (1.0 - LIVE_REMAINDER_RATIO),
            )
        } else {
            // Sufficient space: guarantee minimums, split remainder
            let remainder = content_height - guaranteed;
            (
                MIN_LIVE_HEIGHT + remainder * LIVE_REMAINDER_RATIO,
                MIN_DISCONNECTED_HEIGHT + remainder * (1.0 - LIVE_REMAINDER_RATIO),
            )
        }
    } else if live_count > 0 {
        (content_height, 0.0)
    } else {
        (0.0, content_height)
    };

    // Live Sessions Section
    if !live_sessions.is_empty() {
        ui.horizontal(|ui| {
            ui.colored_label(eframe::egui::Color32::from_rgb(76, 175, 80), "●");
            ui.strong(format!("Live Sessions ({})", live_sessions.len()));
        });
        ui.separator();

        eframe::egui::ScrollArea::vertical()
            .id_salt("live_sessions_scroll")
            .max_height(live_height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let live_refs: Vec<_> = live_sessions.iter().collect();
                if let Some(id) = render_container_grouped_rows(ui, &live_refs, selected_session_id)
                {
                    clicked_session = Some(id);
                }
            });
    }

    // Disconnected Sessions Section
    if !disconnected_sessions.is_empty() {
        if !live_sessions.is_empty() {
            ui.add_space(SECTION_SPACING);
        }
        ui.horizontal(|ui| {
            ui.colored_label(eframe::egui::Color32::from_rgb(117, 117, 117), "○");
            ui.label(format!(
                "Disconnected Sessions ({})",
                disconnected_sessions.len()
            ));
        });
        ui.separator();

        eframe::egui::ScrollArea::vertical()
            .id_salt("disconnected_sessions_scroll")
            .max_height(disconnected_height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let disconnected_refs: Vec<_> = disconnected_sessions.iter().collect();
                if let Some(id) =
                    render_container_grouped_rows(ui, &disconnected_refs, selected_session_id)
                {
                    clicked_session = Some(id);
                }
            });
    }

    clicked_session
}

fn render_container_grouped_rows(
    ui: &mut eframe::egui::Ui,
    sessions: &[&&DisplaySessionRow],
    selected_session_id: &Option<String>,
) -> Option<String> {
    use eframe::egui;

    let mut clicked_session: Option<String> = None;
    let sessions_vec: Vec<&DisplaySessionRow> = sessions.iter().map(|s| **s).collect();
    let grouped = group_by_container(&sessions_vec);

    for (idx, (container_name, container_sessions)) in grouped.iter().enumerate() {
        if idx > 0 {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.separator();
            });
            ui.add_space(4.0);
        }

        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.small(egui::RichText::new(*container_name).strong());
            ui.small(format!("({})", container_sessions.len()));
        });

        if let Some(id) = render_session_rows_clickable(ui, container_sessions, selected_session_id)
        {
            clicked_session = Some(id);
        }
    }

    clicked_session
}

/// Get a short container name (last 8 chars) for the per-row badge.
fn container_short_name(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= 8 {
        name.to_string()
    } else {
        chars[chars.len() - 8..].iter().collect()
    }
}

fn render_session_rows_clickable(
    ui: &mut eframe::egui::Ui,
    sessions: &[&DisplaySessionRow],
    selected_session_id: &Option<String>,
) -> Option<String> {
    use eframe::egui;

    let mut clicked_session: Option<String> = None;

    TableBuilder::new(ui)
        .sense(egui::Sense::click()) // REQUIRED for click detection
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::exact(80.0)) // Liveness dot + container badge
        .column(Column::initial(180.0).at_least(100.0)) // Feature
        .column(Column::exact(80.0)) // Phase
        .column(Column::exact(35.0)) // Iter
        .column(Column::exact(90.0)) // Status
        .column(Column::exact(60.0)) // PID
        .column(Column::exact(70.0)) // Updated
        .header(24.0, |mut header| {
            header.col(|_| {}); // Liveness/container badge column (no header)
            header.col(|ui| {
                ui.strong("Feature");
            });
            header.col(|ui| {
                ui.strong("Phase");
            });
            header.col(|ui| {
                ui.strong("Iter");
            });
            header.col(|ui| {
                ui.strong("Status");
            });
            header.col(|ui| {
                ui.strong("PID");
            });
            header.col(|ui| {
                ui.strong("Updated");
            });
        })
        .body(|mut body| {
            for session in sessions {
                let is_selected = selected_session_id
                    .as_ref()
                    .is_some_and(|id| id == &session.session_id);

                body.row(22.0, |mut row| {
                    if is_selected
                        || session_needs_interaction(
                            &session.phase,
                            session.implementation_phase.as_deref(),
                            session.liveness,
                        )
                    {
                        row.set_selected(true);
                    }

                    // First column: liveness indicator + container badge
                    row.col(|ui| {
                        ui.horizontal(|ui| {
                            // Liveness dot
                            let color = liveness_color(session.liveness);
                            let (rect, _) =
                                ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                            ui.painter().circle_filled(rect.center(), 4.0, color);
                            ui.add_space(4.0);
                            // Container short-name badge
                            let short_name = container_short_name(&session.container_name);
                            ui.colored_label(
                                egui::Color32::GRAY,
                                egui::RichText::new(short_name).small(),
                            );
                        });
                    });
                    row.col(|ui| {
                        ui.label(&session.feature_name);
                    });
                    row.col(|ui| {
                        let color = super::status_colors::get_phase_color(&session.phase);
                        ui.colored_label(color, &session.phase);
                    });
                    row.col(|ui| {
                        ui.label(session.iteration.to_string());
                    });
                    row.col(|ui| {
                        render_status(
                            ui,
                            &session.phase,
                            &session.status,
                            session.implementation_phase.as_deref(),
                        );
                    });
                    row.col(|ui| {
                        ui.label(session.pid.to_string());
                    });
                    row.col(|ui| {
                        ui.label(&session.updated_ago);
                    });

                    // Check click AFTER adding all columns
                    if row.response().clicked() {
                        clicked_session = Some(session.session_id.clone());
                    }
                });
            }
        });

    clicked_session
}

/// Color for liveness indicator. Made public for reuse in session_detail panel.
pub fn liveness_color(liveness: LivenessDisplay) -> eframe::egui::Color32 {
    match liveness {
        LivenessDisplay::Running => eframe::egui::Color32::from_rgb(76, 175, 80),
        LivenessDisplay::Unresponsive => eframe::egui::Color32::from_rgb(255, 183, 77),
        LivenessDisplay::Stopped => eframe::egui::Color32::from_rgb(117, 117, 117),
    }
}

fn render_status(ui: &mut eframe::egui::Ui, phase: &str, status: &str, impl_phase: Option<&str>) {
    let (color, text) = super::status_colors::get_status_display(phase, status, impl_phase);
    ui.colored_label(color, text);
}
