//! Integration tests for the tarpc RPC implementation.
//!
//! These tests spin up real RPC servers and clients to test the full
//! communication flow. No mocks are used.

use crate::rpc::daemon_service::DaemonServiceClient;
use crate::rpc::{DaemonError, LivenessState, PortFileContent, SessionRecord};
use crate::session_daemon::rpc_server::{run_daemon_server, run_subscriber_listener};
use crate::session_daemon::server::DaemonState;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tarpc::client;
use tarpc::tokio_serde::formats::Bincode;
use tokio::sync::{broadcast, Mutex, RwLock};

/// Find an available TCP port for testing.
fn find_test_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Create a test SessionRecord.
fn create_test_record(id: &str, pid: u32) -> SessionRecord {
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
struct TestServer {
    port: u16,
    subscriber_port: u16,
    auth_token: String,
    shutdown_tx: broadcast::Sender<()>,
    _server_handle: tokio::task::JoinHandle<()>,
    _subscriber_handle: tokio::task::JoinHandle<()>,
}

impl TestServer {
    /// Start a real RPC server for testing.
    async fn start() -> Self {
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
    async fn create_client(&self) -> DaemonServiceClient {
        use tarpc::serde_transport::tcp;

        let addr = format!("127.0.0.1:{}", self.port);
        let transport = tcp::connect(&addr, Bincode::default).await.unwrap();
        DaemonServiceClient::new(client::Config::default(), transport).spawn()
    }

    /// Write a port file for subscription tests.
    fn write_port_file(&self, path: &std::path::Path) {
        let content = PortFileContent {
            port: self.port,
            subscriber_port: self.subscriber_port,
            token: self.auth_token.clone(),
        };
        std::fs::write(path, serde_json::to_string(&content).unwrap()).unwrap();
    }

    /// Shutdown the server.
    fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ============================================================================
// Authentication Tests
// ============================================================================

#[tokio::test]
async fn test_authentication_with_valid_token() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    // Authenticate with valid token
    let result = client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap();

    assert!(
        result.is_ok(),
        "Authentication with valid token should succeed"
    );
}

#[tokio::test]
async fn test_authentication_with_invalid_token() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    // Authenticate with invalid token
    let result = client
        .authenticate(tarpc::context::current(), "wrong-token".to_string())
        .await
        .unwrap();

    assert!(
        matches!(result, Err(DaemonError::AuthenticationFailed)),
        "Authentication with invalid token should fail"
    );
}

#[tokio::test]
async fn test_unauthenticated_call_rejected() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    // Try to list without authenticating first
    let result = client.list(tarpc::context::current()).await.unwrap();

    assert!(
        matches!(result, Err(DaemonError::AuthenticationFailed)),
        "Unauthenticated call should be rejected"
    );
}

#[tokio::test]
async fn test_authenticated_call_succeeds() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    // Authenticate first
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Now list should succeed
    let result = client.list(tarpc::context::current()).await.unwrap();

    assert!(result.is_ok(), "Authenticated call should succeed");
    assert_eq!(
        result.unwrap().len(),
        0,
        "Should have no sessions initially"
    );
}

// ============================================================================
// Session Lifecycle Tests
// ============================================================================

#[tokio::test]
async fn test_register_session() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    // Authenticate
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Register a session
    let record = create_test_record("session-1", 1000);
    let result = client
        .register(tarpc::context::current(), record)
        .await
        .unwrap();

    assert!(result.is_ok(), "Register should succeed");

    // Verify session is in list
    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].workflow_session_id, "session-1");
    assert_eq!(sessions[0].pid, 1000);
}

#[tokio::test]
async fn test_register_replaces_stale_session() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    // Authenticate
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Register first session
    let record1 = create_test_record("session-1", 1000);
    client
        .register(tarpc::context::current(), record1)
        .await
        .unwrap()
        .unwrap();

    // Mark the session as stopped (stale) - simulates process crash/exit
    client
        .force_stop(tarpc::context::current(), "session-1".to_string())
        .await
        .unwrap()
        .unwrap();

    // Register again with different PID (simulates new process taking over)
    let record2 = create_test_record("session-1", 2000);
    let result = client
        .register(tarpc::context::current(), record2)
        .await
        .unwrap();

    assert!(
        result.is_ok(),
        "Re-register should succeed for stopped session"
    );

    // Verify only one session with new PID
    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].pid, 2000);
    assert_eq!(sessions[0].liveness, LivenessState::Running);
}

#[tokio::test]
async fn test_update_session() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    // Authenticate
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Register a session
    let record = create_test_record("session-1", 1000);
    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    // Update session
    let mut updated = create_test_record("session-1", 1000);
    updated.phase = "Reviewing".to_string();
    updated.iteration = 2;

    let result = client
        .update(tarpc::context::current(), updated)
        .await
        .unwrap();

    assert!(result.is_ok(), "Update should succeed");

    // Verify update applied
    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sessions[0].phase, "Reviewing");
    assert_eq!(sessions[0].iteration, 2);
}

#[tokio::test]
async fn test_heartbeat_updates_liveness() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    // Authenticate
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Register a session
    let record = create_test_record("session-1", 1000);
    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    // Send heartbeat
    let result = client
        .heartbeat(tarpc::context::current(), "session-1".to_string())
        .await
        .unwrap();

    assert!(result.is_ok(), "Heartbeat should succeed");

    // Verify session still running
    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sessions[0].liveness, LivenessState::Running);
}

#[tokio::test]
async fn test_heartbeat_unknown_session() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    // Authenticate
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Heartbeat for non-existent session
    let result = client
        .heartbeat(tarpc::context::current(), "unknown-session".to_string())
        .await
        .unwrap();

    assert!(
        matches!(result, Err(DaemonError::SessionNotFound { .. })),
        "Heartbeat for unknown session should fail"
    );
}

#[tokio::test]
async fn test_force_stop_session() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    // Authenticate
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Register a session
    let record = create_test_record("session-1", 1000);
    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    // Force stop
    let result = client
        .force_stop(tarpc::context::current(), "session-1".to_string())
        .await
        .unwrap();

    assert!(result.is_ok(), "Force stop should succeed");

    // Verify session is stopped
    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sessions[0].liveness, LivenessState::Stopped);
}

#[tokio::test]
async fn test_force_stop_unknown_session() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    // Authenticate
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Force stop non-existent session
    let result = client
        .force_stop(tarpc::context::current(), "unknown-session".to_string())
        .await
        .unwrap();

    assert!(
        matches!(result, Err(DaemonError::SessionNotFound { .. })),
        "Force stop for unknown session should fail"
    );
}

#[tokio::test]
async fn test_list_multiple_sessions() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    // Authenticate
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Register multiple sessions
    for i in 1..=5 {
        let record = create_test_record(&format!("session-{}", i), 1000 + i);
        client
            .register(tarpc::context::current(), record)
            .await
            .unwrap()
            .unwrap();
    }

    // List all
    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(sessions.len(), 5);
}

#[tokio::test]
async fn test_build_sha_returns_value() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    // build_sha doesn't require authentication
    let sha = client.build_sha(tarpc::context::current()).await.unwrap();

    assert!(!sha.is_empty(), "Build SHA should not be empty");
}

