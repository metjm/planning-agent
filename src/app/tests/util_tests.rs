use super::*;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

#[test]
fn test_build_approval_summary_with_plan_content() {
    let dir = tempdir().unwrap();
    let plan_path = dir.path().join("plan.md");
    let plan_content = "# My Plan\n\n## Steps\n\n1. Step one\n2. Step two";
    fs::write(&plan_path, plan_content).unwrap();

    let summary = build_approval_summary(&plan_path, false, 1);

    assert!(summary.contains("The plan has been approved by AI review."));
    assert!(summary.contains(&format!("Plan file: {}", plan_path.display())));
    assert!(summary.contains("## Plan Contents"));
    assert!(summary.contains("# My Plan"));
    assert!(summary.contains("1. Step one"));
    assert!(summary.contains("2. Step two"));
    assert!(!summary.contains("[i] Implement"));
}

#[test]
fn test_build_approval_summary_with_override() {
    let dir = tempdir().unwrap();
    let plan_path = dir.path().join("plan.md");
    let plan_content = "# My Plan\n\nSome content here.";
    fs::write(&plan_path, plan_content).unwrap();

    let summary = build_approval_summary(&plan_path, true, 3);

    assert!(summary.contains("You chose to proceed without AI approval after 3 review iterations."));
    assert!(summary.contains(&format!("Plan file: {}", plan_path.display())));
    assert!(summary.contains("## Plan Contents"));
    assert!(summary.contains("# My Plan"));
    assert!(summary.contains("[i] Implement"));
    assert!(summary.contains("[d] Decline"));
}

#[test]
fn test_build_approval_summary_missing_file() {
    let dir = tempdir().unwrap();
    let plan_path = dir.path().join("nonexistent.md");

    let summary = build_approval_summary(&plan_path, false, 1);

    assert!(summary.contains("The plan has been approved by AI review."));
    assert!(summary.contains(&format!("Plan file: {}", plan_path.display())));
    assert!(summary.contains("Could not read plan file:"));
    assert!(!summary.contains("## Plan Contents"));
}

#[test]
fn test_build_approval_summary_missing_file_with_override() {
    let dir = tempdir().unwrap();
    let plan_path = dir.path().join("nonexistent.md");

    let summary = build_approval_summary(&plan_path, true, 5);

    assert!(summary.contains("You chose to proceed without AI approval after 5 review iterations."));
    assert!(summary.contains("Could not read plan file:"));
    assert!(summary.contains("[i] Implement"));
    assert!(summary.contains("[d] Decline"));
}

#[test]
fn test_build_resume_command_simple_path() {
    let path = Path::new("/home/user/projects/myapp");
    let cmd = build_resume_command("abc123", path);
    assert_eq!(
        cmd,
        "planning --resume-session abc123 --working-dir /home/user/projects/myapp"
    );
}

#[test]
fn test_build_resume_command_path_with_spaces() {
    let path = Path::new("/home/user/My Projects/my app");
    let cmd = build_resume_command("abc123", path);
    assert_eq!(
        cmd,
        "planning --resume-session abc123 --working-dir \"/home/user/My Projects/my app\""
    );
}

#[test]
fn test_build_resume_command_path_with_special_chars() {
    let path = Path::new("/home/user/$project/test`dir/quote\"here");
    let cmd = build_resume_command("xyz789", path);
    assert_eq!(
        cmd,
        "planning --resume-session xyz789 --working-dir \"/home/user/\\$project/test\\`dir/quote\\\"here\""
    );
}

#[test]
fn test_build_resume_command_path_with_backslash() {
    let path = Path::new("/home/user/path\\with\\backslash");
    let cmd = build_resume_command("def456", path);
    assert_eq!(
        cmd,
        "planning --resume-session def456 --working-dir \"/home/user/path\\\\with\\\\backslash\""
    );
}

#[test]
fn test_build_resume_command_path_with_single_quote() {
    let path = Path::new("/home/user/it's a path");
    let cmd = build_resume_command("test123", path);
    assert_eq!(
        cmd,
        "planning --resume-session test123 --working-dir \"/home/user/it's a path\""
    );
}

#[test]
fn test_shell_quote_path_no_quoting_needed() {
    let path = Path::new("/simple/path/here");
    let quoted = shell_quote_path(path);
    assert_eq!(quoted, "/simple/path/here");
}

#[test]
fn test_shell_quote_path_with_tilde() {
    let path = Path::new("~/projects");
    let quoted = shell_quote_path(path);
    assert_eq!(quoted, "\"~/projects\"");
}

#[test]
fn test_shell_quote_path_with_ampersand() {
    let path = Path::new("/path/with&special");
    let quoted = shell_quote_path(path);
    assert_eq!(quoted, "\"/path/with&special\"");
}

#[test]
fn test_shell_quote_path_with_glob() {
    let path = Path::new("/path/with*glob");
    let quoted = shell_quote_path(path);
    assert_eq!(quoted, "\"/path/with*glob\"");
}
