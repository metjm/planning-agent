use super::*;
use crate::domain::input::NewWorkflowInput;
use crate::domain::types::WorkflowId;
use crate::planning_paths::session_dir;
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
    let input = NewWorkflowInput::new("test-feature-wd", "Test with working dir", 3);
    let workflow_id = WorkflowId::new();
    let temp_dir = tempdir().unwrap();
    let working_dir = temp_dir.path();

    let result =
        pre_create_session_folder_with_working_dir(&input, &workflow_id, Some(working_dir));
    assert!(
        result.is_ok(),
        "pre_create_session_folder_with_working_dir should succeed: {:?}",
        result
    );

    // Verify session_info.json was created with correct working_dir
    let info = SessionInfo::load(&workflow_id.to_string());
    assert!(info.is_ok());
    let info = info.unwrap();
    assert_eq!(info.working_dir, working_dir);
    assert_eq!(info.feature_name, "test-feature-wd");

    // Cleanup
    if let Ok(session_folder) = session_dir(&workflow_id.to_string()) {
        let _ = fs::remove_dir_all(session_folder);
    }
}
