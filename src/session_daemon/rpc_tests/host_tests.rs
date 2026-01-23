//! Host/upstream connection tests for the RPC daemon.

use super::TestHostServer;
use crate::rpc::host_service::{ContainerInfo, HostServiceClient, SessionInfo, PROTOCOL_VERSION};
use crate::rpc::HostError;
use crate::session_daemon::LivenessState;
use tarpc::client;
use tarpc::tokio_serde::formats::Bincode;

#[tokio::test]
async fn test_upstream_handshake_and_session_sync() {
    let host = TestHostServer::start().await;

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
