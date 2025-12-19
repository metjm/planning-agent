use crate::claude::ClaudeInvocation;
use crate::state::State;
use crate::tui::Event;
use anyhow::Result;
use std::path::Path;
use tokio::sync::mpsc;

const ALLOWED_TOOLS: &[&str] = &[
    "Read", "Glob", "Grep", "Write", "WebSearch", "WebFetch", "Skill", "Task",
];

pub async fn run_planning_phase(
    state: &State,
    working_dir: &Path,
    output_tx: mpsc::UnboundedSender<Event>,
) -> Result<()> {
    let prompt = format!(
        r#"Use the Skill tool to invoke the planning skill:
Skill(skill: "planning", args: "{} {}")

The plan should be written to: {}

After the skill completes, verify that the plan file was created."#,
        state.feature_name,
        state.objective,
        state.plan_file.display()
    );

    let system_prompt = r#"You are orchestrating a planning workflow.
Your task is to invoke the planning skill to create an implementation plan.
Do not ask questions - proceed with the skill invocation immediately.
After the plan is created, confirm completion."#;

    let result = ClaudeInvocation::new(prompt)
        .with_system_prompt(system_prompt)
        .with_allowed_tools(ALLOWED_TOOLS.iter().map(|s| s.to_string()).collect())
        .with_working_dir(working_dir.to_path_buf())
        .execute_streaming(output_tx.clone())
        .await?;

    let _ = output_tx.send(Event::Output("[planning-agent] Planning phase complete".to_string()));
    let _ = output_tx.send(Event::Output(format!(
        "[planning-agent] Result preview: {}...",
        result.result.chars().take(200).collect::<String>()
    )));

    Ok(())
}
