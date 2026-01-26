use crate::agents::{AgentContext, AgentType};
use crate::config::WorkflowConfig;
use crate::domain::actor::WorkflowMessage;
use crate::domain::types::{
    AgentId, ConversationId, PhaseLabel, ResumeStrategy as DomainResumeStrategy,
};
use crate::domain::WorkflowCommand as DomainCommand;
use crate::phases::planning_conversation_key;
use crate::planning_paths;
use crate::prompt_format::PromptBuilder;
use crate::session_daemon::{LogCategory, LogLevel, SessionLogger};
use crate::state::{ResumeStrategy, State};
use crate::tui::SessionEventSender;
use anyhow::Result;
use ractor::ActorRef;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::oneshot;

/// System prompt for planning phase - simple instruction to use the planning skill.
pub const PLANNING_SYSTEM_PROMPT: &str =
    r#"Use the "planning" skill to create the plan. Write your plan to the plan-output-path file."#;

pub async fn run_planning_phase_with_context(
    state: &mut State,
    working_dir: &Path,
    config: &WorkflowConfig,
    session_sender: SessionEventSender,
    state_path: &Path,
    session_logger: Arc<SessionLogger>,
    actor_ref: Option<ActorRef<WorkflowMessage>>,
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
    let agent_session =
        state.get_or_create_agent_session(&conversation_id_name, configured_strategy);
    let conversation_id = agent_session.conversation_id.clone();
    let resume_strategy = agent_session.resume_strategy.clone();

    state.record_invocation(&conversation_id_name, "Planning");
    state.set_updated_at();
    state.save_atomic(state_path)?;

    // Dispatch RecordInvocation command to CQRS actor
    dispatch_planning_command(
        &actor_ref,
        &session_logger,
        DomainCommand::RecordInvocation {
            agent_id: AgentId::from(conversation_id_name.as_str()),
            phase: PhaseLabel::Planning,
            conversation_id: conversation_id.clone().map(ConversationId::from),
            resume_strategy: to_domain_resume_strategy(&resume_strategy),
        },
    )
    .await;

    let context = AgentContext {
        session_sender: session_sender.clone(),
        phase: "Planning".to_string(),
        conversation_id,
        resume_strategy,
        cancel_rx: None,
        session_logger: session_logger.clone(),
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

        // Dispatch RecordAgentConversation command to CQRS actor
        dispatch_planning_command(
            &actor_ref,
            &session_logger,
            DomainCommand::RecordAgentConversation {
                agent_id: AgentId::from(conversation_id_name.as_str()),
                resume_strategy: DomainResumeStrategy::ConversationResume,
                conversation_id: Some(ConversationId::from(captured_id.clone())),
            },
        )
        .await;

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
            .input(
                "worktree-path",
                &wt_state.worktree_path.display().to_string(),
            )
            .input("worktree-branch", &wt_state.branch_name)
            .input("original-dir", &wt_state.original_dir.display().to_string());
        if let Some(ref source) = wt_state.source_branch {
            builder = builder.input("source-branch", source);
        }
    }

    builder.build()
}

/// Helper to dispatch planning commands to the CQRS actor.
async fn dispatch_planning_command(
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
                &format!("Failed to send planning command: {}", e),
            );
            return;
        }
        match reply_rx.await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                session_logger.log(
                    LogLevel::Warn,
                    LogCategory::Workflow,
                    &format!("Planning command rejected: {}", e),
                );
            }
            Err(_) => {
                session_logger.log(
                    LogLevel::Warn,
                    LogCategory::Workflow,
                    "Planning command reply channel closed",
                );
            }
        }
    }
}

/// Convert state ResumeStrategy to domain ResumeStrategy.
fn to_domain_resume_strategy(strategy: &ResumeStrategy) -> DomainResumeStrategy {
    match strategy {
        ResumeStrategy::Stateless => DomainResumeStrategy::Stateless,
        ResumeStrategy::ConversationResume => DomainResumeStrategy::ConversationResume,
        ResumeStrategy::ResumeLatest => DomainResumeStrategy::ResumeLatest,
    }
}

#[cfg(test)]
#[path = "tests/planning_tests.rs"]
mod tests;
