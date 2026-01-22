//! JSON-mode implementation phase.
//!
//! This module implements the plan execution phase using JSON-mode agents.
//! It replaces the previous embedded PTY terminal with structured agent execution.

use crate::agents::{AgentContext, AgentType};
use crate::config::WorkflowConfig;
use crate::phases::implementing_conversation_key;
use crate::planning_paths;
use crate::session_logger::{create_session_logger, SessionLogger};
use crate::state::{ResumeStrategy, State};
use crate::tui::SessionEventSender;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::watch;

/// Minimal system prompt - the skill handles the details.
const IMPLEMENTATION_SYSTEM_PROMPT: &str =
    "You are an implementation agent that executes approved plans.";

pub const IMPLEMENTATION_FOLLOWUP_PHASE: &str = "Implementation Follow-up";

/// Result of running the implementation phase.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ImplementationResult {
    /// The full output/transcript from the implementation run
    pub output: String,
    /// Path to the saved implementation log
    pub log_path: std::path::PathBuf,
    /// Whether the agent encountered an error
    pub is_error: bool,
    /// Optional stop reason (max_turns, max_tokens, etc.)
    pub stop_reason: Option<String>,
    /// Conversation ID for resume (if available)
    pub conversation_id: Option<String>,
}

/// Runs the implementation phase to execute an approved plan.
///
/// # Arguments
/// * `state` - The current workflow state
/// * `config` - The workflow configuration
/// * `working_dir` - The working directory for the implementation
/// * `iteration` - The current iteration number (1-indexed)
/// * `feedback` - Optional feedback from a previous review iteration
/// * `session_sender` - Channel to send session events
/// * `session_logger` - Logger for the session
///
/// # Returns
/// An `ImplementationResult` containing the output and metadata from the run.
#[allow(dead_code)]
pub async fn run_implementation_phase(
    state: &State,
    config: &WorkflowConfig,
    working_dir: &Path,
    iteration: u32,
    feedback: Option<&str>,
    session_sender: SessionEventSender,
    session_logger: Arc<SessionLogger>,
) -> Result<ImplementationResult> {
    // Get implementation config
    let impl_config = &config.implementation;
    if !impl_config.enabled {
        anyhow::bail!("Implementation is disabled in config");
    }

    let implementing_config = impl_config
        .implementing
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No implementing agent configured"))?;

    let agent_name = &implementing_config.agent;
    let max_turns = implementing_config.max_turns;

    let agent_config = config.get_agent(agent_name).ok_or_else(|| {
        anyhow::anyhow!("Implementing agent '{}' not found in config", agent_name)
    })?;

    session_sender.send_output(format!(
        "[implementation] Starting implementation round {} using agent: {}",
        iteration, agent_name
    ));

    // Create agent
    let agent = AgentType::from_config(agent_name, agent_config, working_dir.to_path_buf())?;

    // Build the prompt
    let prompt = build_implementation_prompt(state, working_dir, iteration, feedback);

    // Get log path
    let log_path =
        planning_paths::session_implementation_log_path(&state.workflow_session_id, iteration)?;

    // Prepare conversation key for resume
    let conversation_key = implementing_conversation_key(agent_name);

    // Get existing conversation ID if available
    let conversation_id = state
        .agent_conversations
        .get(&conversation_key)
        .and_then(|conv| conv.conversation_id.clone());

    let phase_name = format!("Implementation #{}", iteration);

    let context = AgentContext {
        session_sender: session_sender.clone(),
        phase: phase_name,
        conversation_id: conversation_id.clone(),
        resume_strategy: ResumeStrategy::ConversationResume,
        cancel_rx: None,
        session_logger,
    };

    // Execute the implementation
    let result = agent
        .execute_streaming_with_context(
            prompt,
            Some(IMPLEMENTATION_SYSTEM_PROMPT.to_string()),
            max_turns,
            context,
        )
        .await
        .context("Implementation agent execution failed")?;

    // Save the implementation log
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&log_path, &result.output)
        .with_context(|| format!("Failed to save implementation log: {}", log_path.display()))?;

    session_sender.send_output(format!(
        "[implementation] Implementation log saved to {}",
        log_path.display()
    ));

    session_sender.send_output(format!(
        "[implementation:{}] Implementation phase complete",
        agent_name
    ));

    Ok(ImplementationResult {
        output: result.output,
        log_path,
        is_error: result.is_error,
        stop_reason: None, // TODO: Populate from AgentResult when available
        conversation_id: result.conversation_id,
    })
}

/// Runs a follow-up interaction after implementation is complete.
pub async fn run_implementation_interaction(
    mut state: State,
    config: WorkflowConfig,
    working_dir: PathBuf,
    state_path: PathBuf,
    user_message: String,
    mut session_sender: SessionEventSender,
    cancel_rx: watch::Receiver<bool>,
) -> Result<()> {
    let result = run_implementation_interaction_inner(
        &mut state,
        &config,
        &working_dir,
        &state_path,
        &user_message,
        &mut session_sender,
        cancel_rx,
    )
    .await;

    if let Err(err) = &result {
        session_sender.send_output(format!("[implementation] Follow-up failed: {}", err));
    }

    session_sender.send_implementation_interaction_finished();
    result
}

