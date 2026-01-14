//! Tests for the session daemon server.

use super::server::{handle_message_with_broadcast, DaemonState};
use crate::session_daemon::protocol::{ClientMessage, DaemonMessage, LivenessState, SessionRecord};
use crate::update::BUILD_SHA;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

/// Test fixture that includes state and broadcast channel for testing.
struct TestFixture {
    state: Arc<Mutex<DaemonState>>,
    events_tx: broadcast::Sender<SessionRecord>,
    events_rx: broadcast::Receiver<SessionRecord>,
}

impl TestFixture {
    fn new() -> Self {
        let state = Arc::new(Mutex::new(DaemonState::new()));
        let (events_tx, events_rx) = broadcast::channel::<SessionRecord>(64);
        Self {
            state,
            events_tx,
            events_rx,
        }
    }

    async fn handle_message(&self, line: &str) -> Option<DaemonMessage> {
        let (response, _) =
            handle_message_with_broadcast(line, &self.state, &self.events_tx).await;
        response
    }
}

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

#[tokio::test]
async fn test_daemon_state_register() {
    let fixture = TestFixture::new();

    let record = create_test_record("session-1", 1000);
    let msg = serde_json::to_string(&ClientMessage::Register(record)).unwrap();

    let response = fixture.handle_message(&msg).await;
    assert!(matches!(response, Some(DaemonMessage::Ack { .. })));

    let state_guard = fixture.state.lock().await;
    assert!(state_guard.sessions.contains_key("session-1"));
}

#[tokio::test]
async fn test_daemon_state_register_broadcasts() {
    let mut fixture = TestFixture::new();

    let record = create_test_record("session-1", 1000);
    let msg = serde_json::to_string(&ClientMessage::Register(record)).unwrap();

    fixture.handle_message(&msg).await;

    // Verify broadcast was sent
    let broadcast = fixture.events_rx.try_recv().unwrap();
    assert_eq!(broadcast.workflow_session_id, "session-1");
    assert_eq!(broadcast.pid, 1000);
}

#[tokio::test]
async fn test_daemon_state_heartbeat() {
    let fixture = TestFixture::new();

    // Register first
    let record = create_test_record("session-1", 1000);
    let msg = serde_json::to_string(&ClientMessage::Register(record)).unwrap();
    fixture.handle_message(&msg).await;

    // Send heartbeat
    let heartbeat_msg = serde_json::to_string(&ClientMessage::Heartbeat {
        session_id: "session-1".to_string(),
    })
    .unwrap();
    let response = fixture.handle_message(&heartbeat_msg).await;
    assert!(matches!(response, Some(DaemonMessage::Ack { .. })));

    let state_guard = fixture.state.lock().await;
    let session = state_guard.sessions.get("session-1").unwrap();
    assert_eq!(session.liveness, LivenessState::Running);
}

#[tokio::test]
async fn test_daemon_state_heartbeat_broadcasts() {
    let mut fixture = TestFixture::new();

    // Register first
    let record = create_test_record("session-1", 1000);
    let msg = serde_json::to_string(&ClientMessage::Register(record)).unwrap();
    fixture.handle_message(&msg).await;

    // Consume the register broadcast
    let _ = fixture.events_rx.try_recv();

    // Send heartbeat
    let heartbeat_msg = serde_json::to_string(&ClientMessage::Heartbeat {
        session_id: "session-1".to_string(),
    })
    .unwrap();
    fixture.handle_message(&heartbeat_msg).await;

    // Verify heartbeat broadcast was sent
    let broadcast = fixture.events_rx.try_recv().unwrap();
    assert_eq!(broadcast.workflow_session_id, "session-1");
}

