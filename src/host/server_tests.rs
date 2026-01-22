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
    state.add_container("container-1".to_string(), "Test Container".to_string());

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
    assert_eq!(display[0].container_name, "Test Container");
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

#[tokio::test]
async fn test_heartbeat_handling() {
    // Start server on ephemeral port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let state = Arc::new(Mutex::new(HostState::new()));
    let state_clone = state.clone();
    let (event_tx, _event_rx) = mpsc::unbounded_channel();

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
        container_id: "heartbeat-test".to_string(),
        container_name: "Heartbeat Test".to_string(),
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

    // Record last_message_at before sending heartbeat
    let before_heartbeat = {
        let state_guard = state.lock().await;
        state_guard
            .containers
            .get("heartbeat-test")
            .unwrap()
            .last_message_at
    };

    // Small delay to ensure time difference
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    // Send Heartbeat
    let heartbeat = DaemonToHost::Heartbeat;
    writer
        .write_all(format!("{}\n", serde_json::to_string(&heartbeat).unwrap()).as_bytes())
        .await
        .unwrap();

    // Read Ack
    reader.read_line(&mut line).await.unwrap();
    let ack: HostToDaemon = serde_json::from_str(line.trim()).unwrap();
    assert!(matches!(ack, HostToDaemon::Ack));

    // Verify heartbeat was recorded
    let state_guard = state.lock().await;
    let after_heartbeat = state_guard
        .containers
        .get("heartbeat-test")
        .unwrap()
        .last_message_at;
    assert!(after_heartbeat > before_heartbeat);
}

#[tokio::test]
async fn test_session_gone_handling() {
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
        container_id: "session-gone-test".to_string(),
        container_name: "Session Gone Test".to_string(),
        working_dir: std::path::PathBuf::from("/test"),
        protocol_version: PROTOCOL_VERSION,
    };
    writer
        .write_all(format!("{}\n", serde_json::to_string(&hello).unwrap()).as_bytes())
        .await
        .unwrap();

    // Read Welcome and drain ContainerConnected event
    reader.read_line(&mut line).await.unwrap();
    line.clear();
    let _ = event_rx.recv().await.unwrap(); // ContainerConnected

    // First sync a session
    let sync = DaemonToHost::SyncSessions {
        sessions: vec![SessionInfo {
            session_id: "sess-to-remove".to_string(),
            feature_name: "Removable Session".to_string(),
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

    // Read Ack and drain SessionsUpdated event
    reader.read_line(&mut line).await.unwrap();
    line.clear();
    let _ = event_rx.recv().await.unwrap(); // SessionsUpdated

    // Verify session exists
    {
        let state_guard = state.lock().await;
        assert_eq!(state_guard.active_count(), 1);
    }

    // Send SessionGone
    let gone = DaemonToHost::SessionGone {
        session_id: "sess-to-remove".to_string(),
    };
    writer
        .write_all(format!("{}\n", serde_json::to_string(&gone).unwrap()).as_bytes())
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

    // Verify session is removed
    let state_guard = state.lock().await;
    assert_eq!(state_guard.active_count(), 0);
}

#[tokio::test]
async fn test_protocol_version_mismatch() {
    // Start server on ephemeral port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let state = Arc::new(Mutex::new(HostState::new()));
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();

    let server_handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let _ = handle_connection(stream, state, event_tx).await;
    });

    // Connect as client
    let stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    // Send Hello with wrong protocol version
    let hello = DaemonToHost::Hello {
        container_id: "version-mismatch".to_string(),
        container_name: "Version Mismatch Test".to_string(),
        working_dir: std::path::PathBuf::from("/test"),
        protocol_version: PROTOCOL_VERSION + 100, // Wrong version
    };
    writer
        .write_all(format!("{}\n", serde_json::to_string(&hello).unwrap()).as_bytes())
        .await
        .unwrap();

    // Server should close connection without sending Welcome
    let mut line = String::new();
    let bytes_read = reader.read_line(&mut line).await.unwrap();

    // Connection should be closed (EOF)
    assert_eq!(bytes_read, 0);

    // Wait for server to finish
    let _ = server_handle.await;

    // No ContainerConnected event should have been sent
    // (event_rx should be empty or closed)
    assert!(event_rx.try_recv().is_err());
}

