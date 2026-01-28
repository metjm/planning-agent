//! JSON-mode implementation phase.
//!
//! This module implements the plan execution phase using JSON-mode agents.
//! It replaces the previous embedded PTY terminal with structured agent execution.

use crate::agents::{AgentContext, AgentType};
use crate::config::WorkflowConfig;
use crate::domain::actor::WorkflowMessage;
use crate::domain::types::{AgentId, ConversationId, PhaseLabel, ResumeStrategy, WorktreeState};
use crate::domain::view::WorkflowView;
use crate::domain::WorkflowCommand as DomainCommand;
use crate::phases::implementing_conversation_key;
use crate::planning_paths;
use crate::session_daemon::{LogCategory, LogLevel, SessionLogger};
use crate::tui::SessionEventSender;
use anyhow::{Context, Result};
use ractor::ActorRef;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{oneshot, watch};

/// Minimal system prompt - the skill handles the details.
const IMPLEMENTATION_SYSTEM_PROMPT: &str =
    "You are an implementation agent that executes approved plans.";

pub const IMPLEMENTATION_FOLLOWUP_PHASE: &str = "Implementation Follow-up";

/// Result of running the implementation phase.
#[derive(Debug, Clone)]
pub struct ImplementationResult {
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
/// * `view` - The current workflow view (read-only projection of state)
/// * `config` - The workflow configuration
/// * `working_dir` - The working directory for the implementation
/// * `iteration` - The current iteration number (1-indexed)
/// * `feedback` - Optional feedback from a previous review iteration
/// * `previous_conversation_id` - Conversation ID from previous round (passed directly from
///   orchestrator because the view is stale within a single workflow execution and cannot see
///   IDs captured earlier)
/// * `session_sender` - Channel to send session events
/// * `session_logger` - Logger for the session
/// * `actor_ref` - Optional actor reference for dispatching commands
///
/// # Returns
/// An `ImplementationResult` containing the output and metadata from the run.
#[allow(clippy::too_many_arguments)]
pub async fn run_implementation_phase(
    view: &WorkflowView,
    config: &WorkflowConfig,
    working_dir: &Path,
    iteration: u32,
    feedback: Option<&str>,
    previous_conversation_id: Option<ConversationId>,
    session_sender: SessionEventSender,
    session_logger: Arc<SessionLogger>,
    actor_ref: Option<ActorRef<WorkflowMessage>>,
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
    let prompt = build_implementation_prompt(view, working_dir, iteration, feedback);

    // Get workflow ID from view
    let workflow_id = view
        .workflow_id()
        .ok_or_else(|| anyhow::anyhow!("WorkflowView missing workflow_id"))?;

    // Get log path
    let log_path =
        planning_paths::session_implementation_log_path(&workflow_id.to_string(), iteration)?;

    // Prepare conversation key for resume
    let conversation_key = implementing_conversation_key(agent_name);
    let agent_id = AgentId::from(conversation_key.as_str());

    // Use conversation ID passed from orchestrator (preferred) or fallback to view (first round)
    // NOTE: The view is stale within a single workflow execution and cannot see conversation IDs
    // captured in previous rounds. The orchestrator passes the ID directly to ensure context
    // preservation across rounds. The view fallback handles session resume scenarios.
    let conversation_id = previous_conversation_id.map(|c| c.0).or_else(|| {
        view.agent_conversations()
            .get(&agent_id)
            .and_then(|conv| conv.conversation_id().map(|c| c.0.clone()))
    });

    // Dispatch RecordInvocation command to CQRS actor
    dispatch_implementation_command(
        &actor_ref,
        &session_logger,
        DomainCommand::RecordInvocation {
            agent_id: agent_id.clone(),
            phase: PhaseLabel::Implementing,
            conversation_id: conversation_id.clone().map(ConversationId::from),
            resume_strategy: ResumeStrategy::ConversationResume,
        },
    )
    .await;

    let phase_name = format!("Implementation #{}", iteration);

    let context = AgentContext {
        session_sender: session_sender.clone(),
        phase: phase_name,
        conversation_id: conversation_id.clone(),
        resume_strategy: ResumeStrategy::ConversationResume,
        cancel_rx: None,
        session_logger: session_logger.clone(),
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

    // Store captured conversation ID for future resume
    if let Some(ref captured_id) = result.conversation_id {
        dispatch_implementation_command(
            &actor_ref,
            &session_logger,
            DomainCommand::RecordAgentConversation {
                agent_id,
                resume_strategy: ResumeStrategy::ConversationResume,
                conversation_id: Some(ConversationId::from(captured_id.clone())),
            },
        )
        .await;
    }

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
        log_path,
        is_error: result.is_error,
        stop_reason: result.stop_reason,
        conversation_id: result.conversation_id,
    })
}

/// Runs a follow-up interaction after implementation is complete.
#[allow(clippy::too_many_arguments)]
pub async fn run_implementation_interaction(
    view: &WorkflowView,
    config: &WorkflowConfig,
    working_dir: &Path,
    user_message: &str,
    mut session_sender: SessionEventSender,
    session_logger: Arc<SessionLogger>,
    cancel_rx: watch::Receiver<bool>,
    actor_ref: Option<ActorRef<WorkflowMessage>>,
) -> Result<()> {
    let result = run_implementation_interaction_inner(
        view,
        config,
        working_dir,
        user_message,
        &mut session_sender,
        session_logger,
        cancel_rx,
        actor_ref,
    )
    .await;

    if let Err(err) = &result {
        session_sender.send_output(format!("[implementation] Follow-up failed: {}", err));
    }

    session_sender.send_implementation_interaction_finished();
    result
}

