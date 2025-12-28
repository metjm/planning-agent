use crate::agents::{AgentContext, AgentType};
use crate::config::WorkflowConfig;
use crate::phases::ReviewResult;
use crate::state::{ResumeStrategy, State};
use crate::tui::SessionEventSender;
use anyhow::Result;
use std::path::Path;

const SUMMARY_SYSTEM_PROMPT: &str = r#"You are a concise technical summarizer.
Your task is to provide a brief, focused summary of the content provided.
Keep summaries short (3-5 bullet points max) and highlight only the most important aspects.
Do not include code blocks or lengthy explanations - be succinct."#;

pub fn spawn_summary_generation(
    phase: String,
    state: &State,
    working_dir: &Path,
    config: &WorkflowConfig,
    sender: SessionEventSender,
    reviews: Option<&[ReviewResult]>,
) {

    let plan_path = working_dir.join(&state.plan_file);
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
        match run_summary_generation(&phase_clone, &summary_input, &working_dir, &config, sender.clone()).await {
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
        format!("{}...\n\n[Content truncated, {} characters total]",
                &plan_content[..max_len], plan_content.len())
    } else {
        plan_content.to_string()
    };

    format!(
        r#"Summarize this {} plan. Highlight:
- Key components/features being implemented
- Major files to be modified
- Any risks or considerations mentioned

Plan content:
{}
"#,
        phase, content
    )
}

fn build_review_summary_input(reviews: &[ReviewResult]) -> String {
    let mut input = String::from("Summarize these code review results. Highlight:\n");
    input.push_str("- Overall verdict (approved/rejected)\n");
    input.push_str("- Key issues found\n");
    input.push_str("- Main recommendations\n\n");
    input.push_str("Review results:\n");

    for review in reviews {
        input.push_str(&format!("\n## Reviewer: {}\n", review.agent_name));
        let verdict = if review.needs_revision { "Needs Revision" } else { "Approved" };
        input.push_str(&format!("Verdict: {}\n", verdict));
        input.push_str(&format!("Feedback:\n{}\n", review.feedback));
    }

    input
}

async fn run_summary_generation(
    phase: &str,
    input: &str,
    working_dir: &Path,
    config: &WorkflowConfig,
    sender: SessionEventSender,
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
        session_key: None,
        resume_strategy: ResumeStrategy::Stateless,
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
