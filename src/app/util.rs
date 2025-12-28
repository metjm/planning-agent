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