async fn run_implementation_interaction_inner(
    state: &mut State,
    config: &WorkflowConfig,
    working_dir: &Path,
    state_path: &Path,
    user_message: &str,
    session_sender: &mut SessionEventSender,
    cancel_rx: watch::Receiver<bool>,
) -> Result<()> {
    let impl_config = &config.implementation;
    if !impl_config.enabled {
        anyhow::bail!("Implementation is disabled in config");
    }

    let agent_name = impl_config
        .implementing_agent()
        .ok_or_else(|| anyhow::anyhow!("No implementing agent configured"))?;

    let agent_config = config.get_agent(agent_name).ok_or_else(|| {
        anyhow::anyhow!("Implementing agent '{}' not found in config", agent_name)
    })?;

    let agent = AgentType::from_config(agent_name, agent_config, working_dir.to_path_buf())?;

    let conversation_key = implementing_conversation_key(agent_name);
    let agent_session =
        state.get_or_create_agent_session(&conversation_key, ResumeStrategy::ConversationResume);
    let conversation_id = agent_session.conversation_id.clone().ok_or_else(|| {
        anyhow::anyhow!("No conversation ID available for implementation follow-up")
    })?;

    let session_logger = create_session_logger(&state.workflow_session_id)?;
    session_sender.set_logger(session_logger.clone());

    session_sender.send_output(format!(
        "[implementation] Starting follow-up using agent: {}",
        agent_name
    ));

    let prompt = build_implementation_followup_prompt(state, working_dir, user_message);

    let context = AgentContext {
        session_sender: session_sender.clone(),
        phase: IMPLEMENTATION_FOLLOWUP_PHASE.to_string(),
        conversation_id: Some(conversation_id),
        resume_strategy: ResumeStrategy::ConversationResume,
        cancel_rx: Some(cancel_rx),
        session_logger,
    };

    let result = agent
        .execute_streaming_with_context(
            prompt,
            Some(IMPLEMENTATION_SYSTEM_PROMPT.to_string()),
            None,
            context,
        )
        .await
        .context("Implementation follow-up agent execution failed")?;

    if let Some(conv_id) = result.conversation_id {
        state.update_agent_conversation_id(&conversation_key, conv_id);
    }

    state.set_updated_at();
    state.save_atomic(state_path)?;
    session_sender.send_state_update(state.clone());

    Ok(())
}

/// Builds the implementation prompt with clean format and skill invocation at the end.
fn build_implementation_prompt(
    state: &State,
    working_dir: &Path,
    iteration: u32,
    feedback: Option<&str>,
) -> String {
    // Resolve plan path to absolute
    let plan_path = if state.plan_file.is_absolute() {
        state.plan_file.clone()
    } else {
        working_dir.join(&state.plan_file)
    };

    let feedback_section = match feedback {
        Some(fb) => format!(
            "\n####################### FEEDBACK FROM REVIEW #######################\n{}\n######################################################################\n",
            fb
        ),
        None => String::new(),
    };

    format!(
        r#"Implement the approved plan.

######################### IMPLEMENTATION #{iteration} #########################

Paths:
- Workspace: {workspace}
- Plan file: {plan}
{feedback_section}
Run the "implementation" skill to execute the plan."#,
        iteration = iteration,
        workspace = working_dir.display(),
        plan = plan_path.display(),
        feedback_section = feedback_section,
    )
}

fn build_implementation_followup_prompt(
    state: &State,
    working_dir: &Path,
    user_message: &str,
) -> String {
    let plan_path = if state.plan_file.is_absolute() {
        state.plan_file.clone()
    } else {
        working_dir.join(&state.plan_file)
    };

    format!(
        r#"Continue the implementation.

Paths:
- Workspace: {workspace}
- Plan file: {plan}

############################ USER MESSAGE ############################
{user_message}
######################################################################

Apply the requested changes using available tools."#,
        workspace = working_dir.display(),
        plan = plan_path.display(),
        user_message = user_message,
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
    fn test_build_implementation_prompt_basic() {
        let state = minimal_state();
        let working_dir = PathBuf::from("/tmp/workspace");
        let prompt = build_implementation_prompt(&state, &working_dir, 1, None);

        // Check paths are included
        assert!(prompt.contains("/tmp/workspace"));
        assert!(prompt.contains("/tmp/test-plan/plan.md"));

        // Check iteration
        assert!(prompt.contains("IMPLEMENTATION #1"));

        // Check skill instruction is last
        assert!(prompt.ends_with(r#"Run the "implementation" skill to execute the plan."#));
    }

    #[test]
    fn test_build_implementation_prompt_with_feedback() {
        let state = minimal_state();
        let working_dir = PathBuf::from("/tmp/workspace");
        let feedback = "Missing error handling in src/main.rs";
        let prompt = build_implementation_prompt(&state, &working_dir, 2, Some(feedback));

        // Should include the feedback section
        assert!(prompt.contains("FEEDBACK FROM REVIEW"));
        assert!(prompt.contains("Missing error handling"));

        // Iteration should be 2
        assert!(prompt.contains("IMPLEMENTATION #2"));

        // Skill instruction still last
        assert!(prompt.ends_with(r#"Run the "implementation" skill to execute the plan."#));
    }

    #[test]
    fn test_build_implementation_followup_prompt() {
        let state = minimal_state();
        let working_dir = PathBuf::from("/tmp/workspace");
        let prompt = build_implementation_followup_prompt(&state, &working_dir, "Fix the bug");

        assert!(prompt.contains("USER MESSAGE"));
        assert!(prompt.contains("Fix the bug"));
        assert!(prompt.contains("/tmp/workspace"));
    }
}
