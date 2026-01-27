use crate::agents::{AgentContext, AgentType};
use crate::config::WorkflowConfig;
use crate::domain::actor::WorkflowMessage;
use crate::domain::types::{AgentId, ConversationId, PhaseLabel, ResumeStrategy};
use crate::domain::view::WorkflowView;
use crate::domain::WorkflowCommand as DomainCommand;
use crate::phases::planning_conversation_key;
use crate::phases::ReviewResult;
use crate::planning_paths;
use crate::prompt_format::PromptBuilder;
use crate::session_daemon::{LogCategory, LogLevel, SessionLogger};
use crate::tui::SessionEventSender;
use anyhow::Result;
use ractor::ActorRef;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::oneshot;

const REVISION_SYSTEM_PROMPT: &str = r#"You are revising an implementation plan based on reviewer feedback.
Focus on addressing all blocking issues first, then important improvements.
Verify each finding before making changes. Only address those that require revision.
IMPORTANT: Use absolute paths for all file references in the revised plan.
DO NOT include timelines, schedules, dates, durations, or time estimates in the revised plan.
Examples to reject: "in two weeks", "Phase 1: Week 1-2", "Q1 delivery", "Sprint 1", "by end of day".
"#;

#[allow(clippy::too_many_arguments)]
pub async fn run_revision_phase_with_context(
    view: &WorkflowView,
    working_dir: &Path,
    config: &WorkflowConfig,
    reviews: &[ReviewResult],
    session_sender: SessionEventSender,
    iteration: u32,
    session_logger: Arc<SessionLogger>,
    actor_ref: Option<ActorRef<WorkflowMessage>>,
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
        if session_resume_active {
            " (session resume)"
        } else {
            ""
        }
    ));

    // Compute session folder for supplementary file access
    let workflow_id = view
        .workflow_id()
        .ok_or_else(|| anyhow::anyhow!("WorkflowView missing workflow_id"))?;
    let session_folder = planning_paths::session_dir(&workflow_id.to_string())?;

    let prompt = build_revision_prompt_with_reviews(
        view,
        reviews,
        working_dir,
        &session_folder,
        session_resume_active,
        iteration,
    );

    let phase_name = format!("Revising #{}", iteration);
    // Revision always uses ConversationResume to continue the planning conversation.
    // This ensures the agent has full context from the original planning phase.
    let resume_strategy = ResumeStrategy::ConversationResume;
    // Use the SAME session key as planning phase for session continuity
    let conversation_id_name = planning_conversation_key(agent_name);
    let agent_id = AgentId::from(conversation_id_name.as_str());

    // Get existing conversation state from view (read-only)
    let (conversation_id, conv_resume_strategy) = view
        .agent_conversations()
        .get(&agent_id)
        .map(|state| {
            (
                state.conversation_id().map(|c| c.0.clone()),
                state.resume_strategy(),
            )
        })
        .unwrap_or((None, ResumeStrategy::ConversationResume));

    // Dispatch RevisingStarted command to CQRS actor
    let feedback_summary = build_feedback_summary(reviews);
    dispatch_revising_command(
        &actor_ref,
        &session_logger,
        DomainCommand::RevisingStarted { feedback_summary },
    )
    .await;

    // Dispatch RecordInvocation command to CQRS actor
    dispatch_revising_command(
        &actor_ref,
        &session_logger,
        DomainCommand::RecordInvocation {
            agent_id,
            phase: PhaseLabel::Revising,
            conversation_id: conversation_id.clone().map(ConversationId::from),
            resume_strategy: conv_resume_strategy,
        },
    )
    .await;

    let context = AgentContext {
        session_sender: session_sender.clone(),
        phase: phase_name,
        conversation_id,
        resume_strategy,
        cancel_rx: None,
        session_logger: session_logger.clone(),
    };

    let result = agent
        .execute_streaming_with_context(
            prompt,
            Some(REVISION_SYSTEM_PROMPT.to_string()),
            max_turns,
            context,
        )
        .await?;

    session_sender.send_output(format!("[revision:{}] Revision phase complete", agent_name));
    session_sender.send_output(format!(
        "[revision:{}] Result preview: {}...",
        agent_name,
        result.output.chars().take(200).collect::<String>()
    ));

    Ok(())
}

