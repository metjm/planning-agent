//! Protocol types for session daemon IPC.
//!
//! All communication uses newline-delimited JSON (one JSON object per line).
//! Connections are persistent with multiple request/response exchanges per connection.

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
    /// Path to the state file for this workflow
    pub state_path: PathBuf,
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
        state_path: PathBuf,
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
            state_path,
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

/// Messages sent from client to daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    /// Register a new session
    Register(SessionRecord),
    /// Update an existing session's state
    Update(SessionRecord),
    /// Send a heartbeat for a session
    Heartbeat {
        session_id: String,
    },
    /// Request list of all sessions
    List,
    /// Request daemon shutdown (for updates)
    Shutdown,
    /// Force-stop a session (mark as stopped immediately)
    ForceStop {
        session_id: String,
    },
}

/// Messages sent from daemon to client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum DaemonMessage {
    /// Acknowledgement with daemon's build SHA
    Ack {
        build_sha: String,
    },
    /// List of all sessions
    Sessions(Vec<SessionRecord>),
    /// Daemon is restarting (sent before shutdown)
    Restarting {
        new_sha: String,
    },
    /// Error response
    Error(String),
}

/// Windows-only: Port file content with authentication token.
#[cfg(windows)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortFileContent {
    pub port: u16,
    pub token: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_record_new() {
        let record = SessionRecord::new(
            "session-123".to_string(),
            "test-feature".to_string(),
            PathBuf::from("/test/dir"),
            PathBuf::from("/test/state.json"),
            "Planning".to_string(),
            1,
            "Planning".to_string(),
            12345,
        );

        assert_eq!(record.workflow_session_id, "session-123");
        assert_eq!(record.liveness, LivenessState::Running);
        assert!(!record.updated_at.is_empty());
        assert!(!record.last_heartbeat_at.is_empty());
    }

    #[test]
    fn test_session_record_update_heartbeat() {
        let mut record = SessionRecord::new(
            "session-123".to_string(),
            "test-feature".to_string(),
            PathBuf::from("/test/dir"),
            PathBuf::from("/test/state.json"),
            "Planning".to_string(),
            1,
            "Planning".to_string(),
            12345,
        );

        record.liveness = LivenessState::Unresponsive;
        record.update_heartbeat();

        assert_eq!(record.liveness, LivenessState::Running);
    }

    #[test]
    fn test_client_message_serialization() {
        let msg = ClientMessage::Heartbeat {
            session_id: "test-123".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("Heartbeat"));
        assert!(json.contains("test-123"));

        let parsed: ClientMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            ClientMessage::Heartbeat { session_id } => {
                assert_eq!(session_id, "test-123");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_daemon_message_serialization() {
        let msg = DaemonMessage::Ack {
            build_sha: "abc123".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("Ack"));
        assert!(json.contains("abc123"));

        let parsed: DaemonMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            DaemonMessage::Ack { build_sha } => {
                assert_eq!(build_sha, "abc123");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_liveness_state_display() {
        assert_eq!(format!("{}", LivenessState::Running), "Running");
        assert_eq!(format!("{}", LivenessState::Unresponsive), "Unresponsive");
        assert_eq!(format!("{}", LivenessState::Stopped), "Stopped");
    }

    #[test]
    fn test_register_message_roundtrip() {
        let record = SessionRecord::new(
            "session-456".to_string(),
            "my-feature".to_string(),
            PathBuf::from("/workspace"),
            PathBuf::from("/state.json"),
            "Implementation".to_string(),
            2,
            "Planning".to_string(),
            9999,
        );

        let msg = ClientMessage::Register(record);
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ClientMessage = serde_json::from_str(&json).unwrap();

        match parsed {
            ClientMessage::Register(rec) => {
                assert_eq!(rec.workflow_session_id, "session-456");
                assert_eq!(rec.feature_name, "my-feature");
                assert_eq!(rec.iteration, 2);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_sessions_message_roundtrip() {
        let records = vec![
            SessionRecord::new(
                "s1".to_string(),
                "f1".to_string(),
                PathBuf::from("/d1"),
                PathBuf::from("/s1.json"),
                "Planning".to_string(),
                1,
                "Planning".to_string(),
                100,
            ),
            SessionRecord::new(
                "s2".to_string(),
                "f2".to_string(),
                PathBuf::from("/d2"),
                PathBuf::from("/s2.json"),
                "Complete".to_string(),
                3,
                "Complete".to_string(),
                200,
            ),
        ];

        let msg = DaemonMessage::Sessions(records);
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: DaemonMessage = serde_json::from_str(&json).unwrap();

        match parsed {
            DaemonMessage::Sessions(recs) => {
                assert_eq!(recs.len(), 2);
                assert_eq!(recs[0].workflow_session_id, "s1");
                assert_eq!(recs[1].workflow_session_id, "s2");
            }
            _ => panic!("Wrong variant"),
        }
    }
}