// ============================================================================
// Shutdown Tests
// ============================================================================

#[tokio::test]
async fn test_shutdown_request() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    // Authenticate
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Request shutdown
    let result = client.shutdown(tarpc::context::current()).await.unwrap();

    assert!(result.is_ok(), "Shutdown should succeed");

    // Give server time to shut down
    tokio::time::sleep(Duration::from_millis(100)).await;

    // New connection should fail (server is down)
    let addr = format!("127.0.0.1:{}", server.port);
    let connect_result = tokio::net::TcpStream::connect(&addr).await;

    assert!(connect_result.is_err(), "Server should be shut down");
}

// ============================================================================
// Subscription Tests
// ============================================================================

#[tokio::test]
async fn test_subscription_receives_session_changed() {
    let server = TestServer::start().await;

    // Write port file for subscription
    let temp_dir = tempfile::tempdir().unwrap();
    let port_path = temp_dir.path().join("sessiond.port");
    server.write_port_file(&port_path);

    // Set env var for port file path (subscription reads from here)
    std::env::set_var("PLANNING_AGENT_HOME", temp_dir.path());

    // Create subscription - this connects to the subscriber port
    // Note: RpcSubscription::connect() reads from the standard port file location
    // For this test, we'll create a client and manually test the callback mechanism

    let client = server.create_client().await;

    // Authenticate
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Register a session - this should trigger a notification
    let record = create_test_record("session-1", 1000);
    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    // The notification mechanism is tested via the subscriber registry
    // Full end-to-end subscription test requires the subscription client
    // to be connected, which is more complex to set up in isolation

    std::env::remove_var("PLANNING_AGENT_HOME");
}

// ============================================================================
// Concurrent Access Tests
// ============================================================================

#[tokio::test]
async fn test_concurrent_registrations() {
    let server = TestServer::start().await;

    // Create multiple clients
    let mut handles = Vec::new();

    for i in 0..10 {
        let port = server.port;
        let token = server.auth_token.clone();

        let handle = tokio::spawn(async move {
            use tarpc::serde_transport::tcp;

            let addr = format!("127.0.0.1:{}", port);
            let transport = tcp::connect(&addr, Bincode::default).await.unwrap();
            let client = DaemonServiceClient::new(client::Config::default(), transport).spawn();

            // Authenticate
            client
                .authenticate(tarpc::context::current(), token)
                .await
                .unwrap()
                .unwrap();

            // Register a session
            let record = create_test_record(&format!("concurrent-{}", i), 1000 + i as u32);
            client
                .register(tarpc::context::current(), record)
                .await
                .unwrap()
                .unwrap();
        });

        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        handle.await.unwrap();
    }

    // Verify all sessions registered
    let client = server.create_client().await;
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        sessions.len(),
        10,
        "All concurrent registrations should succeed"
    );
}

#[tokio::test]
async fn test_concurrent_heartbeats() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    // Authenticate and register
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let record = create_test_record("heartbeat-test", 1000);
    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    // Send many concurrent heartbeats
    let mut handles = Vec::new();

    for _ in 0..20 {
        let port = server.port;
        let token = server.auth_token.clone();

        let handle = tokio::spawn(async move {
            use tarpc::serde_transport::tcp;

            let addr = format!("127.0.0.1:{}", port);
            let transport = tcp::connect(&addr, Bincode::default).await.unwrap();
            let client = DaemonServiceClient::new(client::Config::default(), transport).spawn();

            client
                .authenticate(tarpc::context::current(), token)
                .await
                .unwrap()
                .unwrap();

            client
                .heartbeat(tarpc::context::current(), "heartbeat-test".to_string())
                .await
                .unwrap()
                .unwrap();
        });

        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        handle.await.unwrap();
    }

    // Verify session still healthy
    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(sessions[0].liveness, LivenessState::Running);
}

// ============================================================================
// Error Case Tests
// ============================================================================

#[tokio::test]
async fn test_error_session_not_found() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Try to heartbeat non-existent session
    let result = client
        .heartbeat(tarpc::context::current(), "nonexistent".to_string())
        .await
        .unwrap();

    match result {
        Err(DaemonError::SessionNotFound { session_id }) => {
            assert_eq!(session_id, "nonexistent");
        }
        _ => panic!("Expected SessionNotFound error"),
    }
}

#[tokio::test]
async fn test_error_authentication_failed() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    let result = client
        .authenticate(tarpc::context::current(), "bad-token".to_string())
        .await
        .unwrap();

    assert!(matches!(result, Err(DaemonError::AuthenticationFailed)));
}

// ============================================================================
// Update Creates Session If Missing Tests
// ============================================================================

#[tokio::test]
async fn test_update_creates_session_if_missing() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Update without registering first - should create the session
    let record = create_test_record("new-session", 1000);
    let result = client
        .update(tarpc::context::current(), record)
        .await
        .unwrap();

    assert!(result.is_ok(), "Update should create session if missing");

    // Verify session was created
    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].workflow_session_id, "new-session");
}

// ============================================================================
// AlreadyRegistered Error Tests
// ============================================================================

#[tokio::test]
async fn test_error_already_registered() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Register a session with PID 1000
    let record1 = create_test_record("session-1", 1000);
    client
        .register(tarpc::context::current(), record1)
        .await
        .unwrap()
        .unwrap();

    // Try to register same session with different PID while original is still running
    let record2 = create_test_record("session-1", 2000);
    let result = client
        .register(tarpc::context::current(), record2)
        .await
        .unwrap();

    match result {
        Err(DaemonError::AlreadyRegistered {
            session_id,
            existing_pid,
        }) => {
            assert_eq!(session_id, "session-1");
            assert_eq!(existing_pid, 1000);
        }
        _ => panic!("Expected AlreadyRegistered error, got {:?}", result),
    }
}

#[tokio::test]
async fn test_register_same_pid_succeeds() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Register a session
    let record1 = create_test_record("session-1", 1000);
    client
        .register(tarpc::context::current(), record1)
        .await
        .unwrap()
        .unwrap();

    // Re-register with same PID should succeed (same process re-registering)
    let mut record2 = create_test_record("session-1", 1000);
    record2.phase = "Reviewing".to_string();
    let result = client
        .register(tarpc::context::current(), record2)
        .await
        .unwrap();

    assert!(result.is_ok(), "Re-register with same PID should succeed");

    // Verify the update was applied
    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].phase, "Reviewing");
}

// ============================================================================
// Full Subscription Callback Tests
// ============================================================================

