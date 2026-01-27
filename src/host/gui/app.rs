//! Main host application using egui/eframe.

use crate::account_usage::types::AccountId;
#[cfg(feature = "tray-icon")]
use crate::host::gui::tray::{HostTray, TrayCommand};
use crate::host::gui::usage_panel::{self, AccountProvider, DisplayAccountRow};
use crate::host::rpc_server::HostEvent;
use crate::host::state::HostState;
use eframe::egui;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, Mutex};

use super::session_detail::SessionDetailData;
use super::session_selection::{
    DisplayContainerRowLite, PendingFileContent, PendingFileList, SessionSelectionManager,
};
use super::session_table::DisplaySessionRow;

/// Maximum number of log entries to keep.
const MAX_LOG_ENTRIES: usize = 200;

/// Usage fetch interval when sessions are active (2 minutes).
const ACTIVE_FETCH_INTERVAL_SECS: u64 = 120;

/// Usage fetch interval when no sessions are active (10 minutes).
const IDLE_FETCH_INTERVAL_SECS: u64 = 600;

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
    /// Last time we fetched usage (None = never, triggers initial fetch)
    last_usage_fetch: Option<Instant>,
    /// Last error message per account to dedupe logging
    account_error_cache: HashMap<AccountId, String>,
    /// Currently selected session for detail view
    selected_session_id: Option<String>,
    /// Detail data for selected session (populated via RPC)
    session_detail: Option<SessionDetailData>,
    /// Pending file list fetch result.
    pending_file_list: PendingFileList,
    /// Pending file content fetch result.
    pending_file_content: PendingFileContent,
}

