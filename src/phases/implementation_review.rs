//! JSON-mode implementation review phase.
//!
//! This module implements the review phase that compares the implementation
//! against the approved plan and produces a structured verdict.

use crate::agents::{AgentContext, AgentType};
use crate::config::WorkflowConfig;
use crate::phases::implementation_reviewing_conversation_key;
use crate::phases::verdict::{
    extract_implementation_feedback, parse_verification_verdict, VerificationVerdictResult,
};
use crate::planning_paths;
use crate::session_logger::SessionLogger;
use crate::state::{ResumeStrategy, State};
use crate::tui::SessionEventSender;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::sync::Arc;

/// Minimal system prompt - the skill handles the details.
const IMPLEMENTATION_REVIEW_SYSTEM_PROMPT: &str = "You are an implementation review agent.";

/// Result of running the implementation review phase.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ImplementationReviewResult {
    /// The full review report
    pub report: String,
    /// Path to the saved review report
    pub report_path: std::path::PathBuf,
    /// The parsed verdict
    pub verdict: VerificationVerdictResult,
    /// Extracted feedback for the next implementation iteration (if any)
    pub feedback: Option<String>,
}

/// Runs the implementation review phase to compare implementation against plan.
///
/// # Arguments
/// * `state` - The current workflow state
/// * `config` - The workflow configuration
/// * `working_dir` - The working directory to review
/// * `iteration` - The current iteration number (1-indexed)
/// * `implementation_log_path` - Path to the implementation log from the previous phase
/// * `session_sender` - Channel to send session events
/// * `session_logger` - Logger for the session
///
/// # Returns
/// An `ImplementationReviewResult` containing the report and verdict.
#[allow(dead_code)]
pub async fn run_implementation_review_phase(
    state: &State,
    config: &WorkflowConfig,
    working_dir: &Path,
    iteration: u32,
    implementation_log_path: Option<&Path>,
    session_sender: SessionEventSender,
    session_logger: Arc<SessionLogger>,
) -> Result<ImplementationReviewResult> {
    // Get implementation config
    let impl_config = &config.implementation;
    if !impl_config.enabled {
        anyhow::bail!("Implementation is disabled in config");
    }

    let reviewing_config = impl_config
        .reviewing
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No reviewing agent configured"))?;

    let agent_name = &reviewing_config.agent;
    let max_turns = reviewing_config.max_turns;

    let agent_config = config
        .get_agent(agent_name)
        .ok_or_else(|| anyhow::anyhow!("Reviewing agent '{}' not found in config", agent_name))?;

    session_sender.send_output(format!(
        "[implementation-review] Starting review round {} using agent: {}",
        iteration, agent_name
    ));

    // Create agent
    let agent = AgentType::from_config(agent_name, agent_config, working_dir.to_path_buf())?;

    // Build the prompt
    let prompt =
        build_implementation_review_prompt(state, working_dir, iteration, implementation_log_path);

    // Get report path
    let report_path = planning_paths::session_implementation_review_path(
        &state.workflow_session_id,
        iteration,
    )?;

    // Implementation review is stateless per round - we don't need conversation resume
    let _conversation_key = implementation_reviewing_conversation_key(agent_name);

    let phase_name = format!("Implementation Review #{}", iteration);

    let context = AgentContext {
        session_sender: session_sender.clone(),
        phase: phase_name,
        conversation_id: None, // Stateless - no conversation resume
        resume_strategy: ResumeStrategy::Stateless,
        cancel_rx: None,
        session_logger,
    };

    // Execute the review
    let result = agent
        .execute_streaming_with_context(
            prompt,
            Some(IMPLEMENTATION_REVIEW_SYSTEM_PROMPT.to_string()),
            max_turns,
            context,
        )
        .await
        .context("Implementation review agent execution failed")?;

    // Extract report from output
    let mut report = result.output.clone();

    // If output is empty or doesn't contain the verdict, try reading from report file
    if (report.trim().is_empty() || !report.contains("Verdict")) && report_path.exists() {
        if let Ok(file_content) = fs::read_to_string(&report_path) {
            if !file_content.trim().is_empty() {
                report = file_content;
                session_sender.send_output(format!(
                    "[implementation-review] Loaded report from {}",
                    report_path.display()
                ));
            }
        }
    }

    // Save report to file
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&report_path, &report)
        .with_context(|| format!("Failed to save review report: {}", report_path.display()))?;

    session_sender.send_output(format!(
        "[implementation-review] Report saved to {}",
        report_path.display()
    ));

    // Parse verdict
    let verdict = parse_verification_verdict(&report);

    // Log the verdict
    match &verdict {
        VerificationVerdictResult::Approved => {
            session_sender.send_output("[implementation-review] Verdict: APPROVED".to_string());
        }
        VerificationVerdictResult::NeedsRevision => {
            session_sender
                .send_output("[implementation-review] Verdict: NEEDS REVISION".to_string());
        }
        VerificationVerdictResult::ParseFailure { reason } => {
            session_sender.send_output(format!(
                "[implementation-review] WARNING: Could not parse verdict: {}",
                reason
            ));
        }
    }

    // Extract feedback if verdict requires revision
    let feedback = if verdict.needs_revision() {
        extract_implementation_feedback(&report)
    } else {
        None
    };

    Ok(ImplementationReviewResult {
        report,
        report_path,
        verdict,
        feedback,
    })
}

