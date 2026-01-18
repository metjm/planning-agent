//! JSON-mode implementation phase.
//!
//! This module implements the plan execution phase using JSON-mode agents.
//! It replaces the previous embedded PTY terminal with structured agent execution.

use crate::agents::{AgentContext, AgentType};
use crate::config::WorkflowConfig;
use crate::phases::implementing_conversation_key;
use crate::planning_paths;
use crate::prompt_format::{xml_escape, PromptBuilder};
use crate::session_logger::{create_session_logger, SessionLogger};
use crate::state::{ResumeStrategy, State};
use crate::tui::SessionEventSender;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::watch;

const IMPLEMENTATION_SYSTEM_PROMPT: &str = r#"You are an implementation agent that executes approved plans.

Your task is to:
1. Read the approved plan to understand exactly what needs to be implemented
2. Implement each step of the plan using the available tools
3. Make precise, focused changes - only implement what the plan specifies
4. Verify your changes compile and don't break existing functionality

Be precise and methodical:
- Follow the plan exactly, step by step
- Use Read/Write/Glob/Grep/Bash tools to implement changes
- Do not introduce new features or make unrelated changes
- Use absolute paths for all file references

IMPORTANT: You are operating in JSON mode. All tool calls and file operations
should be done via the provided tools, not through a terminal."#;

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

    let agent_config = config
        .get_agent(agent_name)
        .ok_or_else(|| anyhow::anyhow!("Implementing agent '{}' not found in config", agent_name))?;

    session_sender.send_output(format!(
        "[implementation] Starting implementation round {} using agent: {}",
        iteration, agent_name
    ));

    // Create agent
    let agent = AgentType::from_config(agent_name, agent_config, working_dir.to_path_buf())?;

    // Build the prompt
    let prompt = build_implementation_prompt(state, working_dir, iteration, feedback);

    // Get log path
    let log_path = planning_paths::session_implementation_log_path(
        &state.workflow_session_id,
        iteration,
    )?;

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
        session_sender.send_output(format!(
            "[implementation] Follow-up failed: {}",
            err
        ));
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

    let agent_config = config
        .get_agent(agent_name)
        .ok_or_else(|| anyhow::anyhow!("Implementing agent '{}' not found in config", agent_name))?;

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

/// Builds the implementation prompt.
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

    let mut builder = PromptBuilder::new()
        .phase("implementation")
        .instructions(&format!(
            r#"Implementation attempt #{}: implement the approved plan fully within the workspace root.

1. Read the plan file to understand what needs to be implemented
2. Implement each step of the plan in order
3. Use the available tools (Read, Write, Glob, Grep, Bash) to make changes
4. Verify your changes don't break existing functionality

Focus on:
- Implementing exactly what the plan specifies
- Not introducing bugs or unrelated changes
- Maintaining code quality and consistency with existing patterns"#,
            iteration
        ))
        .input("workspace-root", &xml_escape(&working_dir.display().to_string()))
        .input("plan-path", &xml_escape(&plan_path.display().to_string()))
        .input("iteration", &iteration.to_string())
        .tools("Use Read/Write/Glob/Grep/Bash tools; edit files via tools only (no PTY).")
        .constraint("Use absolute paths for all file references");

    // Add feedback if provided
    if let Some(fb) = feedback {
        let feedback_context = format!(
            "# Feedback from Previous Review\n\nThe previous implementation attempt was not approved. Address the following feedback:\n\n{}",
            fb
        );
        builder = builder.context(&xml_escape(&feedback_context));
    }

    builder.build()
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

    PromptBuilder::new()
        .phase("implementation-followup")
        .instructions(
            "Continue the implementation conversation. Respond to the user's follow-up and apply any requested changes using the available tools.",
        )
        .input("workspace-root", &xml_escape(&working_dir.display().to_string()))
        .input("plan-path", &xml_escape(&plan_path.display().to_string()))
        .input("user-message", &xml_escape(user_message))
        .tools("Use Read/Write/Glob/Grep/Bash tools; edit files via tools only (no PTY).")
        .constraint("Use absolute paths for all file references")
        .build()
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
    fn test_build_implementation_prompt_basic() {
        let state = minimal_state();
        let working_dir = PathBuf::from("/tmp/workspace");
        let prompt = build_implementation_prompt(&state, &working_dir, 1, None);

        // Check XML structure
        assert!(prompt.starts_with("<user-prompt>"));
        assert!(prompt.ends_with("</user-prompt>"));
        assert!(prompt.contains("<phase>implementation</phase>"));

        // Check inputs
        assert!(prompt.contains("<workspace-root>"));
        assert!(prompt.contains("<plan-path>"));
        assert!(prompt.contains("<iteration>1</iteration>"));

        // Check instructions
        assert!(prompt.contains("Implementation attempt #1"));
    }

    #[test]
    fn test_build_implementation_prompt_with_feedback() {
        let state = minimal_state();
        let working_dir = PathBuf::from("/tmp/workspace");
        let feedback = "Missing error handling in src/main.rs";
        let prompt = build_implementation_prompt(&state, &working_dir, 2, Some(feedback));

        // Should include the feedback
        assert!(prompt.contains("Feedback from Previous Review"));
        assert!(prompt.contains("Missing error handling"));

        // Iteration should be 2
        assert!(prompt.contains("Implementation attempt #2"));
    }

    #[test]
    fn test_build_implementation_prompt_escapes_special_chars() {
        let mut state = minimal_state();
        state.plan_file = PathBuf::from("/path/with<special>&chars/plan.md");
        let working_dir = PathBuf::from("/tmp/workspace");
        let prompt = build_implementation_prompt(&state, &working_dir, 1, None);

        // Special characters should be escaped
        assert!(prompt.contains("&lt;"));
        assert!(prompt.contains("&amp;"));
    }

    #[test]
    fn test_build_implementation_prompt_with_xml_in_feedback() {
        let state = minimal_state();
        let working_dir = PathBuf::from("/tmp/workspace");
        let feedback = "Add <error-handling> to the <config> module";
        let prompt = build_implementation_prompt(&state, &working_dir, 2, Some(feedback));

        // XML in feedback should be escaped
        assert!(prompt.contains("&lt;error-handling&gt;"));
        assert!(prompt.contains("&lt;config&gt;"));
    }
}
