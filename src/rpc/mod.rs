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
pub use crate::host::SessionInfo;

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
#[path = "tests/rpc_tests.rs"]
mod tests;