#[tokio::test]
async fn test_daemon_state_list() {
    let fixture = TestFixture::new();

    // Register two sessions
    let record1 = create_test_record("session-1", 1000);
    let record2 = create_test_record("session-2", 2000);

    let msg1 = serde_json::to_string(&ClientMessage::Register(record1)).unwrap();
    let msg2 = serde_json::to_string(&ClientMessage::Register(record2)).unwrap();
    fixture.handle_message(&msg1).await;
    fixture.handle_message(&msg2).await;

    // List
    let list_msg = serde_json::to_string(&ClientMessage::List).unwrap();
    let response = fixture.handle_message(&list_msg).await;

    match response {
        Some(DaemonMessage::Sessions(sessions)) => {
            assert_eq!(sessions.len(), 2);
        }
        _ => panic!("Expected Sessions response"),
    }
}

#[tokio::test]
async fn test_daemon_state_force_stop() {
    let fixture = TestFixture::new();

    // Register
    let record = create_test_record("session-1", 1000);
    let msg = serde_json::to_string(&ClientMessage::Register(record)).unwrap();
    fixture.handle_message(&msg).await;

    // Force stop
    let stop_msg = serde_json::to_string(&ClientMessage::ForceStop {
        session_id: "session-1".to_string(),
    })
    .unwrap();
    let response = fixture.handle_message(&stop_msg).await;
    assert!(matches!(response, Some(DaemonMessage::Ack { .. })));

    let state_guard = fixture.state.lock().await;
    let session = state_guard.sessions.get("session-1").unwrap();
    assert_eq!(session.liveness, LivenessState::Stopped);
}

#[tokio::test]
async fn test_daemon_state_force_stop_broadcasts() {
    let mut fixture = TestFixture::new();

    // Register
    let record = create_test_record("session-1", 1000);
    let msg = serde_json::to_string(&ClientMessage::Register(record)).unwrap();
    fixture.handle_message(&msg).await;

    // Consume register broadcast
    let _ = fixture.events_rx.try_recv();

    // Force stop
    let stop_msg = serde_json::to_string(&ClientMessage::ForceStop {
        session_id: "session-1".to_string(),
    })
    .unwrap();
    fixture.handle_message(&stop_msg).await;

    // Verify broadcast with Stopped liveness
    let broadcast = fixture.events_rx.try_recv().unwrap();
    assert_eq!(broadcast.workflow_session_id, "session-1");
    assert_eq!(broadcast.liveness, LivenessState::Stopped);
}

#[tokio::test]
async fn test_daemon_state_replace_stale_session() {
    let fixture = TestFixture::new();

    // Register with PID 1000
    let record1 = create_test_record("session-1", 1000);
    let msg1 = serde_json::to_string(&ClientMessage::Register(record1)).unwrap();
    fixture.handle_message(&msg1).await;

    // Register same session ID with different PID (simulating restart)
    let record2 = create_test_record("session-1", 2000);
    let msg2 = serde_json::to_string(&ClientMessage::Register(record2)).unwrap();
    fixture.handle_message(&msg2).await;

    let state_guard = fixture.state.lock().await;
    let session = state_guard.sessions.get("session-1").unwrap();
    assert_eq!(session.pid, 2000);
}

