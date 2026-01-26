//! RPC service definitions for session daemon and host communication.
//!
//! This module defines the tarpc services for:
//! - Client ↔ Daemon: Session registration, updates, heartbeats, and subscriptions
//! - Daemon ↔ Host: Session aggregation across containers

pub mod daemon_service;
pub mod host_service;

use serde::{Deserialize, Serialize};

// ============================================================================
// TYPE RE-EXPORTS
// ============================================================================
//
// These types are re-exported from existing modules to maintain consistency
// and avoid duplication. During the migration, the old protocol files will
// be deleted and these types will be defined directly in the RPC modules.
// ============================================================================

// Re-export existing types from session_daemon::protocol
// Note: Allow unused for now - will be used by RPC server/client implementations
#[allow(unused_imports)]
pub use crate::session_daemon::protocol::{LivenessState, PortFileContent, SessionRecord};

// Re-export SessionInfo from host_protocol
#[allow(unused_imports)]
pub use crate::host_protocol::SessionInfo;

// Re-export CQRS domain types for event streaming
// Note: Allow unused for now - will be used when event streaming is implemented
#[allow(unused_imports)]
pub use crate::domain::view::WorkflowEventEnvelope;

// ============================================================================
// ERROR TYPES (NEW)
// ============================================================================

/// Errors returned by daemon RPC methods.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonError {
    /// Session not found in registry
    SessionNotFound { session_id: String },
    /// Session already registered by different PID
    AlreadyRegistered {
        session_id: String,
        existing_pid: u32,
    },
    /// Daemon is shutting down
    ShuttingDown,
    /// Authentication failed
    AuthenticationFailed,
    /// Internal error
    Internal { message: String },
}

impl std::fmt::Display for DaemonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DaemonError::SessionNotFound { session_id } => {
                write!(f, "Session not found: {}", session_id)
            }
            DaemonError::AlreadyRegistered {
                session_id,
                existing_pid,
            } => {
                write!(
                    f,
                    "Session {} already registered by PID {}",
                    session_id, existing_pid
                )
            }
            DaemonError::ShuttingDown => write!(f, "Daemon is shutting down"),
            DaemonError::AuthenticationFailed => write!(f, "Authentication failed"),
            DaemonError::Internal { message } => write!(f, "Internal error: {}", message),
        }
    }
}

impl std::error::Error for DaemonError {}

/// Result type for daemon operations.
pub type DaemonResult<T> = Result<T, DaemonError>;

/// Errors returned by host RPC methods.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HostError {
    /// Protocol version mismatch
    ProtocolMismatch { got: u32, expected: u32 },
    /// Container not registered
    ContainerNotRegistered,
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostError::ProtocolMismatch { got, expected } => {
                write!(
                    f,
                    "Protocol version mismatch: got {}, expected {}",
                    got, expected
                )
            }
            HostError::ContainerNotRegistered => write!(f, "Container not registered"),
        }
    }
}

impl std::error::Error for HostError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daemon_error_display_session_not_found() {
        let err = DaemonError::SessionNotFound {
            session_id: "sess-123".to_string(),
        };
        assert_eq!(format!("{}", err), "Session not found: sess-123");
    }

    #[test]
    fn test_daemon_error_display_already_registered() {
        let err = DaemonError::AlreadyRegistered {
            session_id: "sess-456".to_string(),
            existing_pid: 9999,
        };
        assert_eq!(
            format!("{}", err),
            "Session sess-456 already registered by PID 9999"
        );
    }

    #[test]
    fn test_daemon_error_display_shutting_down() {
        let err = DaemonError::ShuttingDown;
        assert_eq!(format!("{}", err), "Daemon is shutting down");
    }

    #[test]
    fn test_daemon_error_display_authentication_failed() {
        let err = DaemonError::AuthenticationFailed;
        assert_eq!(format!("{}", err), "Authentication failed");
    }

    #[test]
    fn test_daemon_error_display_internal() {
        let err = DaemonError::Internal {
            message: "something went wrong".to_string(),
        };
        assert_eq!(format!("{}", err), "Internal error: something went wrong");
    }

    #[test]
    fn test_host_error_display_protocol_mismatch() {
        let err = HostError::ProtocolMismatch {
            got: 1,
            expected: 2,
        };
        assert_eq!(
            format!("{}", err),
            "Protocol version mismatch: got 1, expected 2"
        );
    }

    #[test]
    fn test_host_error_display_container_not_registered() {
        let err = HostError::ContainerNotRegistered;
        assert_eq!(format!("{}", err), "Container not registered");
    }

    #[test]
    fn test_daemon_error_serialization() {
        let err = DaemonError::SessionNotFound {
            session_id: "test-session".to_string(),
        };
        let json = serde_json::to_string(&err).unwrap();
        let parsed: DaemonError = serde_json::from_str(&json).unwrap();
        match parsed {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "test-session");
            }
            _ => panic!("Expected SessionNotFound"),
        }
    }

    #[test]
    fn test_host_error_serialization() {
        let err = HostError::ProtocolMismatch {
            got: 1,
            expected: 2,
        };
        let json = serde_json::to_string(&err).unwrap();
        let parsed: HostError = serde_json::from_str(&json).unwrap();
        match parsed {
            HostError::ProtocolMismatch { got, expected } => {
                assert_eq!(got, 1);
                assert_eq!(expected, 2);
            }
            _ => panic!("Expected ProtocolMismatch"),
        }
    }
}
