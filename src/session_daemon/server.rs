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

/// Default timeout for marking sessions as unresponsive (seconds).
const UNRESPONSIVE_TIMEOUT_SECS: u64 = 25;

/// Default timeout for marking sessions as stopped (seconds).
/// Can be overridden via PLANNING_SESSIOND_STALE_SECS environment variable.
const DEFAULT_STALE_TIMEOUT_SECS: u64 = 60;

/// Registry persistence interval (seconds).
const REGISTRY_PERSIST_INTERVAL_SECS: u64 = 30;

/// Shared daemon state.
struct DaemonState {
    /// Session registry keyed by workflow_session_id
    sessions: HashMap<String, SessionRecord>,
    /// Flag indicating daemon is shutting down
    shutting_down: bool,
}

impl DaemonState {
    fn new() -> Self {
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
        let now = chrono::Utc::now();
        let stale_timeout = Self::stale_timeout_secs();

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

            if elapsed_secs > stale_timeout {
                record.liveness = LivenessState::Stopped;
            } else if elapsed_secs > UNRESPONSIVE_TIMEOUT_SECS {
                record.liveness = LivenessState::Unresponsive;
            }
        }
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

    // Start the listener
    #[cfg(unix)]
    let result = run_unix_server(state.clone(), shutdown_tx.clone()).await;

    #[cfg(windows)]
    let result = run_windows_server(state.clone(), shutdown_tx.clone()).await;

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
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            let mut state_guard = liveness_state.lock().await;
            if state_guard.shutting_down {
                break;
            }
            state_guard.update_liveness_states();
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
        let mut shutdown_rx = shutdown_tx.subscribe();

