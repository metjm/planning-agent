//! Host/upstream connection tests for the RPC daemon.

use super::TestHostServer;
use crate::rpc::host_service::{ContainerInfo, HostServiceClient, SessionInfo, PROTOCOL_VERSION};
use crate::rpc::HostError;
use crate::session_daemon::LivenessState;
use tarpc::client;
use tarpc::tokio_serde::formats::Bincode;

use crate::host::rpc_server::HostEvent;
use std::time::Duration;

#[tokio::test]
async fn test_upstream_handshake_and_session_sync() {
    let mut host = TestHostServer::start().await;

    let addr = format!("127.0.0.1:{}", host.port);
    let transport = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let client = HostServiceClient::new(client::Config::default(), transport).spawn();

    let info = ContainerInfo {
        container_id: "test-container".to_string(),
        container_name: "Test Container".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
        git_sha: "test123".to_string(),
        build_timestamp: 1234567890,
        file_service_port: 0,
    };
    let result = client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap();

    assert!(result.is_ok(), "Hello should succeed");

    // Verify ContainerConnected event was sent
    let event = tokio::time::timeout(Duration::from_secs(1), host.event_rx.recv())
        .await
        .expect("Event timeout")
        .expect("Event channel closed");
    match event {
        HostEvent::ContainerConnected {
            container_id,
            container_name,
        } => {
            assert_eq!(container_id, "test-container");
            assert_eq!(container_name, "Test Container");
        }
        _ => panic!("Expected ContainerConnected event, got {:?}", event),
    }

    let state = host.state.lock().await;
    assert!(state.containers.contains_key("test-container"));
}

#[tokio::test]
async fn test_upstream_session_update_flow() {
    let host = TestHostServer::start().await;

    let addr = format!("127.0.0.1:{}", host.port);
    let transport = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let client = HostServiceClient::new(client::Config::default(), transport).spawn();

    let info = ContainerInfo {
        container_id: "upstream-test".to_string(),
        container_name: "Upstream Test".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
        git_sha: "test123".to_string(),
        build_timestamp: 1234567890,
        file_service_port: 0,
    };
    client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

    let sessions = vec![SessionInfo {
        session_id: "session-1".to_string(),
        feature_name: "test-feature".to_string(),
        phase: "Planning".to_string(),
        iteration: 1,
        status: "Running".to_string(),
        liveness: LivenessState::Running,
        started_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
        pid: 0,
    }];
    client
        .sync_sessions(tarpc::context::current(), sessions)
        .await
        .unwrap();

    {
        let state = host.state.lock().await;
        let container = state.containers.get("upstream-test").unwrap();
        assert_eq!(container.sessions.len(), 1);
        assert!(container.sessions.contains_key("session-1"));
    }

    let updated = SessionInfo {
        session_id: "session-1".to_string(),
        feature_name: "test-feature".to_string(),
        phase: "Reviewing".to_string(),
        iteration: 2,
        status: "AwaitingApproval".to_string(),
        liveness: LivenessState::Running,
        started_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:01:00Z".to_string(),
        pid: 0,
    };
    client
        .session_update(tarpc::context::current(), updated)
        .await
        .unwrap();

    {
        let state = host.state.lock().await;
        let container = state.containers.get("upstream-test").unwrap();
        let session = container.sessions.get("session-1").unwrap();
        assert_eq!(session.phase, "Reviewing");
        assert_eq!(session.iteration, 2);
        assert_eq!(session.status, "AwaitingApproval");
    }

    client
        .session_gone(tarpc::context::current(), "session-1".to_string())
        .await
        .unwrap();

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
    let host = TestHostServer::start().await;

    let addr = format!("127.0.0.1:{}", host.port);
    let transport = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let client = HostServiceClient::new(client::Config::default(), transport).spawn();

    let info = ContainerInfo {
        container_id: "heartbeat-test".to_string(),
        container_name: "Heartbeat Test".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
        git_sha: "test123".to_string(),
        build_timestamp: 1234567890,
        file_service_port: 0,
    };
    client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

    // Heartbeat should not error
    client.heartbeat(tarpc::context::current()).await.unwrap();

    // Verify container still exists
    let state = host.state.lock().await;
    assert!(
        state.containers.contains_key("heartbeat-test"),
        "Container should still exist after heartbeat"
    );
}

