use crate::agents::{AgentContext, AgentType};
use crate::config::WorkflowConfig;
use crate::phases::planning_conversation_key;
use crate::phases::ReviewResult;
use crate::prompt_format::PromptBuilder;
use crate::session_logger::SessionLogger;
use crate::state::{ResumeStrategy, State};
use crate::tui::SessionEventSender;
use anyhow::Result;
use std::path::Path;
use std::sync::Arc;

const REVISION_SYSTEM_PROMPT: &str = r#"You are revising an implementation plan based on reviewer feedback.
Focus on addressing all blocking issues first, then important improvements.
Verify each finding before making changes. Only address those that require revision.
IMPORTANT: Use absolute paths for all file references in the revised plan.
DO NOT include timelines, schedules, dates, durations, or time estimates in the revised plan.
Examples to reject: "in two weeks", "Phase 1: Week 1-2", "Q1 delivery", "Sprint 1", "by end of day".
"#;

#[allow(clippy::too_many_arguments)]
pub async fn run_revision_phase_with_context(
    state: &mut State,
    working_dir: &Path,
    config: &WorkflowConfig,
    reviews: &[ReviewResult],
    session_sender: SessionEventSender,
    iteration: u32,
    state_path: &Path,
    session_logger: Arc<SessionLogger>,
) -> Result<()> {
    // Revision uses the planning agent - this enables session continuity
    let planning_config = &config.workflow.planning;
    let agent_name = &planning_config.agent;
    let max_turns = planning_config.max_turns;

    let agent_config = config
        .get_agent(agent_name)
        .ok_or_else(|| anyhow::anyhow!("Planning agent '{}' not found in config", agent_name))?;

    let agent = AgentType::from_config(agent_name, agent_config, working_dir.to_path_buf())?;

    // Revision always uses ConversationResume to continue the planning session.
    // All agents (Claude, Codex, Gemini) support session resume.
    let session_resume_active = agent.supports_session_resume();

    session_sender.send_output(format!(
        "[revision] Using planning agent: {} with {} review(s){}",
        agent_name,
        reviews.len(),
        if session_resume_active { " (session resume)" } else { "" }
    ));

    let prompt = build_revision_prompt_with_reviews(state, reviews, working_dir, session_resume_active);

    let phase_name = format!("Revising #{}", iteration);
    // Revision always uses ConversationResume to continue the planning conversation.
    // This ensures the agent has full context from the original planning phase.
    let resume_strategy = ResumeStrategy::ConversationResume;
    // Use the SAME session key as planning phase for session continuity
    let conversation_id_name = planning_conversation_key(agent_name);
    let agent_session = state.get_or_create_agent_session(&conversation_id_name, resume_strategy.clone());
    let conversation_id = agent_session.conversation_id.clone();

    state.record_invocation(&conversation_id_name, &phase_name);
    state.set_updated_at();
    state.save_atomic(state_path)?;

    let context = AgentContext {
        session_sender: session_sender.clone(),
        phase: phase_name,
        conversation_id,
        resume_strategy,
        session_logger,
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

fn build_revision_prompt_with_reviews(
    state: &State,
    reviews: &[ReviewResult],
    working_dir: &Path,
    session_resume_active: bool,
) -> String {
    let merged_feedback = reviews
        .iter()
        .map(|r| format!("## {} Review\n\n{}", r.agent_name.to_uppercase(), r.feedback))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");

    let plan_path = state.plan_file.display().to_string();

    if session_resume_active {
        // Continuation prompt - leverages existing session context
        // The agent already knows the workspace, plan file, and original context
        format!(
            "The reviewers have provided feedback on your plan. \
             Please revise the plan at {} to address all issues raised.\n\n\
             IMPORTANT: Do not add timelines, schedules, dates, durations, or time estimates \
             (e.g., \"in two weeks\", \"Sprint 1\", \"Q1 delivery\").\n\n\
             # Reviewer Feedback\n\n{}",
            plan_path,
            merged_feedback
        )
    } else {
        // Full context prompt - for fresh sessions (Codex, Gemini, or session persistence disabled)
        let instructions = format!(
            r#"Revise the plan to address all issues raised by the reviewers.
Preserve the good parts of the existing plan - only modify what needs to change.
DO NOT include timelines, schedules, dates, durations, or time estimates (e.g., "in two weeks", "Sprint 1", "Q1 delivery").

IMPORTANT: Update the plan file at: {}"#,
            plan_path
        );

        PromptBuilder::new()
            .phase("revising")
            .instructions(&instructions)
            .input("workspace-root", &working_dir.display().to_string())
            .input("plan-output-path", &plan_path)
            .context(&format!("# Consolidated Reviewer Feedback\n\n{}", merged_feedback))
            .constraint("Use absolute paths for all file references in the revised plan")
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Phase;
    use std::path::PathBuf;

    fn test_reviews() -> Vec<ReviewResult> {
        vec![
            ReviewResult {
                agent_name: "claude".to_string(),
                needs_revision: true,
                feedback: "Issue 1: Missing tests".to_string(),
                summary: "Missing test coverage".to_string(),
            },
            ReviewResult {
                agent_name: "codex".to_string(),
                needs_revision: true,
                feedback: "Issue 2: Unclear architecture".to_string(),
                summary: "Architecture needs clarification".to_string(),
            },
        ]
    }

    #[test]
    fn test_revision_prompt_includes_plan_path() {
        let mut state = State::new("test", "test objective", 3).unwrap();
        state.phase = Phase::Revising;
        state.plan_file = PathBuf::from("/home/user/.planning-agent/sessions/abc123/plan.md");

        let reviews = test_reviews();
        let working_dir = std::path::Path::new("/workspaces/myproject");

        // Test with session_resume_active = false (full context prompt)
        let prompt = build_revision_prompt_with_reviews(&state, &reviews, working_dir, false);

        eprintln!("Generated revision prompt:\n{}", prompt);

        // The full context prompt should include plan-output-path
        assert!(
            prompt.contains("<plan-output-path>"),
            "Revision prompt should contain <plan-output-path> tag"
        );
        assert!(
            prompt.contains("/home/user/.planning-agent/sessions/abc123/plan.md"),
            "Revision prompt should contain the plan file path"
        );
    }

    #[test]
    fn test_build_revision_prompt_full_context() {
        let mut state = State::new("test", "test objective", 3).unwrap();
        state.phase = Phase::Revising;

        let reviews = test_reviews();
        let working_dir = Path::new("/workspaces/myproject");

        // Test with session_resume_active = false (full context prompt)
        let prompt = build_revision_prompt_with_reviews(&state, &reviews, working_dir, false);

        // Check XML structure
        assert!(prompt.starts_with("<user-prompt>"));
        assert!(prompt.ends_with("</user-prompt>"));
        assert!(prompt.contains("<phase>revising</phase>"));
        // Check feedback content is present
        assert!(prompt.contains("CLAUDE Review"));
        assert!(prompt.contains("CODEX Review"));
        assert!(prompt.contains("Issue 1: Missing tests"));
        assert!(prompt.contains("Issue 2: Unclear architecture"));
        // Check inputs
        assert!(prompt.contains("<workspace-root>/workspaces/myproject</workspace-root>"));
        // Check constraints
        assert!(prompt.contains("Use absolute paths"));
    }

    #[test]
    fn test_build_revision_prompt_session_resume() {
        let mut state = State::new("test", "test objective", 3).unwrap();
        state.phase = Phase::Revising;

        let reviews = test_reviews();
        let working_dir = Path::new("/workspaces/myproject");

        // Test with session_resume_active = true (simplified continuation prompt)
        let prompt = build_revision_prompt_with_reviews(&state, &reviews, working_dir, true);

        // Should NOT be XML structured
        assert!(!prompt.starts_with("<user-prompt>"));
        assert!(!prompt.contains("<phase>revising</phase>"));

        // Should be a simpler continuation prompt
        assert!(prompt.contains("The reviewers have provided feedback"));
        assert!(prompt.contains("Please revise the plan"));

        // Should still contain the feedback
        assert!(prompt.contains("CLAUDE Review"));
        assert!(prompt.contains("CODEX Review"));
        assert!(prompt.contains("Issue 1: Missing tests"));
        assert!(prompt.contains("Issue 2: Unclear architecture"));

        // Should reference the plan file
        assert!(prompt.contains("plan.md"));
    }

    #[test]
    fn revision_system_prompt_contains_no_timeline_directive() {
        assert!(
            REVISION_SYSTEM_PROMPT.contains("DO NOT include timelines"),
            "REVISION_SYSTEM_PROMPT must contain the no-timeline directive"
        );
        assert!(
            REVISION_SYSTEM_PROMPT.contains("in two weeks"),
            "REVISION_SYSTEM_PROMPT must contain example phrase 'in two weeks'"
        );
    }

    #[test]
    fn revision_prompt_session_resume_contains_no_timeline_directive() {
        let mut state = State::new("test", "test objective", 3).unwrap();
        state.phase = Phase::Revising;

        let reviews = test_reviews();
        let working_dir = Path::new("/workspaces/myproject");

        // Test with session_resume_active = true (simplified continuation prompt)
        let prompt = build_revision_prompt_with_reviews(&state, &reviews, working_dir, true);

        assert!(
            prompt.contains("Do not add timelines"),
            "Session resume prompt must contain the no-timeline directive"
        );
    }

    #[test]
    fn revision_prompt_full_context_contains_no_timeline_directive() {
        let mut state = State::new("test", "test objective", 3).unwrap();
        state.phase = Phase::Revising;

        let reviews = test_reviews();
        let working_dir = Path::new("/workspaces/myproject");

        // Test with session_resume_active = false (full context prompt)
        let prompt = build_revision_prompt_with_reviews(&state, &reviews, working_dir, false);

        assert!(
            prompt.contains("DO NOT include timelines"),
            "Full context prompt must contain the no-timeline directive"
        );
    }
}
