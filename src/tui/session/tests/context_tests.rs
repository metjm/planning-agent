use super::*;
use std::path::PathBuf;

#[test]
fn test_context_new_defaults_effective_to_base() {
    let config = WorkflowConfig::default_config();
    let ctx = SessionContext::new(
        PathBuf::from("/base/dir"),
        None,
        PathBuf::from("/state/path.json"),
        config,
    );

    assert_eq!(ctx.base_working_dir, PathBuf::from("/base/dir"));
    assert_eq!(ctx.effective_working_dir, PathBuf::from("/base/dir"));
}

#[test]
fn test_context_new_with_effective() {
    let config = WorkflowConfig::default_config();
    let ctx = SessionContext::new(
        PathBuf::from("/base/dir"),
        Some(PathBuf::from("/worktree/dir")),
        PathBuf::from("/state/path.json"),
        config,
    );

    assert_eq!(ctx.base_working_dir, PathBuf::from("/base/dir"));
    assert_eq!(ctx.effective_working_dir, PathBuf::from("/worktree/dir"));
}

#[test]
fn test_compute_effective_without_worktree() {
    let base = PathBuf::from("/base/dir");
    let result = compute_effective_working_dir(&base, None);
    assert_eq!(result, base);
}

#[test]
fn test_compute_effective_with_nonexistent_worktree() {
    let base = PathBuf::from("/base/dir");
    let wt = WorktreeState {
        worktree_path: PathBuf::from("/nonexistent/worktree"),
        branch_name: "feature".to_string(),
        source_branch: Some("main".to_string()),
        original_dir: base.clone(),
    };
    let result = compute_effective_working_dir(&base, Some(&wt));
    // Should fall back to base since worktree doesn't exist
    assert_eq!(result, base);
}

#[test]
fn test_validate_working_dir_nonexistent() {
    let result = validate_working_dir(Path::new("/definitely/does/not/exist/12345"));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("no longer exists"));
}
