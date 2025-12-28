use crate::phases;
use crate::planning_dir::ensure_planning_agent_dir;
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

pub fn log_workflow(working_dir: &Path, message: &str) {
    // Ensure the .planning-agent directory exists before writing log
    let _ = ensure_planning_agent_dir(working_dir);
    let run_id = get_run_id();
    let log_path = working_dir.join(format!(".planning-agent/workflow-{}.log", run_id));
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&log_path) {
        let timestamp = chrono::Local::now().format("%H:%M:%S");
        let _ = writeln!(f, "[{}] {}", timestamp, message);
    }
}

pub fn debug_log(start: std::time::Instant, msg: &str) {
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/planning-debug.log")
    {
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
    for failure in failures {
        let error = truncate_for_summary(&failure.error, 200);
        summary.push_str(&format!("- {}: {}\n", failure.agent_name, error));
    }

    summary.push_str(
        "\nChoose whether to retry the failed reviewers or continue with the successful reviews.",
    );
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
        summary.push_str("---\n\n## Latest Review Feedback\n\n");
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

pub fn format_window_title(tab_manager: &TabManager) -> String {
    use crate::tui::SessionStatus;

    let session = tab_manager.active();
    let plan_name = session.feature_name();

    let status = match session.status {
        SessionStatus::InputPending => "Input",
        SessionStatus::Planning => session.phase_name(),
        SessionStatus::GeneratingSummary => "Generating Summary",
        SessionStatus::AwaitingApproval => "Awaiting Approval",
        SessionStatus::Complete => "Complete",
        SessionStatus::Error => "Error",
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
}