#[derive(Default)]
struct DisplayData {
    sessions: Vec<DisplaySessionRow>,
    containers: Vec<DisplayContainerRow>,
    accounts: Vec<DisplayAccountRow>,
    active_count: usize,
    approval_count: usize,
    container_count: usize,
    last_update_elapsed_secs: u64,
    /// Active sessions NOT awaiting interaction
    running_count: usize,
    /// Sessions awaiting user interaction
    awaiting_count: usize,
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
    file_service_port: u16,
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
            last_usage_fetch: None,
            account_error_cache: HashMap::new(),
            selected_session_id: None,
            session_detail: None,
            pending_file_list: Arc::new(Mutex::new(None)),
            pending_file_content: Arc::new(Mutex::new(None)),
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
            last_usage_fetch: None,
            account_error_cache: HashMap::new(),
            selected_session_id: None,
            session_detail: None,
            pending_file_list: Arc::new(Mutex::new(None)),
            pending_file_content: Arc::new(Mutex::new(None)),
        }
    }

    fn sync_display_data(&mut self) {
        // Collect all data from state first, then release lock before mutating self
        let collected = {
            // Use try_lock to avoid blocking GUI
            let Ok(mut state) = self.state.try_lock() else {
                return;
            };

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

            // Collect all immutable borrows BEFORE calling sessions() which takes &mut self
            let container_rows: Vec<DisplayContainerRow> = state
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
                        file_service_port: c.file_service_port,
                    }
                })
                .collect();

            let active_count = state.active_count();
            let approval_count = state.approval_count();
            let last_update_elapsed_secs = state.last_update.elapsed().as_secs();

            // Collect account usage data (immutable borrow)
            let mut account_rows: Vec<DisplayAccountRow> = Vec::new();
            let mut seen_accounts: HashSet<AccountId> = HashSet::new();
            let mut errors_to_log: Vec<(AccountId, String, String, String)> = Vec::new();
            let mut accounts_to_clear: Vec<AccountId> = Vec::new();

            for record in state.usage_store.get_all_accounts() {
                seen_accounts.insert(record.account_id.clone());

                // Get display usage, preferring last_successful_usage when current has error
                let (usage_to_display, is_stale) =
                    match super::usage_extrapolation::get_display_usage(record) {
                        Some((usage, stale)) => (usage, stale),
                        None => {
                            // No usable data at all - skip this account
                            continue;
                        }
                    };

                // Log errors for accounts where current fetch failed (but we may still display stale data)
                if let Some(current) = &record.current_usage {
                    if let Some(error) = &current.error {
                        if !is_stale {
                            // Error with no fallback - log it
                            errors_to_log.push((
                                record.account_id.clone(),
                                record.provider.clone(),
                                record.email.clone(),
                                error.clone(),
                            ));
                        }
                        // If is_stale, we'll display the stale data, so don't skip
                    }
                }

                let provider = match AccountProvider::try_from_str(&record.provider) {
                    Some(provider) => provider,
                    None => {
                        eprintln!(
                            "[host-gui] unknown account provider: provider={}, email={}",
                            record.provider, record.email
                        );
                        continue;
                    }
                };

                // Extrapolate session/weekly percent if reset time has passed
                let session_percent =
                    super::usage_extrapolation::extrapolate_usage(&usage_to_display.session_window);
                let weekly_percent =
                    super::usage_extrapolation::extrapolate_usage(&usage_to_display.weekly_window);

                // Get current token validity from most recent state
                let token_valid = record
                    .current_usage
                    .as_ref()
                    .map(|u| u.token_valid)
                    .unwrap_or(usage_to_display.token_valid);

                // Format stale reason
                let stale_reason = if is_stale {
                    super::usage_extrapolation::format_stale_reason(
                        record.last_successful_fetch.as_deref(),
                        token_valid,
                    )
                } else {
                    None
                };

                accounts_to_clear.push(record.account_id.clone());

                account_rows.push(DisplayAccountRow {
                    account_id: record.account_id.clone(),
                    provider,
                    email: record.email.clone(),
                    session_percent,
                    session_reset: usage_to_display
                        .session_window
                        .reset_at
                        .map(|r| format_reset_countdown(r.epoch_seconds))
                        .unwrap_or_default(),
                    weekly_percent,
                    weekly_reset: usage_to_display
                        .weekly_window
                        .reset_at
                        .map(|r| format_reset_countdown(r.epoch_seconds))
                        .unwrap_or_default(),
                    token_valid,
                    is_stale,
                    stale_reason,
                });
            }

            account_rows.sort_by(|a, b| {
                (a.provider.order_index(), &a.email).cmp(&(b.provider.order_index(), &b.email))
            });

            // NOW call sessions() which takes &mut self - do this last
            let sessions = state.sessions();

            let session_rows: Vec<DisplaySessionRow> = sessions
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
            let sessions_len = sessions.len();

            // Return collected data - state lock is released here
            (
                session_rows,
                container_rows,
                active_count,
                approval_count,
                container_count,
                last_update_elapsed_secs,
                account_rows,
                seen_accounts,
                errors_to_log,
                accounts_to_clear,
                sessions_len,
                total_sessions_in_containers,
                container_session_counts,
            )
        };

        // Now state lock is released - safe to mutate self
        let (
            session_rows,
            container_rows,
            active_count,
            approval_count,
            container_count,
            last_update_elapsed_secs,
            account_rows,
            seen_accounts,
            errors_to_log,
            accounts_to_clear,
            new_count,
            total_sessions_in_containers,
            container_session_counts,
        ) = collected;

        // Log session count for debugging (only when it changes)
        if new_count != self.display_data.sessions.len() {
            eprintln!(
                "[host-gui] sync_display_data: {} sessions (raw: {}) from {} containers",
                new_count, total_sessions_in_containers, container_count
            );
            for (id, count) in &container_session_counts {
                eprintln!("[host-gui]   container '{}': {} sessions", id, count);
            }
        }

        // Compute awaiting/running counts using session_needs_interaction
        // Note: session_needs_interaction checks both phase AND liveness, so stopped
        // sessions in awaiting phases are correctly excluded from awaiting_count.
        let awaiting_count = session_rows
            .iter()
            .filter(|s| super::session_table::session_needs_interaction(&s.phase, s.liveness))
            .count();
        let running_count = active_count.saturating_sub(awaiting_count);

        self.display_data.sessions = session_rows;
        self.display_data.containers = container_rows;
        self.display_data.active_count = active_count;
        self.display_data.approval_count = approval_count;
        self.display_data.container_count = container_count;
        self.display_data.last_update_elapsed_secs = last_update_elapsed_secs;
        self.display_data.running_count = running_count;
        self.display_data.awaiting_count = awaiting_count;

        for (account_id, provider, email, error) in errors_to_log {
            self.log_account_error_once(&account_id, &provider, &email, &error);
        }
        for account_id in accounts_to_clear {
            self.account_error_cache.remove(&account_id);
        }
        self.account_error_cache
            .retain(|account_id, _| seen_accounts.contains(account_id));
        self.display_data.accounts = account_rows;
        self.last_sync = Instant::now();
    }

    fn log_account_error_once(
        &mut self,
        account_id: &AccountId,
        provider: &str,
        email: &str,
        error: &str,
    ) {
        let should_log = self
            .account_error_cache
            .get(account_id)
            .map(|prev| prev != error)
            .unwrap_or(true);
        if should_log {
            eprintln!(
                "[host-gui] account usage error: provider={}, email={}, error={}",
                provider, email, error
            );
            self.account_error_cache
                .insert(account_id.clone(), error.to_string());
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

/// Render the stats dashboard above the session table.
fn render_stats_dashboard(
    ui: &mut egui::Ui,
    running_count: usize,
    awaiting_count: usize,
    total_count: usize,
) {
    ui.horizontal(|ui| {
        // Running agents (green)
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(running_count.to_string())
                    .size(48.0)
                    .color(egui::Color32::from_rgb(76, 175, 80)), // Green
            );
            ui.label("Running");
        });

        ui.add_space(32.0);

        // Awaiting interaction (amber/gray)
        ui.vertical(|ui| {
            let color = if awaiting_count > 0 {
                egui::Color32::from_rgb(255, 152, 0) // Amber
            } else {
                egui::Color32::GRAY
            };
            ui.label(
                egui::RichText::new(awaiting_count.to_string())
                    .size(48.0)
                    .color(color),
            );
            ui.label("Awaiting Input");
        });

        ui.add_space(32.0);

        // Total sessions
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(total_count.to_string())
                    .size(48.0)
                    .color(egui::Color32::LIGHT_GRAY),
            );
            ui.label("Total");
        });
    });
}

