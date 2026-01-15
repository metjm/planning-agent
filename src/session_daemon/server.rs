//! Session daemon server implementation.
//!
//! Maintains an in-memory registry of active sessions and handles client connections
//! via Unix socket (or TCP on Windows).

use crate::planning_paths;
use crate::session_daemon::protocol::{ClientMessage, DaemonMessage, LivenessState, SessionRecord};
use crate::update::BUILD_SHA;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, Mutex};

/// Log a daemon server event to ~/.planning-agent/daemon-debug.log
fn daemon_log(msg: &str) {
    use std::io::Write;
    if let Ok(home) = planning_paths::planning_agent_home_dir() {
        let log_path = home.join("daemon-debug.log");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            let now = chrono::Local::now().format("%H:%M:%S%.3f");
            let _ = writeln!(f, "[{}] [server] {}", now, msg);
        }
    }
}

/// Default timeout for marking sessions as unresponsive (seconds).
const UNRESPONSIVE_TIMEOUT_SECS: u64 = 25;

/// Default timeout for marking sessions as stopped (seconds).
/// Can be overridden via PLANNING_SESSIOND_STALE_SECS environment variable.
const DEFAULT_STALE_TIMEOUT_SECS: u64 = 60;

/// Registry persistence interval (seconds).
const REGISTRY_PERSIST_INTERVAL_SECS: u64 = 30;

/// Shared daemon state.
pub(crate) struct DaemonState {
    /// Session registry keyed by workflow_session_id
    pub(crate) sessions: HashMap<String, SessionRecord>,
    /// Flag indicating daemon is shutting down
    pub(crate) shutting_down: bool,
}

impl DaemonState {
    pub(crate) fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            shutting_down: false,
        }
    }

    /// Load sessions from persisted registry file.
    fn load_from_disk(&mut self) -> Result<()> {
        let registry_path = planning_paths::sessiond_registry_path()?;
        if registry_path.exists() {
            let content = std::fs::read_to_string(&registry_path)
                .context("Failed to read registry file")?;
            let records: Vec<SessionRecord> = serde_json::from_str(&content)
                .context("Failed to parse registry file")?;

            // Load records but mark them as stopped (they're from a previous daemon instance)
            for mut record in records {
                record.liveness = LivenessState::Stopped;
                self.sessions.insert(record.workflow_session_id.clone(), record);
            }
        }
        Ok(())
    }

    /// Persist sessions to disk for recovery.
    fn persist_to_disk(&self) -> Result<()> {
        let registry_path = planning_paths::sessiond_registry_path()?;
        let records: Vec<&SessionRecord> = self.sessions.values().collect();
        let content = serde_json::to_string_pretty(&records)
            .context("Failed to serialize registry")?;
        std::fs::write(&registry_path, content)
            .context("Failed to write registry file")?;
        Ok(())
    }

    /// Get stale timeout from environment or default.
    fn stale_timeout_secs() -> u64 {
        std::env::var("PLANNING_SESSIOND_STALE_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_STALE_TIMEOUT_SECS)
    }

    /// Update liveness states based on heartbeat timestamps.
    fn update_liveness_states(&mut self) {
        let _ = self.update_liveness_states_with_changes();
    }

    /// Update liveness states and return records that changed.
    fn update_liveness_states_with_changes(&mut self) -> Vec<SessionRecord> {
        let now = chrono::Utc::now();
        let stale_timeout = Self::stale_timeout_secs();
        let mut changed = Vec::new();

        for record in self.sessions.values_mut() {
            // Skip already stopped sessions
            if record.liveness == LivenessState::Stopped {
                continue;
            }

            // Parse last heartbeat timestamp
            let last_heartbeat = match chrono::DateTime::parse_from_rfc3339(&record.last_heartbeat_at)
            {
                Ok(dt) => dt.with_timezone(&chrono::Utc),
                Err(_) => continue,
            };

            let elapsed_secs = (now - last_heartbeat).num_seconds() as u64;
            let old_liveness = record.liveness;

            if elapsed_secs > stale_timeout {
                record.liveness = LivenessState::Stopped;
            } else if elapsed_secs > UNRESPONSIVE_TIMEOUT_SECS {
                record.liveness = LivenessState::Unresponsive;
            }

            if record.liveness != old_liveness {
                changed.push(record.clone());
            }
        }

        changed
    }
}

