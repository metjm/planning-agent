//! Tests for the session daemon state management.

use crate::rpc::{LivenessState, SessionRecord};
use crate::session_daemon::server::DaemonState;
use crate::update::BUILD_SHA;
use std::path::PathBuf;

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

#[test]
fn test_daemon_state_insert_and_get() {
    let mut state = DaemonState::new();

    let record = create_test_record("session-1", 1000);
    state
        .sessions
        .insert(record.workflow_session_id.clone(), record);

    assert!(state.sessions.contains_key("session-1"));
    let session = state.sessions.get("session-1").unwrap();
    assert_eq!(session.pid, 1000);
    assert_eq!(session.liveness, LivenessState::Running);
}

#[test]
fn test_daemon_state_list_multiple() {
    let mut state = DaemonState::new();

    let record1 = create_test_record("session-1", 1000);
    let record2 = create_test_record("session-2", 2000);

    state
        .sessions
        .insert(record1.workflow_session_id.clone(), record1);
    state
        .sessions
        .insert(record2.workflow_session_id.clone(), record2);

    assert_eq!(state.sessions.len(), 2);
}

#[test]
fn test_daemon_state_force_stop() {
    let mut state = DaemonState::new();

    let record = create_test_record("session-1", 1000);
    state
        .sessions
        .insert(record.workflow_session_id.clone(), record.clone());

    // Force stop by updating liveness
    if let Some(session) = state.sessions.get_mut("session-1") {
        session.liveness = LivenessState::Stopped;
    }

    let session = state.sessions.get("session-1").unwrap();
    assert_eq!(session.liveness, LivenessState::Stopped);
}

#[test]
fn test_daemon_state_replace_session() {
    let mut state = DaemonState::new();

    // Insert with PID 1000
    let record1 = create_test_record("session-1", 1000);
    state
        .sessions
        .insert(record1.workflow_session_id.clone(), record1);

    // Replace with PID 2000
    let record2 = create_test_record("session-1", 2000);
    state
        .sessions
        .insert(record2.workflow_session_id.clone(), record2);

    let session = state.sessions.get("session-1").unwrap();
    assert_eq!(session.pid, 2000);
}

#[test]
fn test_daemon_shutdown_flag() {
    let mut state = DaemonState::new();
    assert!(!state.shutting_down);

    state.shutting_down = true;
    assert!(state.shutting_down);
}

#[test]
fn test_daemon_state_update_state() {
    let mut state = DaemonState::new();

    let record = create_test_record("session-1", 1000);
    state
        .sessions
        .insert(record.workflow_session_id.clone(), record);

    // Update state
    if let Some(session) = state.sessions.get_mut("session-1") {
        session.update_state("Reviewing".to_string(), 2, "AwaitingApproval".to_string());
    }

    let session = state.sessions.get("session-1").unwrap();
    assert_eq!(session.phase, "Reviewing");
    assert_eq!(session.iteration, 2);
    assert_eq!(session.workflow_status, "AwaitingApproval");
}

#[test]
fn test_daemon_liveness_state_transitions() {
    let mut state = DaemonState::new();

    let record = create_test_record("liveness-test", 1000);
    state
        .sessions
        .insert(record.workflow_session_id.clone(), record);

    // Initially Running
    {
        let session = state.sessions.get("liveness-test").unwrap();
        assert_eq!(session.liveness, LivenessState::Running);
    }

    // Transition to Stopped
    if let Some(session) = state.sessions.get_mut("liveness-test") {
        session.liveness = LivenessState::Stopped;
    }

    {
        let session = state.sessions.get("liveness-test").unwrap();
        assert_eq!(session.liveness, LivenessState::Stopped);
    }

    // Transition back to Running via heartbeat update
    if let Some(session) = state.sessions.get_mut("liveness-test") {
        session.update_heartbeat();
    }

    {
        let session = state.sessions.get("liveness-test").unwrap();
        assert_eq!(session.liveness, LivenessState::Running);
    }
}

#[test]
fn test_build_sha_is_set() {
    assert!(!BUILD_SHA.is_empty(), "BUILD_SHA should not be empty");
}

#[test]
fn test_session_record_new() {
    let record = SessionRecord::new(
        "test-id".to_string(),
        "test-feature".to_string(),
        PathBuf::from("/work"),
        PathBuf::from("/work/state.json"),
        "Planning".to_string(),
        1,
        "Running".to_string(),
        12345,
    );

    assert_eq!(record.workflow_session_id, "test-id");
    assert_eq!(record.feature_name, "test-feature");
    assert_eq!(record.phase, "Planning");
    assert_eq!(record.iteration, 1);
    assert_eq!(record.workflow_status, "Running");
    assert_eq!(record.pid, 12345);
    assert_eq!(record.liveness, LivenessState::Running);
}

#[cfg(unix)]
mod process_liveness_tests {
    use super::*;
    use crate::session_daemon::server::process_exists;

    #[test]
    fn test_process_exists_current_process() {
        // Current process should always exist
        let current_pid = std::process::id();
        assert!(
            process_exists(current_pid),
            "Current process {} should exist",
            current_pid
        );
    }

    #[test]
    fn test_process_exists_nonexistent_process() {
        // PID 999999 is very unlikely to exist (most systems have PID limits below this)
        assert!(!process_exists(999999), "PID 999999 should not exist");
    }

    #[test]
    fn test_check_process_liveness_dead_process() {
        let mut state = DaemonState::new();

        // Create a session with a non-existent PID
        let record = create_test_record("dead-pid-session", 999999);
        state
            .sessions
            .insert(record.workflow_session_id.clone(), record);

        // Session should initially be Running
        {
            let session = state.sessions.get("dead-pid-session").unwrap();
            assert_eq!(session.liveness, LivenessState::Running);
        }

        // Check process liveness should mark it as Stopped
        let changed = state.check_process_liveness();

        // Should have one changed record
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].workflow_session_id, "dead-pid-session");
        assert_eq!(changed[0].liveness, LivenessState::Stopped);

        // Verify state is updated
        let session = state.sessions.get("dead-pid-session").unwrap();
        assert_eq!(session.liveness, LivenessState::Stopped);
    }

    #[test]
    fn test_check_process_liveness_live_process() {
        let mut state = DaemonState::new();

        // Create a session with the current process PID (which definitely exists)
        let current_pid = std::process::id();
        let record = create_test_record("live-pid-session", current_pid);
        state
            .sessions
            .insert(record.workflow_session_id.clone(), record);

        // Check process liveness should NOT change anything
        let changed = state.check_process_liveness();

        // Should have no changed records
        assert!(
            changed.is_empty(),
            "Live process should not trigger state change"
        );

        // Session should still be Running
        let session = state.sessions.get("live-pid-session").unwrap();
        assert_eq!(session.liveness, LivenessState::Running);
    }

    #[test]
    fn test_check_process_liveness_skips_stopped_sessions() {
        let mut state = DaemonState::new();

        // Create a session with a non-existent PID but mark it as already stopped
        let mut record = create_test_record("already-stopped", 999999);
        record.liveness = LivenessState::Stopped;
        state
            .sessions
            .insert(record.workflow_session_id.clone(), record);

        // Check process liveness should skip already-stopped sessions
        let changed = state.check_process_liveness();

        // Should have no changed records (already stopped)
        assert!(
            changed.is_empty(),
            "Already stopped session should not trigger state change"
        );
    }
}
