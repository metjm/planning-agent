//! Host service definitions for daemon ↔ host RPC.

use crate::rpc::HostError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// Re-export existing types from host_protocol
// Note: Allow unused for now - will be used by RPC upstream/server implementations
#[allow(unused_imports)]
pub use crate::host_protocol::{SessionInfo, PROTOCOL_VERSION};

/// Container identification for host connection handshake.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerInfo {
    pub container_id: String,
    pub container_name: String,
    pub working_dir: PathBuf,
}

/// Service exposed by the host to container daemons.
#[tarpc::service]
pub trait HostService {
    /// Initial handshake - returns host version if protocol compatible.
    async fn hello(info: ContainerInfo, protocol_version: u32) -> Result<String, HostError>;

    /// Sync all sessions (sent on connect/reconnect).
    async fn sync_sessions(sessions: Vec<SessionInfo>);

    /// Update a single session.
    async fn session_update(session: SessionInfo);

    /// Remove a session.
    async fn session_gone(session_id: String);

    /// Heartbeat to maintain connection liveness.
    async fn heartbeat();
}
