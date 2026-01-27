//! JSON-mode implementation review phase.
//!
//! This module implements the review phase that compares the implementation
//! against the approved plan and produces a structured verdict.

use crate::agents::{AgentContext, AgentType};
use crate::config::WorkflowConfig;
use crate::domain::types::ResumeStrategy;
use crate::domain::view::WorkflowView;
use crate::phases::implementation_reviewing_conversation_key;
use crate::phases::verdict::{
    extract_implementation_feedback, parse_verification_verdict, VerificationVerdictResult,
};
use crate::planning_paths;
use crate::session_daemon::SessionLogger;
use crate::tui::{ReviewKind, SessionEventSender};
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::sync::Arc;

/// Minimal system prompt - the skill handles the details.
const IMPLEMENTATION_REVIEW_SYSTEM_PROMPT: &str = "You are an implementation review agent.";

/// Result of running the implementation review phase.
#[derive(Debug, Clone)]
pub struct ImplementationReviewResult {
    /// The parsed verdict
    pub verdict: VerificationVerdictResult,
    /// Extracted feedback for the next implementation iteration (if any)
    pub feedback: Option<String>,
}

/// Runs the implementation review phase to compare implementation against plan.
///
/// # Arguments
/// * `view` - The current workflow view (read-only projection of state)
/// * `config` - The workflow configuration
/// * `working_dir` - The working directory to review
/// * `iteration` - The current iteration number (1-indexed)
/// * `implementation_log_path` - Path to the implementation log from the previous phase
/// * `session_sender` - Channel to send session events
/// * `session_logger` - Logger for the session
///
/// # Returns
/// An `ImplementationReviewResult` containing the report and verdict.
pub async fn run_implementation_review_phase(
    view: &WorkflowView,
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
        build_implementation_review_prompt(view, working_dir, iteration, implementation_log_path)?;

    // Get report path
    let workflow_id = view
        .workflow_id()
        .ok_or_else(|| anyhow::anyhow!("Workflow ID not set in view"))?;
    let report_path =
        planning_paths::session_implementation_review_path(&workflow_id.0.to_string(), iteration)?;

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

    let review_started_at = std::time::Instant::now();
    session_sender.send_review_round_started(ReviewKind::Implementation, iteration);
    session_sender.send_reviewer_started(
        ReviewKind::Implementation,
        iteration,
        agent_name.to_string(),
    );

    let phase_result: Result<ImplementationReviewResult> = (async {
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

        Ok(ImplementationReviewResult { verdict, feedback })
    })
    .await;

    let duration_ms = review_started_at.elapsed().as_millis() as u64;
    match phase_result {
        Ok(review_result) => {
            match &review_result.verdict {
                VerificationVerdictResult::Approved => {
                    session_sender.send_reviewer_completed(
                        ReviewKind::Implementation,
                        iteration,
                        agent_name.to_string(),
                        true,
                        "Approved".to_string(),
                        duration_ms,
                    );
                    session_sender.send_review_round_completed(
                        ReviewKind::Implementation,
                        iteration,
                        true,
                    );
                }
                VerificationVerdictResult::NeedsRevision => {
                    session_sender.send_reviewer_completed(
                        ReviewKind::Implementation,
                        iteration,
                        agent_name.to_string(),
                        false,
                        "Needs revision".to_string(),
                        duration_ms,
                    );
                    session_sender.send_review_round_completed(
                        ReviewKind::Implementation,
                        iteration,
                        false,
                    );
                }
                VerificationVerdictResult::ParseFailure { reason } => {
                    session_sender.send_reviewer_failed(
                        ReviewKind::Implementation,
                        iteration,
                        agent_name.to_string(),
                        reason.clone(),
                    );
                    session_sender.send_review_round_completed(
                        ReviewKind::Implementation,
                        iteration,
                        false,
                    );
                }
            }
            Ok(review_result)
        }
        Err(err) => {
            session_sender.send_reviewer_failed(
                ReviewKind::Implementation,
                iteration,
                agent_name.to_string(),
                err.to_string(),
            );
            session_sender.send_review_round_completed(
                ReviewKind::Implementation,
                iteration,
                false,
            );
            Err(err).context("Implementation review phase failed after start")
        }
    }
}

/// Builds the implementation review prompt with clean format and skill invocation at the end.
fn build_implementation_review_prompt(
    view: &WorkflowView,
    working_dir: &Path,
    iteration: u32,
    implementation_log_path: Option<&Path>,
) -> Result<String> {
    // Get plan path from view
    let plan_path_ref = view
        .plan_path()
        .ok_or_else(|| anyhow::anyhow!("Plan path not set in view"))?;

    // Resolve plan path to absolute
    let plan_path = if plan_path_ref.0.is_absolute() {
        plan_path_ref.0.clone()
    } else {
        working_dir.join(&plan_path_ref.0)
    };

    // Get workflow_id for review output path
    let workflow_id = view
        .workflow_id()
        .ok_or_else(|| anyhow::anyhow!("Workflow ID not set in view"))?;

    // Get review output path
    let review_output =
        planning_paths::session_implementation_review_path(&workflow_id.0.to_string(), iteration)
            .unwrap_or_else(|_| working_dir.join(format!("review_{}.md", iteration)));

    let log_section = match implementation_log_path {
        Some(log) => format!("- Implementation log: {}\n", log.display()),
        None => String::new(),
    };

    Ok(format!(
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
    ))
}

#[cfg(test)]
#[path = "tests/implementation_review_tests.rs"]
mod tests;
