use crate::agents::{AgentContext, AgentType};
use crate::config::WorkflowConfig;
use crate::domain::actor::WorkflowMessage;
use crate::domain::types::{
    AgentId, ConversationId, PhaseLabel, ResumeStrategy as DomainResumeStrategy, ResumeStrategy,
};
use crate::domain::view::WorkflowView;
use crate::domain::WorkflowCommand as DomainCommand;
use crate::phases::planning_conversation_key;
use crate::planning_paths;
use crate::prompt_format::PromptBuilder;
use crate::session_daemon::{LogCategory, LogLevel, SessionLogger};
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
    view: &WorkflowView,
    working_dir: &Path,
    config: &WorkflowConfig,
    session_sender: SessionEventSender,
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

    let prompt = build_planning_prompt(view, working_dir);

    // Planning always uses ConversationResume to enable revision continuity.
    // The agent will capture its conversation ID on first run, then resume on revision.
    let configured_strategy = ResumeStrategy::ConversationResume;
    // Use namespaced session key to avoid collisions with reviewer sessions
    let conversation_id_name = planning_conversation_key(agent_name);

    // Get conversation state from view if it exists
    let agent_id = AgentId::from(conversation_id_name.as_str());
    let (conversation_id, resume_strategy) = match view.agent_conversations().get(&agent_id) {
        Some(state) => (
            state.conversation_id().map(|c| c.0.clone()),
            state.resume_strategy(),
        ),
        None => (None, configured_strategy),
    };

    // Dispatch RecordInvocation command to CQRS actor (caller handles state persistence)
    dispatch_planning_command(
        &actor_ref,
        &session_logger,
        DomainCommand::RecordInvocation {
            agent_id: agent_id.clone(),
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
        // Dispatch RecordAgentConversation command to CQRS actor (caller handles state persistence)
        dispatch_planning_command(
            &actor_ref,
            &session_logger,
            DomainCommand::RecordAgentConversation {
                agent_id,
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

fn build_planning_prompt(view: &WorkflowView, working_dir: &Path) -> String {
    // Get plan path from view (absolute path in ~/.planning-agent/sessions/)
    let plan_path = view
        .plan_path()
        .map(|p| p.0.display().to_string())
        .unwrap_or_default();

    // Get session folder path for supplementary files
    // session_dir() creates the directory if it doesn't exist; only fails if home dir unavailable
    let workflow_id_str = view
        .workflow_id()
        .map(|id| id.0.to_string())
        .unwrap_or_default();
    let session_folder = planning_paths::session_dir(&workflow_id_str)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| {
            // Defensive fallback: derive from plan_path parent
            view.plan_path()
                .and_then(|p| p.0.parent())
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        });

    let feature_name = view.feature_name().map(|f| f.0.as_str()).unwrap_or("");
    let objective = view.objective().map(|o| o.0.as_str()).unwrap_or("");

    let mut builder = PromptBuilder::new()
        .phase("planning")
        .instructions(r#"Use the "planning" skill to create the plan. Write your plan to the plan-output-path file."#)
        .input("workspace-root", &working_dir.display().to_string())
        .input("feature-name", feature_name)
        .input("objective", objective)
        .input("plan-output-path", &plan_path)
        .input("session-folder-path", &session_folder);

    // Add worktree context if applicable
    if let Some(wt_state) = view.worktree_info() {
        builder = builder
            .input(
                "worktree-path",
                &wt_state.worktree_path().display().to_string(),
            )
            .input("worktree-branch", wt_state.branch_name())
            .input(
                "original-dir",
                &wt_state.original_dir().display().to_string(),
            );
        if let Some(source) = wt_state.source_branch() {
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
