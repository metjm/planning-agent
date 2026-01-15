use crate::phases;
use crate::planning_paths;
use crate::session_logger::{LogCategory, SessionLogger};
use crate::state::State;
use crate::tui::TabManager;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

pub fn get_run_id() -> String {
    use std::sync::OnceLock;
    static RUN_ID: OnceLock<String> = OnceLock::new();
    RUN_ID
        .get_or_init(|| chrono::Local::now().format("%Y%m%d-%H%M%S").to_string())
        .clone()
}

/// Logs a workflow message using the session logger if available.
///
/// This is the preferred logging method for new code.
#[allow(dead_code)]
pub fn log_workflow_with_session(logger: &SessionLogger, message: &str) {
    logger.log(LogCategory::Workflow, message);
}

/// Logs a workflow message using legacy file-based logging.
///
/// **DEPRECATED**: Use `log_workflow_with_session()` for new code.
pub fn log_workflow(working_dir: &Path, message: &str) {
    let run_id = get_run_id();
    // Use home-based log path
    let log_path = match planning_paths::workflow_log_path(working_dir, &run_id) {
        Ok(p) => p,
        Err(_) => return, // Skip logging if we can't determine the path
    };
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&log_path) {
        let timestamp = chrono::Local::now().format("%H:%M:%S");
        let _ = writeln!(f, "[{}] {}", timestamp, message);
    }
}

pub fn debug_log(start: std::time::Instant, msg: &str) {
    // Use home-based debug log path
    let log_path = match planning_paths::debug_log_path() {
        Ok(p) => p,
        Err(_) => return, // Skip logging if we can't determine the path
    };
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&log_path) {
        let now = chrono::Local::now().format("%H:%M:%S%.3f");
        let _ = writeln!(f, "[{}][+{:?}] {}", now, start.elapsed(), msg);
    }
}

pub fn truncate_for_summary(text: &str, max_len: usize) -> String {
    let mut cleaned = text.replace('\n', " ");
    if cleaned.len() > max_len {
        cleaned.truncate(max_len.saturating_sub(3));
        cleaned.push_str("...");
    }
    cleaned
}

pub fn build_review_failure_summary(
    reviews: &HashMap<String, phases::ReviewResult>,
    failures: &[phases::ReviewFailure],
) -> String {
    let mut summary = String::new();
    summary.push_str("# Reviewer Failures Detected\n\n");

    let mut successful = reviews.keys().cloned().collect::<Vec<_>>();
    successful.sort();
    if successful.is_empty() {
        summary.push_str("Successful reviewers: none\n\n");
    } else {
        summary.push_str(&format!(
            "Successful reviewers ({}): {}\n\n",
            successful.len(),
            successful.join(", ")
        ));
    }

    summary.push_str("Failed reviewers:\n");
    let mut has_bundles = false;
    for failure in failures {
        let error = truncate_for_summary(&failure.error, 200);
        summary.push_str(&format!("- {}: {}\n", failure.agent_name, error));
        if let Some(ref path) = failure.bundle_path {
            summary.push_str(&format!("  Diagnostics bundle: {}\n", path.display()));
            has_bundles = true;
        }
    }

    if has_bundles {
        summary.push_str("\n**Note:** Diagnostics bundles may contain sensitive information from logs.\n");
    }

    summary.push_str(
        "\nChoose whether to retry the failed reviewers or continue with the successful reviews.",
    );
    summary
}

/// Builds a summary for all-reviewers-failed scenario.
/// This is separate from build_review_failure_summary because there are no partial reviews
/// to continue with - we need different action options.
pub fn build_all_reviewers_failed_summary(
    failures: &[phases::ReviewFailure],
    retry_attempts: usize,
    max_retries: usize,
) -> String {
    let mut summary = String::new();
    summary.push_str("# All Reviewers Failed\n\n");
    summary.push_str(&format!(
        "All {} reviewer(s) failed after {} retry attempt(s) (max: {}).\n\n",
        failures.len(),
        retry_attempts,
        max_retries
    ));

    summary.push_str("## Failed Reviewers\n\n");
    let mut has_bundles = false;
    for failure in failures {
        let error = truncate_for_summary(&failure.error, 200);
        let failure_type = failure.kind.display_name();
        summary.push_str(&format!("- **{}** ({}): {}\n", failure.agent_name, failure_type, error));
        if let Some(ref path) = failure.bundle_path {
            summary.push_str(&format!("  Diagnostics bundle: {}\n", path.display()));
            has_bundles = true;
        }
    }

    if has_bundles {
        summary.push_str("\n**Note:** Diagnostics bundles may contain sensitive information from logs.\n");
    }

    summary.push_str("\n## Recovery Options\n\n");
    summary.push_str("- **Retry**: Try running all reviewers again\n");
    summary.push_str("- **Stop**: Save state and resume later\n");
    summary.push_str("- **Abort**: Abort the workflow\n");

    summary
}