#[tokio::test]
async fn test_upstream_protocol_mismatch() {
    let host = TestHostServer::start().await;

    let addr = format!("127.0.0.1:{}", host.port);
    let transport = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let client = HostServiceClient::new(client::Config::default(), transport).spawn();

    let info = ContainerInfo {
        container_id: "mismatch-test".to_string(),
        container_name: "Mismatch Test".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
        git_sha: "test123".to_string(),
        build_timestamp: 1234567890,
        file_service_port: 0,
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

    let state = host.state.lock().await;
    assert!(
        !state.containers.contains_key("mismatch-test"),
        "Container should not be registered on protocol mismatch"
    );
}

#[tokio::test]
async fn test_upstream_client_sends_session_updates() {
    let host = TestHostServer::start().await;

    let addr = format!("127.0.0.1:{}", host.port);
    let transport = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let client = HostServiceClient::new(tarpc::client::Config::default(), transport).spawn();

    let info = ContainerInfo {
        container_id: "upstream-client-test".to_string(),
        container_name: "Upstream Client Test".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
        git_sha: "test123".to_string(),
        build_timestamp: 1234567890,
        file_service_port: 0,
    };
    client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

    let session_info = SessionInfo {
        session_id: "upstream-session-1".to_string(),
        feature_name: "test-feature".to_string(),
        phase: "Planning".to_string(),
        iteration: 1,
        status: "Running".to_string(),
        liveness: LivenessState::Running,
        started_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
        pid: 0,
    };
    client
        .session_update(tarpc::context::current(), session_info)
        .await
        .unwrap();

    let state = host.state.lock().await;
    let container = state.containers.get("upstream-client-test").unwrap();
    assert!(container.sessions.contains_key("upstream-session-1"));
}

#[tokio::test]
async fn test_upstream_client_sync_sessions() {
    let host = TestHostServer::start().await;

    let addr = format!("127.0.0.1:{}", host.port);
    let transport = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let client = HostServiceClient::new(tarpc::client::Config::default(), transport).spawn();

    let info = ContainerInfo {
        container_id: "sync-test".to_string(),
        container_name: "Sync Test".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
        git_sha: "test123".to_string(),
        build_timestamp: 1234567890,
        file_service_port: 0,
    };
    client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

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
            pid: 0,
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
            pid: 0,
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
            pid: 0,
        },
    ];
    client
        .sync_sessions(tarpc::context::current(), sessions)
        .await
        .unwrap();

    let state = host.state.lock().await;
    let container = state.containers.get("sync-test").unwrap();
    assert_eq!(container.sessions.len(), 3);
    assert!(container.sessions.contains_key("sync-1"));
    assert!(container.sessions.contains_key("sync-2"));
    assert!(container.sessions.contains_key("sync-3"));

    let session2 = container.sessions.get("sync-2").unwrap();
    assert_eq!(session2.phase, "Reviewing");
    assert_eq!(session2.iteration, 2);
}

#[tokio::test]
async fn test_upstream_client_session_gone() {
    let host = TestHostServer::start().await;

    let addr = format!("127.0.0.1:{}", host.port);
    let transport = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let client = HostServiceClient::new(tarpc::client::Config::default(), transport).spawn();

    let info = ContainerInfo {
        container_id: "gone-test".to_string(),
        container_name: "Gone Test".to_string(),
        working_dir: std::path::PathBuf::from("/work"),
        git_sha: "test123".to_string(),
        build_timestamp: 1234567890,
        file_service_port: 0,
    };
    client
        .hello(tarpc::context::current(), info, PROTOCOL_VERSION)
        .await
        .unwrap()
        .unwrap();

    let session = SessionInfo {
        session_id: "to-be-removed".to_string(),
        feature_name: "feature".to_string(),
        phase: "Planning".to_string(),
        iteration: 1,
        status: "Running".to_string(),
        liveness: LivenessState::Running,
        started_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
        pid: 0,
    };
    client
        .sync_sessions(tarpc::context::current(), vec![session])
        .await
        .unwrap();

    {
        let state = host.state.lock().await;
        let container = state.containers.get("gone-test").unwrap();
        assert!(container.sessions.contains_key("to-be-removed"));
    }

    client
        .session_gone(tarpc::context::current(), "to-be-removed".to_string())
        .await
        .unwrap();

    let state = host.state.lock().await;
    let container = state.containers.get("gone-test").unwrap();
    assert!(!container.sessions.contains_key("to-be-removed"));
}

