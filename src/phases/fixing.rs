use crate::agents::{AgentContext, AgentType};
use crate::config::WorkflowConfig;
use crate::phases::verification::extract_verification_feedback;
use crate::state::ResumeStrategy;
use crate::tui::SessionEventSender;
use crate::verification_state::VerificationState;
use anyhow::Result;
use std::fs;

const FIXING_SYSTEM_PROMPT: &str = r#"You are a fixing agent that addresses issues found during verification.

Your task is to:
1. Read the verification report to understand what needs to be fixed
2. Read the original plan to understand the intended implementation
3. Make the necessary code changes to address the issues
4. Verify your fixes resolve the identified problems

Be precise and focused - only fix what the verification report identifies as issues.
Do not introduce new features or make unrelated changes."#;

/// Runs the fixing phase to address issues found during verification.
pub async fn run_fixing_phase(
    verification_state: &mut VerificationState,
    config: &WorkflowConfig,
    verification_report: &str,
    session_sender: SessionEventSender,
) -> Result<()> {
    let fixing_config = config
        .verification
        .fixing
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No fixing agent configured"))?;

    let agent_name = &fixing_config.agent;
    let max_turns = fixing_config.max_turns;

    let agent_config = config
        .get_agent(agent_name)
        .ok_or_else(|| anyhow::anyhow!("Fixing agent '{}' not found in config", agent_name))?;

    session_sender.send_output(format!(
        "[fixing] Starting fix round {} using agent: {}",
        verification_state.iteration, agent_name
    ));

    let agent = AgentType::from_config(
        agent_name,
        agent_config,
        verification_state.working_dir.clone(),
    )?;

    let prompt = build_fixing_prompt(verification_state, verification_report);

    let phase_name = format!("Fixing #{}", verification_state.iteration);

    let context = AgentContext {
        session_sender: session_sender.clone(),
        phase: phase_name,
        session_key: None, // Fixing is stateless per round
        resume_strategy: ResumeStrategy::Stateless,
    };

    let result = agent
        .execute_streaming_with_context(
            prompt,
            Some(FIXING_SYSTEM_PROMPT.to_string()),
            max_turns,
            context,
        )
        .await?;

    session_sender.send_output(format!(
        "[fixing:{}] Fix phase complete",
        agent_name
    ));
    session_sender.send_output(format!(
        "[fixing:{}] Result preview: {}...",
        agent_name,
        result.output.chars().take(200).collect::<String>()
    ));

    // Save fix output to a log file in the plan folder
    let fix_log_path = verification_state
        .plan_path
        .join(format!("fix_{}.log", verification_state.iteration));
    if let Err(e) = fs::write(&fix_log_path, &result.output) {
        session_sender.send_output(format!(
            "[fixing] Warning: Could not save fix log: {}",
            e
        ));
    }

    Ok(())
}

fn build_fixing_prompt(state: &VerificationState, verification_report: &str) -> String {
    let plan_path = state.plan_file_path();

    // Extract focused feedback from the verification report if available
    let feedback = extract_verification_feedback(verification_report)
        .unwrap_or_else(|| verification_report.to_string());

    format!(
        r###"Fix the implementation issues identified in the verification report.

## Original Plan
Read the implementation plan for context: {}

## Working Directory
Make your fixes in: {}

## Issues to Fix

The verification found the following issues:

{}

## Instructions

1. Read the original plan to understand the intended implementation
2. Review each issue from the verification report
3. Make the necessary code changes to fix each issue
4. Verify your changes compile and don't break existing functionality

Focus on:
- Fixing only the identified issues
- Not introducing new bugs
- Maintaining code quality and consistency with existing patterns

After making your fixes, provide a brief summary of what you changed."###,
        plan_path.display(),
        state.working_dir.display(),
        feedback
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_build_fixing_prompt_with_feedback_tags() {
        let state = VerificationState::new(
            PathBuf::from("/tmp/plan"),
            PathBuf::from("/tmp/working"),
            3,
            None,
        );

        let report = r#"
## Verdict
NEEDS REVISION

<verification-feedback>
1. Missing error handling in src/main.rs:45
2. Unit tests needed for the new function
</verification-feedback>
"#;

        let prompt = build_fixing_prompt(&state, report);

        // Should extract the feedback content
        assert!(prompt.contains("Missing error handling"));
        assert!(prompt.contains("Unit tests needed"));
        // Should not include the tags themselves
        assert!(!prompt.contains("<verification-feedback>"));
    }

    #[test]
    fn test_build_fixing_prompt_without_feedback_tags() {
        let state = VerificationState::new(
            PathBuf::from("/tmp/plan"),
            PathBuf::from("/tmp/working"),
            3,
            None,
        );

        let report = "## Verdict: NEEDS REVISION\n\nSome issues were found.";

        let prompt = build_fixing_prompt(&state, report);

        // Should include the full report as fallback
        assert!(prompt.contains("Some issues were found"));
    }
}