#[tokio::test]
async fn test_daemon_ack_includes_build_sha() {
    let fixture = TestFixture::new();

    // Send a heartbeat (simplest message that returns Ack)
    let heartbeat_msg = serde_json::to_string(&ClientMessage::Heartbeat {
        session_id: "nonexistent".to_string(),
    })
    .unwrap();

    let response = fixture.handle_message(&heartbeat_msg).await;

    match response {
        Some(DaemonMessage::Ack { build_sha }) => {
            assert!(!build_sha.is_empty(), "build_sha should not be empty");
            assert_eq!(
                build_sha, BUILD_SHA,
                "build_sha should match BUILD_SHA constant"
            );
        }
        other => panic!("Expected Ack response, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_daemon_shutdown_response() {
    let fixture = TestFixture::new();

    // Register a session first
    let record = create_test_record("session-shutdown-test", 1234);
    let register_msg = serde_json::to_string(&ClientMessage::Register(record)).unwrap();
    fixture.handle_message(&register_msg).await;

    // Send shutdown
    let shutdown_msg = serde_json::to_string(&ClientMessage::Shutdown).unwrap();
    let response = fixture.handle_message(&shutdown_msg).await;

    // Should get Ack with build_sha
    match response {
        Some(DaemonMessage::Ack { build_sha }) => {
            assert!(!build_sha.is_empty());
        }
        other => panic!("Expected Ack response, got: {:?}", other),
    }

    // State should have shutting_down flag set
    let state_guard = fixture.state.lock().await;
    assert!(
        state_guard.shutting_down,
        "shutting_down flag should be true after Shutdown"
    );
}

#[tokio::test]
async fn test_daemon_update_creates_missing_session() {
    let fixture = TestFixture::new();

    // Don't register, just update directly
    let record = create_test_record("new-session-via-update", 5000);
    let update_msg = serde_json::to_string(&ClientMessage::Update(record)).unwrap();
    let response = fixture.handle_message(&update_msg).await;

    assert!(matches!(response, Some(DaemonMessage::Ack { .. })));

    // Session should exist now
    let state_guard = fixture.state.lock().await;
    assert!(state_guard.sessions.contains_key("new-session-via-update"));
}

#[tokio::test]
async fn test_daemon_update_broadcasts() {
    let mut fixture = TestFixture::new();

    let record = create_test_record("session-1", 1000);
    let update_msg = serde_json::to_string(&ClientMessage::Update(record)).unwrap();
    fixture.handle_message(&update_msg).await;

    // Verify broadcast was sent
    let broadcast = fixture.events_rx.try_recv().unwrap();
    assert_eq!(broadcast.workflow_session_id, "session-1");
}

#[tokio::test]
async fn test_daemon_liveness_state_transitions() {
    let fixture = TestFixture::new();

    // Register a session
    let record = create_test_record("liveness-test", 1000);
    let msg = serde_json::to_string(&ClientMessage::Register(record)).unwrap();
    fixture.handle_message(&msg).await;

    // Initially should be Running
    {
        let state_guard = fixture.state.lock().await;
        let session = state_guard.sessions.get("liveness-test").unwrap();
        assert_eq!(session.liveness, LivenessState::Running);
    }

    // Force stop should transition to Stopped
    let stop_msg = serde_json::to_string(&ClientMessage::ForceStop {
        session_id: "liveness-test".to_string(),
    })
    .unwrap();
    fixture.handle_message(&stop_msg).await;

    {
        let state_guard = fixture.state.lock().await;
        let session = state_guard.sessions.get("liveness-test").unwrap();
        assert_eq!(session.liveness, LivenessState::Stopped);
    }

    // Heartbeat on stopped session resets to Running
    let heartbeat_msg = serde_json::to_string(&ClientMessage::Heartbeat {
        session_id: "liveness-test".to_string(),
    })
    .unwrap();
    fixture.handle_message(&heartbeat_msg).await;

    {
        let state_guard = fixture.state.lock().await;
        let session = state_guard.sessions.get("liveness-test").unwrap();
        assert_eq!(session.liveness, LivenessState::Running);
    }
}

#[tokio::test]
async fn test_subscribe_unsubscribe() {
    let fixture = TestFixture::new();

    // Subscribe
    let subscribe_msg = serde_json::to_string(&ClientMessage::Subscribe).unwrap();
    let response = fixture.handle_message(&subscribe_msg).await;
    assert!(matches!(response, Some(DaemonMessage::Subscribed)));

    // Unsubscribe
    let unsubscribe_msg = serde_json::to_string(&ClientMessage::Unsubscribe).unwrap();
    let response = fixture.handle_message(&unsubscribe_msg).await;
    assert!(matches!(response, Some(DaemonMessage::Unsubscribed)));
}
