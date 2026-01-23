//! Protocol types for communication between container daemons and host application.
//!
//! This module is being migrated from JSON-over-socket to tarpc RPC.
//! The JSON message types are retained for the host-gui feature which
//! still uses the old protocol.

use serde::{Deserialize, Serialize};
#[cfg(any(feature = "host-gui", test))]
use std::path::PathBuf;

// Reuse LivenessState from existing daemon protocol to avoid duplication
pub use crate::session_daemon::LivenessState;

/// Current protocol version.
pub const PROTOCOL_VERSION: u32 = 1;

/// Messages sent from container daemon to host application (JSON protocol).
/// Used by the host-gui feature only. The daemon-side now uses tarpc RPC.
#[cfg(any(feature = "host-gui", test))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonToHost {
    /// Initial handshake from container daemon.
    Hello {
        container_id: String,
        container_name: String,
        working_dir: PathBuf,
        protocol_version: u32,
    },
    /// Full sync of all sessions (sent on connect and after reconnect).
    SyncSessions { sessions: Vec<SessionInfo> },
    /// Single session update.
    SessionUpdate { session: SessionInfo },
    /// Session removed from daemon.
    SessionGone { session_id: String },
    /// Keep-alive heartbeat.
    Heartbeat,
}

/// Messages sent from host to container daemon (JSON protocol).
/// Used by the host-gui feature only. The daemon-side now uses tarpc RPC.
#[cfg(any(feature = "host-gui", test))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HostToDaemon {
    /// Response to Hello.
    Welcome {
        host_version: String,
        protocol_version: u32,
    },
    /// Acknowledgment of received message.
    Ack,
}

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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daemon_to_host_serialization() {
        let msg = DaemonToHost::Hello {
            container_id: "test-id".to_string(),
            container_name: "Test Container".to_string(),
            working_dir: PathBuf::from("/test"),
            protocol_version: PROTOCOL_VERSION,
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"Hello\""));
        assert!(json.contains("\"container_id\":\"test-id\""));

        let parsed: DaemonToHost = serde_json::from_str(&json).unwrap();
        match parsed {
            DaemonToHost::Hello { container_id, .. } => {
                assert_eq!(container_id, "test-id");
            }
            _ => panic!("Expected Hello"),
        }
    }

    #[test]
    fn test_host_to_daemon_serialization() {
        let msg = HostToDaemon::Welcome {
            host_version: "0.1.0".to_string(),
            protocol_version: PROTOCOL_VERSION,
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"Welcome\""));

        let parsed: HostToDaemon = serde_json::from_str(&json).unwrap();
        match parsed {
            HostToDaemon::Welcome {
                protocol_version, ..
            } => {
                assert_eq!(protocol_version, PROTOCOL_VERSION);
            }
            _ => panic!("Expected Welcome"),
        }
    }

    #[test]
    fn test_session_info_serialization() {
        let session = SessionInfo {
            session_id: "sess-123".to_string(),
            feature_name: "Test Feature".to_string(),
            phase: "Planning".to_string(),
            iteration: 1,
            status: "Running".to_string(),
            liveness: LivenessState::Running,
            started_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&session).unwrap();
        let parsed: SessionInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.session_id, "sess-123");
        assert_eq!(parsed.phase, "Planning");
    }

    #[test]
    fn test_session_info_from_session_record() {
        use crate::session_daemon::protocol::SessionRecord;
        use std::path::PathBuf;

        let record = SessionRecord::new(
            "session-456".to_string(),
            "my-feature".to_string(),
            PathBuf::from("/work/dir"),
            PathBuf::from("/work/state.json"),
            "Reviewing".to_string(),
            2,
            "Under Review".to_string(),
            9999,
        );

        let session_info = SessionInfo::from_session_record(&record);

        assert_eq!(session_info.session_id, "session-456");
        assert_eq!(session_info.feature_name, "my-feature");
        assert_eq!(session_info.phase, "Reviewing");
        assert_eq!(session_info.iteration, 2);
        assert_eq!(session_info.status, "Under Review");
        assert_eq!(session_info.liveness, LivenessState::Running);
        // Both started_at and updated_at are set from record.updated_at
        assert_eq!(session_info.started_at, record.updated_at);
        assert_eq!(session_info.updated_at, record.updated_at);
    }
}
