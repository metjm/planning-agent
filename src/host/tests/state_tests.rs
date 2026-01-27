use super::*;
use crate::session_daemon::LivenessState;

fn make_session(id: &str, status: &str) -> SessionInfo {
    SessionInfo {
        session_id: id.to_string(),
        feature_name: format!("feature-{}", id),
        phase: "Planning".to_string(),
        iteration: 1,
        status: status.to_string(),
        liveness: LivenessState::Running,
        started_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        pid: 0,
    }
}

#[test]
fn test_add_container() {
    let mut state = HostState::new();
    state.add_container(
        "c1".to_string(),
        "Container 1".to_string(),
        PathBuf::from("/test/work"),
        "abc123".to_string(),
        1234567890,
        0,
    );

    assert_eq!(state.containers.len(), 1);
    assert!(state.containers.contains_key("c1"));

    // Verify build info is stored
    let container = state.containers.get("c1").unwrap();
    assert_eq!(container.git_sha, "abc123");
    assert_eq!(container.build_timestamp, 1234567890);
    assert_eq!(container.working_dir, PathBuf::from("/test/work"));
}

#[test]
fn test_sync_sessions() {
    let mut state = HostState::new();
    state.add_container(
        "c1".to_string(),
        "Container 1".to_string(),
        PathBuf::from("/test/work"),
        "abc123".to_string(),
        1234567890,
        0,
    );

    let sessions = vec![
        make_session("s1", "Running"),
        make_session("s2", "AwaitingApproval"),
    ];
    state.sync_sessions("c1", sessions);

    assert_eq!(state.active_count(), 2);
    assert_eq!(state.approval_count(), 1);
}

#[test]
fn test_sessions_sorted_by_status() {
    let mut state = HostState::new();
    state.add_container(
        "c1".to_string(),
        "Container 1".to_string(),
        PathBuf::from("/test/work"),
        "abc123".to_string(),
        1234567890,
        0,
    );

    let sessions = vec![
        make_session("s1", "Running"),
        make_session("s2", "AwaitingApproval"),
        make_session("s3", "Complete"),
    ];
    state.sync_sessions("c1", sessions);

    let display = state.sessions();
    assert_eq!(display.len(), 3);
    // AwaitingApproval should be first
    assert!(display[0]
        .session
        .status
        .to_lowercase()
        .contains("approval"));
    // Complete should be last
    assert_eq!(display[2].session.status.to_lowercase(), "complete");
}

#[test]
fn test_remove_container() {
    let mut state = HostState::new();
    state.add_container(
        "c1".to_string(),
        "Container 1".to_string(),
        PathBuf::from("/test/work"),
        "abc123".to_string(),
        1234567890,
        0,
    );
    state.sync_sessions("c1", vec![make_session("s1", "Running")]);

    state.remove_container("c1");

    assert_eq!(state.containers.len(), 0);
    assert_eq!(state.active_count(), 0);
}

#[test]
fn test_update_session() {
    let mut state = HostState::new();
    state.add_container(
        "c1".to_string(),
        "Container 1".to_string(),
        PathBuf::from("/test/work"),
        "abc123".to_string(),
        1234567890,
        0,
    );
    state.sync_sessions("c1", vec![make_session("s1", "Running")]);

    // Update the session
    let updated = make_session("s1", "AwaitingApproval");
    state.update_session("c1", updated);

    assert_eq!(state.approval_count(), 1);
}

#[test]
fn test_remove_session() {
    let mut state = HostState::new();
    state.add_container(
        "c1".to_string(),
        "Container 1".to_string(),
        PathBuf::from("/test/work"),
        "abc123".to_string(),
        1234567890,
        0,
    );
    state.sync_sessions(
        "c1",
        vec![make_session("s1", "Running"), make_session("s2", "Running")],
    );

    state.remove_session("c1", "s1");

    assert_eq!(state.active_count(), 1);
}
