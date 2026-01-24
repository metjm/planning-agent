//! Tests for host RPC server.

use super::*;
use crate::rpc::host_service::HostServiceClient;
use crate::session_daemon::LivenessState;
use std::time::Duration;
use tarpc::client;

/// Create a test SessionInfo.
fn create_test_session(id: &str, status: &str) -> SessionInfo {
    SessionInfo {
        session_id: id.to_string(),
        feature_name: format!("feature-{}", id),
        phase: "Planning".to_string(),
        iteration: 1,
        status: status.to_string(),
        liveness: LivenessState::Running,
        started_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
        pid: 0,
    }
}

/// Test harness for host RPC server.
struct TestHostServer {
    port: u16,
    state: Arc<Mutex<HostState>>,
    event_rx: mpsc::UnboundedReceiver<HostEvent>,
    _server_handle: tokio::task::JoinHandle<()>,
}

impl TestHostServer {
    async fn start() -> Self {
        use tarpc::serde_transport::tcp;

        // Bind to port 0 to get an available port, extract the port, then use that listener
        let state = Arc::new(Mutex::new(HostState::new()));
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        // Use port 0 to let OS assign an available port, avoiding race conditions
        let listener = tcp::listen("127.0.0.1:0", Bincode::default).await.unwrap();
        let port = listener.local_addr().port();

        let server_state = state.clone();
        let server_handle = tokio::spawn(async move {
            let mut listener = listener;
            while let Some(result) = listener.next().await {
                if let Ok(transport) = result {
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

                        // Cleanup on disconnect
                        let container_id = {
                            let id = cleanup_container_id.lock().await;
                            id.clone()
                        };

                        if let Some(container_id) = container_id {
                            let mut state = cleanup_state.lock().await;
                            state.remove_container(&container_id);
                            let _ = cleanup_event_tx
                                .send(HostEvent::ContainerDisconnected { container_id });
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

    async fn create_client(&self) -> HostServiceClient {
        use tarpc::serde_transport::tcp;

        let addr = format!("127.0.0.1:{}", self.port);
        let transport = tcp::connect(&addr, Bincode::default).await.unwrap();
        HostServiceClient::new(client::Config::default(), transport).spawn()
    }
}

// ============================================================================
// Hello/Handshake Tests
// ============================================================================

#[tokio::test]
async fn test_hello_success() {
    let server = TestHostServer::start().await;
    let client = server.create_client().await;

    let info = ContainerInfo {
        container_id: "container-1".to_string(),
        container_name: "Test Container".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
        git_sha: "test123".to_string(),
        build_timestamp: 1234567890,
    };

    let result = client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap();

    assert!(result.is_ok(), "Hello should succeed");
    assert!(!result.unwrap().is_empty(), "Should return host version");

    // Verify container was registered
    let state = server.state.lock().await;
    assert_eq!(state.containers.len(), 1);
    assert!(state.containers.contains_key("container-1"));
}

#[tokio::test]
async fn test_hello_protocol_mismatch() {
    let server = TestHostServer::start().await;
    let client = server.create_client().await;

    let info = ContainerInfo {
        container_id: "container-1".to_string(),
        container_name: "Test Container".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
        git_sha: "test123".to_string(),
        build_timestamp: 1234567890,
    };

    // Use wrong protocol version
    let result = client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION + 1)
        .await
        .unwrap();

    match result {
        Err(HostError::ProtocolMismatch { got, expected }) => {
            assert_eq!(got, PROTOCOL_VERSION + 1);
            assert_eq!(expected, PROTOCOL_VERSION);
        }
        _ => panic!("Expected ProtocolMismatch error"),
    }

    // Verify container was NOT registered
    let state = server.state.lock().await;
    assert_eq!(state.containers.len(), 0);
}

// ============================================================================
// Session Sync Tests
// ============================================================================

#[tokio::test]
async fn test_sync_sessions() {
    let server = TestHostServer::start().await;
    let client = server.create_client().await;

    // First, do hello
    let info = ContainerInfo {
        container_id: "container-1".to_string(),
        container_name: "Test Container".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
        git_sha: "test123".to_string(),
        build_timestamp: 1234567890,
    };
    client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

    // Sync sessions
    let sessions = vec![
        create_test_session("session-1", "Running"),
        create_test_session("session-2", "Planning"),
    ];
    client
        .sync_sessions(tarpc::context::current(), sessions)
        .await
        .unwrap();

    // Verify sessions were stored
    let state = server.state.lock().await;
    let container = state.containers.get("container-1").unwrap();
    assert_eq!(container.sessions.len(), 2);
    assert!(container.sessions.contains_key("session-1"));
    assert!(container.sessions.contains_key("session-2"));
}

#[tokio::test]
async fn test_sync_sessions_replaces_existing() {
    let server = TestHostServer::start().await;
    let client = server.create_client().await;

    let info = ContainerInfo {
        container_id: "container-1".to_string(),
        container_name: "Test Container".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
        git_sha: "test123".to_string(),
        build_timestamp: 1234567890,
    };
    client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

    // First sync
    let sessions1 = vec![
        create_test_session("session-1", "Running"),
        create_test_session("session-2", "Planning"),
    ];
    client
        .sync_sessions(tarpc::context::current(), sessions1)
        .await
        .unwrap();

    // Second sync with different sessions
    let sessions2 = vec![create_test_session("session-3", "Reviewing")];
    client
        .sync_sessions(tarpc::context::current(), sessions2)
        .await
        .unwrap();

    // Verify only new sessions exist
    let state = server.state.lock().await;
    let container = state.containers.get("container-1").unwrap();
    assert_eq!(container.sessions.len(), 1);
    assert!(container.sessions.contains_key("session-3"));
    assert!(!container.sessions.contains_key("session-1"));
}

// ============================================================================
// Session Update Tests
// ============================================================================

#[tokio::test]
async fn test_session_update() {
    let server = TestHostServer::start().await;
    let client = server.create_client().await;

    let info = ContainerInfo {
        container_id: "container-1".to_string(),
        container_name: "Test Container".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
        git_sha: "test123".to_string(),
        build_timestamp: 1234567890,
    };
    client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

    // Sync initial session
    let sessions = vec![create_test_session("session-1", "Planning")];
    client
        .sync_sessions(tarpc::context::current(), sessions)
        .await
        .unwrap();

    // Update session
    let updated = create_test_session("session-1", "Reviewing");
    client
        .session_update(tarpc::context::current(), updated)
        .await
        .unwrap();

    // Verify update
    let state = server.state.lock().await;
    let container = state.containers.get("container-1").unwrap();
    let session = container.sessions.get("session-1").unwrap();
    assert_eq!(session.status, "Reviewing");
}

// ============================================================================
// Session Gone Tests
// ============================================================================

#[tokio::test]
async fn test_session_gone() {
    let server = TestHostServer::start().await;
    let client = server.create_client().await;

    let info = ContainerInfo {
        container_id: "container-1".to_string(),
        container_name: "Test Container".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
        git_sha: "test123".to_string(),
        build_timestamp: 1234567890,
    };
    client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

    // Sync sessions
    let sessions = vec![
        create_test_session("session-1", "Running"),
        create_test_session("session-2", "Planning"),
    ];
    client
        .sync_sessions(tarpc::context::current(), sessions)
        .await
        .unwrap();

    // Remove one session
    client
        .session_gone(tarpc::context::current(), "session-1".to_string())
        .await
        .unwrap();

    // Verify session was removed
    let state = server.state.lock().await;
    let container = state.containers.get("container-1").unwrap();
    assert_eq!(container.sessions.len(), 1);
    assert!(!container.sessions.contains_key("session-1"));
    assert!(container.sessions.contains_key("session-2"));
}

// ============================================================================
// Heartbeat Tests
// ============================================================================

#[tokio::test]
async fn test_heartbeat() {
    let server = TestHostServer::start().await;
    let client = server.create_client().await;

    let info = ContainerInfo {
        container_id: "container-1".to_string(),
        container_name: "Test Container".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
        git_sha: "test123".to_string(),
        build_timestamp: 1234567890,
    };
    client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

    // Get initial heartbeat time
    let initial_time = {
        let state = server.state.lock().await;
        state.containers.get("container-1").unwrap().last_message_at
    };

    // Small delay
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Send heartbeat
    client.heartbeat(tarpc::context::current()).await.unwrap();

    // Verify heartbeat updated
    let state = server.state.lock().await;
    let container = state.containers.get("container-1").unwrap();
    assert!(container.last_message_at > initial_time);
}

// ============================================================================
// Event Notification Tests
// ============================================================================

#[tokio::test]
async fn test_events_on_connect() {
    let mut server = TestHostServer::start().await;
    let client = server.create_client().await;

    let info = ContainerInfo {
        container_id: "container-1".to_string(),
        container_name: "Test Container".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
        git_sha: "test123".to_string(),
        build_timestamp: 1234567890,
    };
    client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

    // Check event was sent
    let event = tokio::time::timeout(Duration::from_secs(1), server.event_rx.recv())
        .await
        .unwrap()
        .unwrap();

    match event {
        HostEvent::ContainerConnected {
            container_id,
            container_name,
        } => {
            assert_eq!(container_id, "container-1");
            assert_eq!(container_name, "Test Container");
        }
        _ => panic!("Expected ContainerConnected event"),
    }
}

#[tokio::test]
async fn test_events_on_sessions_updated() {
    let mut server = TestHostServer::start().await;
    let client = server.create_client().await;

    let info = ContainerInfo {
        container_id: "container-1".to_string(),
        container_name: "Test Container".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
        git_sha: "test123".to_string(),
        build_timestamp: 1234567890,
    };
    client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

    // Drain connect event
    let _ = server.event_rx.recv().await;

    // Sync sessions
    client
        .sync_sessions(
            tarpc::context::current(),
            vec![create_test_session("s1", "Running")],
        )
        .await
        .unwrap();

    // Check SessionsUpdated event
    let event = tokio::time::timeout(Duration::from_secs(1), server.event_rx.recv())
        .await
        .unwrap()
        .unwrap();

    assert!(matches!(event, HostEvent::SessionsUpdated));
}

// ============================================================================
// Multiple Containers Tests
// ============================================================================

#[tokio::test]
async fn test_multiple_containers() {
    let server = TestHostServer::start().await;

    // Connect two clients
    let client1 = server.create_client().await;
    let client2 = server.create_client().await;

    // Hello from both
    let info1 = ContainerInfo {
        container_id: "container-1".to_string(),
        container_name: "Container One".to_string(),
        working_dir: std::path::PathBuf::from("/work1"),
        git_sha: "test123".to_string(),
        build_timestamp: 1234567890,
    };
    client1
        .hello(tarpc::context::current(), info1, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

    let info2 = ContainerInfo {
        container_id: "container-2".to_string(),
        container_name: "Container Two".to_string(),
        working_dir: std::path::PathBuf::from("/work2"),
        git_sha: "test456".to_string(),
        build_timestamp: 1234567891,
    };
    client2
        .hello(tarpc::context::current(), info2, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

    // Each syncs their sessions
    client1
        .sync_sessions(
            tarpc::context::current(),
            vec![create_test_session("c1-s1", "Running")],
        )
        .await
        .unwrap();

    client2
        .sync_sessions(
            tarpc::context::current(),
            vec![create_test_session("c2-s1", "Planning")],
        )
        .await
        .unwrap();

    // Verify both containers and sessions exist
    let state = server.state.lock().await;
    assert_eq!(state.containers.len(), 2);

    let c1 = state.containers.get("container-1").unwrap();
    assert!(c1.sessions.contains_key("c1-s1"));

    let c2 = state.containers.get("container-2").unwrap();
    assert!(c2.sessions.contains_key("c2-s1"));
}

#[tokio::test]
async fn test_disconnect_event_contains_container_id() {
    let mut server = TestHostServer::start().await;
    let client = server.create_client().await;

    let info = ContainerInfo {
        container_id: "disconnect-test".to_string(),
        container_name: "Disconnect Test".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
        git_sha: "test123".to_string(),
        build_timestamp: 1234567890,
    };
    client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

    // Drain connect event
    let _ = server.event_rx.recv().await;

    // Drop the client to trigger disconnect
    drop(client);

    // Wait a bit for disconnect to be processed
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Check for disconnect event
    if let Ok(Some(HostEvent::ContainerDisconnected { container_id })) =
        tokio::time::timeout(Duration::from_secs(1), server.event_rx.recv()).await
    {
        assert_eq!(container_id, "disconnect-test");
    }
}

#[tokio::test]
async fn test_display_sessions_include_container_name() {
    let server = TestHostServer::start().await;
    let client = server.create_client().await;

    let info = ContainerInfo {
        container_id: "display-test".to_string(),
        container_name: "Display Test Container".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
        git_sha: "test123".to_string(),
        build_timestamp: 1234567890,
    };
    client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

    // Sync a session
    client
        .sync_sessions(
            tarpc::context::current(),
            vec![create_test_session("display-session", "Running")],
        )
        .await
        .unwrap();

    // Get display sessions and verify container_name is included
    let mut state = server.state.lock().await;
    let sessions = state.sessions();

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].container_name, "Display Test Container");
    assert_eq!(sessions[0].session.session_id, "display-session");
}
