use crate::claude::ClaudeInvocation;
use crate::state::State;
use crate::tui::Event;
use anyhow::Result;
use std::path::Path;
use tokio::sync::mpsc;

const ALLOWED_TOOLS: &[&str] = &[
    "Read", "Glob", "Grep", "Edit", "Write", "WebSearch", "WebFetch",
];

pub async fn run_revision_phase(
    state: &State,
    working_dir: &Path,
    output_tx: mpsc::UnboundedSender<Event>,
) -> Result<()> {
    let prompt = format!(
        r#"Read the feedback at: {}
Read the current plan at: {}

Revise the plan to address:
1. All "Must Fix" items (blocking issues) - these MUST be addressed
2. All "Should Fix" items (important improvements) - address these if possible
3. Any critical issues mentioned in the feedback

Update the plan file at {} with your revisions.
Preserve the good parts of the existing plan - only modify what needs to change.

When done, confirm that the plan has been updated."#,
        state.feedback_file.display(),
        state.plan_file.display(),
        state.plan_file.display()
    );

    let system_prompt = r#"You are revising an implementation plan based on reviewer feedback.
Focus on addressing all blocking issues first, then important improvements.
Do not ask questions - proceed with reading the feedback and making revisions.
Preserve the structure and good parts of the existing plan."#;

    let result = ClaudeInvocation::new(prompt)
        .with_system_prompt(system_prompt)
        .with_allowed_tools(ALLOWED_TOOLS.iter().map(|s| s.to_string()).collect())
        .with_working_dir(working_dir.to_path_buf())
        .execute_streaming(output_tx.clone())
        .await?;

    let _ = output_tx.send(Event::Output("[planning-agent] Revision phase complete".to_string()));
    let _ = output_tx.send(Event::Output(format!(
        "[planning-agent] Result preview: {}...",
        result.result.chars().take(200).collect::<String>()
    )));

    Ok(())
}
