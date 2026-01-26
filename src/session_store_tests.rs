//! Tests for session_store module.

use super::*;
use crate::planning_paths::{set_home_for_test, TestHomeGuard};
use tempfile::tempdir;

/// Helper to set up an isolated test home directory.
fn test_env() -> (tempfile::TempDir, TestHomeGuard) {
    let dir = tempdir().expect("Failed to create temp dir");
    let guard = set_home_for_test(dir.path().to_path_buf());
    (dir, guard)
}

fn create_test_state() -> State {
    State::new("test-feature", "Test objective", 3).unwrap()
}

fn create_test_ui_state() -> SessionUiState {
    SessionUiState {
        id: 1,
        name: "Test Session".to_string(),
        status: SessionStatus::Planning,
        output_lines: vec!["line1".to_string(), "line2".to_string()],
        scroll_position: 0,
        output_follow_mode: true,
        streaming_lines: Vec::new(),
        streaming_scroll_position: 0,
        streaming_follow_mode: true,
        focused_panel: FocusedPanel::Output,
        total_cost: 0.0,
        bytes_received: 100,
        total_input_tokens: 50,
        total_output_tokens: 30,
        total_cache_creation_tokens: 0,
        total_cache_read_tokens: 0,
        tool_call_count: 5,
        bytes_per_second: 100.0,
        turn_count: 2,
        model_name: Some("claude".to_string()),
        last_stop_reason: None,
        tool_error_count: 0,
        total_tool_duration_ms: 1000,
        completed_tool_count: 5,
        approval_mode: ApprovalMode::None,
        approval_context: ApprovalContext::PlanApproval,
        plan_summary: "Test plan".to_string(),
        plan_summary_scroll: 0,
        user_feedback: String::new(),
        cursor_position: 0,
        feedback_scroll: 0,
        feedback_target: FeedbackTarget::ApprovalDecline,
        input_mode: InputMode::Normal,
        tab_input: String::new(),
        tab_input_cursor: 0,
        tab_input_scroll: 0,
        last_key_was_backslash: false,
        tab_input_pastes: Vec::new(),
        feedback_pastes: Vec::new(),
        error_state: None,
        error_scroll: 0,
        run_tabs: Vec::new(),
        active_run_tab: 0,
        chat_follow_mode: true,
        todos: HashMap::new(),
        todo_scroll_position: 0,
        account_usage: AccountUsage::default(),
        spinner_frame: 0,
        current_run_id: 1,
        plan_modal_open: false,
        plan_modal_scroll: 0,
        review_modal_open: false,
        review_modal_scroll: 0,
        review_modal_tab: 0,
        review_history: Vec::new(),
        review_history_spinner_frame: 0,
        review_history_scroll: 0,
    }
}

#[test]
fn test_snapshot_creation() {
    let state = create_test_state();
    let ui_state = create_test_ui_state();
    let saved_at = chrono::Utc::now().to_rfc3339();

    let snapshot = SessionSnapshot::new_with_timestamp(
        PathBuf::from("/tmp/test"),
        "test-session-id".to_string(),
        PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
        state.clone(),
        ui_state,
        0,
        saved_at,
        "claude-only".to_string(),
        None, // No workflow view for legacy test
        0,    // No event sequence
    );

    assert_eq!(snapshot.version, SNAPSHOT_VERSION);
    assert!(!snapshot.saved_at.is_empty());
    assert_eq!(snapshot.workflow_session_id, "test-session-id");
    assert_eq!(snapshot.workflow_state.feature_name, "test-feature");
    assert_eq!(snapshot.workflow_name, "claude-only");
    assert!(snapshot.workflow_view.is_none());
    assert_eq!(snapshot.last_event_sequence, 0);
}

#[test]
fn test_snapshot_serialization_roundtrip() {
    let state = create_test_state();
    let ui_state = create_test_ui_state();
    let saved_at = chrono::Utc::now().to_rfc3339();

    let snapshot = SessionSnapshot::new_with_timestamp(
        PathBuf::from("/tmp/test"),
        "test-session-id".to_string(),
        PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
        state,
        ui_state,
        5000,
        saved_at,
        "default".to_string(),
        None,
        0,
    );

    let json = serde_json::to_string(&snapshot).unwrap();
    let loaded: SessionSnapshot = serde_json::from_str(&json).unwrap();

    assert_eq!(loaded.version, snapshot.version);
    assert_eq!(loaded.saved_at, snapshot.saved_at);
    assert_eq!(loaded.workflow_session_id, snapshot.workflow_session_id);
    assert_eq!(
        loaded.workflow_state.feature_name,
        snapshot.workflow_state.feature_name
    );
    assert_eq!(loaded.ui_state.name, snapshot.ui_state.name);
    assert_eq!(loaded.total_elapsed_before_resume_ms, 5000);
}

