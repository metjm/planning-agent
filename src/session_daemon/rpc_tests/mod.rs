//! Integration tests for the tarpc RPC implementation.
//!
//! These tests spin up real RPC servers and clients to test the full
//! communication flow. No mocks are used.
//!
//! Tests are split across multiple files to stay under the 750 line limit.

mod auth_tests;
mod concurrent_tests;
mod host_tests;
mod liveness_tests;
mod session_tests;
mod subscription_tests;
mod upgrade_tests;

use crate::rpc::daemon_service::DaemonServiceClient;
use crate::rpc::{PortFileContent, SessionRecord};
use crate::session_daemon::rpc_server::{run_daemon_server, run_subscriber_listener};
use crate::session_daemon::server::DaemonState;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tarpc::client;
use tarpc::tokio_serde::formats::Bincode;
use tokio::sync::{broadcast, Mutex, RwLock};

/// Find an available TCP port for testing.
pub fn find_test_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Create a test SessionRecord.
pub fn create_test_record(id: &str, pid: u32) -> SessionRecord {
    SessionRecord::new(
        id.to_string(),
        "test-feature".to_string(),
        PathBuf::from("/test"),
        PathBuf::from("/test/sessions").join(id),
        "Planning".to_string(),
        1,
        "Planning".to_string(),
        pid,
    )
}

/// Test harness that manages a real RPC server for testing.
pub struct TestServer {
    pub port: u16,
    pub subscriber_port: u16,
    pub auth_token: String,
    pub shutdown_tx: broadcast::Sender<()>,
    _server_handle: tokio::task::JoinHandle<()>,
    _subscriber_handle: tokio::task::JoinHandle<()>,
}

impl TestServer {
    /// Start a real RPC server for testing.
    pub async fn start() -> Self {
        use crate::session_daemon::rpc_server::SubscriberRegistry;

        let port = find_test_port();
        let subscriber_port = find_test_port();
        let auth_token = "test-auth-token-12345".to_string();

        let state = Arc::new(Mutex::new(DaemonState::new()));
        let subscribers = Arc::new(RwLock::new(SubscriberRegistry::new()));
        let (shutdown_tx, _) = broadcast::channel(1);

        // Start main RPC server
        let server_handle = {
            let state = state.clone();
            let subscribers = subscribers.clone();
            let shutdown_tx = shutdown_tx.clone();
            let auth_token = auth_token.clone();
            tokio::spawn(async move {
                let _ = run_daemon_server(
                    state,
                    subscribers,
                    shutdown_tx,
                    None, // No upstream for tests
                    auth_token,
                    port,
                )
                .await;
            })
        };

        // Start subscriber listener
        let subscriber_handle = {
            let subscribers = subscribers.clone();
            let shutdown_tx = shutdown_tx.clone();
            tokio::spawn(async move {
                let _ = run_subscriber_listener(subscribers, shutdown_tx, subscriber_port).await;
            })
        };

        // Give server time to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        Self {
            port,
            subscriber_port,
            auth_token,
            shutdown_tx,
            _server_handle: server_handle,
            _subscriber_handle: subscriber_handle,
        }
    }

    /// Create a client connected to this server.
    pub async fn create_client(&self) -> DaemonServiceClient {
        use tarpc::serde_transport::tcp;

        let addr = format!("127.0.0.1:{}", self.port);
        let transport = tcp::connect(&addr, Bincode::default).await.unwrap();
        DaemonServiceClient::new(client::Config::default(), transport).spawn()
    }

    /// Write a port file for subscription tests.
    pub fn write_port_file(&self, path: &std::path::Path) {
        let content = PortFileContent {
            port: self.port,
            subscriber_port: self.subscriber_port,
            file_service_port: 0,
            token: self.auth_token.clone(),
        };
        std::fs::write(path, serde_json::to_string(&content).unwrap()).unwrap();
    }

    /// Shutdown the server.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// Re-export the real HostState for tests
pub use crate::host::state::HostState;

/// Test host server using the REAL HostServer implementation.
/// No mocks - this is the actual production code.
pub struct TestHostServer {
    pub port: u16,
    pub state: Arc<Mutex<HostState>>,
    pub event_rx: tokio::sync::mpsc::UnboundedReceiver<crate::host::rpc_server::HostEvent>,
    _server_handle: tokio::task::JoinHandle<()>,
}

impl TestHostServer {
    pub async fn start() -> Self {
        use crate::host::rpc_server::HostServer;
        use crate::rpc::host_service::HostService;
        use futures::StreamExt;
        use tarpc::serde_transport::tcp;
        use tarpc::server::{self, Channel};

        let port = find_test_port();
        let state = Arc::new(Mutex::new(HostState::new()));
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();

        let server_state = state.clone();
        let server_handle = tokio::spawn(async move {
            let addr = format!("127.0.0.1:{}", port);
            let mut listener = tcp::listen(&addr, Bincode::default).await.unwrap();

            while let Some(result) = listener.next().await {
                if let Ok(transport) = result {
                    // Use the REAL HostServer, not a mock
                    let server = HostServer::new(server_state.clone(), event_tx.clone());
                    let channel = server::BaseChannel::with_defaults(transport);

                    let cleanup_state = server_state.clone();
                    let cleanup_event_tx = event_tx.clone();
                    let cleanup_container_id = server.container_id.clone();

                    tokio::spawn(async move {
                        channel
                            .execute(server.serve())
                            .for_each(|response| async {
                                tokio::spawn(response);
                            })
                            .await;

                        // Cleanup on disconnect (same as production code)
                        let container_id = {
                            let id = cleanup_container_id.lock().await;
                            id.clone()
                        };

                        if let Some(container_id) = container_id {
                            let mut state = cleanup_state.lock().await;
                            state.remove_container(&container_id);
                            let _ = cleanup_event_tx.send(
                                crate::host::rpc_server::HostEvent::ContainerDisconnected {
                                    container_id,
                                },
                            );
                        }
                    });
                }
            }
        });

        // Give server time to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        Self {
            port,
            state,
            event_rx,
            _server_handle: server_handle,
        }
    }
}
