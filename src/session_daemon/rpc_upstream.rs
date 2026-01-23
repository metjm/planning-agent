//! RPC upstream client for daemon ↔ host communication.
//!
//! This module manages the connection from a container daemon to the host
//! application using tarpc RPC. It:
//! - Connects to the host on port 17717 (or PLANNING_AGENT_HOST_PORT)
//! - Sends session updates via RPC calls
//! - Handles disconnection and reconnection with exponential backoff
//! - Sends periodic heartbeats

use crate::daemon_log::daemon_log;
use crate::rpc::host_service::{ContainerInfo, HostServiceClient, SessionInfo, PROTOCOL_VERSION};
use crate::rpc::SessionRecord;
use crate::session_daemon::server::DaemonState;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tarpc::client;
use tarpc::tokio_serde::formats::Bincode;
use tokio::sync::{mpsc, Mutex};

/// Default port for host connection.
const DEFAULT_HOST_PORT: u16 = 17717;

/// Events to send upstream to host.
#[derive(Debug, Clone)]
pub enum UpstreamEvent {
    /// Single session updated
    SessionUpdate(SessionRecord),
    /// Session has stopped/completed and should be removed
    SessionGone(String),
}

/// Get the host port from environment or default.
/// Always returns a port - upstream connection is enabled by default.
/// Set PLANNING_AGENT_HOST_PORT to override the default port.
/// Set PLANNING_AGENT_HOST_PORT=0 to disable upstream connection.
pub fn host_port() -> Option<u16> {
    match std::env::var("PLANNING_AGENT_HOST_PORT") {
        Ok(s) => {
            let port: u16 = s.parse().unwrap_or(DEFAULT_HOST_PORT);
            if port == 0 {
                None // Explicitly disabled
            } else {
                Some(port)
            }
        }
        Err(_) => Some(DEFAULT_HOST_PORT), // Default enabled
    }
}

/// Manages upstream RPC connection to host application.
pub struct RpcUpstream {
    host: String,
    port: u16,
    container_id: String,
    container_name: String,
    working_dir: PathBuf,
    /// Reference to daemon state for syncing sessions on connect.
    daemon_state: Arc<Mutex<DaemonState>>,
}

