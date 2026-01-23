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
use crate::rpc::{HostError, PortFileContent, SessionRecord};
use crate::session_daemon::rpc_server::{run_daemon_server, run_subscriber_listener};
use crate::session_daemon::server::DaemonState;
use std::collections::HashMap;
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
        PathBuf::from("/test/state.json"),
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

// Host server test infrastructure
use crate::rpc::host_service::{HostService, SessionInfo};

/// Session info stored per container.
#[derive(Default)]
pub struct ContainerState {
    pub sessions: HashMap<String, SessionInfo>,
}

/// Shared state for host server tests.
pub struct HostState {
    pub containers: HashMap<String, ContainerState>,
}

impl HostState {
    pub fn new() -> Self {
        Self {
            containers: HashMap::new(),
        }
    }
}

/// Test implementation of HostService.
#[derive(Clone)]
pub struct TestHostService {
    pub state: Arc<Mutex<HostState>>,
}

impl HostService for TestHostService {
    async fn hello(
        self,
        _: tarpc::context::Context,
        info: crate::rpc::host_service::ContainerInfo,
        protocol_version: u32,
    ) -> Result<String, HostError> {
        if protocol_version != crate::rpc::host_service::PROTOCOL_VERSION {
            return Err(HostError::ProtocolMismatch {
                got: protocol_version,
                expected: crate::rpc::host_service::PROTOCOL_VERSION,
            });
        }
        let mut state = self.state.lock().await;
        state
            .containers
            .insert(info.container_id, ContainerState::default());
        Ok("test-version".to_string())
    }

    async fn heartbeat(self, _: tarpc::context::Context) {
        // No-op for tests
    }

    async fn session_update(self, _: tarpc::context::Context, session: SessionInfo) {
        let mut state = self.state.lock().await;
        // Find the container (we don't have container_id in SessionInfo, use first container)
        if let Some(container) = state.containers.values_mut().next() {
            container
                .sessions
                .insert(session.session_id.clone(), session);
        }
    }

    async fn session_gone(self, _: tarpc::context::Context, session_id: String) {
        let mut state = self.state.lock().await;
        for container in state.containers.values_mut() {
            container.sessions.remove(&session_id);
        }
    }

    async fn sync_sessions(self, _: tarpc::context::Context, sessions: Vec<SessionInfo>) {
        let mut state = self.state.lock().await;
        if let Some(container) = state.containers.values_mut().next() {
            for session in sessions {
                container
                    .sessions
                    .insert(session.session_id.clone(), session);
            }
        }
    }
}

/// Test host server for upstream connection tests.
pub struct TestHostServer {
    pub port: u16,
    pub state: Arc<Mutex<HostState>>,
    _server_handle: tokio::task::JoinHandle<()>,
}

impl TestHostServer {
    pub async fn start() -> Self {
        use futures::StreamExt;
        use tarpc::serde_transport::tcp;
        use tarpc::server::{self, Channel};

        let port = find_test_port();
        let state = Arc::new(Mutex::new(HostState::new()));

        let server_state = state.clone();
        let server_handle = tokio::spawn(async move {
            let addr = format!("127.0.0.1:{}", port);
            let mut listener = tcp::listen(&addr, Bincode::default).await.unwrap();

            while let Some(result) = listener.next().await {
                if let Ok(transport) = result {
                    let service = TestHostService {
                        state: server_state.clone(),
                    };
                    let channel = server::BaseChannel::with_defaults(transport);
                    tokio::spawn(async move {
                        channel
                            .execute(service.serve())
                            .for_each(|response| async {
                                tokio::spawn(response);
                            })
                            .await;
                    });
                }
            }
        });

        // Give server time to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        Self {
            port,
            state,
            _server_handle: server_handle,
        }
    }
}
