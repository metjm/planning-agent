use crate::agents::{AgentContext, AgentType};
use crate::config::WorkflowConfig;
use crate::phases::planning_conversation_key;
use crate::prompt_format::PromptBuilder;
use crate::session_logger::SessionLogger;
use crate::state::{ResumeStrategy, State};
use crate::tui::SessionEventSender;
use anyhow::Result;
use std::path::Path;
use std::sync::Arc;

const PLANNING_SYSTEM_PROMPT: &str = r#"You are a technical planning agent.
Create a detailed implementation plan for the given objective.
Use the available tools to read the codebase and understand the existing structure.

When replacing or refactoring functionality, your plan must remove the old code entirely—no backward compatibility shims, re-exports, or conversion methods are allowed.

DO NOT include timelines, schedules, dates, durations, or time estimates in plans.
Examples to reject: "in two weeks", "Phase 1: Week 1-2", "Q1 delivery", "Sprint 1", "by end of day".

Use the "planning" skill to create the plan.
Before finalizing your plan, perform a self-review against the "plan-review" skill criteria, so you can be confident that it will pass review.
Your plan should be structured to pass review without requiring revision cycles."#;

pub async fn run_planning_phase_with_context(
    state: &mut State,
    working_dir: &Path,
    config: &WorkflowConfig,
    session_sender: SessionEventSender,
    state_path: &Path,
    session_logger: Arc<SessionLogger>,
) -> Result<()> {
    let planning_config = &config.workflow.planning;
    let agent_name = &planning_config.agent;
    let max_turns = planning_config.max_turns;

    let agent_config = config
        .get_agent(agent_name)
        .ok_or_else(|| anyhow::anyhow!("Planning agent '{}' not found in config", agent_name))?;

    session_sender.send_output(format!("[planning] Using agent: {}", agent_name));

    let agent = AgentType::from_config(agent_name, agent_config, working_dir.to_path_buf())?;

    let prompt = build_planning_prompt(state, working_dir);

    // Planning always uses ConversationResume to enable revision continuity.
    // The agent will capture its conversation ID on first run, then resume on revision.
    let configured_strategy = ResumeStrategy::ConversationResume;
    // Use namespaced session key to avoid collisions with reviewer sessions
    let conversation_id_name = planning_conversation_key(agent_name);
    let agent_session = state.get_or_create_agent_session(&conversation_id_name, configured_strategy);
    let conversation_id = agent_session.conversation_id.clone();
    let resume_strategy = agent_session.resume_strategy.clone();

    state.record_invocation(&conversation_id_name, "Planning");
    state.set_updated_at();
    state.save_atomic(state_path)?;

    let context = AgentContext {
        session_sender: session_sender.clone(),
        phase: "Planning".to_string(),
        conversation_id,
        resume_strategy,
        session_logger,
    };

    let result = agent
        .execute_streaming_with_context(
            prompt,
            Some(PLANNING_SYSTEM_PROMPT.to_string()),
            max_turns,
            context,
        )
        .await?;

    // Store captured conversation ID for future resume (e.g., in revising phase)
    if let Some(ref captured_id) = result.conversation_id {
        state.update_agent_conversation_id(&conversation_id_name, captured_id.clone());
        state.set_updated_at();
        state.save_atomic(state_path)?;
        session_sender.send_output(format!(
            "[planning:{}] Captured conversation ID for resume: {}",
            agent_name,
            &captured_id[..8.min(captured_id.len())]
        ));
    }

    session_sender.send_output(format!("[planning:{}] Planning phase complete", agent_name));
    session_sender.send_output(format!(
        "[planning:{}] Result preview: {}...",
        agent_name,
        result.output.chars().take(200).collect::<String>()
    ));

    Ok(())
}

