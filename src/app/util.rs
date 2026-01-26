use crate::phases;
use crate::planning_paths;
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
        summary.push_str(
            "\n**Note:** Diagnostics bundles may contain sensitive information from logs.\n",
        );
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
        summary.push_str(&format!(
            "- **{}** ({}): {}\n",
            failure.agent_name, failure_type, error
        ));
        if let Some(ref path) = failure.bundle_path {
            summary.push_str(&format!("  Diagnostics bundle: {}\n", path.display()));
            has_bundles = true;
        }
    }

    if has_bundles {
        summary.push_str(
            "\n**Note:** Diagnostics bundles may contain sensitive information from logs.\n",
        );
    }

    summary.push_str("\n## Recovery Options\n\n");
    summary.push_str("- **Retry**: Try running all reviewers again\n");
    summary.push_str("- **Stop**: Save state and resume later\n");
    summary.push_str("- **Abort**: Abort the workflow\n");

    summary
}

/// Build a summary for a failure context from a resumed session.
/// This is used when resuming a session that was stopped with an unresolved failure.
pub fn build_resume_failure_summary(failure: &crate::domain::failure::FailureContext) -> String {
    let mut s = String::new();
    s.push_str("# Unresolved Workflow Failure\n\n");
    s.push_str(&format!("**Type**: {}\n", failure.kind.display_name()));
    s.push_str(&format!("**Phase**: {:?}\n", failure.phase));
    if let Some(ref agent) = failure.agent_name {
        s.push_str(&format!("**Agent**: {}\n", agent.as_str()));
    }
    s.push_str(&format!(
        "**Retries**: {}/{}\n",
        failure.retry_count, failure.max_retries
    ));
    s.push_str(&format!(
        "**Failed at**: {}\n\n",
        failure.failed_at.to_rfc3339()
    ));
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
        summary.push_str(
            "_Note: Diagnostics bundles may contain sensitive information from logs._\n\n",
        );
    }

    summary.push_str("## Recovery Options\n\n");
    summary.push_str("- **[r] Retry**: Retry the failed operation\n");
    summary.push_str("- **[s] Stop**: Save state and resume later\n");
    summary.push_str("- **[a] Abort**: Abort the workflow\n");

    summary
}

pub fn build_plan_failure_summary(error: &str, plan_path: &Path, plan_exists: bool) -> String {
    let mut summary = String::new();
    summary.push_str("# Plan Generation Failed\n\n");
    summary.push_str(&format!(
        "**Error:** {}\n\n",
        truncate_for_summary(error, 300)
    ));
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
        summary.push_str(
            "\n_Note: [c] Continue is only available when an existing plan file exists._\n",
        );
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
pub fn build_approval_summary(
    plan_path: &Path,
    approval_overridden: bool,
    iteration: u32,
) -> String {
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
pub fn shell_quote_path(path: &Path) -> String {
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
    };

    if plan_name.is_empty() || plan_name == "New Tab" {
        "Planning Agent".to_string()
    } else {
        format!("[{}] {} - Planning Agent", status, plan_name)
    }
}

/// Extracts a short kebab-case feature name from an objective using Claude.
pub async fn extract_feature_name(
    objective: &str,
    output_tx: Option<&tokio::sync::mpsc::UnboundedSender<crate::tui::Event>>,
) -> anyhow::Result<String> {
    use crate::prompt_format::PromptBuilder;
    use std::process::Stdio;
    use tokio::process::Command;

    if let Some(tx) = output_tx {
        let _ = tx.send(crate::tui::Event::Output(
            "[planning] Extracting feature name...".to_string(),
        ));
    }

    let prompt = PromptBuilder::new()
        .phase("feature-name-extraction")
        .instructions(r#"Extract a short kebab-case feature name (2-4 words, lowercase, hyphens) from the given objective.
Output ONLY the feature name, nothing else.

Example outputs: "sharing-permissions", "user-auth", "api-rate-limiting""#)
        .input("objective", objective)
        .build();

    let output = Command::new("claude")
        .arg("-p")
        .arg(&prompt)
        .arg("--output-format")
        .arg("text")
        .arg("--dangerously-skip-permissions")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?
        .wait_with_output()
        .await?;

    let mut name: String = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect();

    // Truncate absurdly long names (Claude sometimes returns garbage)
    const MAX_FEATURE_NAME_LEN: usize = 50;
    if name.len() > MAX_FEATURE_NAME_LEN {
        name.truncate(MAX_FEATURE_NAME_LEN);
        // Trim trailing hyphens after truncation
        while name.ends_with('-') {
            name.pop();
        }
    }

    if name.is_empty() {
        Ok("feature".to_string())
    } else {
        Ok(name)
    }
}

#[cfg(test)]
#[path = "tests/util_tests.rs"]
mod tests;
