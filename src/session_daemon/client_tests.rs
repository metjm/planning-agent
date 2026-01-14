//! Integration tests for SessionDaemonClient.
//!
//! These tests spawn a real daemon and test communication.
//! Tests share a daemon instance - each test cleans up its own session
//! but does NOT shut down the daemon (which would break parallel tests).

use super::SessionDaemonClient;
use crate::session_daemon::protocol::{LivenessState, SessionRecord};
use std::path::PathBuf;
use std::time::Duration;

fn create_test_record(id: &str) -> SessionRecord {
    SessionRecord::new(
        id.to_string(),
        "integration-test-feature".to_string(),
        PathBuf::from("/tmp/test-working-dir"),
        PathBuf::from("/tmp/test-state.json"),
        "Planning".to_string(),
        1,
        "Planning".to_string(),
        std::process::id(),
    )
}

/// Helper to clean up a specific session (marks it as stopped).
/// Does NOT shut down the daemon - other tests may be using it.
async fn cleanup_session(client: &SessionDaemonClient, session_id: &str) {
    let _ = client.force_stop(session_id).await;
}

#[tokio::test]
async fn test_client_connect_and_spawn_daemon() {
    // Create client - this should spawn daemon if not running
    let client = SessionDaemonClient::new(false);

    // Give daemon time to start
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Client should be connected (or in degraded mode if spawn failed)
    // The important thing is it doesn't panic
    let connected = client.is_connected();

    // If we got here without panic, the basic mechanism works
    // Connection might fail in CI environments without proper setup
    println!("Client connected: {}", connected);
}

#[tokio::test]
async fn test_client_register_and_list() {
    let client = SessionDaemonClient::new(false);
    tokio::time::sleep(Duration::from_millis(500)).await;

    if !client.is_connected() {
        println!("Skipping test - daemon not available");
        return;
    }

    let session_id = "integration-test-session-1";

    // Register a session
    let record = create_test_record(session_id);
    let result = client.register(record.clone()).await;
    assert!(result.is_ok(), "Register failed: {:?}", result.err());

    // List sessions - should include our session
    let sessions = client.list().await;
    assert!(sessions.is_ok(), "List failed: {:?}", sessions.err());

    let sessions = sessions.unwrap();
    let found = sessions.iter().any(|s| s.workflow_session_id == session_id);
    assert!(found, "Registered session not found in list");

    cleanup_session(&client, session_id).await;
}

#[tokio::test]
async fn test_client_update_session() {
    let client = SessionDaemonClient::new(false);
    tokio::time::sleep(Duration::from_millis(500)).await;

    if !client.is_connected() {
        println!("Skipping test - daemon not available");
        return;
    }

    let session_id = "integration-test-session-2";

    // Register
    let mut record = create_test_record(session_id);
    client.register(record.clone()).await.expect("Register failed");

    // Update
    record.phase = "Reviewing".to_string();
    record.iteration = 2;
    let result = client.update(record).await;
    assert!(result.is_ok(), "Update failed: {:?}", result.err());

    // Verify update via list
    let sessions = client.list().await.expect("List failed");
    let session = sessions.iter()
        .find(|s| s.workflow_session_id == session_id)
        .expect("Session not found");

    assert_eq!(session.phase, "Reviewing");
    assert_eq!(session.iteration, 2);

    cleanup_session(&client, session_id).await;
}

#[tokio::test]
async fn test_client_heartbeat() {
    let client = SessionDaemonClient::new(false);
    tokio::time::sleep(Duration::from_millis(500)).await;

    if !client.is_connected() {
        println!("Skipping test - daemon not available");
        return;
    }

    let session_id = "integration-test-session-3";

    // Register
    let record = create_test_record(session_id);
    client.register(record).await.expect("Register failed");

    // Send heartbeat
    let result = client.heartbeat(session_id).await;
    assert!(result.is_ok(), "Heartbeat failed: {:?}", result.err());

    // Session should still be Running
    let sessions = client.list().await.expect("List failed");
    let session = sessions.iter()
        .find(|s| s.workflow_session_id == session_id)
        .expect("Session not found");

    assert_eq!(session.liveness, LivenessState::Running);

    cleanup_session(&client, session_id).await;
}

