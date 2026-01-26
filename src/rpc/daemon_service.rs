//! Daemon service definitions for client ↔ daemon RPC.

use crate::rpc::{DaemonResult, SessionRecord, WorkflowEventEnvelope};

/// Service exposed by the session daemon to clients.
#[tarpc::service]
pub trait DaemonService {
    /// Authenticate with the daemon.
    /// Must be called first before any other RPC operations.
    async fn authenticate(token: String) -> DaemonResult<()>;

    /// Register a new session with the daemon.
    /// Returns the daemon's build SHA on success.
    async fn register(record: SessionRecord) -> DaemonResult<String>;

    /// Update an existing session's state.
    /// Returns the daemon's build SHA on success.
    async fn update(record: SessionRecord) -> DaemonResult<String>;

    /// Send a heartbeat for a session.
    async fn heartbeat(session_id: String) -> DaemonResult<()>;

    /// List all sessions.
    async fn list() -> DaemonResult<Vec<SessionRecord>>;

    /// Force-stop a session.
    async fn force_stop(session_id: String) -> DaemonResult<()>;

    /// Request daemon shutdown (for updates).
    async fn shutdown() -> DaemonResult<()>;

    /// Get daemon build SHA for version checking.
    async fn build_sha() -> String;

    /// Get daemon build timestamp for version comparison.
    async fn build_timestamp() -> u64;

    /// Request daemon to upgrade if caller is newer.
    ///
    /// The daemon compares the caller's timestamp with its own:
    /// - If caller is newer (higher timestamp): daemon initiates shutdown and returns true
    /// - If caller is same age or older: daemon refuses and returns false
    ///
    /// This prevents older clients from accidentally killing newer daemons.
    async fn request_upgrade(caller_timestamp: u64) -> bool;

    /// Forward a workflow event for broadcasting to subscribers.
    /// Called by workflow processes to push CQRS events to the daemon,
    /// which then broadcasts them to all connected subscribers.
    async fn workflow_event(session_id: String, event: WorkflowEventEnvelope) -> DaemonResult<()>;
}

/// Callback service for push notifications (daemon → subscriber).
/// Subscribers implement this service; daemon calls into it.
#[tarpc::service]
pub trait SubscriberCallback {
    /// Called when a session changes.
    async fn session_changed(record: SessionRecord);

    /// Called when daemon is restarting. Subscribers should reconnect.
    async fn daemon_restarting(new_sha: String);

    /// Ping to check if subscriber is still alive. Returns true if healthy.
    async fn ping() -> bool;

    /// Called when a workflow emits an event (CQRS event sourcing).
    /// The session_id identifies which workflow emitted the event.
    /// Note: This is an optional extension - implementations may ignore it.
    async fn workflow_event(session_id: String, event: WorkflowEventEnvelope);
}
