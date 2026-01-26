//! Tests for Codex mode transformation functionality

use super::*;
use crate::config::AggregationMode;

#[test]
fn test_codex_mode_transformation_basic() {
    let yaml = r#"
agents:
  codex:
    command: "codex"
  claude:
    command: "claude"

codex_mode:
  agents:
    codex:
      command: "codex"
      allowed_tools: []
  substitutions:
    claude: codex

workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]

implementation:
  enabled: false
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.transform_to_codex_only().unwrap();

    // Planning agent should be substituted
    assert_eq!(config.workflow.planning.agent, "codex");
}

#[test]
fn test_codex_mode_substitution_preserves_extended_agents() {
    let yaml = r#"
agents:
  codex:
    command: "codex"
  claude:
    command: "claude"

codex_mode:
  substitutions:
    claude: codex

workflow:
  planning:
    agent: codex
  reviewing:
    agents:
      - agent: claude
        id: claude-security
        prompt: "Focus on security"

implementation:
  enabled: false
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.transform_to_codex_only().unwrap();

    // Extended agent ref should have agent name substituted but preserve id and prompt
    match &config.workflow.reviewing.agents[0] {
        AgentRef::Extended(inst) => {
            assert_eq!(inst.agent, "codex"); // substituted
            assert_eq!(inst.id, Some("claude-security".to_string())); // preserved
            assert_eq!(inst.prompt, Some("Focus on security".to_string())); // preserved
        }
        _ => panic!("Expected extended AgentRef"),
    }
}

#[test]
fn test_codex_mode_reviewing_override() {
    let yaml = r#"
agents:
  codex:
    command: "codex"
  claude:
    command: "claude"

codex_mode:
  substitutions:
    claude: codex
  reviewing:
    agents:
      - codex
    aggregation: all_reject

workflow:
  planning:
    agent: codex
  reviewing:
    agents: [claude]
    aggregation: any_rejects

implementation:
  enabled: false
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.transform_to_codex_only().unwrap();

    // Reviewing phase should be completely replaced, not merged
    assert_eq!(config.workflow.reviewing.agents.len(), 1);
    assert_eq!(
        config.workflow.reviewing.aggregation,
        AggregationMode::AllReject
    );
}

#[test]
fn test_codex_mode_invalid_substitution_target() {
    let yaml = r#"
agents:
  codex:
    command: "codex"
  claude:
    command: "claude"

codex_mode:
  substitutions:
    claude: nonexistent

workflow:
  planning:
    agent: codex
  reviewing:
    agents: [codex]

implementation:
  enabled: false
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    let result = config.transform_to_codex_only();
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("substitution target 'nonexistent' not found"));
}

#[test]
fn test_empty_codex_mode_is_noop() {
    let yaml = r#"
agents:
  codex:
    command: "codex"
  claude:
    command: "claude"

workflow:
  planning:
    agent: codex
  reviewing:
    agents: [claude]

implementation:
  enabled: false
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    let original_planning_agent = config.workflow.planning.agent.clone();
    config.transform_to_codex_only().unwrap();

    // Config should be unchanged (no substitutions defined)
    assert_eq!(config.workflow.planning.agent, original_planning_agent);
}

#[test]
fn test_codex_only_config_uses_codex_for_all_phases() {
    let config = WorkflowConfig::codex_only_config();

    // Planning should use codex
    assert_eq!(config.workflow.planning.agent, "codex");

    // Reviewing should use codex (from codex_mode override)
    assert!(config
        .workflow
        .reviewing
        .agents
        .iter()
        .all(|a| a.agent_name() == "codex"));

    // Implementation should use codex for implementing
    assert_eq!(config.implementation.implementing_agent(), Some("codex"));

    // Implementation review should use codex-reviewer (different agent for validation)
    assert_eq!(
        config.implementation.reviewing_agent(),
        Some("codex-reviewer")
    );
}

#[test]
fn test_codex_only_config_has_codex_reviewer_agent() {
    let config = WorkflowConfig::codex_only_config();

    assert!(
        config.agents.contains_key("codex-reviewer"),
        "codex-reviewer agent should exist"
    );

    // codex-reviewer should have the same command as codex
    let codex = config.agents.get("codex").unwrap();
    let codex_reviewer = config.agents.get("codex-reviewer").unwrap();
    assert_eq!(codex.command, codex_reviewer.command);
}

#[test]
fn test_codex_only_config_implementation_enabled() {
    let config = WorkflowConfig::codex_only_config();
    assert!(
        config.implementation.enabled,
        "Implementation should be enabled for codex-only"
    );
}

#[test]
fn test_codex_only_config_validates_successfully() {
    // This should not panic - codex_only_config() calls validate() internally
    let config = WorkflowConfig::codex_only_config();
    // If we get here without panic, validation passed
    assert!(config.implementation.enabled);
}

#[test]
fn test_codex_mode_implementation_override() {
    let yaml = r#"
agents:
  codex:
    command: "codex"
  claude:
    command: "claude"

codex_mode:
  agents:
    codex-reviewer:
      command: "codex"
  substitutions:
    claude: codex
  implementation:
    implementing:
      agent: codex
      max_turns: 50
    reviewing:
      agent: codex-reviewer

workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]

implementation:
  enabled: true
  implementing:
    agent: claude
    max_turns: 100
  reviewing:
    agent: codex
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.transform_to_codex_only().unwrap();

    // Implementation should use overrides from codex_mode
    assert_eq!(
        config.implementation.implementing.as_ref().unwrap().agent,
        "codex"
    );
    assert_eq!(
        config
            .implementation
            .implementing
            .as_ref()
            .unwrap()
            .max_turns,
        Some(50)
    );
    assert_eq!(
        config.implementation.reviewing.as_ref().unwrap().agent,
        "codex-reviewer"
    );
}

#[test]
fn test_codex_mode_implementation_conflict_resolution() {
    let yaml = r#"
agents:
  codex:
    command: "codex"
  claude:
    command: "claude"

codex_mode:
  agents:
    codex-reviewer:
      command: "codex"
  substitutions:
    claude: codex

workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]

implementation:
  enabled: true
  implementing:
    agent: codex
  reviewing:
    agent: claude
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.transform_to_codex_only().unwrap();

    // Both implementing and reviewing would map to codex after substitution
    // But since codex-reviewer exists, reviewing should use it instead
    assert_eq!(
        config.implementation.implementing.as_ref().unwrap().agent,
        "codex"
    );
    assert_eq!(
        config.implementation.reviewing.as_ref().unwrap().agent,
        "codex-reviewer"
    );
}
