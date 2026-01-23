//! RPC server implementation for session daemon.
//!
//! Implements the tarpc DaemonService trait for handling client RPC requests.

use crate::daemon_log::daemon_log;
use crate::planning_paths;
use crate::rpc::daemon_service::{DaemonService, SubscriberCallbackClient};
use crate::rpc::{DaemonError, DaemonResult, LivenessState, PortFileContent, SessionRecord};
use crate::session_daemon::rpc_upstream::UpstreamEvent;
use crate::session_daemon::server::DaemonState;
use crate::update::{BUILD_SHA, BUILD_TIMESTAMP};
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use tarpc::server::{self, Channel};
use tarpc::tokio_serde::formats::Bincode;
use tokio::sync::{broadcast, mpsc, Mutex, RwLock};

/// Unique subscriber ID for tracking connected subscribers.
type SubscriberId = u64;

/// Subscriber tracking - stores callback clients for push notifications.
pub struct SubscriberRegistry {
    /// Map of subscriber ID to their callback client.
    subscribers: HashMap<SubscriberId, SubscriberCallbackClient>,
    /// Next subscriber ID to assign.
    next_id: SubscriberId,
}

impl SubscriberRegistry {
    pub fn new() -> Self {
        Self {
            subscribers: HashMap::new(),
            next_id: 0,
        }
    }

    /// Add a new subscriber with their callback client. Returns the assigned ID.
    pub fn add(&mut self, client: SubscriberCallbackClient) -> SubscriberId {
        let id = self.next_id;
        self.next_id += 1;
        self.subscribers.insert(id, client);
        id
    }

    /// Remove a subscriber by ID.
    pub fn remove(&mut self, id: &SubscriberId) {
        self.subscribers.remove(id);
    }

    /// Broadcast a session change to all subscribers.
    /// Returns IDs of failed subscribers for cleanup.
    pub async fn broadcast_session_changed(&self, record: SessionRecord) -> Vec<SubscriberId> {
        let mut failed = Vec::new();

        for (id, client) in &self.subscribers {
            if client
                .session_changed(tarpc::context::current(), record.clone())
                .await
                .is_err()
            {
                failed.push(*id);
            }
        }

        failed
    }

    /// Broadcast daemon restart to all subscribers.
    pub async fn broadcast_restarting(&self, new_sha: String) -> Vec<SubscriberId> {
        let mut failed = Vec::new();

        for (id, client) in &self.subscribers {
            if client
                .daemon_restarting(tarpc::context::current(), new_sha.clone())
                .await
                .is_err()
            {
                failed.push(*id);
            }
        }

        failed
    }

    /// Get count of active subscribers.
    pub fn count(&self) -> usize {
        self.subscribers.len()
    }

    /// Ping all subscribers to check if they're alive.
    /// Returns IDs of subscribers that failed to respond.
    pub async fn ping_all(&self) -> Vec<SubscriberId> {
        let mut failed = Vec::new();

        for (id, client) in &self.subscribers {
            match client.ping(tarpc::context::current()).await {
                Ok(true) => {} // Healthy
                _ => failed.push(*id),
            }
        }

        failed
    }
}

impl Default for SubscriberRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Server implementation for DaemonService.
#[derive(Clone)]
pub struct DaemonServer {
    state: Arc<Mutex<DaemonState>>,
    subscribers: Arc<RwLock<SubscriberRegistry>>,
    shutdown_tx: broadcast::Sender<()>,
    upstream_tx: Option<mpsc::UnboundedSender<UpstreamEvent>>,
    /// Expected auth token for connections.
    expected_token: Option<String>,
    /// Whether this connection has been authenticated.
    authenticated: Arc<Mutex<bool>>,
}

impl DaemonServer {
    pub fn new(
        state: Arc<Mutex<DaemonState>>,
        subscribers: Arc<RwLock<SubscriberRegistry>>,
        shutdown_tx: broadcast::Sender<()>,
        upstream_tx: Option<mpsc::UnboundedSender<UpstreamEvent>>,
    ) -> Self {
        Self {
            state,
            subscribers,
            shutdown_tx,
            upstream_tx,
            expected_token: None,
            authenticated: Arc::new(Mutex::new(false)),
        }
    }