#[allow(clippy::too_many_arguments)]
async fn run_implementation_interaction_inner(
    view: &WorkflowView,
    config: &WorkflowConfig,
    working_dir: &Path,
    user_message: &str,
    session_sender: &mut SessionEventSender,
    session_logger: Arc<SessionLogger>,
    cancel_rx: watch::Receiver<bool>,
    actor_ref: Option<ActorRef<WorkflowMessage>>,
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
    let agent_id = AgentId::from(conversation_key.as_str());

    // Get conversation ID from view - required for follow-up
    let conversation_id = view
        .agent_conversations()
        .get(&agent_id)
        .and_then(|conv| conv.conversation_id().map(|c| c.0.clone()))
        .ok_or_else(|| {
            anyhow::anyhow!("No conversation ID available for implementation follow-up")
        })?;

    session_sender.send_output(format!(
        "[implementation] Starting follow-up using agent: {}",
        agent_name
    ));

    let prompt = build_implementation_followup_prompt(view, working_dir, user_message);

    // Dispatch RecordInvocation command to CQRS actor
    // Use Implementing phase for follow-up since it's a continuation of implementation
    dispatch_implementation_command(
        &actor_ref,
        &session_logger,
        DomainCommand::RecordInvocation {
            agent_id: agent_id.clone(),
            phase: PhaseLabel::Implementing,
            conversation_id: Some(ConversationId::from(conversation_id.clone())),
            resume_strategy: ResumeStrategy::ConversationResume,
        },
    )
    .await;

    let context = AgentContext {
        session_sender: session_sender.clone(),
        phase: IMPLEMENTATION_FOLLOWUP_PHASE.to_string(),
        conversation_id: Some(conversation_id),
        resume_strategy: ResumeStrategy::ConversationResume,
        cancel_rx: Some(cancel_rx),
        session_logger: session_logger.clone(),
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

    // Store captured conversation ID for future resume via CQRS command
    if let Some(conv_id) = result.conversation_id {
        dispatch_implementation_command(
            &actor_ref,
            &session_logger,
            DomainCommand::RecordAgentConversation {
                agent_id,
                resume_strategy: ResumeStrategy::ConversationResume,
                conversation_id: Some(ConversationId::from(conv_id)),
            },
        )
        .await;
    }

    Ok(())
}

/// Builds the implementation prompt with clean format and skill invocation at the end.
fn build_implementation_prompt(
    view: &WorkflowView,
    working_dir: &Path,
    iteration: u32,
    feedback: Option<&str>,
) -> String {
    // Get plan path from view (already absolute in ~/.planning-agent/sessions/)
    let plan_path = view
        .plan_path()
        .map(|p| p.0.display().to_string())
        .unwrap_or_else(|| "plan.md".to_string());

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
        plan = plan_path,
        feedback_section = feedback_section,
    )
}

fn build_implementation_followup_prompt(
    view: &WorkflowView,
    working_dir: &Path,
    user_message: &str,
) -> String {
    // Get plan path from view (already absolute in ~/.planning-agent/sessions/)
    let plan_path = view
        .plan_path()
        .map(|p| p.0.display().to_string())
        .unwrap_or_else(|| "plan.md".to_string());

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
        plan = plan_path,
        user_message = user_message,
    )
}

/// Builds a prompt instructing the AI to merge worktree changes.
///
/// This is different from `git_worktree::generate_merge_instructions()` which generates
/// user-facing documentation. This prompt instructs the AI to execute git commands.
pub fn build_merge_worktree_prompt(wt_state: &WorktreeState) -> String {
    let target_branch = wt_state.source_branch().unwrap_or("main");
    let worktree_branch = wt_state.branch_name();
    let original_dir = wt_state.original_dir();

    format!(
        r#"Merge the worktree branch into the target branch.

######################### MERGE REQUEST #########################

Current situation:
- Worktree branch: {worktree_branch}
- Target branch: {target_branch}
- Original repository: {original_dir}

Execute the following steps:
1. Navigate to the original repository: cd {original_dir}
2. Fetch latest changes: git fetch origin
3. Checkout the target branch: git checkout {target_branch}
4. Pull latest if needed: git pull origin {target_branch} (skip if no remote tracking)
5. Merge the worktree branch: git merge {worktree_branch}
6. If there are merge conflicts:
   - Analyze each conflict carefully
   - Resolve conflicts intelligently, preserving both the target branch's state and the worktree's changes where appropriate
   - Stage resolved files: git add <resolved-files>
   - Complete the merge: git commit
7. Report the merge result to the user

Important:
- Do NOT delete the worktree or the branch - the user may want to keep them for reference
- If the merge fails or conflicts cannot be auto-resolved, explain the situation clearly
"#,
        worktree_branch = worktree_branch,
        target_branch = target_branch,
        original_dir = original_dir.display(),
    )
}

/// Helper to dispatch implementation commands to the CQRS actor.
async fn dispatch_implementation_command(
    actor_ref: &Option<ActorRef<WorkflowMessage>>,
    session_logger: &Arc<SessionLogger>,
    cmd: DomainCommand,
) {
    if let Some(ref actor) = actor_ref {
        let (reply_tx, reply_rx) = oneshot::channel();
        if let Err(e) =
            actor.send_message(WorkflowMessage::Command(Box::new(cmd.clone()), reply_tx))
        {
            session_logger.log(
                LogLevel::Warn,
                LogCategory::Workflow,
                &format!("Failed to send implementation command: {}", e),
            );
            return;
        }
        match reply_rx.await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                session_logger.log(
                    LogLevel::Warn,
                    LogCategory::Workflow,
                    &format!("Implementation command rejected: {}", e),
                );
            }
            Err(_) => {
                session_logger.log(
                    LogLevel::Warn,
                    LogCategory::Workflow,
                    "Implementation command reply channel closed",
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/implementation_tests.rs"]
mod tests;