/// Build a summary for a failure context from a resumed session.
/// This is used when resuming a session that was stopped with an unresolved failure.
pub fn build_resume_failure_summary(failure: &crate::app::failure::FailureContext) -> String {
    let mut s = String::new();
    s.push_str("# Unresolved Workflow Failure\n\n");
    s.push_str(&format!("**Type**: {}\n", failure.kind.display_name()));
    s.push_str(&format!("**Phase**: {:?}\n", failure.phase));
    if let Some(ref agent) = failure.agent_name {
        s.push_str(&format!("**Agent**: {}\n", agent));
    }
    s.push_str(&format!("**Retries**: {}/{}\n", failure.retry_count, failure.max_retries));
    s.push_str(&format!("**Failed at**: {}\n\n", failure.failed_at));
    s.push_str("This session was stopped with an unresolved failure.\n\n");
    s.push_str("## Recovery Options\n\n");
    s.push_str("- **Retry**: Retry the failed operation\n");
    s.push_str("- **Stop**: Keep session stopped for later\n");
    s.push_str("- **Abort**: Abort the workflow\n");
    s
}

/// Build a summary for a generic workflow failure (agent crash, timeout, etc.).
/// This is shown in the TUI when a phase fails and needs user decision.
pub fn build_workflow_failure_summary(
    phase: &str,
    error: &str,
    agent_name: Option<&str>,
    retry_attempts: usize,
    max_retries: usize,
    bundle_path: Option<&Path>,
) -> String {
    let mut summary = String::new();
    summary.push_str("# Workflow Failed\n\n");
    summary.push_str(&format!("**Phase**: {}\n", phase));
    if let Some(agent) = agent_name {
        summary.push_str(&format!("**Agent**: {}\n", agent));
    }
    summary.push_str(&format!(
        "**Retry attempts**: {}/{}\n\n",
        retry_attempts, max_retries
    ));

    summary.push_str("## Error\n\n");
    summary.push_str(&format!("{}\n\n", truncate_for_summary(error, 500)));

    if let Some(path) = bundle_path {
        summary.push_str(&format!("**Diagnostics bundle**: {}\n\n", path.display()));
        summary.push_str("_Note: Diagnostics bundles may contain sensitive information from logs._\n\n");
    }

    summary.push_str("## Recovery Options\n\n");
    summary.push_str("- **[r] Retry**: Retry the failed operation\n");
    summary.push_str("- **[s] Stop**: Save state and resume later\n");
    summary.push_str("- **[a] Abort**: Abort the workflow\n");

    summary
}

