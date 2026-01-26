//! RPC server implementation for host service.
//!
//! Implements the tarpc HostService trait for handling daemon RPC requests.

#[cfg(any(feature = "host-gui", test))]
use crate::host::state::HostState;

/// Events sent from RPC server to GUI.
#[derive(Debug, Clone)]
pub enum HostEvent {
    /// A new container daemon connected.
    ContainerConnected {
        /// Container ID is only read by GUI code (behind host-gui feature).
        #[cfg_attr(not(feature = "host-gui"), allow(dead_code))]
        container_id: String,
        container_name: String,
    },
    /// A container daemon disconnected.
    ContainerDisconnected { container_id: String },
    /// Sessions were updated (sync, update, or removal).
    SessionsUpdated,
    /// Credentials were reported from a daemon.
    CredentialsReported,
}
use crate::rpc::host_service::{
    AccountUsageInfo, ContainerInfo, CredentialInfo, HostService, SessionInfo, PROTOCOL_VERSION,
};
use crate::rpc::HostError;
#[cfg(any(feature = "host-gui", test))]
use futures::StreamExt;
use std::sync::Arc;
#[cfg(any(feature = "host-gui", test))]
use tarpc::server::{self, Channel};
#[cfg(any(feature = "host-gui", test))]
use tarpc::tokio_serde::formats::Bincode;
use tokio::sync::{mpsc, Mutex};

/// Server implementation for HostService.
#[cfg(any(feature = "host-gui", test))]
#[derive(Clone)]
pub struct HostServer {
    state: Arc<Mutex<HostState>>,
    event_tx: mpsc::UnboundedSender<HostEvent>,
    /// Container ID for this connection (set after hello).
    /// Public for test infrastructure to access during cleanup.
    pub container_id: Arc<Mutex<Option<String>>>,
}

#[cfg(any(feature = "host-gui", test))]
impl HostServer {
    pub fn new(state: Arc<Mutex<HostState>>, event_tx: mpsc::UnboundedSender<HostEvent>) -> Self {
        Self {
            state,
            event_tx,
            container_id: Arc::new(Mutex::new(None)),
        }
    }
}

#[cfg(any(feature = "host-gui", test))]
impl HostService for HostServer {
    async fn hello(
        self,
        _: tarpc::context::Context,
        info: ContainerInfo,
        protocol_version: u32,
    ) -> Result<String, HostError> {
        eprintln!(
            "[host-rpc] Hello received from {} (protocol v{}, git={}, built={})",
            info.container_id, protocol_version, info.git_sha, info.build_timestamp
        );

        // Check protocol version
        if protocol_version != PROTOCOL_VERSION {
            return Err(HostError::ProtocolMismatch {
                got: protocol_version,
                expected: PROTOCOL_VERSION,
            });
        }

        // Register container
        {
            let mut state = self.state.lock().await;
            state.add_container(
                info.container_id.clone(),
                info.container_name.clone(),
                info.working_dir.clone(),
                info.git_sha.clone(),
                info.build_timestamp,
            );
        }

        // Store container ID for this connection
        {
            let mut id = self.container_id.lock().await;
            *id = Some(info.container_id.clone());
        }

        // Notify event listeners
        let _ = self.event_tx.send(HostEvent::ContainerConnected {
            container_id: info.container_id,
            container_name: info.container_name,
        });

        Ok(env!("CARGO_PKG_VERSION").to_string())
    }

    async fn sync_sessions(self, _: tarpc::context::Context, sessions: Vec<SessionInfo>) {
        eprintln!(
            "[host-rpc] sync_sessions received: {} sessions",
            sessions.len()
        );
        for s in &sessions {
            eprintln!(
                "[host-rpc]   - {} (feature: {})",
                s.session_id, s.feature_name
            );
        }

        let container_id = {
            let id = self.container_id.lock().await;
            id.clone()
        };

        if let Some(container_id) = container_id {
            eprintln!(
                "[host-rpc] Storing {} sessions for container {}",
                sessions.len(),
                container_id
            );
            let mut state = self.state.lock().await;
            state.sync_sessions(&container_id, sessions);
            let _ = self.event_tx.send(HostEvent::SessionsUpdated);
        } else {
            eprintln!("[host-rpc] WARNING: sync_sessions received but no container_id set");
        }
    }

