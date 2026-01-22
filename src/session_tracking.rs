//! Session tracking for planning-agent workflows.
//!
//! This module provides a high-level interface for registering and updating
//! sessions with the session daemon, including background heartbeat tasks.

#![allow(dead_code)]

use crate::session_daemon::{LivenessState, SessionDaemonClient, SessionRecord};
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Heartbeat interval in seconds.
const HEARTBEAT_INTERVAL_SECS: u64 = 5;

/// Number of consecutive heartbeat failures before attempting reconnection.
const RECONNECT_THRESHOLD: u32 = 2;

/// Maximum backoff interval for reconnection attempts (seconds).
const MAX_BACKOFF_SECS: u64 = 60;

/// How often to log repeated errors (seconds).
const ERROR_LOG_INTERVAL_SECS: u64 = 60;

/// Information about an active session.
struct SessionInfo {
    pub record: SessionRecord,
}

type SessionMap = HashMap<String, SessionInfo>;

/// Session tracker for a planning-agent process.
///
/// Manages registration, updates, and heartbeats for all sessions in this process.
pub struct SessionTracker {
    /// The daemon client
    client: Arc<Mutex<SessionDaemonClient>>,
    /// Active sessions in this process
    active_sessions: Arc<Mutex<SessionMap>>,
    /// Channel to stop the heartbeat task
    _heartbeat_stop_tx: Option<mpsc::Sender<()>>,
    /// Whether session tracking is disabled
    disabled: bool,
}

impl SessionTracker {
    /// Creates a new session tracker.
    ///
    /// If `no_daemon` is true, creates a disabled tracker that does nothing.
    pub fn new(no_daemon: bool) -> Self {
        if no_daemon {
            return Self {
                client: Arc::new(Mutex::new(SessionDaemonClient::new(true))),
                active_sessions: Arc::new(Mutex::new(SessionMap::new())),
                _heartbeat_stop_tx: None,
                disabled: true,
            };
        }

        let client = Arc::new(Mutex::new(SessionDaemonClient::new(false)));
        let active_sessions: Arc<Mutex<SessionMap>> = Arc::new(Mutex::new(SessionMap::new()));

        // Start heartbeat task
        let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);
        let heartbeat_client = client.clone();
        let heartbeat_sessions = active_sessions.clone();

        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(tokio::time::Duration::from_secs(HEARTBEAT_INTERVAL_SECS));

