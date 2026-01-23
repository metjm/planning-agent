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
async fn test_rpc_upstream_connects_and_syncs() {
    use crate::session_daemon::rpc_upstream::{RpcUpstream, UpstreamEvent};
    use std::time::Duration;
    use tokio::sync::mpsc;

    let host = TestHostServer::start().await;

    // Create channel for upstream events
    let (tx, rx) = mpsc::unbounded_channel();

    // Create RpcUpstream pointing to test host
    // We need to set the PLANNING_AGENT_HOST_ADDRESS env var to point to our test server
    std::env::set_var("PLANNING_AGENT_HOST_ADDRESS", "127.0.0.1");
    std::env::set_var("PLANNING_AGENT_HOST_PORT", host.port.to_string());
    std::env::set_var("PLANNING_AGENT_CONTAINER_ID", "test-upstream-container");
    std::env::set_var("PLANNING_AGENT_CONTAINER_NAME", "Test Upstream Container");

    let upstream = RpcUpstream::new(host.port);

    // Spawn upstream in background
    let upstream_handle = tokio::spawn(async move {
        upstream.run(rx).await;
    });

    // Send a sync event
    let session = crate::rpc::SessionRecord::new(
        "upstream-test-session".to_string(),
        "upstream-feature".to_string(),
        std::path::PathBuf::from("/work"),
        std::path::PathBuf::from("/work/state.json"),
        "Planning".to_string(),
        1,
        "Running".to_string(),
        12345,
    );
    tx.send(UpstreamEvent::SyncSessions(vec![session])).unwrap();

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