    async fn session_update(self, _: tarpc::context::Context, session: SessionInfo) {
        eprintln!(
            "[host-rpc] session_update received: {} (feature: {})",
            session.session_id, session.feature_name
        );

        let container_id = {
            let id = self.container_id.lock().await;
            id.clone()
        };

        if let Some(container_id) = container_id {
            eprintln!(
                "[host-rpc] Storing session {} in container {}",
                session.session_id, container_id
            );
            let mut state = self.state.lock().await;
            state.update_session(&container_id, session);
            let _ = self.event_tx.send(HostEvent::SessionsUpdated);
        } else {
            eprintln!("[host-rpc] WARNING: session_update received but no container_id set (Hello not called?)");
        }
    }

    async fn session_gone(self, _: tarpc::context::Context, session_id: String) {
        let container_id = {
            let id = self.container_id.lock().await;
            id.clone()
        };

        if let Some(container_id) = container_id {
            let mut state = self.state.lock().await;
            state.remove_session(&container_id, &session_id);
            let _ = self.event_tx.send(HostEvent::SessionsUpdated);
        }
    }

    async fn heartbeat(self, _: tarpc::context::Context) {
        let container_id = {
            let id = self.container_id.lock().await;
            id.clone()
        };

        if let Some(container_id) = container_id {
            let mut state = self.state.lock().await;
            state.heartbeat(&container_id);
        }
    }

    async fn report_credentials(
        self,
        _: tarpc::context::Context,
        credentials: Vec<CredentialInfo>,
    ) {
        let container_id = {
            let id = self.container_id.lock().await;
            id.clone()
        };

        if let Some(container_id) = container_id {
            eprintln!(
                "[host-rpc] report_credentials: {} credentials from {}",
                credentials.len(),
                container_id
            );
            for cred in &credentials {
                eprintln!(
                    "[host-rpc]   - {} ({}, valid={}, token_len={})",
                    cred.provider,
                    cred.email,
                    cred.token_valid,
                    cred.access_token.len()
                );
            }

            // Store credentials in state for later API calls
            {
                let mut state = self.state.lock().await;
                state.store_credentials(credentials);
            }

            // Send event to trigger usage fetching
            let _ = self.event_tx.send(HostEvent::CredentialsReported);
        }
    }

    async fn get_account_usage(self, _: tarpc::context::Context) -> Vec<AccountUsageInfo> {
        let state = self.state.lock().await;
        state.get_account_usage()
    }
}

/// Run the host RPC server.
#[cfg(feature = "host-gui")]
pub async fn run_host_rpc_server(
    port: u16,
    state: Arc<Mutex<HostState>>,
    event_tx: mpsc::UnboundedSender<HostEvent>,
) -> anyhow::Result<()> {
    use tarpc::serde_transport::tcp;

    let addr = format!("0.0.0.0:{}", port);

    // Check if localhost:port is already taken (e.g., by VS Code port forwarding)
    // This can cause silent failures where we bind to 0.0.0.0 but localhost traffic
    // goes to the other listener
    if let Ok(stream) = std::net::TcpStream::connect(format!("127.0.0.1:{}", port)) {
        drop(stream);
        anyhow::bail!(
            "Port {} is already in use on localhost (possibly VS Code port forwarding). \
             Use --port to specify a different port.",
            port
        );
    }
    let mut listener = tcp::listen(&addr, Bincode::default).await?;

    eprintln!("[host-rpc] Listening on {}", addr);

    while let Some(result) = listener.next().await {
        match result {
            Ok(transport) => {
                eprintln!("[host-rpc] New connection accepted");
                let server = HostServer::new(state.clone(), event_tx.clone());
                let channel = server::BaseChannel::with_defaults(transport);

                // Clone for cleanup on disconnect
                let cleanup_state = state.clone();
                let cleanup_event_tx = event_tx.clone();
                let cleanup_container_id = server.container_id.clone();

                tokio::spawn(async move {
                    channel
                        .execute(server.serve())
                        .for_each(|response| async {
                            tokio::spawn(response);
                        })
                        .await;

                    // Cleanup on disconnect
                    let container_id = {
                        let id = cleanup_container_id.lock().await;
                        id.clone()
                    };

                    if let Some(container_id) = container_id {
                        let mut state = cleanup_state.lock().await;
                        state.remove_container(&container_id);
                        let _ = cleanup_event_tx.send(HostEvent::ContainerDisconnected {
                            container_id: container_id.clone(),
                        });
                        eprintln!("[host-rpc] Container {} disconnected", container_id);
                    }
                });
            }
            Err(e) => {
                eprintln!("[host-rpc] Accept error: {}", e);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
#[path = "tests/rpc_server_tests.rs"]
mod tests;
