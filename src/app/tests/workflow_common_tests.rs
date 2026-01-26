use super::*;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_plan_file_has_content_returns_false_for_nonexistent() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("nonexistent.md");
    assert!(!plan_file_has_content(&path));
}

#[test]
fn test_plan_file_has_content_returns_false_for_empty_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("empty.md");
    fs::write(&path, "").unwrap();
    assert!(!plan_file_has_content(&path));
}

#[test]
fn test_plan_file_has_content_returns_true_for_file_with_content() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("plan.md");
    fs::write(&path, "# Plan\n\nThis is a plan.").unwrap();
    assert!(plan_file_has_content(&path));
}

#[test]
fn test_pre_create_session_folder_with_working_dir() {
    let state = State::new("test-feature-wd", "Test with working dir", 3).unwrap();
    let temp_dir = tempdir().unwrap();
    let working_dir = temp_dir.path();

    let result = pre_create_session_folder_with_working_dir(&state, Some(working_dir));
    assert!(
        result.is_ok(),
        "pre_create_session_folder_with_working_dir should succeed: {:?}",
        result
    );

    // Verify session_info.json was created with correct working_dir
    let info = SessionInfo::load(&state.workflow_session_id);
    assert!(info.is_ok());
    let info = info.unwrap();
    assert_eq!(info.working_dir, working_dir);
    assert_eq!(info.feature_name, "test-feature-wd");

    // Cleanup
    if let Some(parent) = state.plan_file.parent() {
        let _ = fs::remove_dir_all(parent);
    }
}
