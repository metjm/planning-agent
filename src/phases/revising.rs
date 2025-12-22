use crate::agents::AgentType;
use crate::claude::ClaudeInvocation;
use crate::config::WorkflowConfig;
use crate::phases::ReviewResult;
use crate::state::State;
use crate::tui::Event;
use anyhow::Result;
use std::path::Path;
use tokio::sync::mpsc;

const ALLOWED_TOOLS: &[&str] = &[
    "Read", "Glob", "Grep", "Edit", "Write", "WebSearch", "WebFetch",
];

/// Run revision phase with Claude (legacy behavior)
pub async fn run_revision_phase(
    state: &State,
    working_dir: &Path,
    output_tx: mpsc::UnboundedSender<Event>,
) -> Result<()> {
    let prompt = format!(
        r#"Read the feedback at: {}
Read the current plan at: {}

Revise the plan to address:
1. All "Must Fix" items (blocking issues) - these MUST be addressed
2. All "Should Fix" items (important improvements) - address these if possible
3. Any critical issues mentioned in the feedback

Update the plan file at {} with your revisions.
Preserve the good parts of the existing plan - only modify what needs to change.

When done, confirm that the plan has been updated."#,
        state.feedback_file.display(),
        state.plan_file.display(),
        state.plan_file.display()
    );

    let system_prompt = r#"You are revising an implementation plan based on reviewer feedback.
Focus on addressing all blocking issues first, then important improvements.
Do not ask questions - proceed with reading the feedback and making revisions.
Preserve the structure and good parts of the existing plan."#;

    let result = ClaudeInvocation::new(prompt)
        .with_system_prompt(system_prompt)
        .with_allowed_tools(ALLOWED_TOOLS.iter().map(|s| s.to_string()).collect())
        .with_working_dir(working_dir.to_path_buf())
        .execute_streaming(output_tx.clone())
        .await?;

    let _ = output_tx.send(Event::Output("[planning-agent] Revision phase complete".to_string()));
    let _ = output_tx.send(Event::Output(format!(
        "[planning-agent] Result preview: {}...",
        result.result.chars().take(200).collect::<String>()
    )));

    Ok(())
}

const REVISION_SYSTEM_PROMPT: &str = r#"You are revising an implementation plan based on reviewer feedback.
Focus on addressing all blocking issues first, then important improvements.
Preserve the structure and good parts of the existing plan."#;

/// Run revision phase with a configured agent
#[allow(dead_code)]
pub async fn run_revision_phase_with_config(
    state: &State,
    working_dir: &Path,
    config: &WorkflowConfig,
    output_tx: mpsc::UnboundedSender<Event>,
) -> Result<()> {
    let revising_config = &config.workflow.revising;
    let agent_name = &revising_config.agent;
    let max_turns = revising_config.max_turns;

    let agent_config = config
        .get_agent(agent_name)
        .ok_or_else(|| anyhow::anyhow!("Revising agent '{}' not found in config", agent_name))?;

    let _ = output_tx.send(Event::Output(format!(
        "[revision] Using agent: {}",
        agent_name
    )));

    let agent = AgentType::from_config(agent_name, agent_config, working_dir.to_path_buf())?;

    let prompt = build_revision_prompt(state);

    let result = agent
        .execute_streaming(
            prompt,
            Some(REVISION_SYSTEM_PROMPT.to_string()),
            max_turns,
            output_tx.clone(),
        )
        .await?;

    let _ = output_tx.send(Event::Output(format!(
        "[revision:{}] Revision phase complete",
        agent_name
    )));
    let _ = output_tx.send(Event::Output(format!(
        "[revision:{}] Result preview: {}...",
        agent_name,
        result.output.chars().take(200).collect::<String>()
    )));

    Ok(())
}

/// Run revision phase with merged multi-agent feedback
pub async fn run_revision_phase_with_reviews(
    state: &State,
    working_dir: &Path,
    config: &WorkflowConfig,
    reviews: &[ReviewResult],
    output_tx: mpsc::UnboundedSender<Event>,
) -> Result<()> {
    let revising_config = &config.workflow.revising;
    let agent_name = &revising_config.agent;
    let max_turns = revising_config.max_turns;

    let agent_config = config
        .get_agent(agent_name)
        .ok_or_else(|| anyhow::anyhow!("Revising agent '{}' not found in config", agent_name))?;

    let _ = output_tx.send(Event::Output(format!(
        "[revision] Using agent: {} with {} review(s)",
        agent_name,
        reviews.len()
    )));

    let agent = AgentType::from_config(agent_name, agent_config, working_dir.to_path_buf())?;

    let prompt = build_revision_prompt_with_reviews(state, reviews);

    let result = agent
        .execute_streaming(
            prompt,
            Some(REVISION_SYSTEM_PROMPT.to_string()),
            max_turns,
            output_tx.clone(),
        )
        .await?;

    let _ = output_tx.send(Event::Output(format!(
        "[revision:{}] Revision phase complete",
        agent_name
    )));
    let _ = output_tx.send(Event::Output(format!(
        "[revision:{}] Result preview: {}...",
        agent_name,
        result.output.chars().take(200).collect::<String>()
    )));

    Ok(())
}

/// Build the revision prompt for single feedback file
#[allow(dead_code)]
fn build_revision_prompt(state: &State) -> String {
    format!(
        r#"Read the current plan at: {}
Read the feedback at: {}

Revise the plan to address all issues raised in the feedback.
Preserve the good parts of the existing plan - only modify what needs to change.

Update the plan file with your revisions."#,
        state.plan_file.display(),
        state.feedback_file.display()
    )
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
    fn test_build_revision_prompt() {
        let state = State {
            phase: crate::state::Phase::Revising,
            iteration: 1,
            max_iterations: 3,
            feature_name: "test".to_string(),
            objective: "test objective".to_string(),
            plan_file: PathBuf::from("docs/plans/test.md"),
            feedback_file: PathBuf::from("docs/plans/test_feedback.md"),
            last_feedback_status: None,
        };

        let prompt = build_revision_prompt(&state);
        assert!(prompt.contains("docs/plans/test.md"));
        assert!(prompt.contains("docs/plans/test_feedback.md"));
    }

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
