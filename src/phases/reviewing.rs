use crate::claude::ClaudeInvocation;
use crate::state::State;
use crate::tui::Event;
use anyhow::Result;
use std::path::Path;
use tokio::sync::mpsc;

const ALLOWED_TOOLS: &[&str] = &[
    "Read", "Glob", "Grep", "Write", "WebSearch", "WebFetch", "Skill", "Task",
];

pub async fn run_review_phase(
    state: &State,
    working_dir: &Path,
    output_tx: mpsc::UnboundedSender<Event>,
) -> Result<()> {
    let prompt = format!(
        r#"Use the Skill tool to invoke the plan-review skill:
Skill(skill: "plan-review", args: "{}")

Write the feedback to: {}

IMPORTANT: Your feedback MUST include one of these exact strings in the output:
- "Overall Assessment:** APPROVED" - if the plan is ready for implementation
- "Overall Assessment:** NEEDS REVISION" - if the plan has issues that need to be fixed

The orchestrator will parse the feedback file to determine the next phase."#,
        state.plan_file.display(),
        state.feedback_file.display()
    );

    let system_prompt = r#"You are orchestrating a plan review workflow.
Your task is to invoke the plan-review skill to review an implementation plan.
The review must result in a clear APPROVED or NEEDS REVISION assessment.
Do not ask questions - proceed with the skill invocation immediately."#;

    let result = ClaudeInvocation::new(prompt)
        .with_system_prompt(system_prompt)
        .with_allowed_tools(ALLOWED_TOOLS.iter().map(|s| s.to_string()).collect())
        .with_working_dir(working_dir.to_path_buf())
        .execute_streaming(output_tx.clone())
        .await?;

    let _ = output_tx.send(Event::Output("[planning-agent] Review phase complete".to_string()));
    let _ = output_tx.send(Event::Output(format!(
        "[planning-agent] Result preview: {}...",
        result.result.chars().take(200).collect::<String>()
    )));

    Ok(())
}