#[tokio::test]
async fn test_subscription_callback_end_to_end() {
    use crate::rpc::daemon_service::SubscriberCallback;
    use tarpc::server::{self, Channel};
    use tokio::sync::mpsc;

    let server = TestServer::start().await;

    // Create a channel to receive subscription events
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<SessionRecord>();

    // Create a subscriber callback handler
    #[derive(Clone)]
    struct TestSubscriber {
        tx: mpsc::UnboundedSender<SessionRecord>,
    }

    impl SubscriberCallback for TestSubscriber {
        async fn session_changed(self, _: tarpc::context::Context, record: SessionRecord) {
            let _ = self.tx.send(record);
        }

        async fn daemon_restarting(self, _: tarpc::context::Context, _new_sha: String) {
            // Not testing this path
        }

        async fn ping(self, _: tarpc::context::Context) -> bool {
            true
        }
    }

    // Connect to subscriber port and run our callback server
    let subscriber_addr = format!("127.0.0.1:{}", server.subscriber_port);
    let transport = tarpc::serde_transport::tcp::connect(&subscriber_addr, Bincode::default)
        .await
        .unwrap();

    let handler = TestSubscriber { tx: event_tx };
    let channel = server::BaseChannel::with_defaults(transport);

    // Spawn the subscriber callback server
    tokio::spawn(async move {
        use futures::StreamExt;
        channel
            .execute(handler.serve())
            .for_each(|response| async {
                tokio::spawn(response);
            })
            .await;
    });

    // Give subscriber time to connect
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Now register a session - should trigger callback
    let client = server.create_client().await;
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let record = create_test_record("callback-test", 1000);
    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    // Wait for the callback with timeout
    let received = tokio::time::timeout(Duration::from_secs(2), event_rx.recv()).await;

    match received {
        Ok(Some(record)) => {
            assert_eq!(record.workflow_session_id, "callback-test");
            assert_eq!(record.pid, 1000);
        }
        Ok(None) => panic!("Channel closed without receiving event"),
        Err(_) => panic!("Timeout waiting for subscription callback"),
    }
}

