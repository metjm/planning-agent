//! Main host application using egui/eframe.

use crate::host::server::HostEvent;
use crate::host::state::HostState;
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, Mutex};

/// Main host application.
pub struct HostApp {
    state: Arc<Mutex<HostState>>,
    event_rx: mpsc::UnboundedReceiver<HostEvent>,
    /// Cached display data (updated from async state)
    display_data: DisplayData,
    /// Last time we synced from async state
    last_sync: Instant,
    /// Server port for display
    port: u16,
}

#[derive(Default)]
struct DisplayData {
    sessions: Vec<DisplaySessionRow>,
    active_count: usize,
    approval_count: usize,
    container_count: usize,
    last_update_elapsed_secs: u64,
}

#[derive(Clone)]
struct DisplaySessionRow {
    container_name: String,
    feature_name: String,
    phase: String,
    iteration: u32,
    status: String,
    updated_ago: String,
}

impl HostApp {
    /// Create a new host application.
    pub fn new(
        state: Arc<Mutex<HostState>>,
        event_rx: mpsc::UnboundedReceiver<HostEvent>,
        port: u16,
    ) -> Self {
        Self {
            state,
            event_rx,
            display_data: DisplayData::default(),
            last_sync: Instant::now(),
            port,
        }
    }

    fn sync_display_data(&mut self) {
        // Use try_lock to avoid blocking GUI
        if let Ok(mut state) = self.state.try_lock() {
            let sessions = state.sessions();
            self.display_data.sessions = sessions
                .iter()
                .map(|s| DisplaySessionRow {
                    container_name: s.container_name.clone(),
                    feature_name: s.session.feature_name.clone(),
                    phase: s.session.phase.clone(),
                    iteration: s.session.iteration,
                    status: s.session.status.clone(),
                    updated_ago: format_relative_time(&s.session.updated_at),
                })
                .collect();
            self.display_data.active_count = state.active_count();
            self.display_data.approval_count = state.approval_count();
            self.display_data.container_count = state.containers.len();
            self.display_data.last_update_elapsed_secs = state.last_update.elapsed().as_secs();
            self.last_sync = Instant::now();
        }
    }
}

impl eframe::App for HostApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Process any pending events (non-blocking)
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                HostEvent::ContainerConnected { .. }
                | HostEvent::ContainerDisconnected { .. }
                | HostEvent::SessionsUpdated => {
                    // Will sync on next frame
                }
            }
        }

        // Sync display data periodically (every 100ms) or when state changed
        if self.last_sync.elapsed().as_millis() > 100 {
            self.sync_display_data();
        }

        // Request repaint every second for timestamp updates
        ctx.request_repaint_after(std::time::Duration::from_secs(1));

        // Render UI
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(40.0);

                ui.label(format!(
                    "Sessions: {} active",
                    self.display_data.active_count
                ));
                ui.separator();

                if self.display_data.approval_count > 0 {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 152, 0),
                        format!("{} awaiting approval", self.display_data.approval_count),
                    );
                } else {
                    ui.label("0 awaiting approval");
                }
                ui.separator();

                ui.label(format!(
                    "Last update: {}s ago",
                    self.display_data.last_update_elapsed_secs
                ));
            });
        });

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "Connected: {} containers",
                    self.display_data.container_count
                ));
                ui.separator();
                ui.label(format!("Port: {}", self.port));
                ui.separator();
                ui.label(format!("v{}", env!("CARGO_PKG_VERSION")));
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_session_table(ui);
        });
    }
}

impl HostApp {
    fn render_session_table(&self, ui: &mut egui::Ui) {
        if self.display_data.sessions.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(100.0);
                ui.heading("No sessions connected");
                ui.label("Waiting for container daemons to connect...");
                ui.add_space(20.0);
                ui.label(format!(
                    "Set PLANNING_AGENT_HOST_PORT={} in your containers",
                    self.port
                ));
            });
            return;
        }

        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(120.0).at_least(80.0).resizable(true))
            .column(Column::initial(200.0).at_least(100.0).resizable(true))
            .column(Column::exact(100.0))
            .column(Column::exact(50.0))
            .column(Column::exact(120.0))
            .column(Column::exact(100.0))
            .header(28.0, |mut header| {
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
                    ui.strong("Updated");
                });
            })
            .body(|mut body| {
                for session in &self.display_data.sessions {
                    body.row(28.0, |mut row| {
                        row.col(|ui| {
                            ui.label(&session.container_name);
                        });
                        row.col(|ui| {
                            ui.label(&session.feature_name);
                        });
                        row.col(|ui| {
                            ui.label(&session.phase);
                        });
                        row.col(|ui| {
                            ui.label(session.iteration.to_string());
                        });
                        row.col(|ui| {
                            self.render_status(ui, &session.status);
                        });
                        row.col(|ui| {
                            ui.label(&session.updated_ago);
                        });
                    });
                }
            });
    }

    fn render_status(&self, ui: &mut egui::Ui, status: &str) {
        let (color, text) = match status.to_lowercase().as_str() {
            "running" | "planning" | "reviewing" | "revising" => {
                (egui::Color32::from_rgb(76, 175, 80), "Running")
            }
            "awaitingapproval" | "awaiting_approval" => {
                (egui::Color32::from_rgb(255, 152, 0), "Approval")
            }
            "complete" => (egui::Color32::from_rgb(33, 150, 243), "Complete"),
            "error" => (egui::Color32::from_rgb(244, 67, 54), "Error"),
            "stopped" => (egui::Color32::from_rgb(117, 117, 117), "Stopped"),
            _ => (egui::Color32::from_rgb(158, 158, 158), "Unknown"),
        };
        ui.colored_label(color, text);
    }
}

fn format_relative_time(timestamp: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .map(|dt| {
            let elapsed = chrono::Utc::now().signed_duration_since(dt.with_timezone(&chrono::Utc));
            if elapsed.num_seconds() < 60 {
                "just now".to_string()
            } else if elapsed.num_minutes() < 60 {
                format!("{}m ago", elapsed.num_minutes())
            } else if elapsed.num_hours() < 24 {
                format!("{}h ago", elapsed.num_hours())
            } else {
                format!("{}d ago", elapsed.num_days())
            }
        })
        .unwrap_or_else(|_| "unknown".to_string())
}