impl RpcUpstream {
    /// Create a new upstream connection manager.
    /// Reads container identification from environment variables.
    pub fn new(port: u16, daemon_state: Arc<Mutex<DaemonState>>) -> Self {
        // Host address: "auto" tries localhost then host.docker.internal
        // Can be overridden with PLANNING_AGENT_HOST_ADDRESS
        let host =
            std::env::var("PLANNING_AGENT_HOST_ADDRESS").unwrap_or_else(|_| "auto".to_string());

        let container_id = std::env::var("PLANNING_AGENT_CONTAINER_ID")
            .unwrap_or_else(|_| gethostname::gethostname().to_string_lossy().to_string());

        let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));

        let container_name = std::env::var("PLANNING_AGENT_CONTAINER_NAME").unwrap_or_else(|_| {
            working_dir
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| container_id.clone())
        });

        Self {
            host,
            port,
            container_id,
            container_name,
            working_dir,
            daemon_state,
        }
    }

    /// Run upstream connection loop with automatic reconnection.
    /// This function runs indefinitely, reconnecting on disconnect.
    pub async fn run(self, mut session_rx: mpsc::UnboundedReceiver<UpstreamEvent>) {
        let mut consecutive_failures = 0u32;

        loop {
            match self.connect_and_run(&mut session_rx).await {
                Ok(()) => {
                    // Clean disconnect
                    daemon_log("rpc_upstream", "Host disconnected, reconnecting...");
                    consecutive_failures = 0;
                }
                Err(e) => {
                    consecutive_failures += 1;
                    // Log every 12 failures (once per minute at 5s intervals)
                    if consecutive_failures == 1 || consecutive_failures.is_multiple_of(12) {
                        daemon_log(
                            "rpc_upstream",
                            &format!(
                                "Connection failed (attempt {}): {}",
                                consecutive_failures, e
                            ),
                        );
                    }
                }
            }

            // Always retry every 5 seconds
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    /// Try to connect to the host, racing localhost and host.docker.internal in parallel.
    async fn connect_to_host(&self) -> Result<HostServiceClient> {
        use tarpc::serde_transport::tcp;

        // If explicit host is set, only try that
        if self.host != "auto" {
            let addr = format!("{}:{}", self.host, self.port);
            let transport = tcp::connect(&addr, Bincode::default).await?;
            let client = HostServiceClient::new(client::Config::default(), transport).spawn();
            return Ok(client);
        }

        // Try both localhost and host.docker.internal in parallel.
        // Return the first SUCCESSFUL connection.
        let localhost = format!("127.0.0.1:{}", self.port);
        let docker_host = format!("host.docker.internal:{}", self.port);

        // Spawn both connection attempts as tasks
        let localhost_clone = localhost.clone();
        let docker_host_clone = docker_host.clone();

        let mut localhost_task = tokio::spawn(async move {
            tcp::connect(&localhost_clone, Bincode::default)
                .await
                .map(|t| (t, "localhost"))
        });

        let mut docker_task = tokio::spawn(async move {
            tcp::connect(&docker_host_clone, Bincode::default)
                .await
                .map(|t| (t, "host.docker.internal"))
        });

        // Wait for both with timeout, take first success
        #[allow(unused_assignments)] // last_error is used after the loop
        let result = tokio::time::timeout(Duration::from_millis(2000), async {
            let mut localhost_done = false;
            let mut docker_done = false;
            let mut last_error: Option<std::io::Error> = None;

            loop {
                tokio::select! {
                    result = &mut localhost_task, if !localhost_done => {
                        localhost_done = true;
                        match result {
                            Ok(Ok(transport)) => return Ok(transport),
                            Ok(Err(e)) => last_error = Some(e),
                            Err(e) => last_error = Some(std::io::Error::other(e)),
                        }
                    }
                    result = &mut docker_task, if !docker_done => {
                        docker_done = true;
                        match result {
                            Ok(Ok(transport)) => return Ok(transport),
                            Ok(Err(e)) => last_error = Some(e),
                            Err(e) => last_error = Some(std::io::Error::other(e)),
                        }
                    }
                }

                if localhost_done && docker_done {
                    return Err(last_error
                        .unwrap_or_else(|| std::io::Error::other("Both connections failed")));
                }
            }
        })
        .await;

        match result {
            Ok(Ok((transport, source))) => {
                daemon_log("rpc_upstream", &format!("Connected via {}", source));
                let client = HostServiceClient::new(client::Config::default(), transport).spawn();
                Ok(client)
            }
            Ok(Err(e)) => {
                anyhow::bail!(
                    "Failed to connect to host on port {} (tried localhost and host.docker.internal): {}",
                    self.port,
                    e
                )
            }
            Err(_) => {
                anyhow::bail!(
                    "Failed to connect to host on port {} (connection timed out)",
                    self.port
                )
            }
        }
    }

    async fn connect_and_run(
        &self,
        session_rx: &mut mpsc::UnboundedReceiver<UpstreamEvent>,
    ) -> Result<()> {
        daemon_log("rpc_upstream", "connect_and_run: starting");

        let client = self.connect_to_host().await?;

        daemon_log(
            "rpc_upstream",
            "connect_and_run: RPC connected, starting handshake",
        );

        // Send Hello via RPC
        let container_info = ContainerInfo {
            container_id: self.container_id.clone(),
            container_name: self.container_name.clone(),
            working_dir: self.working_dir.clone(),
            git_sha: crate::update::BUILD_SHA.to_string(),
            build_timestamp: crate::update::BUILD_TIMESTAMP,
        };

        let hello_result = client
            .hello(tarpc::context::current(), container_info, PROTOCOL_VERSION)
            .await?;

        match hello_result {
            Ok(host_version) => {
                daemon_log(
                    "rpc_upstream",
                    &format!("Handshake complete, host version: {}", host_version),
                );
            }
            Err(e) => {
                anyhow::bail!("Handshake failed: {}", e);
            }
        }

        daemon_log(
            "rpc_upstream",
            &format!("Connected to host at {}:{}", self.host, self.port),
        );

        // Sync all current sessions immediately after connecting
        // This ensures the host has up-to-date session info even if events were missed
        {
            let state = self.daemon_state.lock().await;
            let sessions: Vec<SessionRecord> = state.sessions.values().cloned().collect();
            if !sessions.is_empty() {
                daemon_log(
                    "rpc_upstream",
                    &format!("Syncing {} existing sessions to host", sessions.len()),
                );
                let session_infos: Vec<SessionInfo> = sessions
                    .iter()
                    .map(SessionInfo::from_session_record)
                    .collect();
                client
                    .sync_sessions(tarpc::context::current(), session_infos)
                    .await?;
            } else {
                daemon_log("rpc_upstream", "No existing sessions to sync");
            }
        }

        // Main loop: forward session events
        let heartbeat_interval = Duration::from_secs(30);
        let mut heartbeat_timer = tokio::time::interval(heartbeat_interval);

        loop {
            tokio::select! {
                event = session_rx.recv() => {
                    match event {
                        Some(UpstreamEvent::SessionUpdate(record)) => {
                            daemon_log(
                                "rpc_upstream",
                                &format!(
                                    "Sending SessionUpdate to host: {} (feature: {})",
                                    record.workflow_session_id, record.feature_name
                                ),
                            );
                            let session = SessionInfo::from_session_record(&record);
                            client.session_update(tarpc::context::current(), session).await?;
                        }
                        Some(UpstreamEvent::SessionGone(session_id)) => {
                            daemon_log("rpc_upstream", &format!("Sending SessionGone for {}", session_id));
                            client.session_gone(tarpc::context::current(), session_id).await?;
                        }
                        None => {
                            // Channel closed, exit
                            break;
                        }
                    }
                }
                _ = heartbeat_timer.tick() => {
                    client.heartbeat(tarpc::context::current()).await?;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_port_default() {
        // Clear env var if set
        std::env::remove_var("PLANNING_AGENT_HOST_PORT");
        assert_eq!(host_port(), Some(DEFAULT_HOST_PORT));
    }

    #[test]
    fn test_host_port_custom() {
        std::env::set_var("PLANNING_AGENT_HOST_PORT", "12345");
        assert_eq!(host_port(), Some(12345));
        std::env::remove_var("PLANNING_AGENT_HOST_PORT");
    }

    #[test]
    fn test_host_port_disabled() {
        std::env::set_var("PLANNING_AGENT_HOST_PORT", "0");
        assert_eq!(host_port(), None);
        std::env::remove_var("PLANNING_AGENT_HOST_PORT");
    }
}