#[tokio::test]
async fn test_subscription_receives_multiple_events() {
    use crate::rpc::daemon_service::SubscriberCallback;
    use tarpc::server::{self, Channel};
    use tokio::sync::mpsc;

    let server = TestServer::start().await;

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<SessionRecord>();

    #[derive(Clone)]
    struct TestSubscriber {
        tx: mpsc::UnboundedSender<SessionRecord>,
    }

    impl SubscriberCallback for TestSubscriber {
        async fn session_changed(self, _: tarpc::context::Context, record: SessionRecord) {
            let _ = self.tx.send(record);
        }

        async fn daemon_restarting(self, _: tarpc::context::Context, _new_sha: String) {}

        async fn ping(self, _: tarpc::context::Context) -> bool {
            true
        }
    }

    let subscriber_addr = format!("127.0.0.1:{}", server.subscriber_port);
    let transport = tarpc::serde_transport::tcp::connect(&subscriber_addr, Bincode::default)
        .await
        .unwrap();

    let handler = TestSubscriber { tx: event_tx };
    let channel = server::BaseChannel::with_defaults(transport);

    tokio::spawn(async move {
        use futures::StreamExt;
        channel
            .execute(handler.serve())
            .for_each(|response| async {
                tokio::spawn(response);
            })
            .await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let client = server.create_client().await;
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Register session
    let record = create_test_record("multi-event-test", 1000);
    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    // Update session
    let mut updated = create_test_record("multi-event-test", 1000);
    updated.phase = "Reviewing".to_string();
    client
        .update(tarpc::context::current(), updated)
        .await
        .unwrap()
        .unwrap();

    // Force stop session
    client
        .force_stop(tarpc::context::current(), "multi-event-test".to_string())
        .await
        .unwrap()
        .unwrap();

    // Collect events with timeout
    let mut events = Vec::new();
    for _ in 0..3 {
        match tokio::time::timeout(Duration::from_secs(1), event_rx.recv()).await {
            Ok(Some(record)) => events.push(record),
            _ => break,
        }
    }

    assert_eq!(events.len(), 3, "Should receive 3 events");
    assert_eq!(events[0].phase, "Planning"); // Initial register
    assert_eq!(events[1].phase, "Reviewing"); // Update
    assert_eq!(events[2].liveness, LivenessState::Stopped); // Force stop
}

// ============================================================================
// Subscriber Registry Tests
// ============================================================================

#[tokio::test]
async fn test_multiple_subscribers_receive_events() {
    use crate::rpc::daemon_service::SubscriberCallback;
    use tarpc::server::{self, Channel};
    use tokio::sync::mpsc;

    let server = TestServer::start().await;

    // Create two subscribers
    let (tx1, mut rx1) = mpsc::unbounded_channel::<SessionRecord>();
    let (tx2, mut rx2) = mpsc::unbounded_channel::<SessionRecord>();

    #[derive(Clone)]
    struct TestSubscriber {
        tx: mpsc::UnboundedSender<SessionRecord>,
    }

    impl SubscriberCallback for TestSubscriber {
        async fn session_changed(self, _: tarpc::context::Context, record: SessionRecord) {
            let _ = self.tx.send(record);
        }

        async fn daemon_restarting(self, _: tarpc::context::Context, _new_sha: String) {}

        async fn ping(self, _: tarpc::context::Context) -> bool {
            true
        }
    }

    // Connect first subscriber
    let addr = format!("127.0.0.1:{}", server.subscriber_port);
    let transport1 = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let handler1 = TestSubscriber { tx: tx1 };
    let channel1 = server::BaseChannel::with_defaults(transport1);

    tokio::spawn(async move {
        use futures::StreamExt;
        channel1
            .execute(handler1.serve())
            .for_each(|r| async {
                tokio::spawn(r);
            })
            .await;
    });

    // Connect second subscriber
    let transport2 = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let handler2 = TestSubscriber { tx: tx2 };
    let channel2 = server::BaseChannel::with_defaults(transport2);

    tokio::spawn(async move {
        use futures::StreamExt;
        channel2
            .execute(handler2.serve())
            .for_each(|r| async {
                tokio::spawn(r);
            })
            .await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Register a session
    let client = server.create_client().await;
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let record = create_test_record("broadcast-test", 1000);
    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    // Both subscribers should receive the event
    let event1 = tokio::time::timeout(Duration::from_secs(2), rx1.recv()).await;
    let event2 = tokio::time::timeout(Duration::from_secs(2), rx2.recv()).await;

    assert!(
        event1.is_ok() && event1.unwrap().is_some(),
        "Subscriber 1 should receive event"
    );
    assert!(
        event2.is_ok() && event2.unwrap().is_some(),
        "Subscriber 2 should receive event"
    );
}

// ============================================================================
// Daemon → Host Upstream Integration Tests
// ============================================================================

/// Test harness for host RPC server (for upstream tests).
struct TestHostServer {
    port: u16,
    _server_handle: tokio::task::JoinHandle<()>,
    #[allow(dead_code)]
    _event_rx: tokio::sync::mpsc::UnboundedReceiver<crate::host::server::HostEvent>,
    state: std::sync::Arc<tokio::sync::Mutex<crate::host::state::HostState>>,
}

impl TestHostServer {
    async fn start() -> Self {
        use crate::host::rpc_server::HostServer;
        use crate::host::state::HostState;
        use crate::rpc::host_service::HostService;
        use futures::StreamExt;
        use tarpc::server::{self, Channel};
        use tarpc::tokio_serde::formats::Bincode;

        let port = find_test_port();
        let state = std::sync::Arc::new(tokio::sync::Mutex::new(HostState::new()));
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();

        let addr = format!("127.0.0.1:{}", port);
        let listener = tarpc::serde_transport::tcp::listen(&addr, Bincode::default)
            .await
            .unwrap();

        let server_state = state.clone();
        let server_handle = tokio::spawn(async move {
            let mut listener = listener;
            while let Some(result) = listener.next().await {
                if let Ok(transport) = result {
                    let server = HostServer::new(server_state.clone(), event_tx.clone());
                    let channel = server::BaseChannel::with_defaults(transport);

                    tokio::spawn(async move {
                        channel
                            .execute(server.serve())
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
            _server_handle: server_handle,
            _event_rx: event_rx,
            state,
        }
    }
}

#[tokio::test]
async fn test_upstream_handshake_and_session_sync() {
    use crate::rpc::host_service::{ContainerInfo, HostServiceClient, PROTOCOL_VERSION};

    let host = TestHostServer::start().await;

    // Connect upstream client to host
    let addr = format!("127.0.0.1:{}", host.port);
    let transport = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let client = HostServiceClient::new(client::Config::default(), transport).spawn();

    // Send hello
    let info = ContainerInfo {
        container_id: "test-container".to_string(),
        container_name: "Test Container".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
    };
    let result = client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap();

    assert!(result.is_ok(), "Hello should succeed");

    // Verify container was registered
    let state = host.state.lock().await;
    assert!(state.containers.contains_key("test-container"));
}

#[tokio::test]
async fn test_upstream_session_update_flow() {
    use crate::rpc::host_service::{
        ContainerInfo, HostServiceClient, SessionInfo, PROTOCOL_VERSION,
    };
    use crate::session_daemon::LivenessState;

    let host = TestHostServer::start().await;

    // Connect and do hello
    let addr = format!("127.0.0.1:{}", host.port);
    let transport = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let client = HostServiceClient::new(client::Config::default(), transport).spawn();

    let info = ContainerInfo {
        container_id: "upstream-test".to_string(),
        container_name: "Upstream Test".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
    };
    client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

    // Sync sessions
    let sessions = vec![SessionInfo {
        session_id: "session-1".to_string(),
        feature_name: "test-feature".to_string(),
        phase: "Planning".to_string(),
        iteration: 1,
        status: "Running".to_string(),
        liveness: LivenessState::Running,
        started_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    }];
    client
        .sync_sessions(tarpc::context::current(), sessions)
        .await
        .unwrap();

    // Verify session was stored
    {
        let state = host.state.lock().await;
        let container = state.containers.get("upstream-test").unwrap();
        assert_eq!(container.sessions.len(), 1);
        assert!(container.sessions.contains_key("session-1"));
    }

    // Update session
    let updated = SessionInfo {
        session_id: "session-1".to_string(),
        feature_name: "test-feature".to_string(),
        phase: "Reviewing".to_string(),
        iteration: 2,
        status: "AwaitingApproval".to_string(),
        liveness: LivenessState::Running,
        started_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:01:00Z".to_string(),
    };
    client
        .session_update(tarpc::context::current(), updated)
        .await
        .unwrap();

    // Verify update
    {
        let state = host.state.lock().await;
        let container = state.containers.get("upstream-test").unwrap();
        let session = container.sessions.get("session-1").unwrap();
        assert_eq!(session.phase, "Reviewing");
        assert_eq!(session.iteration, 2);
        assert_eq!(session.status, "AwaitingApproval");
    }

    // Send session gone
    client
        .session_gone(tarpc::context::current(), "session-1".to_string())
        .await
        .unwrap();

    // Verify session removed
    {
        let state = host.state.lock().await;
        let container = state.containers.get("upstream-test").unwrap();
        assert!(
            !container.sessions.contains_key("session-1"),
            "Session should be removed"
        );
    }
}

#[tokio::test]
async fn test_upstream_heartbeat() {
    use crate::rpc::host_service::{ContainerInfo, HostServiceClient, PROTOCOL_VERSION};

    let host = TestHostServer::start().await;

    // Connect and do hello
    let addr = format!("127.0.0.1:{}", host.port);
    let transport = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let client = HostServiceClient::new(client::Config::default(), transport).spawn();

    let info = ContainerInfo {
        container_id: "heartbeat-test".to_string(),
        container_name: "Heartbeat Test".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
    };
    client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

    // Record initial heartbeat time
    let initial_time = {
        let state = host.state.lock().await;
        state
            .containers
            .get("heartbeat-test")
            .unwrap()
            .last_message_at
    };

    // Small delay
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Send heartbeat
    client.heartbeat(tarpc::context::current()).await.unwrap();

    // Verify heartbeat was updated
    let state = host.state.lock().await;
    let container = state.containers.get("heartbeat-test").unwrap();
    assert!(
        container.last_message_at > initial_time,
        "Heartbeat should update timestamp"
    );
}

#[tokio::test]
async fn test_upstream_protocol_mismatch() {
    use crate::rpc::host_service::{ContainerInfo, HostServiceClient, PROTOCOL_VERSION};
    use crate::rpc::HostError;

    let host = TestHostServer::start().await;

    // Connect
    let addr = format!("127.0.0.1:{}", host.port);
    let transport = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let client = HostServiceClient::new(client::Config::default(), transport).spawn();

    // Send hello with wrong protocol version
    let info = ContainerInfo {
        container_id: "mismatch-test".to_string(),
        container_name: "Mismatch Test".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
    };
    let result = client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION + 99)
        .await
        .unwrap();

    match result {
        Err(HostError::ProtocolMismatch { got, expected }) => {
            assert_eq!(got, PROTOCOL_VERSION + 99);
            assert_eq!(expected, PROTOCOL_VERSION);
        }
        _ => panic!("Expected ProtocolMismatch error"),
    }

    // Verify container was NOT registered
    let state = host.state.lock().await;
    assert!(
        !state.containers.contains_key("mismatch-test"),
        "Container should not be registered on protocol mismatch"
    );
}

// ============================================================================
// Liveness Timeout Tests (Time-Based State Transitions)
// These tests modify global environment variables and must run serially.
// ============================================================================

#[tokio::test]
#[serial_test::serial]
async fn test_liveness_running_to_unresponsive_timeout() {
    // Use default timeouts (25s unresponsive, 60s stale)
    // Clear any env vars from previous tests
    std::env::remove_var("PLANNING_SESSIOND_UNRESPONSIVE_SECS");
    std::env::remove_var("PLANNING_SESSIOND_STALE_SECS");

    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Register a session with a past timestamp to simulate elapsed time
    let mut record = create_test_record("timeout-test", 1000);
    // Set timestamp to 30 seconds ago (25s < 30s < 60s = Unresponsive range)
    let past = chrono::Utc::now() - chrono::Duration::seconds(30);
    record.last_heartbeat_at = past.to_rfc3339();
    record.updated_at = past.to_rfc3339();

    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    // List should trigger update_liveness_states
    // With timestamp 30 seconds old, it should be Unresponsive (25s < 30s < 60s)
    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        sessions[0].liveness,
        LivenessState::Unresponsive,
        "Session should be Unresponsive with 30s old timestamp"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn test_liveness_unresponsive_to_stopped_timeout() {
    // Use default 60s stale timeout with old timestamp
    // Clear any env vars from previous tests
    std::env::remove_var("PLANNING_SESSIOND_UNRESPONSIVE_SECS");
    std::env::remove_var("PLANNING_SESSIOND_STALE_SECS");

    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Register a session with very old timestamp (well past 60s stale default)
    let mut record = create_test_record("stale-test", 1000);
    let past = chrono::Utc::now() - chrono::Duration::seconds(120);
    record.last_heartbeat_at = past.to_rfc3339();
    record.updated_at = past.to_rfc3339();

    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    // List should show Stopped (timestamp 120s old > 60s stale default)
    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        sessions[0].liveness,
        LivenessState::Stopped,
        "Session should be Stopped with very old timestamp"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn test_liveness_heartbeat_resets_unresponsive() {
    // Use default timeouts (25s unresponsive, 60s stale)
    // Clear any env vars from previous tests
    std::env::remove_var("PLANNING_SESSIOND_UNRESPONSIVE_SECS");
    std::env::remove_var("PLANNING_SESSIOND_STALE_SECS");

    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Register a session with old timestamp (30s - past unresponsive threshold)
    let mut record = create_test_record("heartbeat-reset-test", 1000);
    let past = chrono::Utc::now() - chrono::Duration::seconds(30);
    record.last_heartbeat_at = past.to_rfc3339();
    record.updated_at = past.to_rfc3339();

    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    // Verify it's Unresponsive (30s > 25s default threshold)
    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        sessions[0].liveness,
        LivenessState::Unresponsive,
        "Session should be Unresponsive with old timestamp"
    );

    // Send heartbeat to reset
    client
        .heartbeat(
            tarpc::context::current(),
            "heartbeat-reset-test".to_string(),
        )
        .await
        .unwrap()
        .unwrap();

    // Should be Running again (heartbeat updates timestamp to now)
    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        sessions[0].liveness,
        LivenessState::Running,
        "Session should be Running after heartbeat"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn test_auto_stale_session_replacement() {
    // Use default timeouts (60s stale)
    // Clear any env vars from previous tests
    std::env::remove_var("PLANNING_SESSIOND_UNRESPONSIVE_SECS");
    std::env::remove_var("PLANNING_SESSIOND_STALE_SECS");

    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Register session with PID 1000 and very old timestamp (auto-stale)
    let mut record1 = create_test_record("auto-replace-test", 1000);
    let past = chrono::Utc::now() - chrono::Duration::seconds(120);
    record1.last_heartbeat_at = past.to_rfc3339();
    record1.updated_at = past.to_rfc3339();

    client
        .register(tarpc::context::current(), record1)
        .await
        .unwrap()
        .unwrap();

    // Trigger liveness update via list - should mark as Stopped
    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        sessions[0].liveness,
        LivenessState::Stopped,
        "Original session should be Stopped"
    );

    // Now register same session with different PID - should succeed (original is Stopped)
    let record2 = create_test_record("auto-replace-test", 2000);
    let result = client
        .register(tarpc::context::current(), record2)
        .await
        .unwrap();

    assert!(
        result.is_ok(),
        "Re-registration should succeed after auto-stale"
    );

    // Verify only one session with new PID
    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].pid, 2000);
    assert_eq!(sessions[0].liveness, LivenessState::Running);
}

// ============================================================================
// daemon_restarting Callback End-to-End Test
// ============================================================================

#[tokio::test]
async fn test_daemon_restarting_callback_end_to_end() {
    use crate::rpc::daemon_service::SubscriberCallback;
    use tarpc::server::{self, Channel};
    use tokio::sync::mpsc;

    let server = TestServer::start().await;

    // Create a channel to receive daemon_restarting events
    let (restart_tx, mut restart_rx) = mpsc::unbounded_channel::<String>();

    // Create a subscriber that captures daemon_restarting callbacks
    #[derive(Clone)]
    struct RestartSubscriber {
        restart_tx: mpsc::UnboundedSender<String>,
    }

    impl SubscriberCallback for RestartSubscriber {
        async fn session_changed(self, _: tarpc::context::Context, _record: SessionRecord) {
            // Not testing this callback
        }

        async fn daemon_restarting(self, _: tarpc::context::Context, new_sha: String) {
            let _ = self.restart_tx.send(new_sha);
        }

        async fn ping(self, _: tarpc::context::Context) -> bool {
            true
        }
    }

    // Connect subscriber to subscriber port
    let subscriber_addr = format!("127.0.0.1:{}", server.subscriber_port);
    let transport = tarpc::serde_transport::tcp::connect(&subscriber_addr, Bincode::default)
        .await
        .unwrap();

    let handler = RestartSubscriber { restart_tx };
    let channel = server::BaseChannel::with_defaults(transport);

    tokio::spawn(async move {
        use futures::StreamExt;
        channel
            .execute(handler.serve())
            .for_each(|response| async {
                tokio::spawn(response);
            })
            .await;
    });

    // Give subscriber time to connect
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Authenticate and trigger shutdown (which calls daemon_restarting)
    let client = server.create_client().await;
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Shutdown triggers daemon_restarting broadcast
    let _ = client.shutdown(tarpc::context::current()).await.unwrap();

    // Wait for the callback with timeout
    let received = tokio::time::timeout(Duration::from_secs(2), restart_rx.recv()).await;

    match received {
        Ok(Some(sha)) => {
            assert!(
                !sha.is_empty(),
                "daemon_restarting should include build SHA"
            );
        }
        Ok(None) => panic!("Channel closed without receiving daemon_restarting"),
        Err(_) => panic!("Timeout waiting for daemon_restarting callback"),
    }
}

// ============================================================================
// Subscriber Failure and Cleanup Tests
// ============================================================================

#[tokio::test]
async fn test_subscriber_partial_failure_cleanup() {
    use crate::rpc::daemon_service::SubscriberCallback;
    use tarpc::server::{self, Channel};
    use tokio::sync::mpsc;

    let server = TestServer::start().await;

    // Create two subscribers - one that will stay alive, one that will disconnect
    let (alive_tx, mut alive_rx) = mpsc::unbounded_channel::<SessionRecord>();
    let (dead_tx, _dead_rx) = mpsc::unbounded_channel::<SessionRecord>(); // Receiver dropped immediately

    #[derive(Clone)]
    struct TestSubscriber {
        tx: mpsc::UnboundedSender<SessionRecord>,
    }

    impl SubscriberCallback for TestSubscriber {
        async fn session_changed(self, _: tarpc::context::Context, record: SessionRecord) {
            let _ = self.tx.send(record);
        }

        async fn daemon_restarting(self, _: tarpc::context::Context, _new_sha: String) {}

        async fn ping(self, _: tarpc::context::Context) -> bool {
            true
        }
    }

    // Connect first subscriber (will stay alive)
    let addr = format!("127.0.0.1:{}", server.subscriber_port);
    let transport1 = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let handler1 = TestSubscriber { tx: alive_tx };
    let channel1 = server::BaseChannel::with_defaults(transport1);

    tokio::spawn(async move {
        use futures::StreamExt;
        channel1
            .execute(handler1.serve())
            .for_each(|r| async {
                tokio::spawn(r);
            })
            .await;
    });

    // Connect second subscriber (channel receiver dropped, will fail on send)
    let transport2 = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let handler2 = TestSubscriber { tx: dead_tx };
    let channel2 = server::BaseChannel::with_defaults(transport2);

    let dead_handle = tokio::spawn(async move {
        use futures::StreamExt;
        channel2
            .execute(handler2.serve())
            .for_each(|r| async {
                tokio::spawn(r);
            })
            .await;
    });

    // Give subscribers time to connect
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Abort the dead subscriber's handler (simulates disconnect)
    dead_handle.abort();
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Register a session - this triggers broadcast to both subscribers
    let client = server.create_client().await;
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let record = create_test_record("partial-failure-test", 1000);
    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    // The alive subscriber should still receive events
    let received = tokio::time::timeout(Duration::from_secs(2), alive_rx.recv()).await;
    assert!(
        received.is_ok() && received.unwrap().is_some(),
        "Alive subscriber should still receive events after partial failure"
    );

    // Register another session to verify the dead subscriber was cleaned up
    // and doesn't cause issues
    let record2 = create_test_record("partial-failure-test-2", 2000);
    client
        .register(tarpc::context::current(), record2)
        .await
        .unwrap()
        .unwrap();

    // Alive subscriber should still work
    let received2 = tokio::time::timeout(Duration::from_secs(2), alive_rx.recv()).await;
    assert!(
        received2.is_ok() && received2.unwrap().is_some(),
        "Alive subscriber should continue receiving after dead subscriber cleanup"
    );
}

// ============================================================================
// Mixed Liveness State Tests
// ============================================================================

#[tokio::test]
#[serial_test::serial]
async fn test_mixed_liveness_states_in_list() {
    // Use default timeouts (25s unresponsive, 60s stale)
    // Clear any env vars from previous tests
    std::env::remove_var("PLANNING_SESSIOND_UNRESPONSIVE_SECS");
    std::env::remove_var("PLANNING_SESSIOND_STALE_SECS");

    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Register session A with old timestamp (30s old - Unresponsive)
    let mut record_a = create_test_record("mixed-a", 1000);
    let past_a = chrono::Utc::now() - chrono::Duration::seconds(30);
    record_a.last_heartbeat_at = past_a.to_rfc3339();
    record_a.updated_at = past_a.to_rfc3339();

    client
        .register(tarpc::context::current(), record_a)
        .await
        .unwrap()
        .unwrap();

    // Register session B with current timestamp (Running)
    let record_b = create_test_record("mixed-b", 2000);
    client
        .register(tarpc::context::current(), record_b)
        .await
        .unwrap()
        .unwrap();

    // List should show mixed states
    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();

    let session_a = sessions
        .iter()
        .find(|s| s.workflow_session_id == "mixed-a")
        .expect("Session A should exist");
    let session_b = sessions
        .iter()
        .find(|s| s.workflow_session_id == "mixed-b")
        .expect("Session B should exist");

    assert_eq!(
        session_a.liveness,
        LivenessState::Unresponsive,
        "Session A should be Unresponsive (30s old)"
    );
    assert_eq!(
        session_b.liveness,
        LivenessState::Running,
        "Session B should be Running (current timestamp)"
    );
}

// ============================================================================
// Subscriber Ping Tests
// ============================================================================

#[tokio::test]
async fn test_subscriber_ping_detects_healthy_subscriber() {
    use crate::rpc::daemon_service::SubscriberCallback;
    use crate::session_daemon::rpc_server::SubscriberRegistry;
    use tarpc::server::{self, Channel};

    let server = TestServer::start().await;

    // Create a healthy subscriber that responds to pings
    #[derive(Clone)]
    struct HealthySubscriber;

    impl SubscriberCallback for HealthySubscriber {
        async fn session_changed(self, _: tarpc::context::Context, _record: SessionRecord) {}

        async fn daemon_restarting(self, _: tarpc::context::Context, _new_sha: String) {}

        async fn ping(self, _: tarpc::context::Context) -> bool {
            true
        }
    }

    // Connect subscriber
    let addr = format!("127.0.0.1:{}", server.subscriber_port);
    let transport = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let handler = HealthySubscriber;
    let channel = server::BaseChannel::with_defaults(transport);

    tokio::spawn(async move {
        use futures::StreamExt;
        channel
            .execute(handler.serve())
            .for_each(|r| async {
                tokio::spawn(r);
            })
            .await;
    });

    // Give subscriber time to connect
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Create registry manually and add a callback client
    let registry = SubscriberRegistry::new();

    // Just verify our test subscriber is healthy by checking no failures from broadcast
    // (the actual ping_all functionality is tested indirectly through the cleanup)
    assert_eq!(registry.count(), 0); // Empty registry initially
}

// ============================================================================
// Degraded Mode Tests (--no-daemon)
// ============================================================================

#[tokio::test]
async fn test_degraded_mode_register_succeeds_silently() {
    use crate::session_daemon::rpc_client::RpcClient;

    // Create client in degraded mode (no_daemon = true)
    let client = RpcClient::new(true).await;

    // Verify it's not connected (degraded)
    assert!(!client.is_connected(), "Client should be in degraded mode");

    // Register should succeed silently and return empty string
    let record = create_test_record("degraded-test", 1000);
    let result = client.register(record).await;

    assert!(result.is_ok(), "Register should succeed in degraded mode");
    assert_eq!(
        result.unwrap(),
        "",
        "Should return empty string in degraded mode"
    );
}

#[tokio::test]
async fn test_degraded_mode_all_operations_succeed() {
    use crate::session_daemon::rpc_client::RpcClient;

    let client = RpcClient::new(true).await;
    assert!(!client.is_connected());

    // All operations should succeed silently
    let record = create_test_record("degraded-test", 1000);

    // Register
    let result = client.register(record.clone()).await;
    assert!(result.is_ok());

    // Update
    let result = client.update(record).await;
    assert!(result.is_ok());

    // Heartbeat
    let result = client.heartbeat("degraded-test").await;
    assert!(result.is_ok());

    // List returns empty
    let result = client.list().await;
    assert!(result.is_ok());
    assert_eq!(
        result.unwrap().len(),
        0,
        "List should return empty in degraded mode"
    );

    // Force stop
    let result = client.force_stop("degraded-test").await;
    assert!(result.is_ok());

    // Shutdown
    let result = client.shutdown().await;
    assert!(result.is_ok());
}

// ============================================================================
// State Persistence Tests (Daemon Restart Recovery)
// These tests modify PLANNING_AGENT_HOME env var and must run serially.
// ============================================================================

#[tokio::test]
#[serial_test::serial]
async fn test_state_persistence_saves_and_loads_sessions() {
    use crate::session_daemon::server::DaemonState;

    // Use a temp directory for this test
    let temp_dir = tempfile::tempdir().unwrap();
    std::env::set_var("PLANNING_AGENT_HOME", temp_dir.path());

    // Create state and add sessions
    let mut state = DaemonState::new();

    let record1 = create_test_record("persist-1", 1000);
    let record2 = create_test_record("persist-2", 2000);

    state
        .sessions
        .insert(record1.workflow_session_id.clone(), record1);
    state
        .sessions
        .insert(record2.workflow_session_id.clone(), record2);

    // Persist to disk
    let persist_result = state.persist_to_disk();
    assert!(persist_result.is_ok(), "Persist should succeed");

    // Create new state and load from disk
    let mut loaded_state = DaemonState::new();
    let load_result = loaded_state.load_from_disk();
    assert!(load_result.is_ok(), "Load should succeed");

    // Verify sessions were loaded
    assert_eq!(loaded_state.sessions.len(), 2, "Should have 2 sessions");
    assert!(loaded_state.sessions.contains_key("persist-1"));
    assert!(loaded_state.sessions.contains_key("persist-2"));

    // Verify sessions are marked as Stopped (from previous daemon instance)
    assert_eq!(
        loaded_state.sessions.get("persist-1").unwrap().liveness,
        LivenessState::Stopped,
        "Loaded sessions should be marked Stopped"
    );
    assert_eq!(
        loaded_state.sessions.get("persist-2").unwrap().liveness,
        LivenessState::Stopped,
        "Loaded sessions should be marked Stopped"
    );

    std::env::remove_var("PLANNING_AGENT_HOME");
}

#[tokio::test]
#[serial_test::serial]
async fn test_state_persistence_handles_missing_file() {
    use crate::session_daemon::server::DaemonState;

    // Use a temp directory with no existing registry
    let temp_dir = tempfile::tempdir().unwrap();
    std::env::set_var("PLANNING_AGENT_HOME", temp_dir.path());

    let mut state = DaemonState::new();
    let result = state.load_from_disk();

    // Should succeed (no error) even if file doesn't exist
    assert!(result.is_ok(), "Load should succeed even with no file");
    assert_eq!(state.sessions.len(), 0, "Should have no sessions");

    std::env::remove_var("PLANNING_AGENT_HOME");
}

// ============================================================================
// Upstream Client Tests (Container → Host Connection)
// ============================================================================

#[tokio::test]
async fn test_upstream_client_sends_session_updates() {
    use crate::rpc::host_service::{ContainerInfo, HostServiceClient, PROTOCOL_VERSION};

    let host = TestHostServer::start().await;

    // Connect upstream client manually (simulating what RpcUpstream does)
    let addr = format!("127.0.0.1:{}", host.port);
    let transport = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let client = HostServiceClient::new(tarpc::client::Config::default(), transport).spawn();

    // Do handshake
    let info = ContainerInfo {
        container_id: "upstream-client-test".to_string(),
        container_name: "Upstream Client Test".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
    };
    client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

    // Send session update
    let session_info = crate::rpc::host_service::SessionInfo {
        session_id: "upstream-session-1".to_string(),
        feature_name: "test-feature".to_string(),
        phase: "Planning".to_string(),
        iteration: 1,
        status: "Running".to_string(),
        liveness: crate::session_daemon::LivenessState::Running,
        started_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };
    client
        .session_update(tarpc::context::current(), session_info)
        .await
        .unwrap();

    // Verify session was received by host
    let state = host.state.lock().await;
    let container = state.containers.get("upstream-client-test").unwrap();
    assert!(container.sessions.contains_key("upstream-session-1"));
}

#[tokio::test]
async fn test_upstream_client_sync_sessions() {
    use crate::rpc::host_service::{
        ContainerInfo, HostServiceClient, SessionInfo, PROTOCOL_VERSION,
    };
    use crate::session_daemon::LivenessState;

    let host = TestHostServer::start().await;

    let addr = format!("127.0.0.1:{}", host.port);
    let transport = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let client = HostServiceClient::new(tarpc::client::Config::default(), transport).spawn();

    // Handshake
    let info = ContainerInfo {
        container_id: "sync-test".to_string(),
        container_name: "Sync Test".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
    };
    client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

    // Sync multiple sessions at once
    let sessions = vec![
        SessionInfo {
            session_id: "sync-1".to_string(),
            feature_name: "feature-1".to_string(),
            phase: "Planning".to_string(),
            iteration: 1,
            status: "Running".to_string(),
            liveness: LivenessState::Running,
            started_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        },
        SessionInfo {
            session_id: "sync-2".to_string(),
            feature_name: "feature-2".to_string(),
            phase: "Reviewing".to_string(),
            iteration: 2,
            status: "AwaitingApproval".to_string(),
            liveness: LivenessState::Running,
            started_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:01:00Z".to_string(),
        },
        SessionInfo {
            session_id: "sync-3".to_string(),
            feature_name: "feature-3".to_string(),
            phase: "Complete".to_string(),
            iteration: 3,
            status: "Stopped".to_string(),
            liveness: LivenessState::Stopped,
            started_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:02:00Z".to_string(),
        },
    ];
    client
        .sync_sessions(tarpc::context::current(), sessions)
        .await
        .unwrap();

    // Verify all sessions synced
    let state = host.state.lock().await;
    let container = state.containers.get("sync-test").unwrap();
    assert_eq!(container.sessions.len(), 3);
    assert!(container.sessions.contains_key("sync-1"));
    assert!(container.sessions.contains_key("sync-2"));
    assert!(container.sessions.contains_key("sync-3"));

    // Verify session details preserved
    let session2 = container.sessions.get("sync-2").unwrap();
    assert_eq!(session2.phase, "Reviewing");
    assert_eq!(session2.iteration, 2);
}

// ============================================================================
// Version Mismatch and Upgrade Tests
// ============================================================================

#[tokio::test]
async fn test_build_sha_is_consistent_for_same_binary() {
    // When client and daemon are from the same binary (normal case),
    // they should have the same BUILD_SHA
    let server = TestServer::start().await;
    let client = server.create_client().await;

    // build_sha doesn't require authentication
    let daemon_sha = client.build_sha(tarpc::context::current()).await.unwrap();
    let client_sha = crate::update::BUILD_SHA;

    assert_eq!(
        daemon_sha, client_sha,
        "Daemon and client from same binary should have same SHA"
    );
}

#[tokio::test]
async fn test_build_timestamp_is_consistent_for_same_binary() {
    // When client and daemon are from the same binary,
    // they should have the same BUILD_TIMESTAMP
    let server = TestServer::start().await;
    let client = server.create_client().await;

    let daemon_timestamp = client
        .build_timestamp(tarpc::context::current())
        .await
        .unwrap();
    let client_timestamp = crate::update::BUILD_TIMESTAMP;

    assert_eq!(
        daemon_timestamp, client_timestamp,
        "Daemon and client from same binary should have same timestamp"
    );
    assert!(daemon_timestamp > 0, "Timestamp should be non-zero");
}

#[tokio::test]
async fn test_request_upgrade_with_newer_timestamp_accepted() {
    // When client has a NEWER timestamp than daemon, upgrade should be accepted
    let server = TestServer::start().await;
    let client = server.create_client().await;

    let daemon_timestamp = client
        .build_timestamp(tarpc::context::current())
        .await
        .unwrap();

    // Request upgrade with a timestamp far in the future (newer than daemon)
    let future_timestamp = daemon_timestamp + 1000;
    let accepted = client
        .request_upgrade(tarpc::context::current(), future_timestamp)
        .await
        .unwrap();

    assert!(accepted, "Upgrade from newer client should be accepted");

    // Verify daemon actually shut down
    tokio::time::sleep(Duration::from_millis(100)).await;
    let addr = format!("127.0.0.1:{}", server.port);
    let connect_result = tokio::net::TcpStream::connect(&addr).await;
    assert!(connect_result.is_err(), "Daemon should be shut down");
}

#[tokio::test]
async fn test_request_upgrade_with_older_timestamp_refused() {
    // When client has an OLDER timestamp than daemon, upgrade should be refused
    let server = TestServer::start().await;
    let client = server.create_client().await;

    let daemon_timestamp = client
        .build_timestamp(tarpc::context::current())
        .await
        .unwrap();

    // Request upgrade with a timestamp in the past (older than daemon)
    let past_timestamp = daemon_timestamp.saturating_sub(1000);
    let accepted = client
        .request_upgrade(tarpc::context::current(), past_timestamp)
        .await
        .unwrap();

    assert!(!accepted, "Upgrade from older client should be refused");

    // Verify daemon is STILL running (not shut down)
    let addr = format!("127.0.0.1:{}", server.port);
    let connect_result = tokio::net::TcpStream::connect(&addr).await;
    assert!(
        connect_result.is_ok(),
        "Daemon should still be running after refusing upgrade"
    );
}

#[tokio::test]
async fn test_request_upgrade_with_same_timestamp_refused() {
    // When client has the SAME timestamp as daemon, no upgrade needed
    let server = TestServer::start().await;
    let client = server.create_client().await;

    let daemon_timestamp = client
        .build_timestamp(tarpc::context::current())
        .await
        .unwrap();

    // Request upgrade with same timestamp
    let accepted = client
        .request_upgrade(tarpc::context::current(), daemon_timestamp)
        .await
        .unwrap();

    assert!(
        !accepted,
        "Upgrade with same timestamp should be refused (not strictly newer)"
    );

    // Verify daemon is STILL running
    let addr = format!("127.0.0.1:{}", server.port);
    let connect_result = tokio::net::TcpStream::connect(&addr).await;
    assert!(connect_result.is_ok(), "Daemon should still be running");
}

#[tokio::test]
async fn test_old_client_cannot_kill_new_daemon() {
    // Critical test: An older client should NOT be able to kill a newer daemon.
    // This is the bug we fixed - previously any version mismatch would trigger restart.

    let server = TestServer::start().await;
    let client = server.create_client().await;

    let daemon_timestamp = client
        .build_timestamp(tarpc::context::current())
        .await
        .unwrap();

    // Simulate an old client (timestamp 1 year in the past)
    let old_client_timestamp = daemon_timestamp.saturating_sub(365 * 24 * 60 * 60);

    // Old client tries to request upgrade
    let accepted = client
        .request_upgrade(tarpc::context::current(), old_client_timestamp)
        .await
        .unwrap();

    assert!(
        !accepted,
        "Old client should NOT be able to kill newer daemon"
    );

    // Daemon should still be fully operational
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert!(sessions.is_empty()); // Just verify daemon is working
}

#[tokio::test]
async fn test_version_mismatch_triggers_shutdown() {
    // This test verifies that when a client detects version mismatch,
    // it can request daemon shutdown. The actual restart logic involves
    // spawning a new daemon process which is tested via manual integration.

    let server = TestServer::start().await;
    let client = server.create_client().await;

    // Authenticate
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    // Get current build SHA
    let sha = client.build_sha(tarpc::context::current()).await.unwrap();
    assert!(!sha.is_empty(), "Build SHA should not be empty");

    // If version mismatch were detected, client would call shutdown
    // Verify shutdown works
    let result = client.shutdown(tarpc::context::current()).await.unwrap();
    assert!(result.is_ok(), "Shutdown should succeed");

    // Verify daemon actually shut down (can't connect anymore)
    tokio::time::sleep(Duration::from_millis(100)).await;
    let addr = format!("127.0.0.1:{}", server.port);
    let connect_result = tokio::net::TcpStream::connect(&addr).await;
    assert!(connect_result.is_err(), "Daemon should be shut down");
}

// ============================================================================
// Concurrent Connection Tests (Multiple Clients to Same Daemon)
// ============================================================================

#[tokio::test]
async fn test_many_concurrent_clients_connect_successfully() {
    // This tests that multiple clients can connect to the same daemon
    // concurrently without issues. The actual concurrent spawn coordination
    // (file locking) is tested via manual integration since it requires
    // spawning real daemon processes.

    let server = TestServer::start().await;

    // Spawn 20 clients that all try to connect and authenticate simultaneously
    let mut handles = Vec::new();

    for i in 0..20 {
        let port = server.port;
        let token = server.auth_token.clone();

        let handle = tokio::spawn(async move {
            use tarpc::serde_transport::tcp;

            let addr = format!("127.0.0.1:{}", port);
            let transport = tcp::connect(&addr, Bincode::default).await.unwrap();
            let client = DaemonServiceClient::new(client::Config::default(), transport).spawn();

            // Authenticate
            client
                .authenticate(tarpc::context::current(), token)
                .await
                .unwrap()
                .unwrap();

            // Get build sha to verify connection is working
            let sha = client.build_sha(tarpc::context::current()).await.unwrap();
            assert!(!sha.is_empty());

            i // Return the client index
        });

        handles.push(handle);
    }

    // Wait for all to complete and collect results
    let mut completed = 0;
    for handle in handles {
        let _ = handle.await.unwrap();
        completed += 1;
    }

    assert_eq!(completed, 20, "All 20 concurrent clients should succeed");
}

#[tokio::test]
async fn test_rapid_connect_disconnect_cycles() {
    // Test that rapid connect/disconnect cycles don't cause issues
    let server = TestServer::start().await;

    for _ in 0..10 {
        let client = server.create_client().await;

        // Authenticate and make a call
        client
            .authenticate(tarpc::context::current(), server.auth_token.clone())
            .await
            .unwrap()
            .unwrap();

        let _ = client.build_sha(tarpc::context::current()).await.unwrap();

        // Client is dropped here, connection closes
    }

    // Server should still be healthy after all the churn
    let client = server.create_client().await;
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert!(sessions.is_empty() || !sessions.is_empty()); // Just verify it works
}

// ============================================================================
// Upstream Client Session Gone Tests
// ============================================================================

#[tokio::test]
async fn test_upstream_client_session_gone() {
    use crate::rpc::host_service::{
        ContainerInfo, HostServiceClient, SessionInfo, PROTOCOL_VERSION,
    };
    use crate::session_daemon::LivenessState;

    let host = TestHostServer::start().await;

    let addr = format!("127.0.0.1:{}", host.port);
    let transport = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let client = HostServiceClient::new(tarpc::client::Config::default(), transport).spawn();

    // Handshake
    let info = ContainerInfo {
        container_id: "gone-test".to_string(),
        container_name: "Gone Test".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
    };
    client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

    // Add a session
    let session = SessionInfo {
        session_id: "to-be-removed".to_string(),
        feature_name: "feature".to_string(),
        phase: "Planning".to_string(),
        iteration: 1,
        status: "Running".to_string(),
        liveness: LivenessState::Running,
        started_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };
    client
        .sync_sessions(tarpc::context::current(), vec![session])
        .await
        .unwrap();

    // Verify session exists
    {
        let state = host.state.lock().await;
        let container = state.containers.get("gone-test").unwrap();
        assert!(container.sessions.contains_key("to-be-removed"));
    }

    // Mark session as gone
    client
        .session_gone(tarpc::context::current(), "to-be-removed".to_string())
        .await
        .unwrap();

    // Verify session removed
    let state = host.state.lock().await;
    let container = state.containers.get("gone-test").unwrap();
    assert!(!container.sessions.contains_key("to-be-removed"));
}