            // State for tracking failures and reconnection
            let mut consecutive_failures: u32 = 0;
            let mut last_error_log = std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(ERROR_LOG_INTERVAL_SECS))
                .unwrap_or_else(std::time::Instant::now);
            let mut backoff_secs = HEARTBEAT_INTERVAL_SECS;
            let mut in_reconnect_mode = false;

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let sessions = heartbeat_sessions.lock().await;

                        // Skip heartbeat if no active sessions
                        if sessions.is_empty() {
                            consecutive_failures = 0;
                            continue;
                        }

                        // Try heartbeat for each session
                        let mut any_failed = false;

                        {
                            let client = heartbeat_client.lock().await;
                            for session_id in sessions.keys() {
                                if client.heartbeat(session_id).await.is_err() {
                                    any_failed = true;
                                }
                            }
                        }

                        if any_failed {
                            consecutive_failures += 1;

                            // Track last error time for backoff (no console logging - uses UI)
                            let now = std::time::Instant::now();
                            let should_update_log_time = consecutive_failures == 1
                                || now.duration_since(last_error_log).as_secs() >= ERROR_LOG_INTERVAL_SECS;
                            if should_update_log_time {
                                last_error_log = now;
                            }

                            // Attempt reconnection after threshold failures
                            if consecutive_failures >= RECONNECT_THRESHOLD {
                                if !in_reconnect_mode {
                                    in_reconnect_mode = true;
                                }

                                // Try to reconnect (will spawn daemon if needed)
                                let mut client = heartbeat_client.lock().await;
                                match client.reconnect().await {
                                    Ok(()) => {
                                        consecutive_failures = 0;
                                        backoff_secs = HEARTBEAT_INTERVAL_SECS;
                                        in_reconnect_mode = false;

                                        // Re-register sessions after reconnect
                                        for info in sessions.values() {
                                            let _ = client.register(info.record.clone()).await;
                                        }
                                    }
                                    Err(_) => {
                                        // Increase backoff interval (exponential backoff)
                                        backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
                                        interval = tokio::time::interval(
                                            tokio::time::Duration::from_secs(backoff_secs)
                                        );
                                    }
                                }
                            }
                        } else {
                            // Success - reset failure state
                            if consecutive_failures > 0 || in_reconnect_mode {
                                consecutive_failures = 0;
                                backoff_secs = HEARTBEAT_INTERVAL_SECS;
                                in_reconnect_mode = false;
                                interval = tokio::time::interval(
                                    tokio::time::Duration::from_secs(HEARTBEAT_INTERVAL_SECS)
                                );
                            }
                        }
                    }
                    _ = stop_rx.recv() => {
                        break;
                    }
                }
            }
        });

        Self {
            client,
            active_sessions,
            _heartbeat_stop_tx: Some(stop_tx),
            disabled: false,
        }
    }

    /// Returns true if session tracking is enabled and connected.
    pub async fn is_connected(&self) -> bool {
        if self.disabled {
            return false;
        }
        let client = self.client.lock().await;
        client.is_connected()
    }

    /// Registers a new session with the daemon.
    #[allow(clippy::too_many_arguments)]
    pub async fn register(
        &self,
        workflow_session_id: String,
        feature_name: String,
        working_dir: PathBuf,
        state_path: PathBuf,
        phase: String,
        iteration: u32,
        workflow_status: String,
    ) -> Result<()> {
        if self.disabled {
            return Ok(());
        }

        let pid = std::process::id();
        let record = SessionRecord::new(
            workflow_session_id.clone(),
            feature_name,
            working_dir,
            state_path,
            phase,
            iteration,
            workflow_status,
            pid,
        );

        // Register with daemon
        {
            let client = self.client.lock().await;
            client.register(record.clone()).await?;
        }

        // Track locally
        {
            let mut sessions = self.active_sessions.lock().await;
            sessions.insert(workflow_session_id, SessionInfo { record });
        }

        Ok(())
    }

    /// Updates a session's state in the daemon.
    pub async fn update(
        &self,
        workflow_session_id: &str,
        phase: String,
        iteration: u32,
        workflow_status: String,
    ) -> Result<()> {
        if self.disabled {
            return Ok(());
        }

        let mut sessions = self.active_sessions.lock().await;

        if let Some(info) = sessions.get_mut(workflow_session_id) {
            info.record.update_state(phase, iteration, workflow_status);

            let client = self.client.lock().await;
            client.update(info.record.clone()).await?;
        }

        Ok(())
    }

    /// Marks a session as stopped in the daemon.
    pub async fn mark_stopped(&self, workflow_session_id: &str) -> Result<()> {
        if self.disabled {
            return Ok(());
        }

        let mut sessions = self.active_sessions.lock().await;

        if let Some(info) = sessions.get_mut(workflow_session_id) {
            info.record.liveness = LivenessState::Stopped;

            let client = self.client.lock().await;
            client.update(info.record.clone()).await?;
        }

        // Remove from active sessions
        sessions.remove(workflow_session_id);

        Ok(())
    }

    /// Lists all sessions from the daemon.
    pub async fn list(&self) -> Result<Vec<SessionRecord>> {
        if self.disabled {
            return Ok(Vec::new());
        }

        let client = self.client.lock().await;
        client.list().await
    }

    /// Force-stops a session (marks as stopped immediately).
    pub async fn force_stop(&self, session_id: &str) -> Result<()> {
        if self.disabled {
            return Ok(());
        }

        let client = self.client.lock().await;
        client.force_stop(session_id).await?;
        Ok(())
    }

    /// Requests daemon shutdown (for updates).
    pub async fn shutdown_daemon(&self) -> Result<()> {
        if self.disabled {
            return Ok(());
        }

        let client = self.client.lock().await;
        client.shutdown().await?;
        Ok(())
    }

    /// Attempts to reconnect to the daemon.
    pub async fn reconnect(&self) -> Result<()> {
        if self.disabled {
            return Ok(());
        }

        let mut client = self.client.lock().await;
        client.reconnect().await?;

        // Re-register all active sessions
        let sessions = self.active_sessions.lock().await;
        for info in sessions.values() {
            client.register(info.record.clone()).await?;
        }

        Ok(())
    }

    /// Returns whether tracking is disabled.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tracker_disabled() {
        let tracker = SessionTracker::new(true);
        assert!(tracker.is_disabled());
        assert!(!tracker.is_connected().await);

        // Operations should be no-ops
        let result = tracker
            .register(
                "test-id".to_string(),
                "test-feature".to_string(),
                PathBuf::from("/test"),
                PathBuf::from("/test/state.json"),
                "Planning".to_string(),
                1,
                "Planning".to_string(),
            )
            .await;
        assert!(result.is_ok());
    }
}