        tokio::spawn(async move {
            let (reader, writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut writer = writer;

            loop {
                let mut line = String::new();

                tokio::select! {
                    result = reader.read_line(&mut line) => {
                        match result {
                            Ok(0) => break, // EOF
                            Ok(_) => {
                                let response = handle_message(&line, &conn_state).await;
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

                                // Check if we should shut down
                                if matches!(response, DaemonMessage::Ack { .. }) {
                                    let state_guard = conn_state.lock().await;
                                    if state_guard.shutting_down {
                                        break;
                                    }
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        // Send Restarting message
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

    // TODO: Set restrictive ACLs on port file (Windows-specific)

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
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            let mut state_guard = liveness_state.lock().await;
            if state_guard.shutting_down {
                break;
            }
            state_guard.update_liveness_states();
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
        let mut shutdown_rx = shutdown_tx.subscribe();

        tokio::spawn(async move {
            let (reader, writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut writer = writer;

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

                tokio::select! {
                    result = reader.read_line(&mut line) => {
                        match result {
                            Ok(0) => break, // EOF
                            Ok(_) => {
                                let response = handle_message(&line, &conn_state).await;
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

                                // Check if we should shut down
                                if matches!(response, DaemonMessage::Ack { .. }) {
                                    let state_guard = conn_state.lock().await;
                                    if state_guard.shutting_down {
                                        break;
                                    }
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        // Send Restarting message
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
async fn handle_message(line: &str, state: &Arc<Mutex<DaemonState>>) -> DaemonMessage {
    let message: ClientMessage = match serde_json::from_str(line.trim()) {
        Ok(msg) => msg,
        Err(e) => {
            return DaemonMessage::Error(format!("Invalid message: {}", e));
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

            state_guard.sessions.insert(session_id, record);
            DaemonMessage::Ack {
                build_sha: BUILD_SHA.to_string(),
            }
        }

        ClientMessage::Update(record) => {
            let session_id = record.workflow_session_id.clone();
            if let Some(existing) = state_guard.sessions.get_mut(&session_id) {
                // Only update if PID matches (or this is a force update)
                if existing.pid == record.pid {
                    existing.update_state(record.phase, record.iteration, record.workflow_status);
                }
            } else {
                // Session not found, register it
                state_guard.sessions.insert(session_id, record);
            }
            DaemonMessage::Ack {
                build_sha: BUILD_SHA.to_string(),
            }
        }

        ClientMessage::Heartbeat { session_id } => {
            if let Some(record) = state_guard.sessions.get_mut(&session_id) {
                record.update_heartbeat();
            }
            DaemonMessage::Ack {
                build_sha: BUILD_SHA.to_string(),
            }
        }

        ClientMessage::List => {
            // Update liveness states before returning
            state_guard.update_liveness_states();
            let sessions: Vec<SessionRecord> = state_guard.sessions.values().cloned().collect();
            DaemonMessage::Sessions(sessions)
        }

        ClientMessage::Shutdown => {
            state_guard.shutting_down = true;
            // Persist before shutdown
            let _ = state_guard.persist_to_disk();
            DaemonMessage::Ack {
                build_sha: BUILD_SHA.to_string(),
            }
        }

        ClientMessage::ForceStop { session_id } => {
            if let Some(record) = state_guard.sessions.get_mut(&session_id) {
                record.liveness = LivenessState::Stopped;
                DaemonMessage::Ack {
                    build_sha: BUILD_SHA.to_string(),
                }
            } else {
                DaemonMessage::Error(format!("Session not found: {}", session_id))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn create_test_record(id: &str, pid: u32) -> SessionRecord {
        SessionRecord::new(
            id.to_string(),
            "test-feature".to_string(),
            PathBuf::from("/test"),
            PathBuf::from("/test/state.json"),
            "Planning".to_string(),
            1,
            "Planning".to_string(),
            pid,
        )
    }

    #[tokio::test]
    async fn test_daemon_state_register() {
        let state = Arc::new(Mutex::new(DaemonState::new()));

        let record = create_test_record("session-1", 1000);
        let msg = serde_json::to_string(&ClientMessage::Register(record)).unwrap();

        let response = handle_message(&msg, &state).await;
        assert!(matches!(response, DaemonMessage::Ack { .. }));

        let state_guard = state.lock().await;
        assert!(state_guard.sessions.contains_key("session-1"));
    }

    #[tokio::test]
    async fn test_daemon_state_heartbeat() {
        let state = Arc::new(Mutex::new(DaemonState::new()));

        // Register first
        let record = create_test_record("session-1", 1000);
        let msg = serde_json::to_string(&ClientMessage::Register(record)).unwrap();
        handle_message(&msg, &state).await;

        // Send heartbeat
        let heartbeat_msg = serde_json::to_string(&ClientMessage::Heartbeat {
            session_id: "session-1".to_string(),
        })
        .unwrap();
        let response = handle_message(&heartbeat_msg, &state).await;
        assert!(matches!(response, DaemonMessage::Ack { .. }));

        let state_guard = state.lock().await;
        let session = state_guard.sessions.get("session-1").unwrap();
        assert_eq!(session.liveness, LivenessState::Running);
    }

    #[tokio::test]
    async fn test_daemon_state_list() {
        let state = Arc::new(Mutex::new(DaemonState::new()));

        // Register two sessions
        let record1 = create_test_record("session-1", 1000);
        let record2 = create_test_record("session-2", 2000);

        let msg1 = serde_json::to_string(&ClientMessage::Register(record1)).unwrap();
        let msg2 = serde_json::to_string(&ClientMessage::Register(record2)).unwrap();
        handle_message(&msg1, &state).await;
        handle_message(&msg2, &state).await;

        // List
        let list_msg = serde_json::to_string(&ClientMessage::List).unwrap();
        let response = handle_message(&list_msg, &state).await;

        match response {
            DaemonMessage::Sessions(sessions) => {
                assert_eq!(sessions.len(), 2);
            }
            _ => panic!("Expected Sessions response"),
        }
    }

    #[tokio::test]
    async fn test_daemon_state_force_stop() {
        let state = Arc::new(Mutex::new(DaemonState::new()));

        // Register
        let record = create_test_record("session-1", 1000);
        let msg = serde_json::to_string(&ClientMessage::Register(record)).unwrap();
        handle_message(&msg, &state).await;

        // Force stop
        let stop_msg = serde_json::to_string(&ClientMessage::ForceStop {
            session_id: "session-1".to_string(),
        })
        .unwrap();
        let response = handle_message(&stop_msg, &state).await;
        assert!(matches!(response, DaemonMessage::Ack { .. }));

        let state_guard = state.lock().await;
        let session = state_guard.sessions.get("session-1").unwrap();
        assert_eq!(session.liveness, LivenessState::Stopped);
    }

    #[tokio::test]
    async fn test_daemon_state_replace_stale_session() {
        let state = Arc::new(Mutex::new(DaemonState::new()));

        // Register with PID 1000
        let record1 = create_test_record("session-1", 1000);
        let msg1 = serde_json::to_string(&ClientMessage::Register(record1)).unwrap();
        handle_message(&msg1, &state).await;

        // Register same session ID with different PID (simulating restart)
        let record2 = create_test_record("session-1", 2000);
        let msg2 = serde_json::to_string(&ClientMessage::Register(record2)).unwrap();
        handle_message(&msg2, &state).await;

        let state_guard = state.lock().await;
        let session = state_guard.sessions.get("session-1").unwrap();
        assert_eq!(session.pid, 2000);
    }

    #[tokio::test]
    async fn test_daemon_ack_includes_build_sha() {
        // This test verifies that the Ack response includes build_sha,
        // which is the mechanism used for version mismatch detection
        let state = Arc::new(Mutex::new(DaemonState::new()));

        // Send a heartbeat (simplest message that returns Ack)
        let heartbeat_msg = serde_json::to_string(&ClientMessage::Heartbeat {
            session_id: "nonexistent".to_string(),
        }).unwrap();

        let response = handle_message(&heartbeat_msg, &state).await;

        match response {
            DaemonMessage::Ack { build_sha } => {
                // build_sha should be non-empty (it's the BUILD_SHA constant)
                assert!(!build_sha.is_empty(), "build_sha should not be empty");
                // It should be the same as our BUILD_SHA
                assert_eq!(build_sha, BUILD_SHA, "build_sha should match BUILD_SHA constant");
            }
            other => panic!("Expected Ack response, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_daemon_shutdown_response() {
        let state = Arc::new(Mutex::new(DaemonState::new()));

        // Register a session first
        let record = create_test_record("session-shutdown-test", 1234);
        let register_msg = serde_json::to_string(&ClientMessage::Register(record)).unwrap();
        handle_message(&register_msg, &state).await;

        // Send shutdown
        let shutdown_msg = serde_json::to_string(&ClientMessage::Shutdown).unwrap();
        let response = handle_message(&shutdown_msg, &state).await;

        // Should get Ack with build_sha
        match response {
            DaemonMessage::Ack { build_sha } => {
                assert!(!build_sha.is_empty());
            }
            other => panic!("Expected Ack response, got: {:?}", other),
        }

        // State should have shutting_down flag set
        let state_guard = state.lock().await;
        assert!(state_guard.shutting_down, "shutting_down flag should be true after Shutdown");
    }

    #[tokio::test]
    async fn test_daemon_update_creates_missing_session() {
        // Tests that Update creates a session if it doesn't exist
        let state = Arc::new(Mutex::new(DaemonState::new()));

        // Don't register, just update directly
        let record = create_test_record("new-session-via-update", 5000);
        let update_msg = serde_json::to_string(&ClientMessage::Update(record)).unwrap();
        let response = handle_message(&update_msg, &state).await;

        assert!(matches!(response, DaemonMessage::Ack { .. }));

        // Session should exist now
        let state_guard = state.lock().await;
        assert!(state_guard.sessions.contains_key("new-session-via-update"));
    }

    #[tokio::test]
    async fn test_daemon_liveness_state_transitions() {
        let state = Arc::new(Mutex::new(DaemonState::new()));

        // Register a session
        let record = create_test_record("liveness-test", 1000);
        let msg = serde_json::to_string(&ClientMessage::Register(record)).unwrap();
        handle_message(&msg, &state).await;

        // Initially should be Running
        {
            let state_guard = state.lock().await;
            let session = state_guard.sessions.get("liveness-test").unwrap();
            assert_eq!(session.liveness, LivenessState::Running);
        }

        // Force stop should transition to Stopped
        let stop_msg = serde_json::to_string(&ClientMessage::ForceStop {
            session_id: "liveness-test".to_string(),
        }).unwrap();
        handle_message(&stop_msg, &state).await;

        {
            let state_guard = state.lock().await;
            let session = state_guard.sessions.get("liveness-test").unwrap();
            assert_eq!(session.liveness, LivenessState::Stopped);
        }

        // Heartbeat on stopped session should still work but keep it stopped
        // (per implementation, heartbeat resets to Running - this tests that behavior)
        let heartbeat_msg = serde_json::to_string(&ClientMessage::Heartbeat {
            session_id: "liveness-test".to_string(),
        }).unwrap();
        handle_message(&heartbeat_msg, &state).await;

        {
            let state_guard = state.lock().await;
            let session = state_guard.sessions.get("liveness-test").unwrap();
            // After heartbeat, session goes back to Running
            assert_eq!(session.liveness, LivenessState::Running);
        }
    }
}
