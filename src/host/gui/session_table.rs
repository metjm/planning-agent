//! Session table rendering for the host GUI.

use crate::session_daemon::LivenessState;
use egui_extras::{Column, TableBuilder};

/// Check if a session requires user interaction based on its phase and liveness.
///
/// A session needs interaction when:
/// 1. It is in one of these planning phases:
///    - "AwaitingPlanningDecision" - max iterations reached in planning/review cycle
///    - "Complete" - plan approved, awaiting user decision (approve/implement/decline)
/// 2. AND it is NOT stopped (liveness != Stopped)
///
/// Stopped sessions cannot receive user input until restarted, so they are excluded
/// from the interaction count even if in an awaiting phase.
pub fn session_needs_interaction(phase: &str, liveness: LivenessDisplay) -> bool {
    // Stopped sessions cannot receive input
    if matches!(liveness, LivenessDisplay::Stopped) {
        return false;
    }

    let phase_lower = phase.to_lowercase();
    matches!(
        phase_lower.as_str(),
        "complete" | "awaitingplanningdecision"
    )
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
}

#[derive(Clone, Copy)]
pub enum LivenessDisplay {
    Running,
    Unresponsive,
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

/// Render the session table with separate sections for live and disconnected sessions.
pub fn render_session_table(ui: &mut eframe::egui::Ui, sessions: &[DisplaySessionRow]) {
    if sessions.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label("No active sessions");
        });
        return;
    }

    // Partition sessions into live and disconnected based on liveness
    let (live_sessions, disconnected_sessions): (Vec<_>, Vec<_>) = sessions.iter().partition(|s| {
        matches!(
            s.liveness,
            LivenessDisplay::Running | LivenessDisplay::Unresponsive
        )
    });

    eframe::egui::ScrollArea::vertical().show(ui, |ui| {
        // Live Sessions Section
        if !live_sessions.is_empty() {
            ui.horizontal(|ui| {
                ui.colored_label(eframe::egui::Color32::from_rgb(76, 175, 80), "●");
                ui.strong(format!("Live Sessions ({})", live_sessions.len()));
            });
            ui.separator();
            render_session_rows(ui, &live_sessions);
        }

        // Disconnected Sessions Section (below live)
        if !disconnected_sessions.is_empty() {
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.colored_label(eframe::egui::Color32::from_rgb(117, 117, 117), "○");
                ui.label(format!(
                    "Disconnected Sessions ({})",
                    disconnected_sessions.len()
                ));
            });
            ui.separator();
            render_session_rows(ui, &disconnected_sessions);
        }
    });
}

/// Render session rows as a table.
fn render_session_rows(ui: &mut eframe::egui::Ui, sessions: &[&DisplaySessionRow]) {
    use eframe::egui;

    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::exact(16.0)) // Liveness indicator
        .column(Column::initial(100.0).at_least(80.0)) // Container
        .column(Column::initial(180.0).at_least(100.0)) // Feature
        .column(Column::exact(80.0)) // Phase
        .column(Column::exact(35.0)) // Iter
        .column(Column::exact(90.0)) // Status
        .column(Column::exact(60.0)) // PID
        .column(Column::exact(70.0)) // Updated
        .header(24.0, |mut header| {
            header.col(|_| {}); // Liveness - no header
            header.col(|ui| {
                ui.strong("Container");
            });
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
                body.row(22.0, |mut row| {
                    // Highlight entire row if awaiting user interaction
                    if session_needs_interaction(&session.phase, session.liveness) {
                        row.set_selected(true);
                    }

                    // Liveness indicator
                    row.col(|ui| {
                        let color = match session.liveness {
                            LivenessDisplay::Running => egui::Color32::from_rgb(76, 175, 80),
                            LivenessDisplay::Unresponsive => egui::Color32::from_rgb(255, 183, 77),
                            LivenessDisplay::Stopped => egui::Color32::from_rgb(117, 117, 117),
                        };
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.0, color);
                    });
                    row.col(|ui| {
                        ui.label(&session.container_name);
                    });
                    row.col(|ui| {
                        ui.label(&session.feature_name);
                    });
                    row.col(|ui| {
                        let phase_color = super::status_colors::get_phase_color(&session.phase);
                        ui.colored_label(phase_color, &session.phase);
                    });
                    row.col(|ui| {
                        ui.label(session.iteration.to_string());
                    });
                    row.col(|ui| {
                        render_status(ui, &session.phase, &session.status);
                    });
                    row.col(|ui| {
                        ui.label(session.pid.to_string());
                    });
                    row.col(|ui| {
                        ui.label(&session.updated_ago);
                    });
                });
            }
        });
}

/// Render the workflow status with descriptive text and color coding.
fn render_status(ui: &mut eframe::egui::Ui, phase: &str, status: &str) {
    let (color, text) = super::status_colors::get_status_display(phase, status);
    ui.colored_label(color, text);
}
