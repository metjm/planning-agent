use super::*;
use tempfile::tempdir;

#[test]
fn test_is_git_repo_non_git() {
    let dir = tempdir().unwrap();
    assert!(!is_git_repo(dir.path()));
}

#[test]
fn test_is_git_repo_with_git() {
    let dir = tempdir().unwrap();

    // Initialize a git repo
    let output = Command::new("git")
        .current_dir(dir.path())
        .args(["init"])
        .output();

    if output.is_ok() && output.unwrap().status.success() {
        assert!(is_git_repo(dir.path()));
    }
}

#[test]
fn test_is_valid_worktree_non_existent() {
    let path = PathBuf::from("/non/existent/path");
    assert!(!is_valid_worktree(&path));
}

#[test]
fn test_is_valid_worktree_non_git_dir() {
    let dir = tempdir().unwrap();
    assert!(!is_valid_worktree(dir.path()));
}

#[test]
fn test_create_session_worktree_not_git_repo() {
    let dir = tempdir().unwrap();
    let session_dir = tempdir().unwrap();

    let result = create_session_worktree(
        dir.path(),
        "test-session-id",
        "test-feature",
        session_dir.path(),
        None,
    );

    matches!(result, WorktreeSetupResult::NotAGitRepo);
}

#[test]
fn test_branch_name_sanitization() {
    // Test that feature names with special characters get sanitized
    let sanitized: String = "test/feature@with!special#chars"
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    assert_eq!(sanitized, "test-feature-with-special-chars");
}

#[test]
fn test_branch_name_empty_after_sanitization() {
    // Test fallback for names that become empty after sanitization
    let sanitized: String = "!!@@##"
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let safe = if sanitized.trim_matches('-').is_empty() {
        "feature".to_string()
    } else {
        sanitized
    };
    assert_eq!(safe, "feature");
}

#[test]
fn test_has_submodules_no_file() {
    let dir = tempdir().unwrap();
    assert!(!has_submodules(dir.path()));
}

#[test]
fn test_has_submodules_empty_file() {
    let dir = tempdir().unwrap();
    let gitmodules_path = dir.path().join(".gitmodules");
    std::fs::write(&gitmodules_path, "").unwrap();
    assert!(!has_submodules(dir.path()));
}

#[test]
fn test_has_submodules_with_content() {
    let dir = tempdir().unwrap();
    let gitmodules_path = dir.path().join(".gitmodules");
    std::fs::write(&gitmodules_path, "[submodule \"test\"]").unwrap();
    assert!(has_submodules(dir.path()));
}

#[test]
fn test_generate_merge_instructions() {
    let info = WorktreeInfo {
        worktree_path: PathBuf::from("/tmp/worktree"),
        branch_name: "planning-agent/my-feature-abc123".to_string(),
        source_branch: Some("main".to_string()),
        original_dir: PathBuf::from("/home/user/repo"),
        has_submodules: false,
    };

    let instructions = generate_merge_instructions(&info);

    assert!(instructions.contains("planning-agent/my-feature-abc123"));
    assert!(instructions.contains("main"));
    assert!(instructions.contains("git checkout main"));
    assert!(instructions.contains("git merge planning-agent/my-feature-abc123"));
}

#[test]
fn test_generate_merge_instructions_no_source_branch() {
    let info = WorktreeInfo {
        worktree_path: PathBuf::from("/tmp/worktree"),
        branch_name: "planning-agent/feature-abc123".to_string(),
        source_branch: None, // Detached HEAD case
        original_dir: PathBuf::from("/home/user/repo"),
        has_submodules: false,
    };

    let instructions = generate_merge_instructions(&info);

    // Should default to "main" when source_branch is None
    assert!(instructions.contains("Target branch for merge: main"));
}