/// Integration test: RpcUpstream connects to host and syncs sessions.
/// This tests the actual RpcUpstream implementation, not just the raw client.
#[tokio::test]
#[serial_test::serial]
async fn test_rpc_upstream_connects_and_syncs() {
    use crate::session_daemon::rpc_upstream::{RpcUpstream, UpstreamEvent};
    use crate::session_daemon::server::DaemonState;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::{mpsc, Mutex};

    let host = TestHostServer::start().await;

    // Create channel for upstream events
    let (tx, rx) = mpsc::unbounded_channel();

    // Create empty daemon state for upstream
    let daemon_state = Arc::new(Mutex::new(DaemonState::new()));

    // Create RpcUpstream pointing to test host
    // We need to set the PLANNING_AGENT_HOST_ADDRESS env var to point to our test server
    std::env::set_var("PLANNING_AGENT_HOST_ADDRESS", "127.0.0.1");
    std::env::set_var("PLANNING_AGENT_HOST_PORT", host.port.to_string());
    std::env::set_var("PLANNING_AGENT_CONTAINER_ID", "test-upstream-container");
    std::env::set_var("PLANNING_AGENT_CONTAINER_NAME", "Test Upstream Container");

    let upstream = RpcUpstream::new(host.port, daemon_state, 0);

    // Spawn upstream in background
    let upstream_handle = tokio::spawn(async move {
        upstream.run(rx).await;
    });

    // Send a session update event
    let session = crate::rpc::SessionRecord::new(
        "upstream-test-session".to_string(),
        "upstream-feature".to_string(),
        std::path::PathBuf::from("/work"),
        std::path::PathBuf::from("/work/sessions/upstream-test-session"),
        "Planning".to_string(),
        1,
        "Running".to_string(),
        12345,
    );
    tx.send(UpstreamEvent::SessionUpdate(session)).unwrap();

    // Wait for connection and sync to happen
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify container was registered in host state
    let state = host.state.lock().await;
    assert!(
        state.containers.contains_key("test-upstream-container"),
        "Container should be registered in host state. Got containers: {:?}",
        state.containers.keys().collect::<Vec<_>>()
    );

    // Verify session was synced
    let container = state.containers.get("test-upstream-container").unwrap();
    assert!(
        container.sessions.contains_key("upstream-test-session"),
        "Session should be synced to host"
    );

    // Cleanup
    upstream_handle.abort();
    std::env::remove_var("PLANNING_AGENT_HOST_ADDRESS");
    std::env::remove_var("PLANNING_AGENT_HOST_PORT");
    std::env::remove_var("PLANNING_AGENT_CONTAINER_ID");
    std::env::remove_var("PLANNING_AGENT_CONTAINER_NAME");
}

