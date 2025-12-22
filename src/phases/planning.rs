use crate::agents::{AgentContext, AgentType};
use crate::config::WorkflowConfig;
use crate::state::State;
use crate::tui::SessionEventSender;
use anyhow::Result;
use std::path::Path;

const PLANNING_SYSTEM_PROMPT: &str = r#"You are a technical planning agent.
Create a detailed implementation plan for the given objective.
Use the available tools to read the codebase and understand the existing structure.
Write your plan to the specified file path."#;

/// Run planning phase with a configured agent
pub async fn run_planning_phase_with_context(
    state: &State,
    working_dir: &Path,
    config: &WorkflowConfig,
    session_sender: SessionEventSender,
) -> Result<()> {
    let planning_config = &config.workflow.planning;
    let agent_name = &planning_config.agent;
    let max_turns = planning_config.max_turns;

    let agent_config = config
        .get_agent(agent_name)
        .ok_or_else(|| anyhow::anyhow!("Planning agent '{}' not found in config", agent_name))?;

    session_sender.send_output(format!("[planning] Using agent: {}", agent_name));

    let agent = AgentType::from_config(agent_name, agent_config, working_dir.to_path_buf())?;

    let prompt = build_planning_prompt(state);

    // Create agent context for chat message routing
    let context = AgentContext {
        session_sender: session_sender.clone(),
        phase: "Planning".to_string(),
    };

    let result = agent
        .execute_streaming_with_context(
            prompt,
            Some(PLANNING_SYSTEM_PROMPT.to_string()),
            max_turns,
            context,
        )
        .await?;

    session_sender.send_output(format!("[planning:{}] Planning phase complete", agent_name));
    session_sender.send_output(format!(
        "[planning:{}] Result preview: {}...",
        agent_name,
        result.output.chars().take(200).collect::<String>()
    ));

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
