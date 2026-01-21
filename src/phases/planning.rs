use crate::agents::{AgentContext, AgentType};
use crate::config::WorkflowConfig;
use crate::phases::planning_conversation_key;
use crate::planning_paths;
use crate::prompt_format::PromptBuilder;
use crate::session_logger::SessionLogger;
use crate::state::{ResumeStrategy, State};
use crate::tui::SessionEventSender;
use anyhow::Result;
use std::path::Path;
use std::sync::Arc;

/// System prompt for planning phase - simple instruction to use the planning skill.
pub const PLANNING_SYSTEM_PROMPT: &str = r#"Use the "planning" skill to create the plan. Write your plan to the plan-output-path file."#;

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
        cancel_rx: None,
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
        // Conversation IDs are ASCII identifiers, safe to slice at char boundary
        let id_preview = captured_id.get(..8).unwrap_or(captured_id);
        session_sender.send_output(format!(
            "[planning:{}] Captured conversation ID for resume: {}",
            agent_name, id_preview
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
    // state.plan_file is now an absolute path (in ~/.planning-agent/sessions/)
    let plan_path = state.plan_file.display().to_string();

    // Get session folder path for supplementary files
    // session_dir() creates the directory if it doesn't exist; only fails if home dir unavailable
    let session_folder = planning_paths::session_dir(&state.workflow_session_id)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| {
            // Defensive fallback: derive from plan_file parent
            state
                .plan_file
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        });

    let mut builder = PromptBuilder::new()
        .phase("planning")
        .instructions(r#"Use the "planning" skill to create the plan. Write your plan to the plan-output-path file."#)
        .input("workspace-root", &working_dir.display().to_string())
        .input("feature-name", &state.feature_name)
        .input("objective", &state.objective)
        .input("plan-output-path", &plan_path)
        .input("session-folder-path", &session_folder);

    // Add worktree context if applicable
    if let Some(ref wt_state) = state.worktree_info {
        builder = builder
            .input("worktree-path", &wt_state.worktree_path.display().to_string())
            .input("worktree-branch", &wt_state.branch_name)
            .input("original-dir", &wt_state.original_dir.display().to_string());
        if let Some(ref source) = wt_state.source_branch {
            builder = builder.input("source-branch", source);
        }
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
            sequential_review: None,
        }
    }

    #[test]
    fn planning_system_prompt_references_skill() {
        assert!(
            PLANNING_SYSTEM_PROMPT.contains("planning"),
            "PLANNING_SYSTEM_PROMPT should reference the planning skill"
        );
        assert!(
            PLANNING_SYSTEM_PROMPT.contains("plan-output-path"),
            "PLANNING_SYSTEM_PROMPT should reference plan-output-path"
        );
    }

    #[test]
    fn build_planning_prompt_includes_plan_output_path() {
        let state = minimal_state();
        let working_dir = PathBuf::from("/tmp/workspace");
        let prompt = build_planning_prompt(&state, &working_dir);

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
    fn build_planning_prompt_includes_session_folder() {
        let state = minimal_state();
        let working_dir = PathBuf::from("/tmp/workspace");
        let prompt = build_planning_prompt(&state, &working_dir);

        assert!(
            prompt.contains("<session-folder-path>"),
            "Planning prompt should contain <session-folder-path> tag"
        );
    }

    #[test]
    fn build_planning_prompt_includes_workspace_root() {
        let state = minimal_state();
        let working_dir = PathBuf::from("/tmp/workspace");
        let prompt = build_planning_prompt(&state, &working_dir);

        assert!(
            prompt.contains("<workspace-root>"),
            "Planning prompt should contain <workspace-root> tag"
        );
        assert!(
            prompt.contains("/tmp/workspace"),
            "Planning prompt should contain the workspace path"
        );
    }

    #[test]
    fn build_planning_prompt_includes_objective() {
        let state = minimal_state();
        let working_dir = PathBuf::from("/tmp/workspace");
        let prompt = build_planning_prompt(&state, &working_dir);

        assert!(
            prompt.contains("<objective>"),
            "Planning prompt should contain <objective> tag"
        );
        assert!(
            prompt.contains("Test objective"),
            "Planning prompt should contain the objective"
        );
    }

    #[test]
    fn build_planning_prompt_references_skill() {
        let state = minimal_state();
        let working_dir = PathBuf::from("/tmp/workspace");
        let prompt = build_planning_prompt(&state, &working_dir);

        assert!(
            prompt.contains("planning"),
            "Planning prompt should reference the planning skill"
        );
    }
}
