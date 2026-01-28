//! Tests for session_store module.

use super::*;
use crate::domain::types::{
    FeatureName, FeedbackPath, MaxIterations, Objective, PlanPath, TimestampUtc, WorkingDir,
};
use crate::domain::view::WorkflowView;
use crate::domain::WorkflowEvent;
use crate::planning_paths::{set_home_for_test, TestHomeGuard};
use crate::tui::scroll::ScrollState;
use std::path::PathBuf;
use tempfile::tempdir;
use uuid::Uuid;

/// Helper to set up an isolated test home directory.
fn test_env() -> (tempfile::TempDir, TestHomeGuard) {
    let dir = tempdir().expect("Failed to create temp dir");
    let guard = set_home_for_test(dir.path().to_path_buf());
    (dir, guard)
}

fn create_test_workflow_view() -> WorkflowView {
    let mut view = WorkflowView::default();
    let agg_id = Uuid::new_v4().to_string();

    view.apply_event(
        &agg_id,
        &WorkflowEvent::WorkflowCreated {
            feature_name: FeatureName::from("test-feature"),
            objective: Objective::from("Test objective"),
            working_dir: WorkingDir::from(PathBuf::from("/tmp/test").as_path()),
            max_iterations: MaxIterations(3),
            plan_path: PlanPath::from(PathBuf::from("/tmp/test/plan.md")),
            feedback_path: FeedbackPath::from(PathBuf::from("/tmp/test/feedback.md")),
            created_at: TimestampUtc::now(),
        },
        1,
    );

    view
}

fn create_test_ui_state() -> SessionUiState {
    SessionUiState {
        id: 1,
        name: "Test Session".to_string(),
        status: SessionStatus::Planning,
        output_lines: vec!["line1".to_string(), "line2".to_string()],
        output_scroll: ScrollState::new(),
        streaming_lines: Vec::new(),
        streaming_scroll: ScrollState::new(),
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
        todos: HashMap::new(),
        todo_scroll: ScrollState::new(),
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
        review_history_scroll: ScrollState::new(),
    }
}

#[test]
fn test_snapshot_creation() {
    let workflow_view = create_test_workflow_view();
    let ui_state = create_test_ui_state();
    let saved_at = chrono::Utc::now().to_rfc3339();

    let snapshot = SessionSnapshot::new_with_timestamp(
        PathBuf::from("/tmp/test"),
        "test-session-id".to_string(),
        PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
        ui_state,
        0,
        saved_at,
        "claude-only".to_string(),
        workflow_view,
        0,
    );

    assert_eq!(snapshot.version, SNAPSHOT_VERSION);
    assert!(!snapshot.saved_at.is_empty());
    assert_eq!(snapshot.workflow_session_id, "test-session-id");
    assert_eq!(
        snapshot.workflow_view.feature_name().unwrap().0,
        "test-feature"
    );
    assert_eq!(snapshot.workflow_name, "claude-only");
    assert_eq!(snapshot.last_event_sequence, 0);
}

#[test]
fn test_snapshot_serialization_roundtrip() {
    let workflow_view = create_test_workflow_view();
    let ui_state = create_test_ui_state();
    let saved_at = chrono::Utc::now().to_rfc3339();

    let snapshot = SessionSnapshot::new_with_timestamp(
        PathBuf::from("/tmp/test"),
        "test-session-id".to_string(),
        PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
        ui_state,
        5000,
        saved_at,
        "default".to_string(),
        workflow_view,
        0,
    );

    let json = serde_json::to_string(&snapshot).unwrap();
    let loaded: SessionSnapshot = serde_json::from_str(&json).unwrap();

    assert_eq!(loaded.version, snapshot.version);
    assert_eq!(loaded.saved_at, snapshot.saved_at);
    assert_eq!(loaded.workflow_session_id, snapshot.workflow_session_id);
    assert_eq!(
        loaded.workflow_view.feature_name().map(|f| f.0.as_str()),
        snapshot.workflow_view.feature_name().map(|f| f.0.as_str())
    );
    assert_eq!(loaded.ui_state.name, snapshot.ui_state.name);
    assert_eq!(loaded.total_elapsed_before_resume_ms, 5000);
}

#[test]
fn test_snapshot_info() {
    let workflow_view = create_test_workflow_view();
    let ui_state = create_test_ui_state();
    let saved_at = chrono::Utc::now().to_rfc3339();

    let snapshot = SessionSnapshot::new_with_timestamp(
        PathBuf::from("/tmp/test"),
        "test-session-id".to_string(),
        PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
        ui_state,
        0,
        saved_at,
        "claude-only".to_string(),
        workflow_view,
        0,
    );

    let info = snapshot.info();
    assert_eq!(info.workflow_session_id, "test-session-id");
    assert_eq!(info.feature_name, "test-feature");
    assert_eq!(info.phase, "Planning");
    assert_eq!(info.iteration, 1);
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

    let workflow_view = create_test_workflow_view();
    let ui_state = create_test_ui_state();
    let session_id = format!("test-session-{}", uuid::Uuid::new_v4());
    let saved_at = chrono::Utc::now().to_rfc3339();

    let snapshot = SessionSnapshot::new_with_timestamp(
        PathBuf::from("/tmp/test"),
        session_id.clone(),
        PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
        ui_state,
        0,
        saved_at,
        "claude-only".to_string(),
        workflow_view,
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
    assert_eq!(
        loaded.workflow_view.feature_name().unwrap().0,
        "test-feature"
    );

    let _ = delete_snapshot(&session_id);
}
