use crate::agents::AgentType;
use crate::claude::ClaudeInvocation;
use crate::config::WorkflowConfig;
use crate::state::State;
use crate::tui::Event;
use anyhow::Result;
use std::path::Path;
use tokio::sync::mpsc;

const ALLOWED_TOOLS: &[&str] = &[
    "Read", "Glob", "Grep", "Write", "WebSearch", "WebFetch", "Skill", "Task",
];

/// Run planning phase with Claude (legacy behavior)
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

const PLANNING_SYSTEM_PROMPT: &str = r#"You are a technical planning agent.
Create a detailed implementation plan for the given objective.
Use the available tools to read the codebase and understand the existing structure.
Write your plan to the specified file path."#;

/// Run planning phase with a configured agent
pub async fn run_planning_phase_with_config(
    state: &State,
    working_dir: &Path,
    config: &WorkflowConfig,
    output_tx: mpsc::UnboundedSender<Event>,
) -> Result<()> {
    let planning_config = &config.workflow.planning;
    let agent_name = &planning_config.agent;
    let max_turns = planning_config.max_turns;

    let agent_config = config
        .get_agent(agent_name)
        .ok_or_else(|| anyhow::anyhow!("Planning agent '{}' not found in config", agent_name))?;

    let _ = output_tx.send(Event::Output(format!(
        "[planning] Using agent: {}",
        agent_name
    )));

    let agent = AgentType::from_config(agent_name, agent_config, working_dir.to_path_buf())?;

    let prompt = build_planning_prompt(state);

    let result = agent
        .execute_streaming(
            prompt,
            Some(PLANNING_SYSTEM_PROMPT.to_string()),
            max_turns,
            output_tx.clone(),
        )
        .await?;

    let _ = output_tx.send(Event::Output(format!(
        "[planning:{}] Planning phase complete",
        agent_name
    )));
    let _ = output_tx.send(Event::Output(format!(
        "[planning:{}] Result preview: {}...",
        agent_name,
        result.output.chars().take(200).collect::<String>()
    )));

    Ok(())
}

/// Build the planning prompt for configurable agents
fn build_planning_prompt(state: &State) -> String {
    format!(
        r#"Create a detailed implementation plan for the following:

Feature Name: {}
Objective: {}

Requirements:
1. Analyze the existing codebase to understand the current architecture
2. Identify all files that need to be modified or created
3. Break down the implementation into clear, actionable steps
4. Consider edge cases and potential issues
5. Include a testing strategy

Write your plan to: {}

Use the Read, Glob, and Grep tools to explore the codebase as needed."#,
        state.feature_name,
        state.objective,
        state.plan_file.display()
    )
}
