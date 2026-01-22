#[cfg(test)]
mod tests {
    use crate::app::util::*;
    use crate::phases;
    use crate::state::State;
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

        assert!(
            summary.contains("You chose to proceed without AI approval after 3 review iterations.")
        );
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

        assert!(
            summary.contains("You chose to proceed without AI approval after 5 review iterations.")
        );
        assert!(summary.contains("Could not read plan file:"));
        assert!(summary.contains("[i] Implement"));
        assert!(summary.contains("[d] Decline"));
    }

    #[test]
    fn test_build_max_iterations_summary_with_preview_and_full_feedback() {
        let dir = tempdir().unwrap();
        let working_dir = dir.path();

        let mut state = State::new("test-feature", "Test objective", 3).unwrap();
        state.iteration = 3;

        let long_feedback = "Line 1: First issue found\n\
                             Line 2: Second issue found\n\
                             Line 3: Third issue found\n\
                             Line 4: Fourth issue found\n\
                             Line 5: Fifth issue found\n\
                             Line 6: Sixth issue found\n\
                             Line 7: Seventh issue found\n\
                             Line 8: Additional detailed feedback here";

        let reviews = vec![phases::ReviewResult {
            agent_name: "test-reviewer".to_string(),
            needs_revision: true,
            feedback: long_feedback.to_string(),
            summary: "Multiple issues found in the plan".to_string(),
        }];

        let summary = build_max_iterations_summary(&state, working_dir, &reviews);

        assert!(summary.contains("## Review Summary"));
        assert!(summary.contains("**1 reviewer(s):** 1 needs revision, 0 approved"));
        assert!(summary.contains("**Needs Revision:** TEST-REVIEWER"));
        assert!(summary.contains(
            "- **TEST-REVIEWER** - **NEEDS REVISION**: Multiple issues found in the plan"
        ));

        assert!(summary.contains("## Latest Review Feedback (Preview)"));
        assert!(summary.contains("Scroll down for full feedback"));
        assert!(summary.contains("TEST-REVIEWER (NEEDS REVISION)"));

        assert!(summary.contains("## Full Review Feedback"));
        assert!(summary.contains("Line 6: Sixth issue found"));
        assert!(summary.contains("Line 7: Seventh issue found"));
        assert!(summary.contains("Line 8: Additional detailed feedback here"));

        assert!(summary.contains("[p] Proceed"));
        assert!(summary.contains("[c] Continue Review"));
        assert!(summary.contains("[d] Restart with Feedback"));
    }

    #[test]
    fn test_build_max_iterations_summary_empty_reviews() {
        let dir = tempdir().unwrap();
        let working_dir = dir.path();

        let mut state = State::new("test-feature", "Test objective", 3).unwrap();
        state.iteration = 3;

        let reviews: Vec<phases::ReviewResult> = vec![];

        let summary = build_max_iterations_summary(&state, working_dir, &reviews);

        assert!(summary.contains("No review feedback available"));

        assert!(!summary.contains("## Review Summary"));
        assert!(!summary.contains("## Latest Review Feedback (Preview)"));
        assert!(!summary.contains("## Full Review Feedback"));

        assert!(summary.contains("[p] Proceed"));
        assert!(summary.contains("[c] Continue Review"));
        assert!(summary.contains("[d] Restart with Feedback"));
    }

    #[test]
    fn test_build_max_iterations_summary_multiple_reviewers() {
        let dir = tempdir().unwrap();
        let working_dir = dir.path();

        let mut state = State::new("test-feature", "Test objective", 3).unwrap();
        state.iteration = 2;

        let reviews = vec![
            phases::ReviewResult {
                agent_name: "reviewer-1".to_string(),
                needs_revision: true,
                feedback: "Issue A\nIssue B\nIssue C".to_string(),
                summary: "Several issues need addressing".to_string(),
            },
            phases::ReviewResult {
                agent_name: "reviewer-2".to_string(),
                needs_revision: false,
                feedback: "Looks good to me".to_string(),
                summary: "Plan is well structured".to_string(),
            },
        ];

        let summary = build_max_iterations_summary(&state, working_dir, &reviews);

        assert!(summary.contains("## Review Summary"));
        assert!(summary.contains("**2 reviewer(s):** 1 needs revision, 1 approved"));
        assert!(summary.contains("**Needs Revision:** REVIEWER-1"));
        assert!(summary.contains("**Approved:** REVIEWER-2"));

        assert!(summary
            .contains("- **REVIEWER-1** - **NEEDS REVISION**: Several issues need addressing"));
        assert!(summary.contains("- **REVIEWER-2** - **APPROVED**: Plan is well structured"));

        assert!(summary.contains("REVIEWER-1 (NEEDS REVISION)"));
        assert!(summary.contains("REVIEWER-2 (APPROVED)"));

        assert!(summary.contains("Issue A"));
        assert!(summary.contains("Issue B"));
        assert!(summary.contains("Issue C"));
        assert!(summary.contains("Looks good to me"));
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
}