    pub fn with_auth_token(mut self, token: String) -> Self {
        self.expected_token = Some(token);
        self
    }

    /// Notify all subscribers of a session change and clean up dead ones.
    /// Also forwards the update to the upstream host connection if configured.
    async fn notify_subscribers(&self, record: SessionRecord) {
        // Notify local subscribers
        let failed = {
            let registry = self.subscribers.read().await;
            registry.broadcast_session_changed(record.clone()).await
        };

        if !failed.is_empty() {
            let mut registry = self.subscribers.write().await;
            for id in failed {
                registry.remove(&id);
                daemon_log("rpc_server", &format!("Removed dead subscriber: {}", id));
            }
        }

        // Forward to upstream host connection if configured
        if let Some(upstream_tx) = &self.upstream_tx {
            // Send SessionGone for stopped sessions, SessionUpdate otherwise
            if record.liveness == LivenessState::Stopped {
                let _ = upstream_tx.send(UpstreamEvent::SessionGone(
                    record.workflow_session_id.clone(),
                ));
            } else {
                let _ = upstream_tx.send(UpstreamEvent::SessionUpdate(record));
            }
        }
    }

    async fn check_authenticated(&self) -> DaemonResult<()> {
        if self.expected_token.is_some() {
            let auth = self.authenticated.lock().await;
            if !*auth {
                return Err(DaemonError::AuthenticationFailed);
            }
        }
        Ok(())
    }
}

impl DaemonService for DaemonServer {
    async fn authenticate(self, _: tarpc::context::Context, token: String) -> DaemonResult<()> {
        if let Some(expected) = &self.expected_token {
            if token == *expected {
                let mut auth = self.authenticated.lock().await;
                *auth = true;
                daemon_log("rpc_server", "Client authenticated successfully");
                return Ok(());
            }
        }
        daemon_log("rpc_server", "Client authentication failed");
        Err(DaemonError::AuthenticationFailed)
    }

    async fn register(
        self,
        _: tarpc::context::Context,
        record: SessionRecord,
    ) -> DaemonResult<String> {
        self.check_authenticated().await?;

        let record_clone = record.clone();

        {
            let mut state = self.state.lock().await;

            // Check for stale session with different PID
            if let Some(existing) = state.sessions.get(&record.workflow_session_id) {
                if existing.pid != record.pid && existing.liveness != LivenessState::Stopped {
                    // Only reject if existing session is still running
                    return Err(DaemonError::AlreadyRegistered {
                        session_id: record.workflow_session_id,
                        existing_pid: existing.pid,
                    });
                }
            }

            state
                .sessions
                .insert(record.workflow_session_id.clone(), record);
        }

        // Notify subscribers outside the state lock
        self.notify_subscribers(record_clone).await;
        Ok(BUILD_SHA.to_string())
    }

    async fn update(
        self,
        _: tarpc::context::Context,
        record: SessionRecord,
    ) -> DaemonResult<String> {
        self.check_authenticated().await?;

        let updated_record = {
            let mut state = self.state.lock().await;
            let session_id = record.workflow_session_id.clone();

            if let Some(existing) = state.sessions.get_mut(&session_id) {
                if existing.pid == record.pid {
                    existing.update_state(
                        record.phase.clone(),
                        record.iteration,
                        record.workflow_status.clone(),
                    );
                    if record.liveness == LivenessState::Stopped {
                        existing.liveness = LivenessState::Stopped;
                    }
                }
                existing.clone()
            } else {
                state.sessions.insert(session_id, record.clone());
                record
            }
        };

        self.notify_subscribers(updated_record).await;
        Ok(BUILD_SHA.to_string())
    }

