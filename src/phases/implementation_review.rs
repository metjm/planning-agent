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
use crate::prompt_format::{xml_escape, PromptBuilder};
use crate::session_logger::SessionLogger;
use crate::state::{ResumeStrategy, State};
use crate::tui::SessionEventSender;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::sync::Arc;

const IMPLEMENTATION_REVIEW_SYSTEM_PROMPT: &str = r#"You are an implementation review agent that compares implementations against their approved plans.

Your task is to:
1. Read the approved plan to understand what was supposed to be implemented
2. Inspect the repository state (file contents, structure, changes)
3. Compare each requirement in the plan against the actual implementation
4. Generate a structured review report with a clear verdict

Be thorough but fair:
- Minor differences in implementation approach are acceptable if they achieve the plan's goals
- Focus on functional correctness and completeness
- Verify that all required changes were made
- Check for regressions or unintended side effects

IMPORTANT:
- Use absolute paths for all file references
- Your report MUST include a "Verdict" section followed by either "APPROVED" or "NEEDS REVISION"
- If there are issues, wrap detailed fix instructions in <implementation-feedback> tags"#;

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

/// Builds the implementation review prompt.
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

    let output_format = format!(
        r###"Your report MUST follow this structure:

```markdown
# Implementation Review Report - Round {}

## Plan Summary
[Brief summary of what the plan intended to implement]

## Implementation Checklist
- [x] Feature/step that was implemented correctly
- [ ] Feature/step that is missing or incorrect
...

## Findings

### Correctly Implemented
1. [Description of correctly implemented feature]
   **Location**: [absolute/path/to/file:line]

### Issues Found
1. **Issue**: [Description]
   **Location**: [absolute/path/to/file:line]
   **Expected**: [What the plan specified]
   **Actual**: [What was implemented or missing]

## Verdict
APPROVED (if implementation matches plan)
NEEDS REVISION (if there are issues to fix)

<implementation-feedback>
[Detailed feedback for the implementation agent if NEEDS REVISION.
Include specific instructions on what needs to be fixed, using absolute paths.
Be clear and actionable so the next implementation attempt can succeed.]
</implementation-feedback>
```

CRITICAL: Your report MUST include "## Verdict" followed by either "APPROVED" or "NEEDS REVISION".
If there are issues, wrap detailed fix instructions in <implementation-feedback> tags."###,
        iteration
    );

    let mut builder = PromptBuilder::new()
        .phase("implementation-review")
        .instructions(
            r#"Review the implementation against the approved plan.

1. Read the plan file to understand what was supposed to be implemented
2. Explore the repository to see what was actually implemented
3. Optionally read the implementation log to understand what changes were made
4. Compare each requirement/step in the plan against the implementation
5. Note any discrepancies, missing features, or deviations
6. Produce a structured report with a clear verdict

You may use Bash to run `git status` and `git diff --stat` to see what changed.
You may use Read/Glob/Grep to inspect the codebase."#,
        )
        .input(
            "workspace-root",
            &xml_escape(&working_dir.display().to_string()),
        )
        .input("plan-path", &xml_escape(&plan_path.display().to_string()))
        .input("iteration", &iteration.to_string())
        .tools("Use Read/Glob/Grep/Bash tools to inspect the implementation.")
        .constraint(&format!(
            "Use absolute paths for all file references (e.g., {}/src/main.rs:45)",
            working_dir.display()
        ))
        .output_format(&output_format);

    // Add implementation log path if provided
    if let Some(log_path) = implementation_log_path {
        builder = builder.input(
            "implementation-log-path",
            &xml_escape(&log_path.display().to_string()),
        );
    }

    builder.build()
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
        }
    }

    #[test]
    fn test_build_implementation_review_prompt_basic() {
        let state = minimal_state();
        let working_dir = PathBuf::from("/tmp/workspace");
        let prompt = build_implementation_review_prompt(&state, &working_dir, 1, None);

        // Check XML structure
        assert!(prompt.starts_with("<user-prompt>"));
        assert!(prompt.ends_with("</user-prompt>"));
        assert!(prompt.contains("<phase>implementation-review</phase>"));

        // Check inputs
        assert!(prompt.contains("<workspace-root>"));
        assert!(prompt.contains("<plan-path>"));
        assert!(prompt.contains("<iteration>1</iteration>"));

        // Check output format instructions
        assert!(prompt.contains("## Verdict"));
        assert!(prompt.contains("APPROVED"));
        assert!(prompt.contains("NEEDS REVISION"));
        assert!(prompt.contains("<implementation-feedback>"));
    }

    #[test]
    fn test_build_implementation_review_prompt_with_log_path() {
        let state = minimal_state();
        let working_dir = PathBuf::from("/tmp/workspace");
        let log_path = PathBuf::from("/tmp/session/implementation_1.log");
        let prompt =
            build_implementation_review_prompt(&state, &working_dir, 1, Some(&log_path));

        // Should include the implementation log path
        assert!(prompt.contains("<implementation-log-path>"));
        assert!(prompt.contains("implementation_1.log"));
    }

    #[test]
    fn test_build_implementation_review_prompt_escapes_special_chars() {
        let mut state = minimal_state();
        state.plan_file = PathBuf::from("/path/with<special>&chars/plan.md");
        let working_dir = PathBuf::from("/tmp/workspace");
        let prompt = build_implementation_review_prompt(&state, &working_dir, 1, None);

        // Special characters should be escaped
        assert!(prompt.contains("&lt;"));
        assert!(prompt.contains("&amp;"));
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