/// Runs the session daemon server.
pub async fn run_daemon() -> Result<()> {
    // Write PID file
    let pid = std::process::id();
    let pid_path = planning_paths::sessiond_pid_path()?;
    std::fs::write(&pid_path, pid.to_string())
        .context("Failed to write PID file")?;

    // Write build SHA file
    let sha_path = planning_paths::sessiond_build_sha_path()?;
    std::fs::write(&sha_path, BUILD_SHA)
        .context("Failed to write build SHA file")?;

    // Initialize state and load persisted registry
    let state = Arc::new(Mutex::new(DaemonState::new()));
    {
        let mut state_guard = state.lock().await;
        if let Err(e) = state_guard.load_from_disk() {
            eprintln!("[sessiond] Warning: Failed to load registry: {}", e);
        }
    }

    // Create shutdown broadcast channel
    let (shutdown_tx, _) = broadcast::channel::<String>(1);

    // Create session events broadcast channel for push notifications
    let (events_tx, _) = broadcast::channel::<SessionRecord>(64);

    // Start the listener
    #[cfg(unix)]
    let result = run_unix_server(state.clone(), shutdown_tx.clone(), events_tx.clone()).await;

    #[cfg(windows)]
    let result = run_windows_server(state.clone(), shutdown_tx.clone(), events_tx.clone()).await;

    // Cleanup on exit
    let _ = std::fs::remove_file(&pid_path);
    let _ = std::fs::remove_file(&sha_path);

    #[cfg(unix)]
    {
        if let Ok(socket_path) = planning_paths::sessiond_socket_path() {
            let _ = std::fs::remove_file(&socket_path);
        }
    }

    result
}

