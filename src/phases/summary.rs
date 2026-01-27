use crate::agents::{AgentContext, AgentType};
use crate::config::WorkflowConfig;
use crate::domain::types::ResumeStrategy;
use crate::domain::view::WorkflowView;
use crate::phases::ReviewResult;
use crate::prompt_format::PromptBuilder;
use crate::session_daemon::SessionLogger;
use crate::tui::SessionEventSender;
use anyhow::Result;
use std::path::Path;
use std::sync::Arc;

const SUMMARY_SYSTEM_PROMPT: &str = r#"You are a concise technical summarizer.
Your task is to provide a brief, focused summary of the content provided.
Keep summaries short (3-5 bullet points max) and highlight only the most important aspects.
Do not include code blocks or lengthy explanations - be succinct.
When referencing files, use absolute paths."#;

pub fn spawn_summary_generation(
    phase: String,
    view: &WorkflowView,
    working_dir: &Path,
    config: &WorkflowConfig,
    sender: SessionEventSender,
    reviews: Option<&[ReviewResult]>,
    session_logger: Arc<SessionLogger>,
) {
    let plan_path = view
        .plan_path()
        .map(|p| p.as_path().to_path_buf())
        .unwrap_or_else(|| working_dir.join("plan.md"));
    let working_dir = working_dir.to_path_buf();
    let config = config.clone();
    let phase_clone = phase.clone();

    let summary_input = if phase.starts_with("Reviewing") {
        if let Some(reviews) = reviews {
            build_review_summary_input(reviews)
        } else {
            "No review data available.".to_string()
        }
    } else {
        match std::fs::read_to_string(&plan_path) {
            Ok(content) => build_plan_summary_input(&content, &phase),
            Err(e) => format!("Failed to read plan file: {}", e),
        }
    };

    sender.send_run_tab_summary_generating(phase.clone());

    tokio::spawn(async move {
        match run_summary_generation(
            &phase_clone,
            &summary_input,
            &working_dir,
            &config,
            sender.clone(),
            session_logger,
        )
        .await
        {
            Ok(summary) => {
                sender.send_run_tab_summary_ready(phase_clone, summary);
            }
            Err(e) => {
                sender.send_run_tab_summary_error(phase_clone, e.to_string());
            }
        }
    });
}

fn build_plan_summary_input(plan_content: &str, phase: &str) -> String {
    let max_len = 8000;
    let content = if plan_content.len() > max_len {
        // Find valid UTF-8 boundary at or before max_len
        let truncate_at = (0..=max_len)
            .rev()
            .find(|&i| plan_content.is_char_boundary(i))
            .unwrap_or(0);
        format!(
            "{}...\n\n[Content truncated, {} characters total]",
            plan_content.get(..truncate_at).unwrap_or(""),
            plan_content.len()
        )
    } else {
        plan_content.to_string()
    };

    PromptBuilder::new()
        .phase("summary")
        .instructions(&format!(
            r#"Summarize this {} plan. Highlight:
- Key components/features being implemented
- Major files to be modified (use absolute paths)
- Any risks or considerations mentioned"#,
            phase
        ))
        .context(&format!("# Plan Content\n\n{}", content))
        .build()
}

fn build_review_summary_input(reviews: &[ReviewResult]) -> String {
    let mut review_content = String::new();
    for review in reviews {
        review_content.push_str(&format!("\n## Reviewer: {}\n", review.agent_name));
        let verdict = if review.needs_revision {
            "Needs Revision"
        } else {
            "Approved"
        };
        review_content.push_str(&format!("Verdict: {}\n", verdict));
        review_content.push_str(&format!("Feedback:\n{}\n", review.feedback));
    }

    PromptBuilder::new()
        .phase("summary")
        .instructions(
            r#"Summarize these code review results. Highlight:
- Overall verdict (approved/rejected)
- Key issues found
- Main recommendations"#,
        )
        .context(&format!("# Review Results\n{}", review_content))
        .build()
}

async fn run_summary_generation(
    phase: &str,
    input: &str,
    working_dir: &Path,
    config: &WorkflowConfig,
    sender: SessionEventSender,
    session_logger: Arc<SessionLogger>,
) -> Result<String> {
    let agent_name = &config.workflow.planning.agent;

    let agent_config = config
        .get_agent(agent_name)
        .ok_or_else(|| anyhow::anyhow!("Summary agent '{}' not found in config", agent_name))?;

    let agent = AgentType::from_config(agent_name, agent_config, working_dir.to_path_buf())?;

    let max_turns = Some(1);

    let context = AgentContext {
        session_sender: sender,
        phase: phase.to_string(),
        conversation_id: None,
        resume_strategy: ResumeStrategy::Stateless,
        cancel_rx: None,
        session_logger,
    };

    let result = agent
        .execute_streaming_with_context(
            input.to_string(),
            Some(SUMMARY_SYSTEM_PROMPT.to_string()),
            max_turns,
            context,
        )
        .await?;

    Ok(result.output)
}

#[cfg(test)]
#[path = "tests/summary_tests.rs"]
mod tests;
