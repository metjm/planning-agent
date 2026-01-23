//! Main host application using egui/eframe.

#[cfg(not(target_os = "linux"))]
use crate::host::gui::tray::{HostTray, TrayCommand};
use crate::host::server::HostEvent;
use crate::host::state::HostState;
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::collections::HashSet;
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
    /// System tray icon (only on platforms with tray support)
    #[cfg(not(target_os = "linux"))]
    tray: Option<HostTray>,
    /// Sessions we've already notified about (for deduplication)
    notified_sessions: HashSet<String>,
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
    session_id: String,
    container_name: String,
    feature_name: String,
    phase: String,
    iteration: u32,
    status: String,
    updated_ago: String,
}

impl HostApp {
    /// Create a new host application.
    #[cfg(not(target_os = "linux"))]
    pub fn new(
        state: Arc<Mutex<HostState>>,
        event_rx: mpsc::UnboundedReceiver<HostEvent>,
        port: u16,
    ) -> Self {
        // Try to create tray icon (may fail on some platforms)
        let tray = match HostTray::new() {
            Ok(t) => {
                eprintln!("[host] System tray icon created");
                Some(t)
            }
            Err(e) => {
                eprintln!("[host] Warning: Could not create tray icon: {}", e);
                None
            }
        };

        Self {
            state,
            event_rx,
            display_data: DisplayData::default(),
            last_sync: Instant::now(),
            port,
            tray,
            notified_sessions: HashSet::new(),
        }
    }

    /// Create a new host application (Linux - no tray support).
    #[cfg(target_os = "linux")]
    pub fn new(
        state: Arc<Mutex<HostState>>,
        event_rx: mpsc::UnboundedReceiver<HostEvent>,
        port: u16,
    ) -> Self {
        eprintln!("[host] System tray not available on Linux (gtk3-rs deprecated)");

        Self {
            state,
            event_rx,
            display_data: DisplayData::default(),
            last_sync: Instant::now(),
            port,
            notified_sessions: HashSet::new(),
        }
    }

    fn sync_display_data(&mut self) {
        // Use try_lock to avoid blocking GUI
        if let Ok(mut state) = self.state.try_lock() {
            let sessions = state.sessions();
            self.display_data.sessions = sessions
                .iter()
                .map(|s| DisplaySessionRow {
                    session_id: s.session.session_id.clone(),
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

    /// Check for new sessions awaiting approval and send notifications.
    fn check_and_notify(&mut self) {
        // Find sessions awaiting approval that we haven't notified about yet
        let awaiting_approval: Vec<_> = self
            .display_data
            .sessions
            .iter()
            .filter(|s| {
                let status_lower = s.status.to_lowercase();
                (status_lower.contains("approval") || status_lower == "awaitingapproval")
                    && !self.notified_sessions.contains(&s.session_id)
            })
            .collect();

        for session in awaiting_approval {
            // Mark as notified
            self.notified_sessions.insert(session.session_id.clone());

            // Send notification
            if let Err(e) = notify_rust::Notification::new()
                .summary("Planning Agent - Approval Required")
                .body(&format!(
                    "{} on {} is waiting for approval",
                    session.feature_name, session.container_name
                ))
                .timeout(notify_rust::Timeout::Milliseconds(5000))
                .show()
            {
                eprintln!("[host] Warning: Could not send notification: {}", e);
            }
        }

        // Clean up notified_sessions for sessions that are no longer awaiting approval
        let current_awaiting: HashSet<String> = self
            .display_data
            .sessions
            .iter()
            .filter(|s| {
                let status_lower = s.status.to_lowercase();
                status_lower.contains("approval") || status_lower == "awaitingapproval"
            })
            .map(|s| s.session_id.clone())
            .collect();

        self.notified_sessions
            .retain(|id| current_awaiting.contains(id));
    }

    /// Handle tray icon commands (only on platforms with tray support).
    #[cfg(not(target_os = "linux"))]
    fn handle_tray_commands(&mut self, ctx: &egui::Context) {
        if let Some(ref tray) = self.tray {
            while let Some(cmd) = tray.try_recv_command() {
                match cmd {
                    TrayCommand::ShowWindow => {
                        // Request focus on the window
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                    TrayCommand::Quit => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
        }
    }

    /// No-op on Linux (tray not supported).
    #[cfg(target_os = "linux")]
    fn handle_tray_commands(&mut self, _ctx: &egui::Context) {
        // Tray not available on Linux
    }
}

impl eframe::App for HostApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Handle tray icon commands
        self.handle_tray_commands(ctx);

        // Process any pending events (non-blocking)
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                HostEvent::ContainerConnected {
                    container_id,
                    container_name,
                } => {
                    eprintln!(
                        "[host-gui] Container connected: {} ({})",
                        container_name, container_id
                    );
                }
                HostEvent::ContainerDisconnected { container_id } => {
                    eprintln!("[host-gui] Container disconnected: {}", container_id);
                }
                HostEvent::SessionsUpdated => {
                    // Will sync on next frame
                }
            }
        }

        // Sync display data periodically (every 100ms) or when state changed
        if self.last_sync.elapsed().as_millis() > 100 {
            self.sync_display_data();
            // Check for new sessions awaiting approval and notify
            self.check_and_notify();
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