/// Full end-to-end test: Client → Daemon → Host
///
/// This test verifies that when a planning session registers with the daemon,
/// the session information flows all the way through to the host.
///
/// Components tested:
/// 1. Host server receives container connections and session updates
/// 2. Daemon receives client registrations and forwards to host
/// 3. Client can register sessions with the daemon
#[tokio::test]
#[serial_test::serial]
async fn test_full_stack_client_to_daemon_to_host() {
    use super::find_test_port;
    use crate::rpc::daemon_service::DaemonServiceClient;
    use crate::rpc::SessionRecord;
    use crate::session_daemon::rpc_server::{run_daemon_server, SubscriberRegistry};
    use crate::session_daemon::server::DaemonState;
    use std::sync::Arc;
    use tokio::sync::{broadcast, mpsc, Mutex, RwLock};

    // 1. Start the host server
    let host = TestHostServer::start().await;

    // 2. Start a daemon with upstream connection to the host
    let daemon_port = find_test_port();
    let auth_token = "test-auth-token".to_string();

    let daemon_state = Arc::new(Mutex::new(DaemonState::new()));
    let subscribers = Arc::new(RwLock::new(SubscriberRegistry::new()));
    let (shutdown_tx, _) = broadcast::channel(1);

    // Create upstream channel and spawn upstream connection
    let (upstream_tx, upstream_rx) = mpsc::unbounded_channel();

    // Set env vars for upstream to connect to our test host
    std::env::set_var("PLANNING_AGENT_HOST_ADDRESS", "127.0.0.1");
    std::env::set_var("PLANNING_AGENT_CONTAINER_ID", "e2e-test-container");
    std::env::set_var("PLANNING_AGENT_CONTAINER_NAME", "E2E Test Container");

    // Spawn upstream connection
    let upstream =
        crate::session_daemon::rpc_upstream::RpcUpstream::new(host.port, daemon_state.clone(), 0);
    let upstream_handle = tokio::spawn(async move {
        upstream.run(upstream_rx).await;
    });

    // Start daemon server
    let daemon_handle = {
        let state = daemon_state.clone();
        let subs = subscribers.clone();
        let shutdown = shutdown_tx.clone();
        let token = auth_token.clone();
        let tx = Some(upstream_tx.clone());
        tokio::spawn(async move {
            let _ = run_daemon_server(state, subs, shutdown, tx, token, daemon_port).await;
        })
    };

    // Give servers time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 3. Connect a client to the daemon and register a session
    let addr = format!("127.0.0.1:{}", daemon_port);
    let transport = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .expect("Failed to connect to daemon");
    let client = DaemonServiceClient::new(client::Config::default(), transport).spawn();

    // Authenticate
    client
        .authenticate(tarpc::context::current(), auth_token.clone())
        .await
        .expect("Auth RPC failed")
        .expect("Auth rejected");

    // Register a session
    let session = SessionRecord::new(
        "e2e-test-session".to_string(),
        "e2e-test-feature".to_string(),
        std::path::PathBuf::from("/work/e2e-test"),
        std::path::PathBuf::from("/work/sessions/e2e-test-session"),
        "Planning".to_string(),
        1,
        "Running".to_string(),
        99999,
    );
    client
        .register(tarpc::context::current(), session.clone())
        .await
        .expect("Register RPC failed")
        .expect("Register rejected");

    // 4. Wait for upstream to sync and verify host received the session
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Check host state
    let host_state = host.state.lock().await;

    // Verify container is registered
    assert!(
        host_state.containers.contains_key("e2e-test-container"),
        "Container should be registered in host. Got: {:?}",
        host_state.containers.keys().collect::<Vec<_>>()
    );

    // Verify session is in the container
    let container = host_state.containers.get("e2e-test-container").unwrap();
    assert!(
        container.sessions.contains_key("e2e-test-session"),
        "Session should be synced to host. Got sessions: {:?}",
        container.sessions.keys().collect::<Vec<_>>()
    );

    // Verify session details
    let host_session = container.sessions.get("e2e-test-session").unwrap();
    assert_eq!(host_session.feature_name, "e2e-test-feature");
    assert_eq!(host_session.phase, "Planning");
    assert_eq!(host_session.iteration, 1);

    // 5. Test session update flows through
    drop(host_state); // Release lock

    client
        .update(
            tarpc::context::current(),
            SessionRecord::new(
                "e2e-test-session".to_string(),
                "e2e-test-feature".to_string(),
                std::path::PathBuf::from("/work/e2e-test"),
                std::path::PathBuf::from("/work/sessions/e2e-test-session"),
                "Reviewing".to_string(),
                2,
                "AwaitingApproval".to_string(),
                99999,
            ),
        )
        .await
        .expect("Update RPC failed")
        .expect("Update rejected");

    tokio::time::sleep(Duration::from_millis(200)).await;

    let host_state = host.state.lock().await;
    let container = host_state.containers.get("e2e-test-container").unwrap();
    let host_session = container.sessions.get("e2e-test-session").unwrap();
    assert_eq!(host_session.phase, "Reviewing", "Phase should be updated");
    assert_eq!(host_session.iteration, 2, "Iteration should be updated");

    // Cleanup
    drop(host_state);
    let _ = shutdown_tx.send(());
    upstream_handle.abort();
    daemon_handle.abort();
    std::env::remove_var("PLANNING_AGENT_HOST_ADDRESS");
    std::env::remove_var("PLANNING_AGENT_CONTAINER_ID");
    std::env::remove_var("PLANNING_AGENT_CONTAINER_NAME");
}
