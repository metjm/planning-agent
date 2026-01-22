//! Tests for the host TCP server.

use crate::host::server::handle_connection;
use crate::host::state::HostState;
use crate::host_protocol::{
    DaemonToHost, HostToDaemon, LivenessState, SessionInfo, PROTOCOL_VERSION,
};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};

#[tokio::test]
async fn test_protocol_handshake() {
    // Start server on ephemeral port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    // Spawn server task
    let state = Arc::new(Mutex::new(HostState::new()));
    let (event_tx, _event_rx) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let _ = handle_connection(stream, state, event_tx).await;
    });

    // Connect as client
    let stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    // Send Hello
    let hello = DaemonToHost::Hello {
        container_id: "test-container".to_string(),
        container_name: "Test".to_string(),
        working_dir: std::path::PathBuf::from("/test"),
        protocol_version: PROTOCOL_VERSION,
    };
    let json = serde_json::to_string(&hello).unwrap();
    writer
        .write_all(format!("{}\n", json).as_bytes())
        .await
        .unwrap();

    // Read Welcome
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let welcome: HostToDaemon = serde_json::from_str(line.trim()).unwrap();

    match welcome {
        HostToDaemon::Welcome {
            protocol_version, ..
        } => {
            assert_eq!(protocol_version, PROTOCOL_VERSION);
        }
        _ => panic!("Expected Welcome"),
    }
}

#[tokio::test]
async fn test_session_sync() {
    let mut state = HostState::new();

    // Add container
    state.add_container(
        "container-1".to_string(),
        "Test Container".to_string(),
        std::path::PathBuf::from("/test"),
    );

    // Sync sessions
    let sessions = vec![
        SessionInfo {
            session_id: "session-1".to_string(),
            feature_name: "feature-1".to_string(),
            phase: "Planning".to_string(),
            iteration: 1,
            status: "Running".to_string(),
            liveness: LivenessState::Running,
            started_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        },
        SessionInfo {
            session_id: "session-2".to_string(),
            feature_name: "feature-2".to_string(),
            phase: "Reviewing".to_string(),
            iteration: 2,
            status: "AwaitingApproval".to_string(),
            liveness: LivenessState::Running,
            started_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        },
    ];
    state.sync_sessions("container-1", sessions);

    // Verify state
    assert_eq!(state.active_count(), 2);
    assert_eq!(state.approval_count(), 1);

    let display = state.sessions();
    assert_eq!(display.len(), 2);
    // AwaitingApproval should be first (due to sorting)
    assert!(display[0]
        .session
        .status
        .to_lowercase()
        .contains("approval"));
    // Verify container info is passed through
    assert_eq!(display[0].container_id, "container-1");
    assert_eq!(display[0].container_name, "Test Container");

    // Verify container state
    let container = state.containers.get("container-1").unwrap();
    assert_eq!(container.working_dir, std::path::PathBuf::from("/test"));
    // connected_at should be recent
    assert!(container.connected_at.elapsed().as_secs() < 5);
}

#[tokio::test]
async fn test_session_update_flow() {
    // Start server on ephemeral port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let state = Arc::new(Mutex::new(HostState::new()));
    let state_clone = state.clone();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let _ = handle_connection(stream, state_clone, event_tx).await;
    });

    // Connect as client
    let stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    // Send Hello
    let hello = DaemonToHost::Hello {
        container_id: "test-container".to_string(),
        container_name: "Test".to_string(),
        working_dir: std::path::PathBuf::from("/test"),
        protocol_version: PROTOCOL_VERSION,
    };
    writer
        .write_all(format!("{}\n", serde_json::to_string(&hello).unwrap()).as_bytes())
        .await
        .unwrap();

    // Read Welcome
    reader.read_line(&mut line).await.unwrap();
    line.clear();

    // Verify ContainerConnected event
    let event = event_rx.recv().await.unwrap();
    match event {
        crate::host::server::HostEvent::ContainerConnected {
            container_id,
            container_name,
        } => {
            assert_eq!(container_id, "test-container");
            assert_eq!(container_name, "Test");
        }
        _ => panic!("Expected ContainerConnected"),
    }

    // Send SyncSessions
    let sync = DaemonToHost::SyncSessions {
        sessions: vec![SessionInfo {
            session_id: "sess-1".to_string(),
            feature_name: "Test Feature".to_string(),
            phase: "Planning".to_string(),
            iteration: 1,
            status: "Running".to_string(),
            liveness: LivenessState::Running,
            started_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        }],
    };
    writer
        .write_all(format!("{}\n", serde_json::to_string(&sync).unwrap()).as_bytes())
        .await
        .unwrap();

    // Read Ack
    reader.read_line(&mut line).await.unwrap();
    let ack: HostToDaemon = serde_json::from_str(line.trim()).unwrap();
    assert!(matches!(ack, HostToDaemon::Ack));

    // Verify SessionsUpdated event
    let event = event_rx.recv().await.unwrap();
    assert!(matches!(
        event,
        crate::host::server::HostEvent::SessionsUpdated
    ));

    // Verify state contains the session
    let state_guard = state.lock().await;
    assert_eq!(state_guard.active_count(), 1);
}

#[tokio::test]
async fn test_container_disconnect() {
    // Start server on ephemeral port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let state = Arc::new(Mutex::new(HostState::new()));
    let state_clone = state.clone();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();

    let server_handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let _ = handle_connection(stream, state_clone, event_tx).await;
    });

    // Connect as client
    let stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    // Send Hello
    let hello = DaemonToHost::Hello {
        container_id: "disconnect-test".to_string(),
        container_name: "Disconnect Test".to_string(),
        working_dir: std::path::PathBuf::from("/test"),
        protocol_version: PROTOCOL_VERSION,
    };
    writer
        .write_all(format!("{}\n", serde_json::to_string(&hello).unwrap()).as_bytes())
        .await
        .unwrap();

    // Read Welcome
    reader.read_line(&mut line).await.unwrap();

    // Verify ContainerConnected event
    let event = event_rx.recv().await.unwrap();
    assert!(matches!(
        event,
        crate::host::server::HostEvent::ContainerConnected { .. }
    ));

    // Verify container is registered
    {
        let state_guard = state.lock().await;
        assert!(state_guard.containers.contains_key("disconnect-test"));
    }

    // Drop the client connection (simulating disconnect)
    drop(writer);
    drop(reader);

    // Wait for server to detect disconnect
    let _ = server_handle.await;

    // Verify ContainerDisconnected event
    let event = event_rx.recv().await.unwrap();
    match event {
        crate::host::server::HostEvent::ContainerDisconnected { container_id } => {
            assert_eq!(container_id, "disconnect-test");
        }
        _ => panic!("Expected ContainerDisconnected"),
    }

    // Verify container is removed from state
    {
        let state_guard = state.lock().await;
        assert!(!state_guard.containers.contains_key("disconnect-test"));
    }
}
