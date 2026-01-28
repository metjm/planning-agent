//! Tests for /merge-worktree validation helper function.

use super::*;
use std::path::PathBuf;

#[test]
fn test_validate_merge_worktree_no_worktree() {
    let result = validate_merge_worktree(None);
    assert_eq!(result, MergeWorktreeValidation::NoWorktree);
}

#[test]
fn test_validate_merge_worktree_deleted() {
    // Create a WorktreeState pointing to a non-existent directory
    let wt = WorktreeState::new(
        PathBuf::from("/nonexistent/path/that/does/not/exist"),
        "test-branch".to_string(),
        Some("main".to_string()),
        PathBuf::from("/project"),
    );

    let result = validate_merge_worktree(Some(&wt));
    assert_eq!(result, MergeWorktreeValidation::WorktreeDeleted);
}

#[test]
fn test_validate_merge_worktree_valid() {
    // Use a directory that definitely exists
    let existing_path = std::env::current_dir().unwrap();

    let wt = WorktreeState::new(
        existing_path.clone(),
        "test-branch".to_string(),
        Some("main".to_string()),
        PathBuf::from("/project"),
    );

    let result = validate_merge_worktree(Some(&wt));
    match result {
        MergeWorktreeValidation::Valid(state) => {
            assert_eq!(state.worktree_path(), existing_path);
            assert_eq!(state.branch_name(), "test-branch");
        }
        _ => panic!("Expected Valid variant"),
    }
}
