//! Protocol types for communication between container daemons and host application.
//!
//! This module contains types shared between the daemon and host RPC services.

use serde::{Deserialize, Serialize};

// Reuse LivenessState from existing daemon protocol to avoid duplication
pub use crate::session_daemon::LivenessState;

/// Current protocol version.
pub const PROTOCOL_VERSION: u32 = 1;

/// Session information for wire transmission.
/// Uses string fields for phase/status like existing SessionRecord,
/// keeping the host protocol consistent with local daemon protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub feature_name: String,
    /// Workflow phase as string (e.g., "Planning", "Reviewing", "Complete")
    pub phase: String,
    pub iteration: u32,
    /// Workflow status as string (e.g., "Running", "AwaitingApproval", "Error")
    pub status: String,
    /// Reuses LivenessState from session_daemon::protocol
    pub liveness: LivenessState,
    pub started_at: String,
    pub updated_at: String,
    /// Process ID of the workflow process.
    /// Uses serde(default) for graceful degradation if host receives message
    /// from older daemon that doesn't send pid - will deserialize to 0.
    #[serde(default)]
    pub pid: u32,
}

impl SessionInfo {
    /// Convert from local SessionRecord to wire format SessionInfo.
    pub fn from_session_record(record: &crate::session_daemon::SessionRecord) -> Self {
        Self {
            session_id: record.workflow_session_id.clone(),
            feature_name: record.feature_name.clone(),
            phase: record.phase.clone(),
            iteration: record.iteration,
            status: record.workflow_status.clone(),
            liveness: record.liveness,
            started_at: record.updated_at.clone(), // Use updated_at as proxy for started_at
            updated_at: record.updated_at.clone(),
            pid: record.pid,
        }
    }
}

#[cfg(test)]
#[path = "tests/host_protocol_tests.rs"]
mod tests;
