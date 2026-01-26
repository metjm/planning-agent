//! Protocol types for session daemon IPC.
//!
//! Types in this module are shared between the tarpc RPC services and client code.
//! The actual RPC service definitions are in the `crate::rpc` module.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Liveness state tracked by the daemon.
///
/// This is separate from workflow `SessionStatus` which represents the workflow phase.
/// `LivenessState` represents whether the owning process is still running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LivenessState {
    /// Recent heartbeat received (within 25s)
    #[default]
    Running,
    /// Heartbeat stale (25-60s), process may be hung
    Unresponsive,
    /// Heartbeat very stale (>60s) or explicit stop
    Stopped,
}

impl std::fmt::Display for LivenessState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LivenessState::Running => write!(f, "Running"),
            LivenessState::Unresponsive => write!(f, "Unresponsive"),
            LivenessState::Stopped => write!(f, "Stopped"),
        }
    }
}

/// A session record in the daemon registry.
///
/// Contains both workflow metadata and daemon-tracked liveness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    /// The workflow session ID (matches State.workflow_session_id)
    pub workflow_session_id: String,
    /// Feature name for display
    pub feature_name: String,
    /// Working directory where the workflow is running
    pub working_dir: PathBuf,
    /// Path to the session directory containing events.jsonl and snapshot.json
    pub session_dir: PathBuf,
    /// Current workflow phase (e.g., "Planning", "Implementation")
    pub phase: String,
    /// Current iteration number
    pub iteration: u32,
    /// Serialized workflow SessionStatus (e.g., "Planning", "Complete", "Error")
    pub workflow_status: String,
    /// Daemon-computed liveness state
    pub liveness: LivenessState,
    /// Timestamp of last state update (RFC3339)
    pub updated_at: String,
    /// Timestamp of last heartbeat (RFC3339)
    pub last_heartbeat_at: String,
    /// PID of the owning process
    pub pid: u32,
}

impl SessionRecord {
    /// Creates a new session record with current timestamp and Running liveness.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workflow_session_id: String,
        feature_name: String,
        working_dir: PathBuf,
        session_dir: PathBuf,
        phase: String,
        iteration: u32,
        workflow_status: String,
        pid: u32,
    ) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            workflow_session_id,
            feature_name,
            working_dir,
            session_dir,
            phase,
            iteration,
            workflow_status,
            liveness: LivenessState::Running,
            updated_at: now.clone(),
            last_heartbeat_at: now,
            pid,
        }
    }

    /// Updates the heartbeat timestamp.
    pub fn update_heartbeat(&mut self) {
        self.last_heartbeat_at = chrono::Utc::now().to_rfc3339();
        self.liveness = LivenessState::Running;
    }

    /// Updates the session state.
    pub fn update_state(&mut self, phase: String, iteration: u32, workflow_status: String) {
        self.phase = phase;
        self.iteration = iteration;
        self.workflow_status = workflow_status;
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self.last_heartbeat_at = self.updated_at.clone();
        self.liveness = LivenessState::Running;
    }
}

/// Port file content with authentication token.
/// Used on all platforms for TCP-based RPC communication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortFileContent {
    /// Main RPC port
    pub port: u16,
    /// Subscriber callback port
    pub subscriber_port: u16,
    /// Authentication token
    pub token: String,
}

#[cfg(test)]
#[path = "tests/protocol_tests.rs"]
mod tests;