/// Integration tests for SessionTracker with real daemon communication.
///
/// Each test uses unique session IDs (UUIDs) to avoid conflicts with:
/// - Other tests running in parallel
/// - Other planning-agent instances on the same system
/// - Leftover sessions from previous test runs
#[cfg(test)]
#[cfg(unix)]
mod integration_tests {
    use super::*;
    use crate::session_daemon::LivenessState;
    use std::time::Duration;
    use uuid::Uuid;

    /// Generate a unique session ID for this test run.
    fn unique_session_id(prefix: &str) -> String {
        format!("{}-{}", prefix, Uuid::new_v4())
    }

    /// Helper to clean up a specific session (marks it as stopped).
    /// Does NOT shut down the daemon - other tests may be using it.
    async fn cleanup_session(tracker: &SessionTracker, session_id: &str) {
        let _ = tracker.force_stop(session_id).await;
    }

    /// Wait for a session to reach the expected liveness state.
    /// This handles race conditions where state updates take time to propagate.
    async fn wait_for_liveness(
        tracker: &SessionTracker,
        session_id: &str,
        expected: LivenessState,
        timeout: Duration,
    ) -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + timeout;

        while tokio::time::Instant::now() < deadline {
            if let Ok(sessions) = tracker.list().await {
                if let Some(session) = sessions
                    .iter()
                    .find(|s| s.workflow_session_id == session_id)
                {
                    if session.liveness == expected {
                        return Ok(());
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        // Get final state for error message
        if let Ok(sessions) = tracker.list().await {
            if let Some(session) = sessions
                .iter()
                .find(|s| s.workflow_session_id == session_id)
            {
                return Err(format!(
                    "Session {} liveness is {:?}, expected {:?}",
                    session_id, session.liveness, expected
                ));
            }
        }
        Err(format!("Session {} not found", session_id))
    }

    #[tokio::test]
    async fn test_tracker_register_and_list() {
        let tracker = SessionTracker::new(false);
        tokio::time::sleep(Duration::from_millis(500)).await;

        if !tracker.is_connected().await {
            println!("Skipping test - daemon not available");
            return;
        }

        let session_id = unique_session_id("tracker-register");

        // Register
        let result = tracker
            .register(
                session_id.clone(),
                "test-feature".to_string(),
                PathBuf::from("/tmp/test"),
                PathBuf::from("/tmp/test/state.json"),
                "Planning".to_string(),
                1,
                "Planning".to_string(),
            )
            .await;
        assert!(result.is_ok(), "Register failed: {:?}", result.err());

        // List should include our session
        let sessions = tracker.list().await;
        assert!(sessions.is_ok(), "List failed: {:?}", sessions.err());

        let sessions = sessions.unwrap();
        let found = sessions.iter().any(|s| s.workflow_session_id == session_id);
        assert!(found, "Session not found in list");

        cleanup_session(&tracker, &session_id).await;
    }

    #[tokio::test]
    async fn test_tracker_update() {
        let tracker = SessionTracker::new(false);
        tokio::time::sleep(Duration::from_millis(500)).await;

        if !tracker.is_connected().await {
            println!("Skipping test - daemon not available");
            return;
        }

        let session_id = unique_session_id("tracker-update");

        // Register
        tracker
            .register(
                session_id.clone(),
                "test-feature".to_string(),
                PathBuf::from("/tmp/test"),
                PathBuf::from("/tmp/test/state.json"),
                "Planning".to_string(),
                1,
                "Planning".to_string(),
            )
            .await
            .expect("Register failed");

        // Update phase
        let result = tracker
            .update(
                &session_id,
                "Reviewing".to_string(),
                2,
                "Reviewing".to_string(),
            )
            .await;
        assert!(result.is_ok(), "Update failed: {:?}", result.err());

        // Verify via list
        let sessions = tracker.list().await.expect("List failed");
        let session = sessions
            .iter()
            .find(|s| s.workflow_session_id == session_id)
            .expect("Session not found");

        assert_eq!(session.phase, "Reviewing");
        assert_eq!(session.iteration, 2);

        cleanup_session(&tracker, &session_id).await;
    }

    #[tokio::test]
    async fn test_tracker_mark_stopped() {
        let tracker = SessionTracker::new(false);
        tokio::time::sleep(Duration::from_millis(500)).await;

        if !tracker.is_connected().await {
            println!("Skipping test - daemon not available");
            return;
        }

        let session_id = unique_session_id("tracker-mark-stopped");

        // Register
        tracker
            .register(
                session_id.clone(),
                "test-feature".to_string(),
                PathBuf::from("/tmp/test"),
                PathBuf::from("/tmp/test/state.json"),
                "Planning".to_string(),
                1,
                "Planning".to_string(),
            )
            .await
            .expect("Register failed");

        // Mark stopped
        let result = tracker.mark_stopped(&session_id).await;
        assert!(result.is_ok(), "Mark stopped failed: {:?}", result.err());

        // Wait for session to be marked as Stopped (handles race conditions)
        wait_for_liveness(
            &tracker,
            &session_id,
            LivenessState::Stopped,
            Duration::from_secs(2),
        )
        .await
        .expect("Session should be marked as Stopped");
        // Already stopped, no additional cleanup needed
    }

    #[tokio::test]
    async fn test_tracker_force_stop() {
        let tracker = SessionTracker::new(false);
        tokio::time::sleep(Duration::from_millis(500)).await;

        if !tracker.is_connected().await {
            println!("Skipping test - daemon not available");
            return;
        }

        let session_id = unique_session_id("tracker-force-stop");

        // Register
        tracker
            .register(
                session_id.clone(),
                "test-feature".to_string(),
                PathBuf::from("/tmp/test"),
                PathBuf::from("/tmp/test/state.json"),
                "Planning".to_string(),
                1,
                "Planning".to_string(),
            )
            .await
            .expect("Register failed");

        // Force stop
        let result = tracker.force_stop(&session_id).await;
        assert!(result.is_ok(), "Force stop failed: {:?}", result.err());

        // Wait for session to be Stopped (handles race conditions)
        wait_for_liveness(
            &tracker,
            &session_id,
            LivenessState::Stopped,
            Duration::from_secs(2),
        )
        .await
        .expect("Session should be Stopped");
        // Already stopped, no additional cleanup needed
    }

    #[tokio::test]
    async fn test_tracker_full_workflow_lifecycle() {
        // Simulates a complete workflow lifecycle through SessionTracker
        let tracker = SessionTracker::new(false);
        tokio::time::sleep(Duration::from_millis(500)).await;

        if !tracker.is_connected().await {
            println!("Skipping test - daemon not available");
            return;
        }

        let session_id = unique_session_id("tracker-lifecycle");

        // 1. Register at workflow start
        tracker
            .register(
                session_id.clone(),
                "lifecycle-feature".to_string(),
                PathBuf::from("/tmp/lifecycle-test"),
                PathBuf::from("/tmp/lifecycle-test/state.json"),
                "Planning".to_string(),
                1,
                "Planning".to_string(),
            )
            .await
            .expect("Register failed");

        // Verify initial state
        let sessions = tracker.list().await.expect("List failed");
        let session = sessions
            .iter()
            .find(|s| s.workflow_session_id == session_id)
            .expect("Session not found after register");
        assert_eq!(session.phase, "Planning");
        assert_eq!(session.liveness, LivenessState::Running);

        // 2. Update: Planning -> Reviewing
        tracker
            .update(
                &session_id,
                "Reviewing".to_string(),
                1,
                "Reviewing".to_string(),
            )
            .await
            .expect("Update to Reviewing failed");

        let sessions = tracker.list().await.expect("List failed");
        let session = sessions
            .iter()
            .find(|s| s.workflow_session_id == session_id)
            .expect("Session not found");
        assert_eq!(session.phase, "Reviewing");

        // 3. Update: Reviewing -> Revising
        tracker
            .update(
                &session_id,
                "Revising".to_string(),
                2,
                "Revising".to_string(),
            )
            .await
            .expect("Update to Revising failed");

        let sessions = tracker.list().await.expect("List failed");
        let session = sessions
            .iter()
            .find(|s| s.workflow_session_id == session_id)
            .expect("Session not found");
        assert_eq!(session.phase, "Revising");
        assert_eq!(session.iteration, 2);

        // 4. Update: Revising -> Complete
        tracker
            .update(
                &session_id,
                "Complete".to_string(),
                2,
                "Complete".to_string(),
            )
            .await
            .expect("Update to Complete failed");

        // 5. Mark stopped at workflow end - this also serves as cleanup
        tracker
            .mark_stopped(&session_id)
            .await
            .expect("Mark stopped failed");

        // Wait for final state (handles race conditions)
        wait_for_liveness(
            &tracker,
            &session_id,
            LivenessState::Stopped,
            Duration::from_secs(2),
        )
        .await
        .expect("Session should be Stopped");

        // Verify final phase
        let sessions = tracker.list().await.expect("List failed");
        let session = sessions
            .iter()
            .find(|s| s.workflow_session_id == session_id)
            .expect("Session not found");
        assert_eq!(session.phase, "Complete");
        // Already stopped, no additional cleanup needed
    }

    #[tokio::test]
    async fn test_tracker_reconnect() {
        let tracker = SessionTracker::new(false);
        tokio::time::sleep(Duration::from_millis(500)).await;

        if !tracker.is_connected().await {
            println!("Skipping test - daemon not available");
            return;
        }

        let session_id = unique_session_id("tracker-reconnect");

        // Register
        tracker
            .register(
                session_id.clone(),
                "test-feature".to_string(),
                PathBuf::from("/tmp/test"),
                PathBuf::from("/tmp/test/state.json"),
                "Planning".to_string(),
                1,
                "Planning".to_string(),
            )
            .await
            .expect("Register failed");

        // Reconnect
        let result = tracker.reconnect().await;
        assert!(result.is_ok(), "Reconnect failed: {:?}", result.err());

        // Session should still be there (re-registered)
        let sessions = tracker.list().await.expect("List failed");
        let found = sessions.iter().any(|s| s.workflow_session_id == session_id);
        assert!(found, "Session not found after reconnect");

        cleanup_session(&tracker, &session_id).await;
    }
}
