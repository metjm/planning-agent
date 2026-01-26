//! Integration tests for SessionTracker with real daemon communication.
//!
//! These tests spin up REAL daemon servers - no mocking.
//! Each test gets its own isolated daemon instance.
//! Tests are serial because set_home_for_test uses thread-local storage
//! and tokio tasks can migrate between threads.

use crate::session_daemon::rpc_tests::TestServer;
use crate::session_daemon::LivenessState;
use crate::session_daemon::SessionTracker;
use serial_test::serial;
use std::path::PathBuf;
use std::time::Duration;
use uuid::Uuid;

/// Test environment with isolated daemon.
/// Each test gets its own planning agent home directory AND its own daemon server.
struct TestEnv {
    _temp_dir: tempfile::TempDir,
    _home_guard: crate::planning_paths::TestHomeGuard,
    _server: TestServer,
}

impl TestEnv {
    async fn new() -> Self {
        // Create temp directory for this test
        let temp_dir = tempfile::TempDir::new().expect("Failed to create temp dir");

        // Use the clean test API to set the home directory
        let home_guard = crate::planning_paths::set_home_for_test(temp_dir.path().to_path_buf());

        // Start a real daemon server
        let server = TestServer::start().await;

        // Write port file so RpcClient can find our test daemon
        let port_path = temp_dir.path().join("sessiond.port");
        server.write_port_file(&port_path);

        Self {
            _temp_dir: temp_dir,
            _home_guard: home_guard,
            _server: server,
        }
    }
}

/// Generate a unique session ID for this test run.
fn unique_session_id(prefix: &str) -> String {
    format!("{}-{}", prefix, Uuid::new_v4())
}

/// Helper to clean up a specific session (marks it as stopped).
/// Does NOT shut down the daemon - other tests may be using it.
async fn cleanup_session(tracker: &SessionTracker, session_id: &str) {
    let _ = tracker.force_stop(session_id).await;
}