fn build_revision_prompt_with_reviews(
    view: &WorkflowView,
    reviews: &[ReviewResult],
    working_dir: &Path,
    session_folder: &Path,
    session_resume_active: bool,
    iteration: u32,
) -> String {
    let plan_path = view
        .plan_path()
        .map(|p| p.0.display().to_string())
        .unwrap_or_else(|| "plan.md".to_string());

    // Build summary table
    let mut summary_table =
        String::from("| Reviewer | Verdict | Summary |\n|----------|---------|---------|");
    for review in reviews {
        let verdict = if review.needs_revision {
            "NEEDS REVISION"
        } else {
            "APPROVED"
        };
        summary_table.push_str(&format!(
            "\n| {} | {} | {} |",
            review.agent_name, verdict, review.summary
        ));
    }

    // Build feedback file paths lists
    let (needs_revision_reviews, approved_reviews): (Vec<_>, Vec<_>) =
        reviews.iter().partition(|r| r.needs_revision);

    let mut feedback_files = String::new();

    if !needs_revision_reviews.is_empty() {
        feedback_files
            .push_str("Read the detailed feedback from each reviewer who requested revision:");
        for review in &needs_revision_reviews {
            let feedback_path =
                session_folder.join(format!("feedback_{}_{}.md", iteration, review.agent_name));
            feedback_files.push_str(&format!(
                "\n- {}: {}",
                review.agent_name,
                feedback_path.display()
            ));
        }
    }

    if !approved_reviews.is_empty() {
        if !feedback_files.is_empty() {
            feedback_files.push_str("\n\n");
        }
        feedback_files.push_str("Reviewers who approved (no action needed):");
        for review in &approved_reviews {
            let feedback_path =
                session_folder.join(format!("feedback_{}_{}.md", iteration, review.agent_name));
            feedback_files.push_str(&format!(
                "\n- {}: {}",
                review.agent_name,
                feedback_path.display()
            ));
        }
    }

    if session_resume_active {
        // Continuation prompt - leverages existing session context
        // The agent already knows the workspace, plan file, and original context
        format!(
            "The reviewers have provided feedback on your plan. \
             Please revise the plan at {} to address all issues raised.\n\n\
             IMPORTANT: Do not add timelines, schedules, dates, durations, or time estimates \
             (e.g., \"in two weeks\", \"Sprint 1\", \"Q1 delivery\").\n\n\
             You may create supplementary files in the session folder: {}\n\n\
             # Review Summary\n\n{}\n\n\
             # Feedback Files\n\n{}\n\n\
             Please address all issues raised by reviewers who requested revision.",
            plan_path,
            session_folder.display(),
            summary_table,
            feedback_files
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

        let context = format!(
            "# Review Summary\n\n{}\n\n# Feedback Files\n\n{}\n\n\
             Please address all issues raised by reviewers who requested revision.",
            summary_table, feedback_files
        );

        PromptBuilder::new()
            .phase("revising")
            .instructions(&instructions)
            .input("workspace-root", &working_dir.display().to_string())
            .input("plan-output-path", &plan_path)
            .input("session-folder-path", &session_folder.display().to_string())
            .context(&context)
            .constraint("Use absolute paths for all file references in the revised plan")
            .build()
    }
}

/// Build a summary of reviewer feedback for the RevisingStarted event.
fn build_feedback_summary(reviews: &[ReviewResult]) -> String {
    let mut summary = String::new();
    for review in reviews {
        if review.needs_revision {
            if !summary.is_empty() {
                summary.push_str("; ");
            }
            summary.push_str(&format!("{}: {}", review.agent_name, review.summary));
        }
    }
    if summary.is_empty() {
        "No revision feedback".to_string()
    } else {
        summary
    }
}

/// Helper to dispatch revising commands to the CQRS actor.
async fn dispatch_revising_command(
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
                &format!("Failed to send revising command: {}", e),
            );
            return;
        }
        match reply_rx.await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                session_logger.log(
                    LogLevel::Warn,
                    LogCategory::Workflow,
                    &format!("Revising command rejected: {}", e),
                );
            }
            Err(_) => {
                session_logger.log(
                    LogLevel::Warn,
                    LogCategory::Workflow,
                    "Revising command reply channel closed",
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/revising_tests.rs"]
mod tests;
