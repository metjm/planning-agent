//! Main host application using egui/eframe.

#[cfg(feature = "tray-icon")]
use crate::host::gui::tray::{HostTray, TrayCommand};
use crate::host::rpc_server::HostEvent;
use crate::host::state::HostState;
use crate::session_daemon::LivenessState;
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, Mutex};

/// Maximum number of log entries to keep.
const MAX_LOG_ENTRIES: usize = 200;

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
    /// System tray icon (requires host-gui-tray feature)
    #[cfg(feature = "tray-icon")]
    tray: Option<HostTray>,
    /// Sessions we've already notified about (for deduplication)
    notified_sessions: HashSet<String>,
    /// Event log buffer (bounded)
    log_entries: VecDeque<LogEntry>,
}

#[derive(Default)]
struct DisplayData {
    sessions: Vec<DisplaySessionRow>,
    containers: Vec<DisplayContainerRow>,
    active_count: usize,
    approval_count: usize,
    container_count: usize,
    last_update_elapsed_secs: u64,
}

#[derive(Clone)]
struct DisplayContainerRow {
    container_id: String,
    container_name: String,
    working_dir: String,
    git_sha_short: String,
    build_time: String,
    connected_duration: String,
    ping_ago: String,
    ping_healthy: bool,
    session_count: usize,
}

#[derive(Clone)]
struct DisplaySessionRow {
    session_id: String,
    container_name: String,
    feature_name: String,
    phase: String,
    iteration: u32,
    status: String,
    liveness: LivenessDisplay,
    pid: u32,
    updated_ago: String,
}

#[derive(Clone, Copy)]
enum LivenessDisplay {
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

#[derive(Clone)]
struct LogEntry {
    timestamp: String,
    message: String,
    level: LogLevel,
}

#[derive(Clone, Copy)]
enum LogLevel {
    Info,
    Warning,
}

impl HostApp {
    /// Create a new host application (with tray support).
    #[cfg(feature = "tray-icon")]
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
            log_entries: VecDeque::new(),
        }
    }

    /// Create a new host application (without tray support).
    #[cfg(not(feature = "tray-icon"))]
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
            notified_sessions: HashSet::new(),
            log_entries: VecDeque::new(),
        }
    }

    fn sync_display_data(&mut self) {
        // Use try_lock to avoid blocking GUI
        if let Ok(mut state) = self.state.try_lock() {
            // Get container count before sessions() to avoid borrow conflict
            let container_count = state.containers.len();
            // Debug: collect session counts per container before mutable borrow
            let total_sessions_in_containers: usize =
                state.containers.values().map(|c| c.sessions.len()).sum();
            let container_session_counts: Vec<(String, usize)> = state
                .containers
                .iter()
                .filter(|(_, c)| !c.sessions.is_empty())
                .map(|(id, c)| (id.clone(), c.sessions.len()))
                .collect();
            let sessions = state.sessions();
            // Log session count for debugging (only when it changes)
            let new_count = sessions.len();
            if new_count != self.display_data.sessions.len() {
                eprintln!(
                    "[host-gui] sync_display_data: {} sessions (raw: {}) from {} containers",
                    new_count, total_sessions_in_containers, container_count
                );
                // Log per-container session counts for debugging
                for (id, count) in &container_session_counts {
                    eprintln!("[host-gui]   container '{}': {} sessions", id, count);
                }
            }
            self.display_data.sessions = sessions
                .iter()
                .map(|s| DisplaySessionRow {
                    session_id: s.session.session_id.clone(),
                    container_name: s.container_name.clone(),
                    feature_name: s.session.feature_name.clone(),
                    phase: s.session.phase.clone(),
                    iteration: s.session.iteration,
                    status: s.session.status.clone(),
                    liveness: s.session.liveness.into(),
                    pid: s.session.pid,
                    updated_ago: format_relative_time(&s.session.updated_at),
                })
                .collect();
            // Collect container info with enhanced display data
            self.display_data.containers = state
                .containers
                .iter()
                .map(|(id, c)| {
                    let ping_elapsed = c.last_message_at.elapsed();
                    let ping_healthy = ping_elapsed.as_secs() < 60;
                    let connected_elapsed = c.connected_at.elapsed();

                    DisplayContainerRow {
                        container_id: id.clone(),
                        container_name: c.container_name.clone(),
                        working_dir: c.working_dir.to_string_lossy().to_string(),
                        git_sha_short: c.git_sha.chars().take(7).collect(),
                        build_time: format_build_timestamp(c.build_timestamp),
                        connected_duration: format_duration(connected_elapsed),
                        ping_ago: format_ping_duration(ping_elapsed),
                        ping_healthy,
                        session_count: c.sessions.len(),
                    }
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

    /// Handle tray icon commands (requires host-gui-tray feature).
    #[cfg(feature = "tray-icon")]
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

    /// No-op without tray-icon feature.
    #[cfg(not(feature = "tray-icon"))]
    fn handle_tray_commands(&mut self, _ctx: &egui::Context) {}
}

impl eframe::App for HostApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Handle tray icon commands
        self.handle_tray_commands(ctx);

        // Process events and log them
        self.process_events();

        // Sync display data periodically (every 100ms) or when state changed
        if self.last_sync.elapsed().as_millis() > 100 {
            self.sync_display_data();
            // Check for new sessions awaiting approval and notify
            self.check_and_notify();
        }

        // Request repaint every second for timestamp updates
        ctx.request_repaint_after(std::time::Duration::from_secs(1));

        // Header panel with stats
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong(format!("Planning Agent Host :{}", self.port));
                ui.separator();
                ui.label(format!(
                    "Containers: {}",
                    self.display_data.containers.len()
                ));
                ui.separator();
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
                }
            });
        });

        // Bottom log panel
        egui::TopBottomPanel::bottom("log_panel")
            .resizable(true)
            .min_height(60.0)
            .default_height(100.0)
            .max_height(200.0)
            .show(ctx, |ui| {
                self.render_log_panel(ui);
            });

        // Left container sidebar
        egui::SidePanel::left("containers")
            .resizable(true)
            .default_width(220.0)
            .min_width(180.0)
            .max_width(350.0)
            .show(ctx, |ui| {
                self.render_container_panel(ui);
            });

        // Central session table
        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_session_table(ui);
        });
    }
}