impl eframe::App for HostApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Handle tray icon commands
        self.handle_tray_commands(ctx);

        // Process events and log them
        self.process_events();

        // Check if we need to fetch usage (on startup or periodically)
        self.maybe_fetch_usage();

        // Check for pending async results (file list / content)
        self.check_pending_results();

        // Handle container disconnect while detail panel is open
        self.handle_container_disconnect_for_detail();

        // Sync display data periodically (every 100ms) or when state changed
        if self.last_sync.elapsed().as_millis() > 100 {
            self.sync_display_data();
            // Check for new sessions awaiting approval and notify
            self.check_and_notify();

            // Update tray icon with current counts (only if changed - method handles caching)
            #[cfg(feature = "tray-icon")]
            if let Some(ref mut tray) = self.tray {
                tray.update_icon(
                    self.display_data.running_count,
                    self.display_data.awaiting_count,
                );
            }
        }

        // Sync detail panel fields from latest display_data
        self.sync_detail_from_display_data();

        // Request repaint every second for timestamp updates
        ctx.request_repaint_after(std::time::Duration::from_secs(1));

        // Handle Escape key to close detail panel
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && self.selected_session_id.is_some() {
            self.selected_session_id = None;
            self.session_detail = None;
        }

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

        // Stats dashboard panel (after header, before central panel)
        egui::TopBottomPanel::top("stats_dashboard")
            .resizable(false)
            .show(ctx, |ui| {
                render_stats_dashboard(
                    ui,
                    self.display_data.running_count,
                    self.display_data.awaiting_count,
                    self.display_data.sessions.len(),
                );
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

        // Right usage sidebar
        egui::SidePanel::right("usage")
            .resizable(true)
            .default_width(200.0)
            .min_width(160.0)
            .max_width(300.0)
            .show(ctx, |ui| {
                self.render_usage_panel(ui);
            });

        // Session detail panel (right side, only shown when session selected)
        if self.selected_session_id.is_some() {
            egui::SidePanel::right("session_detail")
                .resizable(true)
                .default_width(400.0)
                .min_width(300.0)
                .max_width(600.0)
                .show(ctx, |ui| {
                    self.handle_session_detail_panel(ui);
                });
        }

        // Central session table (handles clicks)
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(clicked_id) = super::session_table::render_session_table(
                ui,
                &self.display_data.sessions,
                &self.selected_session_id,
            ) {
                self.select_session(&clicked_id);
            }
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
                HostEvent::CredentialsReported => {
                    self.add_log_entry(LogEntry {
                        timestamp: now,
                        message: "Credentials reported from daemon".to_string(),
                        level: LogLevel::Info,
                    });
                    // Trigger usage fetching (async - will update store)
                    self.trigger_usage_fetch();
                }
            }
        }
    }

    /// Check if we need to fetch usage (on startup or after activity-based interval).
    ///
    /// Polling intervals follow the plan:
    /// - Active sessions (Planning/Reviewing/Revising): Poll every 2 minutes
    /// - Idle (no active sessions): Poll every 10 minutes
    fn maybe_fetch_usage(&mut self) {
        // Determine if we have active sessions
        let has_active_sessions = self.display_data.sessions.iter().any(|s| {
            let phase_lower = s.phase.to_lowercase();
            phase_lower == "planning" || phase_lower == "reviewing" || phase_lower == "revising"
        });

        // Choose interval based on activity
        let interval = if has_active_sessions {
            ACTIVE_FETCH_INTERVAL_SECS
        } else {
            IDLE_FETCH_INTERVAL_SECS
        };

        let should_fetch = match self.last_usage_fetch {
            None => true, // Never fetched - fetch on startup
            Some(last) => last.elapsed().as_secs() >= interval,
        };

        if should_fetch {
            let mode = if has_active_sessions {
                "active"
            } else {
                "idle"
            };
            eprintln!(
                "[host-gui] Triggering usage fetch ({} mode, {}s interval)",
                mode, interval
            );
            self.last_usage_fetch = Some(Instant::now());
            self.trigger_usage_fetch();
        }
    }

    /// Trigger background usage fetch using credentials from daemon.
    /// Uses blocking HTTP calls via ureq, so it's safe to call from async context.
    fn trigger_usage_fetch(&mut self) {
        let state = self.state.clone();
        // Spawn tokio task to fetch usage (ureq is blocking but tokio handles this)
        tokio::spawn(async move {
            let mut state = state.lock().await;
            // Get credentials from daemon (stored via RPC)
            let credentials = state.get_credentials();
            if credentials.is_empty() {
                eprintln!("[host-gui] No credentials available from daemon");
                return;
            }
            crate::account_usage::fetcher::fetch_usage_with_credentials(
                &mut state.usage_store,
                credentials,
                None,
            );
            if let Err(e) = state.usage_store.save() {
                eprintln!("[host-gui] Failed to save usage store: {}", e);
            }
        });
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

    /// Render the usage sidebar panel.
    fn render_usage_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Account Usage");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("↻")
                    .on_hover_text("Refresh all accounts")
                    .clicked()
                {
                    self.last_usage_fetch = None; // Force immediate refresh
                }
            });
        });
        ui.separator();

        egui::ScrollArea::vertical().show(ui, |ui| {
            usage_panel::render_usage_panel_content(ui, &self.display_data.accounts);
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
                            LogLevel::Info => egui::Color32::LIGHT_GRAY,
                            LogLevel::Warning => egui::Color32::from_rgb(255, 183, 77),
                        };
                        ui.colored_label(msg_color, &entry.message);
                    });
                }
            });
    }

    /// Get containers as lite structs for session selection helper.
    fn get_containers_lite(&self) -> Vec<DisplayContainerRowLite> {
        self.display_data
            .containers
            .iter()
            .map(|c| DisplayContainerRowLite {
                container_id: c.container_id.clone(),
                container_name: c.container_name.clone(),
                file_service_port: c.file_service_port,
            })
            .collect()
    }

    /// Check for pending async results and update session_detail.
    fn check_pending_results(&mut self) {
        SessionSelectionManager::check_pending_results(
            &self.pending_file_list,
            &self.pending_file_content,
            &mut self.session_detail,
        );
    }

    /// Select a session and initiate RPC fetch for file list.
    fn select_session(&mut self, session_id: &str) {
        let containers = self.get_containers_lite();
        let (new_selected, new_detail) = SessionSelectionManager::select_session(
            session_id,
            &self.selected_session_id,
            &self.display_data.sessions,
            &containers,
            self.pending_file_list.clone(),
        );
        self.selected_session_id = new_selected;
        self.session_detail = new_detail;
    }

    /// Handle container disconnect while detail panel is open.
    fn handle_container_disconnect_for_detail(&mut self) {
        let containers = self.get_containers_lite();
        SessionSelectionManager::handle_container_disconnect_for_detail(
            &mut self.session_detail,
            &containers,
        );
    }

    /// Sync detail panel data from display_data (keeps updated_ago fresh).
    fn sync_detail_from_display_data(&mut self) {
        SessionSelectionManager::sync_detail_from_display_data(
            &mut self.session_detail,
            &self.display_data.sessions,
        );
    }

    /// Wrapper method that delegates to session_detail::render_session_detail_panel
    /// and handles the returned state (close, file clicks).
    fn handle_session_detail_panel(&mut self, ui: &mut egui::Ui) {
        let containers = self.get_containers_lite();
        SessionSelectionManager::render_and_handle_detail_panel(
            ui,
            &mut self.session_detail,
            &mut self.selected_session_id,
            self.pending_file_content.clone(),
            &containers,
        );
    }
}

// Helper functions re-exported from helpers module
use super::helpers::{
    format_build_timestamp, format_duration, format_ping_duration, format_relative_time,
    format_reset_countdown, truncate_path,
};