pub fn build_max_iterations_summary(
    state: &State,
    working_dir: &Path,
    last_reviews: &[phases::ReviewResult],
) -> String {
    let plan_path = working_dir.join(&state.plan_file);

    let mut summary = format!(
        "The plan has been reviewed {} times but has not been approved by AI.\n\nPlan file: {}\n\n",
        state.iteration,
        plan_path.display()
    );

    if let Some(ref status) = state.last_feedback_status {
        summary.push_str(&format!("Last review verdict: {:?}\n\n", status));
    }

    if !last_reviews.is_empty() {
        // New top section: Review Summary with verdict grouping
        summary.push_str("---\n\n## Review Summary\n\n");

        // Count verdicts
        let needs_revision_count = last_reviews.iter().filter(|r| r.needs_revision).count();
        let approved_count = last_reviews.len() - needs_revision_count;

        summary.push_str(&format!(
            "**{} reviewer(s):** {} needs revision, {} approved\n\n",
            last_reviews.len(),
            needs_revision_count,
            approved_count
        ));

        // Group reviewers by verdict
        let needs_revision: Vec<_> = last_reviews
            .iter()
            .filter(|r| r.needs_revision)
            .collect();
        let approved: Vec<_> = last_reviews
            .iter()
            .filter(|r| !r.needs_revision)
            .collect();

        if !needs_revision.is_empty() {
            let names: Vec<_> = needs_revision
                .iter()
                .map(|r| r.agent_name.to_uppercase())
                .collect();
            summary.push_str(&format!("**Needs Revision:** {}\n\n", names.join(", ")));
        }

        if !approved.is_empty() {
            let names: Vec<_> = approved
                .iter()
                .map(|r| r.agent_name.to_uppercase())
                .collect();
            summary.push_str(&format!("**Approved:** {}\n\n", names.join(", ")));
        }

        // Per-agent summary bullets
        for review in last_reviews {
            let verdict = if review.needs_revision {
                "NEEDS REVISION"
            } else {
                "APPROVED"
            };
            let truncated_summary = truncate_for_summary(&review.summary, 120);
            summary.push_str(&format!(
                "- **{}** - **{}**: {}\n",
                review.agent_name.to_uppercase(),
                verdict,
                truncated_summary
            ));
        }
        summary.push('\n');

        // Preview section: concise cut-off view
        summary.push_str("---\n\n## Latest Review Feedback (Preview)\n\n");
        summary.push_str("_Scroll down for full feedback_\n\n");
        for review in last_reviews {
            let verdict = if review.needs_revision {
                "NEEDS REVISION"
            } else {
                "APPROVED"
            };
            summary.push_str(&format!(
                "### {} ({})\n\n",
                review.agent_name.to_uppercase(),
                verdict
            ));
            let preview: String = review.feedback.lines().take(5).collect::<Vec<_>>().join("\n");
            summary.push_str(&format!("{}\n\n", truncate_for_summary(&preview, 300)));
        }

        // Full feedback section: complete review content
        summary.push_str("---\n\n## Full Review Feedback\n\n");
        for review in last_reviews {
            let verdict = if review.needs_revision {
                "NEEDS REVISION"
            } else {
                "APPROVED"
            };
            summary.push_str(&format!(
                "### {} ({})\n\n",
                review.agent_name.to_uppercase(),
                verdict
            ));
            summary.push_str(&format!("{}\n\n", review.feedback));
        }
    } else {
        summary.push_str("---\n\n_No review feedback available._\n\n");
    }

    summary.push_str("---\n\n");
    summary.push_str("Choose an action:\n");
    summary.push_str("- **[p] Proceed**: Accept the current plan and continue to implementation\n");
    summary.push_str("- **[c] Continue Review**: Run another review cycle (adds 1 to max iterations)\n");
    summary
        .push_str("- **[d] Restart with Feedback**: Provide feedback to restart the entire workflow\n");

    summary
}

pub fn build_plan_failure_summary(
    error: &str,
    plan_path: &Path,
    plan_exists: bool,
) -> String {
    let mut summary = String::new();
    summary.push_str("# Plan Generation Failed\n\n");
    summary.push_str(&format!("**Error:** {}\n\n", truncate_for_summary(error, 300)));
    summary.push_str(&format!("**Plan file:** {}\n\n", plan_path.display()));

    if plan_exists {
        summary.push_str("An existing plan file was found from a previous attempt.\n\n");
        summary.push_str("---\n\n");
        summary.push_str("Choose an action:\n");
        summary.push_str("- **[r] Retry**: Re-run plan generation from scratch\n");
        summary.push_str("- **[c] Continue**: Use the existing plan file and proceed to review\n");
        summary.push_str("- **[a] Abort**: End the workflow\n");
    } else {
        summary.push_str("No existing plan file found.\n\n");
        summary.push_str("---\n\n");
        summary.push_str("Choose an action:\n");
        summary.push_str("- **[r] Retry**: Re-run plan generation\n");
        summary.push_str("- **[a] Abort**: End the workflow\n");
        summary.push_str("\n_Note: [c] Continue is only available when an existing plan file exists._\n");
    }

    summary
}

pub fn shorten_model_name(full_name: &str) -> String {
    if full_name.contains("opus") {
        if full_name.contains("4-5") || full_name.contains("4.5") {
            "opus-4.5".to_string()
        } else {
            "opus".to_string()
        }
    } else if full_name.contains("sonnet") {
        "sonnet".to_string()
    } else if full_name.contains("haiku") {
        "haiku".to_string()
    } else {
        full_name.split('-').take(2).collect::<Vec<_>>().join("-")
    }
}

/// Builds the approval summary for AI-approved plans, including the full plan content.
///
/// This summary is shown in the approval dialog when the AI has approved a plan.
/// It includes the plan path and the full plan content for user review.
pub fn build_approval_summary(plan_path: &Path, approval_overridden: bool, iteration: u32) -> String {
    let mut summary = if approval_overridden {
        format!(
            "You chose to proceed without AI approval after {} review iterations.\n\n\
             Plan file: {}\n",
            iteration,
            plan_path.display()
        )
    } else {
        format!(
            "The plan has been approved by AI review.\n\nPlan file: {}\n",
            plan_path.display()
        )
    };

    // Read plan content and append it to the summary
    match std::fs::read_to_string(plan_path) {
        Ok(content) => {
            summary.push_str("\n---\n\n## Plan Contents\n\n");
            summary.push_str(&content);
        }
        Err(e) => {
            summary.push_str(&format!("\n---\n\n_Could not read plan file: {}_\n", e));
        }
    }

    if approval_overridden {
        summary.push_str("\n---\n\nAvailable actions:\n");
        summary.push_str("- **[i] Implement**: Launch Claude to implement the unapproved plan\n");
        summary.push_str("- **[d] Decline**: Provide feedback and restart the workflow\n");
    }

    summary
}

