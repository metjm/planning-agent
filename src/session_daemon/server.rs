//! Session daemon state management.
//!
//! Contains the shared daemon state used by both old and new RPC implementations.

use crate::planning_paths;
use crate::session_daemon::protocol::{LivenessState, SessionRecord};
use anyhow::{Context, Result};
use std::collections::HashMap;

/// Check if a process with the given PID is still running.
/// Returns false if the process has exited.
pub(crate) fn process_exists(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // On Unix, kill with signal 0 checks process existence without sending a signal.
        // Returns 0 if process exists, -1 with ESRCH if it doesn't.
        // Using nix::libc for consistency with existing code (see rpc_client.rs:369).
        unsafe { nix::libc::kill(pid as nix::libc::pid_t, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        // On non-Unix platforms, assume process exists (fall back to timeout-based detection).
        // Windows support can be added later if needed.
        // Silence unused variable warning - pid is used on Unix.
        let _ = pid;
        true
    }
}

/// Default timeout for marking sessions as unresponsive (seconds).
/// Changed from 25 seconds to 3 seconds for faster detection.
/// Can be overridden via PLANNING_SESSIOND_UNRESPONSIVE_SECS environment variable.
const DEFAULT_UNRESPONSIVE_TIMEOUT_SECS: u64 = 3;

/// Default timeout for marking sessions as stopped (seconds).
/// Changed from 60 seconds to 10 seconds.
/// Can be overridden via PLANNING_SESSIOND_STALE_SECS environment variable.
const DEFAULT_STALE_TIMEOUT_SECS: u64 = 10;

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
    pub(crate) fn load_from_disk(&mut self) -> Result<()> {
        let registry_path = planning_paths::sessiond_registry_path()?;
        if registry_path.exists() {
            let content =
                std::fs::read_to_string(&registry_path).context("Failed to read registry file")?;
            let records: Vec<SessionRecord> =
                serde_json::from_str(&content).context("Failed to parse registry file")?;

            // Load records but mark them as stopped (they're from a previous daemon instance)
            for mut record in records {
                record.liveness = LivenessState::Stopped;
                self.sessions
                    .insert(record.workflow_session_id.clone(), record);
            }
        }
        Ok(())
    }

    /// Persist sessions to disk for recovery.
    pub(crate) fn persist_to_disk(&self) -> Result<()> {
        let registry_path = planning_paths::sessiond_registry_path()?;
        let records: Vec<&SessionRecord> = self.sessions.values().collect();
        let content =
            serde_json::to_string_pretty(&records).context("Failed to serialize registry")?;
        std::fs::write(&registry_path, content).context("Failed to write registry file")?;
        Ok(())
    }

    /// Get unresponsive timeout from environment or default.
    fn unresponsive_timeout_secs() -> u64 {
        std::env::var("PLANNING_SESSIOND_UNRESPONSIVE_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_UNRESPONSIVE_TIMEOUT_SECS)
    }

    /// Get stale timeout from environment or default.
    fn stale_timeout_secs() -> u64 {
        std::env::var("PLANNING_SESSIOND_STALE_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_STALE_TIMEOUT_SECS)
    }

    /// Update liveness states based on heartbeat timestamps.
    pub(crate) fn update_liveness_states(&mut self) {
        // We don't need the changed records here, just the side effect of updating states.
        // The return value is used by check_process_liveness for notifications.
        let _ = self.update_liveness_states_with_changes();
    }

    /// Update liveness states and return records that changed.
    fn update_liveness_states_with_changes(&mut self) -> Vec<SessionRecord> {
        let now = chrono::Utc::now();
        let unresponsive_timeout = Self::unresponsive_timeout_secs();
        let stale_timeout = Self::stale_timeout_secs();
        let mut changed = Vec::new();

        for record in self.sessions.values_mut() {
            // Skip already stopped sessions
            if record.liveness == LivenessState::Stopped {
                continue;
            }

            // Parse last heartbeat timestamp
            let last_heartbeat =
                match chrono::DateTime::parse_from_rfc3339(&record.last_heartbeat_at) {
                    Ok(dt) => dt.with_timezone(&chrono::Utc),
                    Err(_) => continue,
                };

            let elapsed_secs = (now - last_heartbeat).num_seconds() as u64;
            let old_liveness = record.liveness;

            if elapsed_secs > stale_timeout {
                record.liveness = LivenessState::Stopped;
            } else if elapsed_secs > unresponsive_timeout {
                record.liveness = LivenessState::Unresponsive;
            }

            if record.liveness != old_liveness {
                changed.push(record.clone());
            }
        }

        changed
    }

    /// Check all Running/Unresponsive sessions and mark as Stopped if their process has exited.
    /// Returns records that changed state (with liveness=Stopped).
    pub(crate) fn check_process_liveness(&mut self) -> Vec<SessionRecord> {
        let mut changed = Vec::new();

        for record in self.sessions.values_mut() {
            // Only check sessions that might still be alive
            if record.liveness == LivenessState::Stopped {
                continue;
            }

            // Check if the process still exists
            if !process_exists(record.pid) {
                record.liveness = LivenessState::Stopped;
                record.updated_at = chrono::Utc::now().to_rfc3339();
                changed.push(record.clone());
            }
        }

        changed
    }
}