fn build_planning_prompt(state: &State, working_dir: &Path) -> String {
    // state.plan_file is now an absolute path (in ~/.planning-agent/plans/)
    let plan_path = state.plan_file.display().to_string();
    let instructions = format!(
        r#"Create a detailed implementation plan for the given objective.

Requirements:
1. Analyze the existing codebase to understand the current architecture
2. Identify all files that need to be modified or created (use absolute paths)
3. Break down the implementation into clear, actionable steps
4. Consider edge cases and potential issues
5. Include a testing strategy
6. When replacing functionality, remove old code entirely—update all callers and do not add backward-compatibility shims or re-exports
7. DO NOT include timelines, schedules, dates, durations, or time estimates (e.g., "in two weeks", "Sprint 1", "Q1 delivery")

IMPORTANT: Write the final plan to this file: {}"#,
        plan_path
    );

    let mut builder = PromptBuilder::new()
        .phase("planning")
        .instructions(&instructions)
        .input("workspace-root", &working_dir.display().to_string())
        .input("feature-name", &state.feature_name)
        .input("objective", &state.objective)
        .input("plan-output-path", &plan_path)
        .constraint("Use absolute paths for all file references in your plan")
        .tools("Use the Read, Glob, and Grep tools to explore the codebase as needed.");

    // Add worktree context if applicable
    if let Some(ref wt_state) = state.worktree_info {
        builder = builder
            .input("worktree-path", &wt_state.worktree_path.display().to_string())
            .input("worktree-branch", &wt_state.branch_name)
            .input("original-dir", &wt_state.original_dir.display().to_string());
        if let Some(ref source) = wt_state.source_branch {
            builder = builder.input("source-branch", source);
        }
        builder = builder.constraint("All file operations should be performed in the worktree directory, not the original directory.");
    }

    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Phase;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn minimal_state() -> State {
        State {
            phase: Phase::Planning,
            iteration: 1,
            max_iterations: 3,
            feature_name: "test-feature".to_string(),
            objective: "Test objective".to_string(),
            plan_file: PathBuf::from("/tmp/test-plan.md"),
            feedback_file: PathBuf::from("/tmp/test-feedback.md"),
            last_feedback_status: None,
            approval_overridden: false,
            workflow_session_id: "test-session".to_string(),
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
    fn planning_prompt_includes_no_backward_compatibility_directive() {
        assert!(
            PLANNING_SYSTEM_PROMPT.contains("no backward compatibility"),
            "PLANNING_SYSTEM_PROMPT should contain 'no backward compatibility'"
        );
    }

    #[test]
    fn build_planning_prompt_includes_deletion_requirement() {
        let state = minimal_state();
        let working_dir = PathBuf::from("/tmp/workspace");
        let prompt = build_planning_prompt(&state, &working_dir);

        assert!(
            prompt.contains("remove old code entirely"),
            "Planning prompt should contain 'remove old code entirely'"
        );
        assert!(
            prompt.contains("backward-compatibility shims"),
            "Planning prompt should contain 'backward-compatibility shims'"
        );
    }

    #[test]
    fn build_planning_prompt_includes_plan_output_path() {
        let state = minimal_state();
        let working_dir = PathBuf::from("/tmp/workspace");
        let prompt = build_planning_prompt(&state, &working_dir);

        // Print for debugging
        eprintln!("Generated prompt:\n{}", prompt);

        assert!(
            prompt.contains("<plan-output-path>"),
            "Planning prompt should contain <plan-output-path> tag"
        );
        assert!(
            prompt.contains("/tmp/test-plan.md"),
            "Planning prompt should contain the plan file path"
        );
    }

    #[test]
    fn planning_system_prompt_contains_no_timeline_directive() {
        assert!(
            PLANNING_SYSTEM_PROMPT.contains("DO NOT include timelines"),
            "PLANNING_SYSTEM_PROMPT must contain the no-timeline directive"
        );
        assert!(
            PLANNING_SYSTEM_PROMPT.contains("in two weeks"),
            "PLANNING_SYSTEM_PROMPT must contain example phrase 'in two weeks'"
        );
    }

    #[test]
    fn build_planning_prompt_contains_no_timeline_directive() {
        let state = minimal_state();
        let working_dir = PathBuf::from("/tmp/workspace");
        let prompt = build_planning_prompt(&state, &working_dir);

        assert!(
            prompt.contains("DO NOT include timelines"),
            "Planning prompt must contain the no-timeline directive"
        );
    }
}