impl HostApp {
    /// Process events from the event channel and log them.
    fn process_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            let now = chrono::Local::now().format("%H:%M:%S").to_string();

            match event {
                HostEvent::ContainerConnected {
                    container_id,
                    container_name,
                } => {
                    self.add_log_entry(LogEntry {
                        timestamp: now,
                        message: format!(
                            "Container '{}' ({}) connected",
                            container_name, container_id
                        ),
                        level: LogLevel::Info,
                    });
                }
                HostEvent::ContainerDisconnected { container_id } => {
                    self.add_log_entry(LogEntry {
                        timestamp: now,
                        message: format!("Container '{}' disconnected", container_id),
                        level: LogLevel::Warning,
                    });
                }
                HostEvent::SessionsUpdated => {
                    // Don't log every heartbeat, just note significant changes
                }
            }
        }
    }

    /// Add a log entry to the bounded buffer.
    fn add_log_entry(&mut self, entry: LogEntry) {
        self.log_entries.push_back(entry);
        while self.log_entries.len() > MAX_LOG_ENTRIES {
            self.log_entries.pop_front();
        }
    }

    /// Render the container sidebar panel.
    fn render_container_panel(&self, ui: &mut egui::Ui) {
        ui.heading("Containers");
        ui.separator();

        egui::ScrollArea::vertical().show(ui, |ui| {
            if self.display_data.containers.is_empty() {
                ui.label("No containers connected");
                ui.label("Waiting for daemons...");
                return;
            }

            for container in &self.display_data.containers {
                ui.push_id(&container.container_id, |ui| {
                    // Health indicator color
                    let health_color = if container.ping_healthy {
                        egui::Color32::from_rgb(76, 175, 80) // Green
                    } else {
                        egui::Color32::from_rgb(255, 152, 0) // Orange
                    };

                    ui.horizontal(|ui| {
                        // Colored dot for health
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.0, health_color);
                        ui.strong(&container.container_name);
                    });

                    // Compact stats
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.small(format!("Ping: {}", container.ping_ago));
                        ui.small("|");
                        ui.small(format!("{} sessions", container.session_count));
                    });

                    ui.small(format!("Connected: {}", container.connected_duration));
                    ui.small(format!(
                        "Build: {} ({})",
                        container.git_sha_short, container.build_time
                    ));
                    ui.small(format!(
                        "Dir: {}",
                        truncate_path(&container.working_dir, 25)
                    ));

                    ui.add_space(8.0);
                });
            }
        });
    }

    /// Render the log panel at the bottom.
    fn render_log_panel(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Event Log");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.small(format!("{} entries", self.log_entries.len()));
            });
        });
        ui.separator();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for entry in &self.log_entries {
                    ui.horizontal(|ui| {
                        // Timestamp in muted color
                        ui.colored_label(egui::Color32::from_rgb(128, 128, 128), &entry.timestamp);

                        // Message with level-based color
                        let msg_color = match entry.level {
                            LogLevel::Info => egui::Color32::WHITE,
                            LogLevel::Warning => egui::Color32::from_rgb(255, 183, 77),
                        };
                        ui.colored_label(msg_color, &entry.message);
                    });
                }
            });
    }

    /// Render the session table with liveness and PID columns.
    fn render_session_table(&self, ui: &mut egui::Ui) {
        if self.display_data.sessions.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("No active sessions");
            });
            return;
        }

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
                for session in &self.display_data.sessions {
                    body.row(22.0, |mut row| {
                        // Liveness indicator
                        row.col(|ui| {
                            let color = match session.liveness {
                                LivenessDisplay::Running => egui::Color32::from_rgb(76, 175, 80),
                                LivenessDisplay::Unresponsive => {
                                    egui::Color32::from_rgb(255, 183, 77)
                                }
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
                            ui.label(&session.phase);
                        });
                        row.col(|ui| {
                            ui.label(session.iteration.to_string());
                        });
                        row.col(|ui| {
                            self.render_status(ui, &session.status);
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

/// Format a Unix timestamp into a human-readable date/time.
fn format_build_timestamp(timestamp: u64) -> String {
    use chrono::{TimeZone, Utc};
    if timestamp == 0 {
        return "unknown".to_string();
    }
    Utc.timestamp_opt(timestamp as i64, 0)
        .single()
        .map(|dt| dt.format("%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "invalid".to_string())
}

/// Format a duration as a human-readable string (e.g., "5m", "2h 30m").
fn format_duration(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        if mins > 0 {
            format!("{}h {}m", hours, mins)
        } else {
            format!("{}h", hours)
        }
    }
}

/// Format ping duration (e.g., "2s ago", "45s ago").
fn format_ping_duration(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else {
        format!("{}h ago", secs / 3600)
    }
}

/// Truncate a path for display, showing the end portion.
fn truncate_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        path.to_string()
    } else {
        format!("...{}", &path[path.len().saturating_sub(max_len - 3)..])
    }
}
