//! Host service definitions for daemon ↔ host RPC.

use crate::rpc::HostError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// Re-export existing types from host_protocol
// Note: Allow unused for now - will be used by RPC upstream/server implementations
#[allow(unused_imports)]
pub use crate::host::{SessionInfo, PROTOCOL_VERSION};

/// Container identification for host connection handshake.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerInfo {
    pub container_id: String,
    pub container_name: String,
    pub working_dir: PathBuf,
    /// Git commit SHA the daemon was built from.
    pub git_sha: String,
    /// Unix timestamp when the daemon was built.
    pub build_timestamp: u64,
}

/// Credential info sent from daemon to host (includes tokens for API calls).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialInfo {
    pub provider: String,
    pub email: String,
    pub token_valid: bool,
    pub expires_at: Option<i64>,
    /// Access token for API calls (OAuth token or JWT).
    pub access_token: String,
    /// Account ID (only for Codex, None for others).
    pub account_id: Option<String>,
}

/// Usage info returned from host to daemon/GUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountUsageInfo {
    pub account_id: String,
    pub provider: String,
    pub email: String,
    pub plan_type: Option<String>,
    pub rate_limit_tier: Option<String>,
    pub session_percent: Option<u8>,
    pub session_reset_at: Option<i64>,
    pub weekly_percent: Option<u8>,
    pub weekly_reset_at: Option<i64>,
    pub fetched_at: String,
    pub token_valid: bool,
    pub error: Option<String>,
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

    /// Report available credentials from a daemon.
    /// Called on connect/reconnect and when credentials change.
    async fn report_credentials(credentials: Vec<CredentialInfo>);

    /// Get current usage for all accounts.
    /// Returns the host's view of all tracked accounts.
    async fn get_account_usage() -> Vec<AccountUsageInfo>;
}
