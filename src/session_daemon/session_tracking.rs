//! Session tracking for planning-agent workflows.
//!
//! This module provides a high-level interface for registering and updating
//! sessions with the session daemon, including background heartbeat tasks.

use crate::session_daemon::{LivenessState, RpcClient, SessionRecord, WorkflowEventEnvelope};
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Heartbeat interval in milliseconds.
/// Changed from 5 seconds to 500ms for faster liveness detection.
const HEARTBEAT_INTERVAL_MS: u64 = 500;

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
    /// The RPC daemon client
    client: Arc<Mutex<RpcClient>>,
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
    pub async fn new(no_daemon: bool) -> Self {
        if no_daemon {
            return Self {
                client: Arc::new(Mutex::new(RpcClient::new(true).await)),
                active_sessions: Arc::new(Mutex::new(SessionMap::new())),
                _heartbeat_stop_tx: None,
                disabled: true,
            };
        }

        let client = Arc::new(Mutex::new(RpcClient::new(false).await));
        let active_sessions: Arc<Mutex<SessionMap>> = Arc::new(Mutex::new(SessionMap::new()));

        // Start heartbeat task
        let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);
        let heartbeat_client = client.clone();
        let heartbeat_sessions = active_sessions.clone();

        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(tokio::time::Duration::from_millis(HEARTBEAT_INTERVAL_MS));

            // State for tracking failures and reconnection
            let mut consecutive_failures: u32 = 0;
            let mut last_error_log = std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(ERROR_LOG_INTERVAL_SECS))
                .unwrap_or_else(std::time::Instant::now);
            let mut backoff_secs = HEARTBEAT_INTERVAL_MS;
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
                                        backoff_secs = HEARTBEAT_INTERVAL_MS;
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
                                backoff_secs = HEARTBEAT_INTERVAL_MS;
                                in_reconnect_mode = false;
                                interval = tokio::time::interval(
                                    tokio::time::Duration::from_millis(HEARTBEAT_INTERVAL_MS)
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
    #[cfg(test)]
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
        session_dir: PathBuf,
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
            session_dir,
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

    /// Forwards a workflow event to the daemon for broadcasting to subscribers.
    pub async fn workflow_event(
        &self,
        session_id: &str,
        event: WorkflowEventEnvelope,
    ) -> Result<()> {
        if self.disabled {
            return Ok(());
        }

        let client = self.client.lock().await;
        client.workflow_event(session_id, event).await
    }

    /// Lists all sessions from the daemon.
    #[cfg(test)]
    pub async fn list(&self) -> Result<Vec<SessionRecord>> {
        if self.disabled {
            return Ok(Vec::new());
        }

        let client = self.client.lock().await;
        client.list().await
    }

    /// Force-stops a session (marks as stopped immediately).
    #[cfg(test)]
    pub async fn force_stop(&self, session_id: &str) -> Result<()> {
        if self.disabled {
            return Ok(());
        }

        // Remove from active sessions so heartbeat task stops sending heartbeats
        {
            let mut sessions = self.active_sessions.lock().await;
            sessions.remove(session_id);
        }

        let client = self.client.lock().await;
        client.force_stop(session_id).await?;
        Ok(())
    }

    /// Attempts to reconnect to the daemon.
    #[cfg(test)]
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
}

#[cfg(test)]
#[path = "tests/session_tracking_tests.rs"]
mod tests;