    async fn heartbeat(self, _: tarpc::context::Context, session_id: String) -> DaemonResult<()> {
        self.check_authenticated().await?;

        let maybe_record = {
            let mut state = self.state.lock().await;
            if let Some(record) = state.sessions.get_mut(&session_id) {
                record.update_heartbeat();
                Some(record.clone())
            } else {
                None
            }
        };

        match maybe_record {
            Some(record) => {
                self.notify_subscribers(record).await;
                Ok(())
            }
            None => Err(DaemonError::SessionNotFound {
                session_id: session_id.clone(),
            }),
        }
    }

    async fn list(self, _: tarpc::context::Context) -> DaemonResult<Vec<SessionRecord>> {
        self.check_authenticated().await?;

        let mut state = self.state.lock().await;
        state.update_liveness_states();
        Ok(state.sessions.values().cloned().collect())
    }

    async fn force_stop(self, _: tarpc::context::Context, session_id: String) -> DaemonResult<()> {
        self.check_authenticated().await?;

        let maybe_record = {
            let mut state = self.state.lock().await;
            if let Some(record) = state.sessions.get_mut(&session_id) {
                record.liveness = LivenessState::Stopped;
                Some(record.clone())
            } else {
                None
            }
        };

        match maybe_record {
            Some(record) => {
                self.notify_subscribers(record).await;
                Ok(())
            }
            None => Err(DaemonError::SessionNotFound { session_id }),
        }
    }

    async fn shutdown(self, _: tarpc::context::Context) -> DaemonResult<()> {
        self.check_authenticated().await?;

        // Notify subscribers of restart
        {
            let registry = self.subscribers.read().await;
            let _ = registry.broadcast_restarting(BUILD_SHA.to_string()).await;
        }

        let mut state = self.state.lock().await;
        state.shutting_down = true;
        let _ = state.persist_to_disk();
        let _ = self.shutdown_tx.send(());
        Ok(())
    }

    async fn build_sha(self, _: tarpc::context::Context) -> String {
        BUILD_SHA.to_string()
    }

    async fn build_timestamp(self, _: tarpc::context::Context) -> u64 {
        BUILD_TIMESTAMP
    }

    async fn request_upgrade(self, _: tarpc::context::Context, caller_timestamp: u64) -> bool {
        // Only allow upgrade if caller is strictly newer than us
        // This prevents older clients from killing newer daemons
        if caller_timestamp > BUILD_TIMESTAMP && BUILD_TIMESTAMP > 0 {
            daemon_log(
                "rpc_server",
                &format!(
                    "Upgrade requested: caller={} > daemon={}, initiating shutdown",
                    caller_timestamp, BUILD_TIMESTAMP
                ),
            );

            // Notify subscribers of restart
            {
                let registry = self.subscribers.read().await;
                let _ = registry.broadcast_restarting(BUILD_SHA.to_string()).await;
            }

            // Initiate shutdown
            let mut state = self.state.lock().await;
            state.shutting_down = true;
            let _ = state.persist_to_disk();
            let _ = self.shutdown_tx.send(());

            true
        } else {
            daemon_log(
                "rpc_server",
                &format!(
                    "Upgrade refused: caller={} <= daemon={} (or daemon timestamp is 0)",
                    caller_timestamp, BUILD_TIMESTAMP
                ),
            );
            false
        }
    }
}