#[tokio::test]
async fn test_client_force_stop() {
    let client = SessionDaemonClient::new(false);
    tokio::time::sleep(Duration::from_millis(500)).await;

    if !client.is_connected() {
        println!("Skipping test - daemon not available");
        return;
    }

    let session_id = "integration-test-session-4";

    // Register
    let record = create_test_record(session_id);
    client.register(record).await.expect("Register failed");

    // Force stop
    let result = client.force_stop(session_id).await;
    assert!(result.is_ok(), "Force stop failed: {:?}", result.err());

    // Session should be Stopped
    let sessions = client.list().await.expect("List failed");
    let session = sessions.iter()
        .find(|s| s.workflow_session_id == session_id)
        .expect("Session not found");

    assert_eq!(session.liveness, LivenessState::Stopped);
    // Already stopped, no additional cleanup needed
}

#[tokio::test]
async fn test_client_reconnect() {
    let mut client = SessionDaemonClient::new(false);
    tokio::time::sleep(Duration::from_millis(500)).await;

    if !client.is_connected() {
        println!("Skipping test - daemon not available");
        return;
    }

    let session_id = "integration-test-session-5";

    // Register a session
    let record = create_test_record(session_id);
    client.register(record).await.expect("Register failed");

    // Reconnect (should work even if already connected)
    let result = client.reconnect().await;
    assert!(result.is_ok(), "Reconnect failed: {:?}", result.err());

    // Should still be able to list
    let sessions = client.list().await;
    assert!(sessions.is_ok(), "List after reconnect failed");

    cleanup_session(&client, session_id).await;
}

#[tokio::test]
async fn test_client_full_workflow_cycle() {
    // This test simulates a complete workflow lifecycle
    let client = SessionDaemonClient::new(false);
    tokio::time::sleep(Duration::from_millis(500)).await;

    if !client.is_connected() {
        println!("Skipping test - daemon not available");
        return;
    }

    let session_id = "integration-test-workflow-cycle";

    // 1. Register (workflow start)
    let record = SessionRecord::new(
        session_id.to_string(),
        "test-feature".to_string(),
        PathBuf::from("/tmp/test"),
        PathBuf::from("/tmp/test/state.json"),
        "Planning".to_string(),
        1,
        "Planning".to_string(),
        std::process::id(),
    );
    client.register(record).await.expect("Register failed");

    // Verify registration
    let sessions = client.list().await.expect("List failed");
    assert!(sessions.iter().any(|s| s.workflow_session_id == session_id));

    // 2. Update (phase transition to Reviewing)
    let mut record = sessions.iter()
        .find(|s| s.workflow_session_id == session_id)
        .unwrap()
        .clone();
    record.phase = "Reviewing".to_string();
    record.workflow_status = "Reviewing".to_string();
    client.update(record).await.expect("Update to Reviewing failed");

    // 3. Heartbeat (keep alive during review)
    client.heartbeat(session_id).await.expect("Heartbeat failed");

    // 4. Update (phase transition to Revising)
    let sessions = client.list().await.expect("List failed");
    let mut record = sessions.iter()
        .find(|s| s.workflow_session_id == session_id)
        .unwrap()
        .clone();
    record.phase = "Revising".to_string();
    record.iteration = 2;
    client.update(record).await.expect("Update to Revising failed");

    // 5. Force stop (workflow complete) - this also serves as cleanup
    client.force_stop(session_id).await.expect("Force stop failed");

    // Verify final state
    let sessions = client.list().await.expect("Final list failed");
    let final_session = sessions.iter()
        .find(|s| s.workflow_session_id == session_id)
        .expect("Session not found");

    assert_eq!(final_session.phase, "Revising");
    assert_eq!(final_session.iteration, 2);
    assert_eq!(final_session.liveness, LivenessState::Stopped);
    // Already stopped, no additional cleanup needed
}