/// Builds the implementation review prompt with clean format and skill invocation at the end.
fn build_implementation_review_prompt(
    state: &State,
    working_dir: &Path,
    iteration: u32,
    implementation_log_path: Option<&Path>,
) -> String {
    // Resolve plan path to absolute
    let plan_path = if state.plan_file.is_absolute() {
        state.plan_file.clone()
    } else {
        working_dir.join(&state.plan_file)
    };

    // Get review output path
    let review_output = planning_paths::session_implementation_review_path(
        &state.workflow_session_id,
        iteration,
    )
    .unwrap_or_else(|_| working_dir.join(format!("review_{}.md", iteration)));

    let log_section = match implementation_log_path {
        Some(log) => format!("- Implementation log: {}\n", log.display()),
        None => String::new(),
    };

    format!(
        r#"Review the implementation against the approved plan.

##################### IMPLEMENTATION REVIEW #{iteration} #####################

Paths:
- Workspace: {workspace}
- Plan file: {plan}
- Review output: {review_output}
{log_section}
Run the "implementation-review" skill to perform the review."#,
        iteration = iteration,
        workspace = working_dir.display(),
        plan = plan_path.display(),
        review_output = review_output.display(),
        log_section = log_section,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn minimal_state() -> State {
        use crate::state::Phase;
        use std::collections::HashMap;

        State {
            phase: Phase::Complete,
            iteration: 1,
            max_iterations: 3,
            feature_name: "test-feature".to_string(),
            objective: "Test objective".to_string(),
            plan_file: PathBuf::from("/tmp/test-plan/plan.md"),
            feedback_file: PathBuf::from("/tmp/test-feedback.md"),
            last_feedback_status: None,
            approval_overridden: false,
            workflow_session_id: "test-session-id".to_string(),
            agent_conversations: HashMap::new(),
            invocations: Vec::new(),
            updated_at: String::new(),
            last_failure: None,
            failure_history: Vec::new(),
            worktree_info: None,
            implementation_state: None,
            sequential_review: None,
        }
    }

    #[test]
    fn test_build_implementation_review_prompt_basic() {
        let state = minimal_state();
        let working_dir = PathBuf::from("/tmp/workspace");
        let prompt = build_implementation_review_prompt(&state, &working_dir, 1, None);

        // Check paths are included
        assert!(prompt.contains("/tmp/workspace"));
        assert!(prompt.contains("/tmp/test-plan/plan.md"));

        // Check iteration
        assert!(prompt.contains("IMPLEMENTATION REVIEW #1"));

        // Check skill instruction is last
        assert!(prompt.ends_with(r#"Run the "implementation-review" skill to perform the review."#));
    }

    #[test]
    fn test_build_implementation_review_prompt_with_log_path() {
        let state = minimal_state();
        let working_dir = PathBuf::from("/tmp/workspace");
        let log_path = PathBuf::from("/tmp/session/implementation_1.log");
        let prompt =
            build_implementation_review_prompt(&state, &working_dir, 1, Some(&log_path));

        // Should include the implementation log path
        assert!(prompt.contains("Implementation log:"));
        assert!(prompt.contains("implementation_1.log"));

        // Skill instruction still last
        assert!(prompt.ends_with(r#"Run the "implementation-review" skill to perform the review."#));
    }

    #[test]
    fn test_implementation_review_result_with_approved_verdict() {
        let result = ImplementationReviewResult {
            report: "## Verdict\nAPPROVED\n\nAll good!".to_string(),
            report_path: PathBuf::from("/tmp/review.md"),
            verdict: VerificationVerdictResult::Approved,
            feedback: None,
        };

        assert!(result.verdict.is_approved());
        assert!(result.feedback.is_none());
    }

    #[test]
    fn test_implementation_review_result_with_needs_revision_verdict() {
        let result = ImplementationReviewResult {
            report: "## Verdict\nNEEDS REVISION\n\n<implementation-feedback>Fix this</implementation-feedback>".to_string(),
            report_path: PathBuf::from("/tmp/review.md"),
            verdict: VerificationVerdictResult::NeedsRevision,
            feedback: Some("Fix this".to_string()),
        };

        assert!(result.verdict.needs_revision());
        assert_eq!(result.feedback, Some("Fix this".to_string()));
    }
}