/// Run the daemon RPC server (TCP - all platforms).
///
/// Uses TCP on all platforms for consistency. Auth token required for all connections.
pub async fn run_daemon_server(
    state: Arc<Mutex<DaemonState>>,
    subscribers: Arc<RwLock<SubscriberRegistry>>,
    shutdown_tx: broadcast::Sender<()>,
    upstream_tx: Option<mpsc::UnboundedSender<UpstreamEvent>>,
    auth_token: String,
    port: u16,
) -> anyhow::Result<()> {
    use tarpc::serde_transport::tcp;

    let addr = format!("127.0.0.1:{}", port);
    let mut listener = tcp::listen(&addr, Bincode::default).await?;

    daemon_log("rpc_server", &format!("RPC server listening on {}", addr));

    let mut shutdown_rx = shutdown_tx.subscribe();

    loop {
        tokio::select! {
            Some(result) = listener.next() => {
                match result {
                    Ok(transport) => {
                        let server = DaemonServer::new(
                            state.clone(),
                            subscribers.clone(),
                            shutdown_tx.clone(),
                            upstream_tx.clone(),
                        ).with_auth_token(auth_token.clone());

                        let channel = server::BaseChannel::with_defaults(transport);

                        tokio::spawn(async move {
                            channel.execute(server.serve()).for_each(|response| async {
                                tokio::spawn(response);
                            }).await;
                        });
                    }
                    Err(e) => {
                        daemon_log("rpc_server", &format!("Accept error: {}", e));
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                break;
            }
        }
    }

    Ok(())
}

/// Run the subscriber listener (TCP - all platforms).
///
/// Security model: Since the port binds to 127.0.0.1 (localhost only), and
/// the auth token is stored in a file readable only by processes on this
/// machine, any process that can read the token can connect.
pub async fn run_subscriber_listener(
    subscribers: Arc<RwLock<SubscriberRegistry>>,
    shutdown_tx: broadcast::Sender<()>,
    subscriber_port: u16,
) -> anyhow::Result<()> {
    use tarpc::client;
    use tarpc::serde_transport::tcp;

    let addr = format!("127.0.0.1:{}", subscriber_port);
    let mut listener = tcp::listen(&addr, Bincode::default).await?;

    daemon_log(
        "rpc_server",
        &format!("Subscriber listener on {} (localhost only)", addr),
    );

    let mut shutdown_rx = shutdown_tx.subscribe();

    loop {
        tokio::select! {
            Some(result) = listener.next() => {
                match result {
                    Ok(transport) => {
                        // Accept connection from localhost
                        // Security is provided by:
                        // 1. Binding to 127.0.0.1 (no network access)
                        // 2. Token file readable only by local processes
                        // 3. Subscriber must first authenticate via main RPC
                        let callback_client = SubscriberCallbackClient::new(
                            client::Config::default(),
                            transport,
                        ).spawn();

                        let subscriber_id = {
                            let mut registry = subscribers.write().await;
                            registry.add(callback_client)
                        };

                        daemon_log("rpc_server", &format!("Subscriber connected: {}", subscriber_id));
                    }
                    Err(e) => {
                        daemon_log("rpc_server", &format!("Subscriber accept error: {}", e));
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                break;
            }
        }
    }

    Ok(())
}

/// Background task to periodically clean up dead subscriber connections.
/// Sends a ping to each subscriber and removes those that don't respond.
pub async fn run_subscriber_cleanup(
    subscribers: Arc<RwLock<SubscriberRegistry>>,
    shutdown_tx: broadcast::Sender<()>,
) {
    let mut shutdown_rx = shutdown_tx.subscribe();
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // Ping all subscribers and collect those that failed
                let failed = {
                    let registry = subscribers.read().await;
                    if registry.count() == 0 {
                        continue;
                    }
                    registry.ping_all().await
                };

                // Remove failed subscribers
                if !failed.is_empty() {
                    let mut registry = subscribers.write().await;
                    for id in &failed {
                        registry.remove(id);
                        daemon_log("rpc_server", &format!("Cleanup: removed dead subscriber {}", id));
                    }
                    daemon_log("rpc_server", &format!("Cleanup: {} dead subscribers removed, {} remaining",
                        failed.len(), registry.count()));
                }
            }
            _ = shutdown_rx.recv() => {
                break;
            }
        }
    }
}

/// Find an available TCP port.
pub async fn find_available_port() -> anyhow::Result<u16> {
    use tokio::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    Ok(listener.local_addr()?.port())
}

/// Main entry point for the new tarpc-based daemon.
///
/// This replaces the old JSON-over-socket implementation with tarpc RPC.
pub async fn run_daemon_rpc() -> anyhow::Result<()> {
    use anyhow::Context;

    // Write PID file for process management
    let pid = std::process::id();
    let pid_path = planning_paths::sessiond_pid_path()?;
    std::fs::write(&pid_path, pid.to_string()).context("Failed to write PID file")?;

    // Write build SHA file for version detection by clients
    let sha_path = planning_paths::sessiond_build_sha_path()?;
    std::fs::write(&sha_path, BUILD_SHA).context("Failed to write build SHA file")?;

    // Initialize state (reuse existing DaemonState)
    let state = Arc::new(Mutex::new(DaemonState::new()));
    let subscribers = Arc::new(RwLock::new(SubscriberRegistry::new()));

    // Load persisted session registry for recovery after restart
    {
        let mut state_guard = state.lock().await;
        if let Err(e) = state_guard.load_from_disk() {
            daemon_log(
                "rpc_server",
                &format!("Warning: Failed to load registry: {}", e),
            );
        }
    }

    // Create shutdown broadcast channel
    let (shutdown_tx, _) = broadcast::channel::<()>(1);

    // Generate auth token for TCP connections (all platforms)
    let auth_token: String =
        rand::Rng::sample_iter(rand::thread_rng(), &rand::distributions::Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();

    // Find available ports for main RPC and subscriber listener
    let main_port = find_available_port().await?;
    let subscriber_port = find_available_port().await?;

    // Write port file with PortFileContent
    let port_path = planning_paths::sessiond_port_path()?;
    let port_content = PortFileContent {
        port: main_port,
        subscriber_port,
        token: auth_token.clone(),
    };
    std::fs::write(&port_path, serde_json::to_string(&port_content)?)?;

    daemon_log(
        "rpc_server",
        &format!(
            "Daemon starting on ports {} (main) and {} (subscriber)",
            main_port, subscriber_port
        ),
    );

    // Initialize upstream connection if PLANNING_AGENT_HOST_PORT is set
    let upstream_tx = if let Some(host_port) = crate::session_daemon::rpc_upstream::host_port() {
        daemon_log(
            "rpc_server",
            &format!("Upstream host connection enabled on port {}", host_port),
        );

        // Create event channel for upstream notifications
        let (tx, rx) = mpsc::unbounded_channel();

        // Spawn upstream connection task
        let upstream_conn = crate::session_daemon::rpc_upstream::RpcUpstream::new(host_port);
        tokio::spawn(async move {
            upstream_conn.run(rx).await;
        });

        // Initial sync of existing sessions
        let state_for_sync = state.clone();
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            let sessions: Vec<SessionRecord> = {
                let state_guard = state_for_sync.lock().await;
                state_guard.sessions.values().cloned().collect()
            };
            if !sessions.is_empty() {
                let _ = tx_clone.send(UpstreamEvent::SyncSessions(sessions));
            }
        });

        Some(tx)
    } else {
        None
    };

    // Spawn subscriber listener
    let sub_subscribers = subscribers.clone();
    let sub_shutdown = shutdown_tx.clone();
    tokio::spawn(async move {
        if let Err(e) =
            run_subscriber_listener(sub_subscribers, sub_shutdown, subscriber_port).await
        {
            daemon_log("rpc_server", &format!("Subscriber listener error: {}", e));
        }
    });

    // Spawn subscriber cleanup task
    let cleanup_subscribers = subscribers.clone();
    let cleanup_shutdown = shutdown_tx.clone();
    tokio::spawn(async move {
        run_subscriber_cleanup(cleanup_subscribers, cleanup_shutdown).await;
    });

    // Run main RPC server (blocks until shutdown)
    run_daemon_server(
        state.clone(),
        subscribers.clone(),
        shutdown_tx.clone(),
        upstream_tx,
        auth_token,
        main_port,
    )
    .await?;

    // Persist registry to disk before shutdown
    {
        let state_guard = state.lock().await;
        if let Err(e) = state_guard.persist_to_disk() {
            daemon_log(
                "rpc_server",
                &format!("Warning: Failed to persist registry: {}", e),
            );
        }
    }

    // Cleanup files on exit
    let _ = std::fs::remove_file(&port_path);
    let _ = std::fs::remove_file(&pid_path);

    Ok(())
}