/// Unix socket server implementation.
#[cfg(unix)]
async fn run_unix_server(
    state: Arc<Mutex<DaemonState>>,
    shutdown_tx: broadcast::Sender<String>,
    events_tx: broadcast::Sender<SessionRecord>,
) -> Result<()> {
    use tokio::net::UnixListener;

    let socket_path = planning_paths::sessiond_socket_path()?;

    // Check for existing socket
    if socket_path.exists() {
        // Try connecting to see if another daemon is running
        if tokio::net::UnixStream::connect(&socket_path).await.is_ok() {
            anyhow::bail!("Another session daemon is already running");
        }
        // Stale socket, remove it
        std::fs::remove_file(&socket_path)
            .context("Failed to remove stale socket")?;
    }

    let listener = UnixListener::bind(&socket_path)
        .context("Failed to bind Unix socket")?;

    eprintln!("[sessiond] Listening on {}", socket_path.display());

    // Spawn registry persistence task
    let persist_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(
            tokio::time::Duration::from_secs(REGISTRY_PERSIST_INTERVAL_SECS)
        );
        loop {
            interval.tick().await;
            let state_guard = persist_state.lock().await;
            if state_guard.shutting_down {
                break;
            }
            if let Err(e) = state_guard.persist_to_disk() {
                eprintln!("[sessiond] Warning: Failed to persist registry: {}", e);
            }
        }
    });

    // Spawn liveness update task
    let liveness_state = state.clone();
    let liveness_events_tx = events_tx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            let mut state_guard = liveness_state.lock().await;
            if state_guard.shutting_down {
                break;
            }
            // Update liveness and broadcast any changes
            let changed = state_guard.update_liveness_states_with_changes();
            for record in changed {
                let _ = liveness_events_tx.send(record);
            }
        }
    });

    // Accept connections
    loop {
        let (stream, _) = listener.accept().await?;

        // Check shutdown flag
        {
            let state_guard = state.lock().await;
            if state_guard.shutting_down {
                break;
            }
        }

        let conn_state = state.clone();
        let conn_events_tx = events_tx.clone();
        let mut shutdown_rx = shutdown_tx.subscribe();

        tokio::spawn(async move {
            daemon_log("connection handler started");
            let (reader, writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut writer = writer;
            let mut subscribed = false;
            let mut events_rx: Option<broadcast::Receiver<SessionRecord>> = None;

            loop {
                let mut line = String::new();

                // Build the select based on subscription state
                if subscribed {
                    daemon_log("in subscriber loop, waiting for data or events");
                    let events_receiver = events_rx.as_mut().unwrap();
                    tokio::select! {
                        result = reader.read_line(&mut line) => {
                            daemon_log(&format!("subscriber: read_line result: {:?}", result.as_ref().map(|n| *n)));
                            match result {
                                Ok(0) => {
                                    daemon_log("subscriber: got EOF, breaking");
                                    break;
                                }
                                Ok(_) => {
                                    daemon_log(&format!("subscriber: received: {}", line.trim()));
                                    let (response, should_subscribe) = handle_message_with_broadcast(&line, &conn_state, &conn_events_tx).await;
                                    if should_subscribe && !subscribed {
                                        subscribed = true;
                                        events_rx = Some(conn_events_tx.subscribe());
                                    }
                                    if let Some(response) = response {
                                        let response_json = match serde_json::to_string(&response) {
                                            Ok(json) => json,
                                            Err(e) => {
                                                eprintln!("[sessiond] Failed to serialize response: {}", e);
                                                continue;
                                            }
                                        };
                                        if writer.write_all(format!("{}\n", response_json).as_bytes()).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                        event = events_receiver.recv() => {
                            // Forward push notification to client
                            if let Ok(record) = event {
                                let msg = DaemonMessage::SessionChanged(record);
                                if let Ok(json) = serde_json::to_string(&msg) {
                                    if writer.write_all(format!("{}\n", json).as_bytes()).await.is_err() {
                                        break;
                                    }
                                }
                            }
                        }
                        _ = shutdown_rx.recv() => {
                            let msg = DaemonMessage::Restarting {
                                new_sha: BUILD_SHA.to_string(),
                            };
                            if let Ok(json) = serde_json::to_string(&msg) {
                                let _ = writer.write_all(format!("{}\n", json).as_bytes()).await;
                            }
                            break;
                        }
                    }
                } else {
                    daemon_log("in non-subscriber loop, waiting for data");
                    tokio::select! {
                        result = reader.read_line(&mut line) => {
                            daemon_log(&format!("non-sub: read_line result: {:?}", result.as_ref().map(|n| *n)));
                            match result {
                                Ok(0) => {
                                    daemon_log("non-sub: got EOF, breaking");
                                    break;
                                }
                                Ok(_) => {
                                    daemon_log(&format!("non-sub: received: {}", line.trim()));
                                    let (response, should_subscribe) = handle_message_with_broadcast(&line, &conn_state, &conn_events_tx).await;
                                    daemon_log(&format!("non-sub: should_subscribe={}", should_subscribe));
                                    if should_subscribe {
                                        daemon_log("non-sub: setting subscribed=true, creating events_rx");
                                        subscribed = true;
                                        events_rx = Some(conn_events_tx.subscribe());
                                    }
                                    if let Some(ref response) = response {
                                        daemon_log(&format!("non-sub: sending response: {:?}", response));
                                        let response_json = match serde_json::to_string(&response) {
                                            Ok(json) => json,
                                            Err(e) => {
                                                eprintln!("[sessiond] Failed to serialize response: {}", e);
                                                continue;
                                            }
                                        };
                                        if writer.write_all(format!("{}\n", response_json).as_bytes()).await.is_err() {
                                            daemon_log("non-sub: write failed, breaking");
                                            break;
                                        }
                                        daemon_log("non-sub: response sent successfully");

                                        // Check if we should shut down
                                        if matches!(response, DaemonMessage::Ack { .. }) {
                                            let state_guard = conn_state.lock().await;
                                            if state_guard.shutting_down {
                                                daemon_log("non-sub: shutting_down flag set, breaking");
                                                break;
                                            }
                                        }
                                    }
                                    daemon_log("non-sub: continuing loop");
                                }
                                Err(e) => {
                                    daemon_log(&format!("non-sub: read error: {}, breaking", e));
                                    break;
                                }
                            }
                        }
                        _ = shutdown_rx.recv() => {
                            daemon_log("non-sub: shutdown signal received, sending Restarting");
                            let msg = DaemonMessage::Restarting {
                                new_sha: BUILD_SHA.to_string(),
                            };
                            if let Ok(json) = serde_json::to_string(&msg) {
                                let _ = writer.write_all(format!("{}\n", json).as_bytes()).await;
                            }
                            break;
                        }
                    }
                }
            }
            daemon_log("connection handler ended");
        });
    }

    // Final persist before exit
    {
        let state_guard = state.lock().await;
        let _ = state_guard.persist_to_disk();
    }

    Ok(())
}

/// Windows TCP server implementation with authentication.
#[cfg(windows)]
async fn run_windows_server(
    state: Arc<Mutex<DaemonState>>,
    shutdown_tx: broadcast::Sender<String>,
    events_tx: broadcast::Sender<SessionRecord>,
) -> Result<()> {
    use crate::session_daemon::protocol::PortFileContent;
    use rand::Rng;
    use tokio::net::TcpListener;

    // Bind to localhost only (not 0.0.0.0)
    let listener = TcpListener::bind("127.0.0.1:0").await
        .context("Failed to bind TCP socket")?;

    let local_addr = listener.local_addr()?;
    let port = local_addr.port();

    // Generate authentication token
    let mut rng = rand::thread_rng();
    let token_bytes: [u8; 32] = rng.gen();
    let token = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &token_bytes);

    // Write port file with token
    let port_path = planning_paths::sessiond_port_path()?;
    let port_content = PortFileContent { port, token: token.clone() };
    let port_json = serde_json::to_string(&port_content)?;
    std::fs::write(&port_path, &port_json)
        .context("Failed to write port file")?;

    // Security: Port file contains auth token. Current mitigations:
    // - File is in user's home directory (~/.planning-agent/)
    // - Token is randomly generated per daemon instance
    // - TCP is bound to localhost only (127.0.0.1)
    // Future hardening: Set restrictive ACLs via Windows security APIs

    eprintln!("[sessiond] Listening on 127.0.0.1:{}", port);

    // Spawn registry persistence task
    let persist_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(
            tokio::time::Duration::from_secs(REGISTRY_PERSIST_INTERVAL_SECS)
        );
        loop {
            interval.tick().await;
            let state_guard = persist_state.lock().await;
            if state_guard.shutting_down {
                break;
            }
            if let Err(e) = state_guard.persist_to_disk() {
                eprintln!("[sessiond] Warning: Failed to persist registry: {}", e);
            }
        }
    });

    // Spawn liveness update task
    let liveness_state = state.clone();
    let liveness_events_tx = events_tx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            let mut state_guard = liveness_state.lock().await;
            if state_guard.shutting_down {
                break;
            }
            // Update liveness and broadcast any changes
            let changed = state_guard.update_liveness_states_with_changes();
            for record in changed {
                let _ = liveness_events_tx.send(record);
            }
        }
    });

    // Accept connections
    loop {
        let (stream, _) = listener.accept().await?;

        // Check shutdown flag
        {
            let state_guard = state.lock().await;
            if state_guard.shutting_down {
                break;
            }
        }

        let conn_state = state.clone();
        let conn_token = token.clone();
        let conn_events_tx = events_tx.clone();
        let mut shutdown_rx = shutdown_tx.subscribe();

        tokio::spawn(async move {
            let (reader, writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut writer = writer;
            let mut subscribed = false;
            let mut events_rx: Option<broadcast::Receiver<SessionRecord>> = None;

            // First message must be authentication token
            let mut auth_line = String::new();
            match reader.read_line(&mut auth_line).await {
                Ok(0) => return, // EOF
                Ok(_) => {
                    if auth_line.trim() != conn_token {
                        eprintln!("[sessiond] Authentication failed");
                        return;
                    }
                }
                Err(_) => return,
            }

            loop {
                let mut line = String::new();

                if subscribed {
                    let events_receiver = events_rx.as_mut().unwrap();
                    tokio::select! {
                        result = reader.read_line(&mut line) => {
                            match result {
                                Ok(0) => break,
                                Ok(_) => {
                                    let (response, should_subscribe) = handle_message_with_broadcast(&line, &conn_state, &conn_events_tx).await;
                                    if should_subscribe && !subscribed {
                                        subscribed = true;
                                        events_rx = Some(conn_events_tx.subscribe());
                                    }
                                    if let Some(response) = response {
                                        let response_json = match serde_json::to_string(&response) {
                                            Ok(json) => json,
                                            Err(e) => {
                                                eprintln!("[sessiond] Failed to serialize response: {}", e);
                                                continue;
                                            }
                                        };
                                        if writer.write_all(format!("{}\n", response_json).as_bytes()).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                        event = events_receiver.recv() => {
                            if let Ok(record) = event {
                                let msg = DaemonMessage::SessionChanged(record);
                                if let Ok(json) = serde_json::to_string(&msg) {
                                    if writer.write_all(format!("{}\n", json).as_bytes()).await.is_err() {
                                        break;
                                    }
                                }
                            }
                        }
                        _ = shutdown_rx.recv() => {
                            let msg = DaemonMessage::Restarting {
                                new_sha: BUILD_SHA.to_string(),
                            };
                            if let Ok(json) = serde_json::to_string(&msg) {
                                let _ = writer.write_all(format!("{}\n", json).as_bytes()).await;
                            }
                            break;
                        }
                    }
                } else {
                    tokio::select! {
                        result = reader.read_line(&mut line) => {
                            match result {
                                Ok(0) => break,
                                Ok(_) => {
                                    let (response, should_subscribe) = handle_message_with_broadcast(&line, &conn_state, &conn_events_tx).await;
                                    if should_subscribe {
                                        subscribed = true;
                                        events_rx = Some(conn_events_tx.subscribe());
                                    }
                                    if let Some(response) = response {
                                        let response_json = match serde_json::to_string(&response) {
                                            Ok(json) => json,
                                            Err(e) => {
                                                eprintln!("[sessiond] Failed to serialize response: {}", e);
                                                continue;
                                            }
                                        };
                                        if writer.write_all(format!("{}\n", response_json).as_bytes()).await.is_err() {
                                            break;
                                        }

                                        if matches!(response, DaemonMessage::Ack { .. }) {
                                            let state_guard = conn_state.lock().await;
                                            if state_guard.shutting_down {
                                                break;
                                            }
                                        }
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                        _ = shutdown_rx.recv() => {
                            let msg = DaemonMessage::Restarting {
                                new_sha: BUILD_SHA.to_string(),
                            };
                            if let Ok(json) = serde_json::to_string(&msg) {
                                let _ = writer.write_all(format!("{}\n", json).as_bytes()).await;
                            }
                            break;
                        }
                    }
                }
            }
        });
    }

    // Cleanup port file
    let _ = std::fs::remove_file(&port_path);

    // Final persist before exit
    {
        let state_guard = state.lock().await;
        let _ = state_guard.persist_to_disk();
    }

    Ok(())
}

/// Handle a single client message and return the response.
/// Returns (Option<response>, should_subscribe) tuple.
pub(crate) async fn handle_message_with_broadcast(
    line: &str,
    state: &Arc<Mutex<DaemonState>>,
    events_tx: &broadcast::Sender<SessionRecord>,
) -> (Option<DaemonMessage>, bool) {
    let message: ClientMessage = match serde_json::from_str(line.trim()) {
        Ok(msg) => msg,
        Err(e) => {
            return (Some(DaemonMessage::Error(format!("Invalid message: {}", e))), false);
        }
    };

    let mut state_guard = state.lock().await;

    match message {
        ClientMessage::Register(record) => {
            let session_id = record.workflow_session_id.clone();

            // Check for existing session with same ID but different PID (stale entry)
            if let Some(existing) = state_guard.sessions.get(&session_id) {
                if existing.pid != record.pid {
                    // Different PID means the old process died, allow override
                    eprintln!(
                        "[sessiond] Replacing stale session {} (old PID: {}, new PID: {})",
                        session_id, existing.pid, record.pid
                    );
                }
            }

            state_guard.sessions.insert(session_id, record.clone());
            // Broadcast to subscribers
            let _ = events_tx.send(record);
            (Some(DaemonMessage::Ack {
                build_sha: BUILD_SHA.to_string(),
            }), false)
        }

        ClientMessage::Update(record) => {
            let session_id = record.workflow_session_id.clone();
            let updated_record = if let Some(existing) = state_guard.sessions.get_mut(&session_id) {
                // Only update if PID matches (or this is a force update)
                if existing.pid == record.pid {
                    existing.update_state(record.phase, record.iteration, record.workflow_status);
                }
                existing.clone()
            } else {
                // Session not found, register it
                state_guard.sessions.insert(session_id, record.clone());
                record
            };
            // Broadcast to subscribers
            let _ = events_tx.send(updated_record);
            (Some(DaemonMessage::Ack {
                build_sha: BUILD_SHA.to_string(),
            }), false)
        }

        ClientMessage::Heartbeat { session_id } => {
            if let Some(record) = state_guard.sessions.get_mut(&session_id) {
                record.update_heartbeat();
                // Broadcast to subscribers
                let _ = events_tx.send(record.clone());
            }
            (Some(DaemonMessage::Ack {
                build_sha: BUILD_SHA.to_string(),
            }), false)
        }

        ClientMessage::List => {
            // Update liveness states before returning
            state_guard.update_liveness_states();
            let sessions: Vec<SessionRecord> = state_guard.sessions.values().cloned().collect();
            (Some(DaemonMessage::Sessions(sessions)), false)
        }

        ClientMessage::Shutdown => {
            state_guard.shutting_down = true;
            // Persist before shutdown
            let _ = state_guard.persist_to_disk();
            (Some(DaemonMessage::Ack {
                build_sha: BUILD_SHA.to_string(),
            }), false)
        }

        ClientMessage::ForceStop { session_id } => {
            if let Some(record) = state_guard.sessions.get_mut(&session_id) {
                record.liveness = LivenessState::Stopped;
                // Broadcast to subscribers
                let _ = events_tx.send(record.clone());
                (Some(DaemonMessage::Ack {
                    build_sha: BUILD_SHA.to_string(),
                }), false)
            } else {
                (Some(DaemonMessage::Error(format!("Session not found: {}", session_id))), false)
            }
        }

        ClientMessage::Subscribe => {
            (Some(DaemonMessage::Subscribed), true)
        }

        ClientMessage::Unsubscribe => {
            (Some(DaemonMessage::Unsubscribed), false)
        }
    }
}