#[test]
fn test_snapshot_info() {
    let state = create_test_state();
    let ui_state = create_test_ui_state();
    let saved_at = chrono::Utc::now().to_rfc3339();

    let snapshot = SessionSnapshot::new_with_timestamp(
        PathBuf::from("/tmp/test"),
        "test-session-id".to_string(),
        PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
        state,
        ui_state,
        0,
        saved_at,
        "claude-only".to_string(),
        None,
        0,
    );

    let info = snapshot.info();
    assert_eq!(info.workflow_session_id, "test-session-id");
    assert_eq!(info.feature_name, "test-feature");
    assert_eq!(info.phase, "Planning");
    assert_eq!(info.iteration, 1);
}

#[test]
fn test_conflict_detection_no_conflict() {
    let mut state = create_test_state();
    state.set_updated_at_with("2025-12-29T14:00:00Z");

    let ui_state = create_test_ui_state();
    let snapshot = SessionSnapshot::new_with_timestamp(
        PathBuf::from("/tmp/test"),
        "test-session-id".to_string(),
        PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
        state.clone(),
        ui_state,
        0,
        "2025-12-29T15:00:00Z".to_string(),
        "claude-only".to_string(),
        None,
        0,
    );

    let conflict = check_conflict(&snapshot, &state);
    assert!(conflict.is_none());
}

#[test]
fn test_conflict_detection_with_conflict() {
    let mut state = create_test_state();
    state.set_updated_at_with("2025-12-29T16:00:00Z");

    let ui_state = create_test_ui_state();
    let original_state = create_test_state();
    let snapshot = SessionSnapshot::new_with_timestamp(
        PathBuf::from("/tmp/test"),
        "test-session-id".to_string(),
        PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
        original_state,
        ui_state,
        0,
        "2025-12-29T15:00:00Z".to_string(),
        "claude-only".to_string(),
        None,
        0,
    );

    let conflict = check_conflict(&snapshot, &state);
    assert!(conflict.is_some());
    assert_eq!(conflict.unwrap(), "2025-12-29T16:00:00Z");
}

#[test]
fn test_session_centric_snapshot_path() {
    let (_temp_dir, _guard) = test_env();

    let session_id = format!("test-session-{}", uuid::Uuid::new_v4());
    let path = get_snapshot_path(&session_id).unwrap();

    // With test override, path is: temp_dir/sessions/<session_id>/session.json
    assert!(path.to_string_lossy().contains("/sessions/"));
    assert!(path.to_string_lossy().contains(&session_id));
    assert!(path.to_string_lossy().ends_with("/session.json"));
}

#[test]
fn test_save_and_load_session_centric_snapshot() {
    let (_temp_dir, _guard) = test_env();

    let state = create_test_state();
    let ui_state = create_test_ui_state();
    let session_id = format!("test-session-{}", uuid::Uuid::new_v4());
    let saved_at = chrono::Utc::now().to_rfc3339();

    let snapshot = SessionSnapshot::new_with_timestamp(
        PathBuf::from("/tmp/test"),
        session_id.clone(),
        PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
        state,
        ui_state,
        0,
        saved_at,
        "claude-only".to_string(),
        None,
        0,
    );

    let save_result = save_snapshot(&snapshot);
    assert!(save_result.is_ok());
    let saved_path = save_result.unwrap();

    assert!(saved_path.to_string_lossy().contains(&session_id));
    assert!(saved_path.to_string_lossy().ends_with("/session.json"));

    let load_result = load_snapshot(&session_id);
    assert!(load_result.is_ok());
    let loaded = load_result.unwrap();

    assert_eq!(loaded.workflow_session_id, session_id);
    assert_eq!(loaded.workflow_state.feature_name, "test-feature");

    let _ = delete_snapshot(&session_id);
}
