use super::*;

#[test]
fn test_session_browser_state_new() {
    let state = SessionBrowserState::new();
    assert!(!state.open);
    assert!(state.entries.is_empty());
    assert_eq!(state.selected_idx, 0);
    assert!(!state.daemon_connected);
    assert!(state.confirmation_pending.is_none());
}

#[test]
fn test_session_browser_close() {
    let mut state = SessionBrowserState::new();
    state.open = true;
    state.confirmation_pending = Some(ConfirmationState::ForceStop {
        session_id: "test".to_string(),
    });
    state.entries.push(SessionEntry {
        session_id: "test".to_string(),
        feature_name: "test".to_string(),
        phase: "Planning".to_string(),
        iteration: 1,
        workflow_status: "Planning".to_string(),
        liveness: LivenessState::Running,
        last_seen_at: "2024-01-01T00:00:00Z".to_string(),
        last_seen_relative: "just now".to_string(),
        working_dir: PathBuf::from("/test"),
        is_current_dir: true,
        has_snapshot: true,
        is_resumable: false,
        pid: Some(1234),
        is_live: true,
    });

    state.close();
    assert!(!state.open);
    assert!(state.entries.is_empty());
    assert!(state.confirmation_pending.is_none());
}

#[test]
fn test_format_relative_time() {
    // Test "just now"
    let now = chrono::Utc::now().to_rfc3339();
    assert_eq!(format_relative_time(&now), "just now");

    // Test invalid timestamp
    assert_eq!(format_relative_time("invalid"), "unknown");
}

#[test]
fn test_session_entry_resumability() {
    // Live Running session with snapshot - not resumable
    let entry = SessionEntry {
        session_id: "test".to_string(),
        feature_name: "test".to_string(),
        phase: "Planning".to_string(),
        iteration: 1,
        workflow_status: "Planning".to_string(),
        liveness: LivenessState::Running,
        last_seen_at: "2024-01-01T00:00:00Z".to_string(),
        last_seen_relative: "just now".to_string(),
        working_dir: PathBuf::from("/test"),
        is_current_dir: true,
        has_snapshot: true,
        is_resumable: false, // Running sessions aren't resumable
        pid: Some(1234),
        is_live: true,
    };
    assert!(!entry.is_resumable);

    // Stopped session with snapshot - resumable
    let entry2 = SessionEntry {
        liveness: LivenessState::Stopped,
        has_snapshot: true,
        is_resumable: true,
        ..entry.clone()
    };
    assert!(entry2.is_resumable);
}

fn create_test_record(id: &str, phase: &str, iteration: u32) -> SessionRecord {
    SessionRecord::new(
        id.to_string(),
        format!("{}-feature", id),
        PathBuf::from("/tmp/test"),
        PathBuf::from("/tmp/sessions").join(id),
        phase.to_string(),
        iteration,
        phase.to_string(),
        std::process::id(),
    )
}

fn create_test_entry(id: &str, phase: &str, iteration: u32) -> SessionEntry {
    SessionEntry {
        session_id: id.to_string(),
        feature_name: format!("{}-feature", id),
        phase: phase.to_string(),
        iteration,
        workflow_status: phase.to_string(),
        liveness: LivenessState::Running,
        last_seen_at: "2024-01-01T00:00:00Z".to_string(),
        last_seen_relative: "just now".to_string(),
        working_dir: PathBuf::from("/tmp/test"),
        is_current_dir: false,
        has_snapshot: false,
        is_resumable: false,
        pid: Some(std::process::id()),
        is_live: true,
    }
}

#[test]
fn test_apply_session_update_new_entry() {
    let mut state = SessionBrowserState::new();
    state.current_working_dir = PathBuf::from("/tmp/test");
    assert!(state.entries.is_empty());

    // Apply update for new session
    let record = create_test_record("new-session", "Planning", 1);
    state.apply_session_update(record);

    // Should have added entry
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.entries[0].session_id, "new-session");
    assert_eq!(state.entries[0].phase, "Planning");
    assert_eq!(state.entries[0].iteration, 1);
    assert!(state.entries[0].is_live);
}

#[test]
fn test_apply_session_update_existing_entry() {
    let mut state = SessionBrowserState::new();
    state.current_working_dir = PathBuf::from("/tmp/test");

    // Add initial entry
    state
        .entries
        .push(create_test_entry("existing-session", "Planning", 1));
    assert_eq!(state.entries.len(), 1);

    // Apply update - should update existing entry
    let mut record = create_test_record("existing-session", "Reviewing", 2);
    record.workflow_status = "Reviewing".to_string();
    state.apply_session_update(record);

    // Should still have 1 entry, but updated
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.entries[0].session_id, "existing-session");
    assert_eq!(state.entries[0].phase, "Reviewing");
    assert_eq!(state.entries[0].iteration, 2);
}

#[test]
fn test_apply_session_update_preserves_has_snapshot() {
    let mut state = SessionBrowserState::new();
    state.current_working_dir = PathBuf::from("/tmp/test");

    // Add entry with snapshot
    let mut entry = create_test_entry("snapshot-session", "Planning", 1);
    entry.has_snapshot = true;
    state.entries.push(entry);

    // Apply update - should preserve has_snapshot
    let record = create_test_record("snapshot-session", "Reviewing", 1);
    state.apply_session_update(record);

    assert_eq!(state.entries.len(), 1);
    assert!(state.entries[0].has_snapshot);
}

#[test]
fn test_apply_session_update_updates_liveness() {
    let mut state = SessionBrowserState::new();
    state.current_working_dir = PathBuf::from("/tmp/test");

    // Add running entry
    state
        .entries
        .push(create_test_entry("live-session", "Planning", 1));

    // Create stopped record
    let mut record = create_test_record("live-session", "Planning", 1);
    record.liveness = LivenessState::Stopped;
    state.apply_session_update(record);

    assert_eq!(state.entries[0].liveness, LivenessState::Stopped);
}

#[test]
fn test_apply_session_update_resumability() {
    let mut state = SessionBrowserState::new();
    state.current_working_dir = PathBuf::from("/tmp/test");

    // Add entry with snapshot, running
    let mut entry = create_test_entry("resumable-session", "Planning", 1);
    entry.has_snapshot = true;
    entry.liveness = LivenessState::Running;
    entry.is_resumable = false; // Running = not resumable
    state.entries.push(entry);

    // When stopped, should become resumable
    let mut record = create_test_record("resumable-session", "Planning", 1);
    record.liveness = LivenessState::Stopped;
    state.apply_session_update(record);

    // has_snapshot=true + Stopped = resumable
    assert!(state.entries[0].is_resumable);
}

#[test]
fn test_apply_session_update_multiple_sessions() {
    let mut state = SessionBrowserState::new();
    state.current_working_dir = PathBuf::from("/tmp/test");

    // Add two entries
    state
        .entries
        .push(create_test_entry("session-a", "Planning", 1));
    state
        .entries
        .push(create_test_entry("session-b", "Planning", 1));

    // Update only session-b
    let record = create_test_record("session-b", "Reviewing", 2);
    state.apply_session_update(record);

    // session-a unchanged, session-b updated
    let session_a = state
        .entries
        .iter()
        .find(|e| e.session_id == "session-a")
        .unwrap();
    let session_b = state
        .entries
        .iter()
        .find(|e| e.session_id == "session-b")
        .unwrap();

    assert_eq!(session_a.phase, "Planning");
    assert_eq!(session_a.iteration, 1);
    assert_eq!(session_b.phase, "Reviewing");
    assert_eq!(session_b.iteration, 2);
}