/// Builds a shell-safe resume command for a stopped session.
///
/// The command includes the session ID and working directory, with proper
/// shell quoting for paths that contain spaces or special characters.
pub fn build_resume_command(session_id: &str, working_dir: &Path) -> String {
    let quoted_dir = shell_quote_path(working_dir);
    format!(
        "planning --resume-session {} --working-dir {}",
        session_id, quoted_dir
    )
}

/// Quotes a path for safe use in shell commands.
///
/// - Returns the path as-is if it contains only safe characters
/// - Wraps in double quotes and escapes internal double quotes/backslashes
///   if the path contains spaces or special characters
fn shell_quote_path(path: &Path) -> String {
    let s = path.display().to_string();

    // Check if quoting is needed
    let needs_quoting = s.chars().any(|c| {
        matches!(
            c,
            ' ' | '\t'
                | '\n'
                | '"'
                | '\''
                | '\\'
                | '$'
                | '`'
                | '!'
                | '&'
                | '|'
                | ';'
                | '('
                | ')'
                | '<'
                | '>'
                | '*'
                | '?'
                | '['
                | ']'
                | '{'
                | '}'
                | '#'
                | '~'
        )
    });

    if !needs_quoting {
        return s;
    }

    // Use double quotes, escaping internal double quotes and backslashes
    let mut quoted = String::with_capacity(s.len() + 10);
    quoted.push('"');
    for c in s.chars() {
        match c {
            '"' | '\\' | '$' | '`' => {
                quoted.push('\\');
                quoted.push(c);
            }
            _ => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

pub fn format_window_title(tab_manager: &TabManager) -> String {
    use crate::tui::SessionStatus;

    let session = tab_manager.active();
    let plan_name = session.feature_name();

    let status = match session.status {
        SessionStatus::InputPending => "Input",
        SessionStatus::Planning => session.phase_name(),
        SessionStatus::GeneratingSummary => "Generating Summary",
        SessionStatus::AwaitingApproval => "Awaiting Approval",
        SessionStatus::Stopped => "Stopped",
        SessionStatus::Complete => "Complete",
        SessionStatus::Error => "Error",
        SessionStatus::Verifying => "Verifying",
        SessionStatus::Fixing => "Fixing",
        SessionStatus::VerificationComplete => "Verified",
    };

    if plan_name.is_empty() || plan_name == "New Tab" {
        "Planning Agent".to_string()
    } else {
        format!("[{}] {} - Planning Agent", status, plan_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
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
        // Should not contain override actions
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
        // Should contain override actions
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
        // Should not contain plan contents header since file doesn't exist
        assert!(!summary.contains("## Plan Contents"));
    }

    #[test]
    fn test_build_approval_summary_missing_file_with_override() {
        let dir = tempdir().unwrap();
        let plan_path = dir.path().join("nonexistent.md");

        let summary = build_approval_summary(&plan_path, true, 5);

        assert!(summary.contains("You chose to proceed without AI approval after 5 review iterations."));
        assert!(summary.contains("Could not read plan file:"));
        // Should still contain override actions even if file is missing
        assert!(summary.contains("[i] Implement"));
        assert!(summary.contains("[d] Decline"));
    }

    #[test]
    fn test_build_max_iterations_summary_with_preview_and_full_feedback() {
        let dir = tempdir().unwrap();
        let working_dir = dir.path();

        let mut state = State::new("test-feature", "Test objective", 3).unwrap();
        state.iteration = 3;

        // Create a review with long feedback (more than 5 lines)
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

        // Verify new Review Summary section at top
        assert!(summary.contains("## Review Summary"));
        assert!(summary.contains("**1 reviewer(s):** 1 needs revision, 0 approved"));
        assert!(summary.contains("**Needs Revision:** TEST-REVIEWER"));
        assert!(summary.contains("- **TEST-REVIEWER** - **NEEDS REVISION**: Multiple issues found in the plan"));

        // Verify preview section exists with truncated content
        assert!(summary.contains("## Latest Review Feedback (Preview)"));
        assert!(summary.contains("Scroll down for full feedback"));
        assert!(summary.contains("TEST-REVIEWER (NEEDS REVISION)"));

        // Verify full feedback section exists with complete content
        assert!(summary.contains("## Full Review Feedback"));
        assert!(summary.contains("Line 6: Sixth issue found"));
        assert!(summary.contains("Line 7: Seventh issue found"));
        assert!(summary.contains("Line 8: Additional detailed feedback here"));

        // Verify action choices are present
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

        // Verify empty reviews message
        assert!(summary.contains("No review feedback available"));

        // Should NOT contain any review sections
        assert!(!summary.contains("## Review Summary"));
        assert!(!summary.contains("## Latest Review Feedback (Preview)"));
        assert!(!summary.contains("## Full Review Feedback"));

        // Verify action choices are still present
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

        // Verify new Review Summary section with verdict grouping
        assert!(summary.contains("## Review Summary"));
        assert!(summary.contains("**2 reviewer(s):** 1 needs revision, 1 approved"));
        assert!(summary.contains("**Needs Revision:** REVIEWER-1"));
        assert!(summary.contains("**Approved:** REVIEWER-2"));

        // Verify per-agent summary bullets with verdicts
        assert!(summary.contains("- **REVIEWER-1** - **NEEDS REVISION**: Several issues need addressing"));
        assert!(summary.contains("- **REVIEWER-2** - **APPROVED**: Plan is well structured"));

        // Verify both reviewers appear in preview
        assert!(summary.contains("REVIEWER-1 (NEEDS REVISION)"));
        assert!(summary.contains("REVIEWER-2 (APPROVED)"));

        // Verify full feedback contains both
        assert!(summary.contains("Issue A"));
        assert!(summary.contains("Issue B"));
        assert!(summary.contains("Issue C"));
        assert!(summary.contains("Looks good to me"));
    }

    #[test]
    fn test_build_resume_command_simple_path() {
        let path = Path::new("/home/user/projects/myapp");
        let cmd = super::build_resume_command("abc123", path);
        assert_eq!(
            cmd,
            "planning --resume-session abc123 --working-dir /home/user/projects/myapp"
        );
    }

    #[test]
    fn test_build_resume_command_path_with_spaces() {
        let path = Path::new("/home/user/My Projects/my app");
        let cmd = super::build_resume_command("abc123", path);
        assert_eq!(
            cmd,
            "planning --resume-session abc123 --working-dir \"/home/user/My Projects/my app\""
        );
    }

    #[test]
    fn test_build_resume_command_path_with_special_chars() {
        // Path with dollar sign, backtick, and double quote
        let path = Path::new("/home/user/$project/test`dir/quote\"here");
        let cmd = super::build_resume_command("xyz789", path);
        assert_eq!(
            cmd,
            "planning --resume-session xyz789 --working-dir \"/home/user/\\$project/test\\`dir/quote\\\"here\""
        );
    }

    #[test]
    fn test_build_resume_command_path_with_backslash() {
        let path = Path::new("/home/user/path\\with\\backslash");
        let cmd = super::build_resume_command("def456", path);
        assert_eq!(
            cmd,
            "planning --resume-session def456 --working-dir \"/home/user/path\\\\with\\\\backslash\""
        );
    }

    #[test]
    fn test_build_resume_command_path_with_single_quote() {
        let path = Path::new("/home/user/it's a path");
        let cmd = super::build_resume_command("test123", path);
        // Single quote doesn't need escaping in double quotes
        assert_eq!(
            cmd,
            "planning --resume-session test123 --working-dir \"/home/user/it's a path\""
        );
    }

    #[test]
    fn test_shell_quote_path_no_quoting_needed() {
        let path = Path::new("/simple/path/here");
        let quoted = super::shell_quote_path(path);
        assert_eq!(quoted, "/simple/path/here");
    }

    #[test]
    fn test_shell_quote_path_with_tilde() {
        // Tilde is a shell metacharacter
        let path = Path::new("~/projects");
        let quoted = super::shell_quote_path(path);
        assert_eq!(quoted, "\"~/projects\"");
    }

    #[test]
    fn test_shell_quote_path_with_ampersand() {
        let path = Path::new("/path/with&special");
        let quoted = super::shell_quote_path(path);
        assert_eq!(quoted, "\"/path/with&special\"");
    }

    #[test]
    fn test_shell_quote_path_with_glob() {
        let path = Path::new("/path/with*glob");
        let quoted = super::shell_quote_path(path);
        assert_eq!(quoted, "\"/path/with*glob\"");
    }
}