#[tokio::test]
async fn test_multi_container_scenario() {
    // Start server on ephemeral port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let state = Arc::new(Mutex::new(HostState::new()));
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();

    // Spawn server that accepts multiple connections
    let state_for_server = state.clone();
    let event_tx_for_server = event_tx.clone();
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let conn_state = state_for_server.clone();
                    let conn_event_tx = event_tx_for_server.clone();
                    tokio::spawn(async move {
                        let _ = handle_connection(stream, conn_state, conn_event_tx).await;
                    });
                }
                Err(_) => break,
            }
        }
    });

    // Helper to connect a container
    async fn connect_container(
        port: u16,
        container_id: &str,
        container_name: &str,
    ) -> (
        BufReader<tokio::net::tcp::OwnedReadHalf>,
        tokio::net::tcp::OwnedWriteHalf,
    ) {
        let stream = TcpStream::connect(format!("127.0.0.1:{}", port))
            .await
            .unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        let hello = DaemonToHost::Hello {
            container_id: container_id.to_string(),
            container_name: container_name.to_string(),
            working_dir: std::path::PathBuf::from("/test"),
            protocol_version: PROTOCOL_VERSION,
        };
        writer
            .write_all(format!("{}\n", serde_json::to_string(&hello).unwrap()).as_bytes())
            .await
            .unwrap();

        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();

        (reader, writer)
    }

    // Connect three containers
    let (mut reader1, mut writer1) = connect_container(port, "container-1", "Container 1").await;
    let _ = event_rx.recv().await.unwrap(); // ContainerConnected 1

    let (mut reader2, mut writer2) = connect_container(port, "container-2", "Container 2").await;
    let _ = event_rx.recv().await.unwrap(); // ContainerConnected 2

    let (mut reader3, mut writer3) = connect_container(port, "container-3", "Container 3").await;
    let _ = event_rx.recv().await.unwrap(); // ContainerConnected 3

    // Verify all containers are registered
    {
        let state_guard = state.lock().await;
        assert_eq!(state_guard.containers.len(), 3);
        assert!(state_guard.containers.contains_key("container-1"));
        assert!(state_guard.containers.contains_key("container-2"));
        assert!(state_guard.containers.contains_key("container-3"));
    }

    // Each container syncs different sessions
    let mut line = String::new();

    // Container 1: 2 sessions
    let sync1 = DaemonToHost::SyncSessions {
        sessions: vec![
            SessionInfo {
                session_id: "c1-sess-1".to_string(),
                feature_name: "Feature A".to_string(),
                phase: "Planning".to_string(),
                iteration: 1,
                status: "Running".to_string(),
                liveness: LivenessState::Running,
                started_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
            },
            SessionInfo {
                session_id: "c1-sess-2".to_string(),
                feature_name: "Feature B".to_string(),
                phase: "Reviewing".to_string(),
                iteration: 1,
                status: "AwaitingApproval".to_string(),
                liveness: LivenessState::Running,
                started_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
            },
        ],
    };
    writer1
        .write_all(format!("{}\n", serde_json::to_string(&sync1).unwrap()).as_bytes())
        .await
        .unwrap();
    reader1.read_line(&mut line).await.unwrap();
    line.clear();
    let _ = event_rx.recv().await.unwrap(); // SessionsUpdated

    // Container 2: 1 session
    let sync2 = DaemonToHost::SyncSessions {
        sessions: vec![SessionInfo {
            session_id: "c2-sess-1".to_string(),
            feature_name: "Feature C".to_string(),
            phase: "Complete".to_string(),
            iteration: 2,
            status: "Complete".to_string(),
            liveness: LivenessState::Stopped,
            started_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        }],
    };
    writer2
        .write_all(format!("{}\n", serde_json::to_string(&sync2).unwrap()).as_bytes())
        .await
        .unwrap();
    reader2.read_line(&mut line).await.unwrap();
    line.clear();
    let _ = event_rx.recv().await.unwrap(); // SessionsUpdated

    // Container 3: 1 session awaiting approval
    let sync3 = DaemonToHost::SyncSessions {
        sessions: vec![SessionInfo {
            session_id: "c3-sess-1".to_string(),
            feature_name: "Feature D".to_string(),
            phase: "Reviewing".to_string(),
            iteration: 1,
            status: "AwaitingApproval".to_string(),
            liveness: LivenessState::Running,
            started_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        }],
    };
    writer3
        .write_all(format!("{}\n", serde_json::to_string(&sync3).unwrap()).as_bytes())
        .await
        .unwrap();
    reader3.read_line(&mut line).await.unwrap();
    line.clear();
    let _ = event_rx.recv().await.unwrap(); // SessionsUpdated

    // Verify aggregated state
    {
        let mut state_guard = state.lock().await;
        // active_count excludes "Complete" sessions, so: 2 from c1 + 1 from c3 = 3
        assert_eq!(state_guard.active_count(), 3);
        assert_eq!(state_guard.approval_count(), 2); // 2 awaiting approval

        // Verify sessions() returns all sessions including complete
        let sessions = state_guard.sessions();
        assert_eq!(sessions.len(), 4);

        let container_names: std::collections::HashSet<_> =
            sessions.iter().map(|s| s.container_name.as_str()).collect();
        assert!(container_names.contains("Container 1"));
        assert!(container_names.contains("Container 2"));
        assert!(container_names.contains("Container 3"));
    }

    // Disconnect one container and verify cleanup
    drop(writer2);
    drop(reader2);

    // Wait for disconnect event
    loop {
        let event = event_rx.recv().await.unwrap();
        if matches!(
            event,
            crate::host::server::HostEvent::ContainerDisconnected { .. }
        ) {
            break;
        }
    }

    // Verify container-2 sessions are removed
    {
        let state_guard = state.lock().await;
        assert_eq!(state_guard.containers.len(), 2);
        assert!(!state_guard.containers.contains_key("container-2"));
        assert_eq!(state_guard.active_count(), 3); // Only sessions from containers 1 and 3
    }
}