/// Wait for a session to reach the expected liveness state.
/// This handles race conditions where state updates take time to propagate.
async fn wait_for_liveness(
    tracker: &SessionTracker,
    session_id: &str,
    expected: LivenessState,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;

    while tokio::time::Instant::now() < deadline {
        if let Ok(sessions) = tracker.list().await {
            if let Some(session) = sessions
                .iter()
                .find(|s| s.workflow_session_id == session_id)
            {
                if session.liveness == expected {
                    return Ok(());
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Get final state for error message
    if let Ok(sessions) = tracker.list().await {
        if let Some(session) = sessions
            .iter()
            .find(|s| s.workflow_session_id == session_id)
        {
            return Err(format!(
                "Session {} liveness is {:?}, expected {:?}",
                session_id, session.liveness, expected
            ));
        }
    }
    Err(format!("Session {} not found", session_id))
}

#[tokio::test]
#[serial]
async fn test_tracker_register_and_list() {
    let _env = TestEnv::new().await;
    let tracker = SessionTracker::new(false).await;
    // Daemon is guaranteed to be running via TestEnv
    assert!(
        tracker.is_connected().await,
        "SessionTracker should be connected to test daemon"
    );

    let session_id = unique_session_id("tracker-register");

    // Register
    let result = tracker
        .register(
            session_id.clone(),
            "test-feature".to_string(),
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp/test/state.json"),
            "Planning".to_string(),
            1,
            "Planning".to_string(),
        )
        .await;
    assert!(result.is_ok(), "Register failed: {:?}", result.err());

    // List should include our session
    let sessions = tracker.list().await;
    assert!(sessions.is_ok(), "List failed: {:?}", sessions.err());

    let sessions = sessions.unwrap();
    let found = sessions.iter().any(|s| s.workflow_session_id == session_id);
    assert!(found, "Session not found in list");

    cleanup_session(&tracker, &session_id).await;
}

#[tokio::test]
#[serial]
async fn test_tracker_update() {
    let _env = TestEnv::new().await;
    let tracker = SessionTracker::new(false).await;
    // Daemon is guaranteed to be running via TestEnv
    assert!(
        tracker.is_connected().await,
        "SessionTracker should be connected to test daemon"
    );

    let session_id = unique_session_id("tracker-update");

    // Register
    tracker
        .register(
            session_id.clone(),
            "test-feature".to_string(),
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp/test/state.json"),
            "Planning".to_string(),
            1,
            "Planning".to_string(),
        )
        .await
        .expect("Register failed");

    // Update phase
    let result = tracker
        .update(
            &session_id,
            "Reviewing".to_string(),
            2,
            "Reviewing".to_string(),
        )
        .await;
    assert!(result.is_ok(), "Update failed: {:?}", result.err());

    // Verify via list
    let sessions = tracker.list().await.expect("List failed");
    let session = sessions
        .iter()
        .find(|s| s.workflow_session_id == session_id)
        .expect("Session not found");

    assert_eq!(session.phase, "Reviewing");
    assert_eq!(session.iteration, 2);

    cleanup_session(&tracker, &session_id).await;
}

#[tokio::test]
#[serial]
async fn test_tracker_mark_stopped() {
    let _env = TestEnv::new().await;
    let tracker = SessionTracker::new(false).await;
    // Daemon is guaranteed to be running via TestEnv
    assert!(
        tracker.is_connected().await,
        "SessionTracker should be connected to test daemon"
    );

    let session_id = unique_session_id("tracker-mark-stopped");

    // Register
    tracker
        .register(
            session_id.clone(),
            "test-feature".to_string(),
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp/test/state.json"),
            "Planning".to_string(),
            1,
            "Planning".to_string(),
        )
        .await
        .expect("Register failed");

    // Mark stopped
    let result = tracker.mark_stopped(&session_id).await;
    assert!(result.is_ok(), "Mark stopped failed: {:?}", result.err());

    // Wait for session to be marked as Stopped (handles race conditions)
    wait_for_liveness(
        &tracker,
        &session_id,
        LivenessState::Stopped,
        Duration::from_secs(2),
    )
    .await
    .expect("Session should be marked as Stopped");
    // Already stopped, no additional cleanup needed
}

#[tokio::test]
#[serial]
async fn test_tracker_force_stop() {
    let _env = TestEnv::new().await;
    let tracker = SessionTracker::new(false).await;
    // Daemon is guaranteed to be running via TestEnv
    assert!(
        tracker.is_connected().await,
        "SessionTracker should be connected to test daemon"
    );

    let session_id = unique_session_id("tracker-force-stop");

    // Register
    tracker
        .register(
            session_id.clone(),
            "test-feature".to_string(),
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp/test/state.json"),
            "Planning".to_string(),
            1,
            "Planning".to_string(),
        )
        .await
        .expect("Register failed");

    // Force stop
    let result = tracker.force_stop(&session_id).await;
    assert!(result.is_ok(), "Force stop failed: {:?}", result.err());

    // Wait for session to be Stopped (handles race conditions)
    wait_for_liveness(
        &tracker,
        &session_id,
        LivenessState::Stopped,
        Duration::from_secs(2),
    )
    .await
    .expect("Session should be Stopped");
    // Already stopped, no additional cleanup needed
}

#[tokio::test]
#[serial]
async fn test_tracker_full_workflow_lifecycle() {
    // Simulates a complete workflow lifecycle through SessionTracker
    let _env = TestEnv::new().await;
    let tracker = SessionTracker::new(false).await;
    // Daemon is guaranteed to be running via TestEnv
    assert!(
        tracker.is_connected().await,
        "SessionTracker should be connected to test daemon"
    );

    let session_id = unique_session_id("tracker-lifecycle");

    // 1. Register at workflow start
    tracker
        .register(
            session_id.clone(),
            "lifecycle-feature".to_string(),
            PathBuf::from("/tmp/lifecycle-test"),
            PathBuf::from("/tmp/lifecycle-test/state.json"),
            "Planning".to_string(),
            1,
            "Planning".to_string(),
        )
        .await
        .expect("Register failed");

    // Verify initial state
    let sessions = tracker.list().await.expect("List failed");
    let session = sessions
        .iter()
        .find(|s| s.workflow_session_id == session_id)
        .expect("Session not found after register");
    assert_eq!(session.phase, "Planning");
    assert_eq!(session.liveness, LivenessState::Running);

    // 2. Update: Planning -> Reviewing
    tracker
        .update(
            &session_id,
            "Reviewing".to_string(),
            1,
            "Reviewing".to_string(),
        )
        .await
        .expect("Update to Reviewing failed");

    let sessions = tracker.list().await.expect("List failed");
    let session = sessions
        .iter()
        .find(|s| s.workflow_session_id == session_id)
        .expect("Session not found");
    assert_eq!(session.phase, "Reviewing");

    // 3. Update: Reviewing -> Revising
    tracker
        .update(
            &session_id,
            "Revising".to_string(),
            2,
            "Revising".to_string(),
        )
        .await
        .expect("Update to Revising failed");

    let sessions = tracker.list().await.expect("List failed");
    let session = sessions
        .iter()
        .find(|s| s.workflow_session_id == session_id)
        .expect("Session not found");
    assert_eq!(session.phase, "Revising");
    assert_eq!(session.iteration, 2);

    // 4. Update: Revising -> Complete
    tracker
        .update(
            &session_id,
            "Complete".to_string(),
            2,
            "Complete".to_string(),
        )
        .await
        .expect("Update to Complete failed");

    // 5. Mark stopped at workflow end - this also serves as cleanup
    tracker
        .mark_stopped(&session_id)
        .await
        .expect("Mark stopped failed");

    // Wait for final state (handles race conditions)
    wait_for_liveness(
        &tracker,
        &session_id,
        LivenessState::Stopped,
        Duration::from_secs(2),
    )
    .await
    .expect("Session should be Stopped");

    // Verify final phase
    let sessions = tracker.list().await.expect("List failed");
    let session = sessions
        .iter()
        .find(|s| s.workflow_session_id == session_id)
        .expect("Session not found");
    assert_eq!(session.phase, "Complete");
    // Already stopped, no additional cleanup needed
}

#[tokio::test]
#[serial]
async fn test_tracker_reconnect() {
    let _env = TestEnv::new().await;
    let tracker = SessionTracker::new(false).await;
    // Daemon is guaranteed to be running via TestEnv
    assert!(
        tracker.is_connected().await,
        "SessionTracker should be connected to test daemon"
    );

    let session_id = unique_session_id("tracker-reconnect");

    // Register
    tracker
        .register(
            session_id.clone(),
            "test-feature".to_string(),
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp/test/state.json"),
            "Planning".to_string(),
            1,
            "Planning".to_string(),
        )
        .await
        .expect("Register failed");

    // Reconnect
    let result = tracker.reconnect().await;
    assert!(result.is_ok(), "Reconnect failed: {:?}", result.err());

    // Session should still be there (re-registered)
    let sessions = tracker.list().await.expect("List failed");
    let found = sessions.iter().any(|s| s.workflow_session_id == session_id);
    assert!(found, "Session not found after reconnect");

    cleanup_session(&tracker, &session_id).await;
}

/// Test that the async SessionTracker::new() creates a connection that stays alive.
///
/// This test verifies the fix for a bug where RpcClient::new_blocking() created
/// a tarpc client in a separate tokio runtime. When that runtime was dropped,
/// the connection would die with "connection already shutdown" errors.
///
/// The fix was to make new() async so the tarpc client is created in the
/// current runtime, keeping the connection alive.
#[tokio::test]
#[serial]
async fn test_tracker_connection_stays_alive() {
    let _env = TestEnv::new().await;
    let tracker = SessionTracker::new(false).await;

    // Small delay to let the daemon start (if spawned)
    // Daemon is guaranteed to be running via TestEnv
    assert!(
        tracker.is_connected().await,
        "SessionTracker should be connected to test daemon"
    );

    let session_id = unique_session_id("connection-alive");

    // Step 1: Register a session
    let result = tracker
        .register(
            session_id.clone(),
            "test-feature".to_string(),
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp/test/state.json"),
            "Planning".to_string(),
            1,
            "Running".to_string(),
        )
        .await;
    assert!(
        result.is_ok(),
        "Register failed (connection may have died): {:?}",
        result.err()
    );

    // Step 2: Simulate some work happening (this is where the old bug would manifest)
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Step 3: Update the session - this would fail with "connection already shutdown"
    // if the connection was created in a separate runtime
    let result = tracker
        .update(
            &session_id,
            "Reviewing".to_string(),
            2,
            "Running".to_string(),
        )
        .await;
    assert!(
        result.is_ok(),
        "Update failed (connection may have died): {:?}",
        result.err()
    );

    // Step 4: Another delay to ensure connection is still alive
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Step 5: List sessions - another operation that requires live connection
    let sessions = tracker.list().await;
    assert!(
        sessions.is_ok(),
        "List failed (connection may have died): {:?}",
        sessions.err()
    );

    let sessions = sessions.unwrap();
    let session = sessions
        .iter()
        .find(|s| s.workflow_session_id == session_id);
    assert!(session.is_some(), "Session not found in list");
    assert_eq!(session.unwrap().phase, "Reviewing");
    assert_eq!(session.unwrap().iteration, 2);

    // Step 6: Final operation - mark stopped
    let result = tracker.mark_stopped(&session_id).await;
    assert!(
        result.is_ok(),
        "Mark stopped failed (connection may have died): {:?}",
        result.err()
    );

    println!("Connection stayed alive through all operations");
}
