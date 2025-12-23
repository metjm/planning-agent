use crate::agents::{AgentContext, AgentType};
use crate::config::WorkflowConfig;
use crate::phases::ReviewResult;
use crate::state::State;
use crate::tui::SessionEventSender;
use anyhow::Result;
use std::path::Path;

const REVISION_SYSTEM_PROMPT: &str = r#"You are revising an implementation plan based on reviewer feedback.
Focus on addressing all blocking issues first, then important improvements.
Preserve the structure and good parts of the existing plan."#;

/// Run revision phase with merged multi-agent feedback
pub async fn run_revision_phase_with_context(
    state: &State,
    working_dir: &Path,
    config: &WorkflowConfig,
    reviews: &[ReviewResult],
    session_sender: SessionEventSender,
    iteration: u32,
) -> Result<()> {
    let revising_config = &config.workflow.revising;
    let agent_name = &revising_config.agent;
    let max_turns = revising_config.max_turns;

    let agent_config = config
        .get_agent(agent_name)
        .ok_or_else(|| anyhow::anyhow!("Revising agent '{}' not found in config", agent_name))?;

    session_sender.send_output(format!(
        "[revision] Using agent: {} with {} review(s)",
        agent_name,
        reviews.len()
    ));

    let agent = AgentType::from_config(agent_name, agent_config, working_dir.to_path_buf())?;

    let prompt = build_revision_prompt_with_reviews(state, reviews);

    // Create agent context for chat message routing
    let context = AgentContext {
        session_sender: session_sender.clone(),
        phase: format!("Revising #{}", iteration),
    };

    let result = agent
        .execute_streaming_with_context(
            prompt,
            Some(REVISION_SYSTEM_PROMPT.to_string()),
            max_turns,
            context,
        )
        .await?;

    session_sender.send_output(format!(
        "[revision:{}] Revision phase complete",
        agent_name
    ));
    session_sender.send_output(format!(
        "[revision:{}] Result preview: {}...",
        agent_name,
        result.output.chars().take(200).collect::<String>()
    ));

    Ok(())
}

/// Build revision prompt with merged multi-reviewer feedback
fn build_revision_prompt_with_reviews(state: &State, reviews: &[ReviewResult]) -> String {
    let merged_feedback = reviews
        .iter()
        .map(|r| format!("## {} Review\n\n{}", r.agent_name.to_uppercase(), r.feedback))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");

    format!(
        r#"Read the current plan at: {}

# Consolidated Reviewer Feedback

{}

Revise the plan to address all issues raised by the reviewers.
Preserve the good parts of the existing plan - only modify what needs to change.

Update the plan file with your revisions."#,
        state.plan_file.display(),
        merged_feedback
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_build_revision_prompt_with_reviews() {
        let state = State {
            phase: crate::state::Phase::Revising,
            iteration: 1,
            max_iterations: 3,
            feature_name: "test".to_string(),
            objective: "test objective".to_string(),
            plan_file: PathBuf::from("docs/plans/test.md"),
            feedback_file: PathBuf::from("docs/plans/test_feedback.md"),
            last_feedback_status: None,
            approval_overridden: false,
        };

        let reviews = vec![
            ReviewResult {
                agent_name: "claude".to_string(),
                needs_revision: true,
                feedback: "Issue 1: Missing tests".to_string(),
            },
            ReviewResult {
                agent_name: "codex".to_string(),
                needs_revision: true,
                feedback: "Issue 2: Unclear architecture".to_string(),
            },
        ];

        let prompt = build_revision_prompt_with_reviews(&state, &reviews);
        assert!(prompt.contains("CLAUDE Review"));
        assert!(prompt.contains("CODEX Review"));
        assert!(prompt.contains("Issue 1: Missing tests"));
        assert!(prompt.contains("Issue 2: Unclear architecture"));
    }
}
