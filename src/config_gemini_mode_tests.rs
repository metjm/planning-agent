//! Tests for Gemini mode transformation functionality

use super::*;
use crate::config::AggregationMode;

#[test]
fn test_gemini_mode_transformation_basic() {
    let yaml = r#"
agents:
  gemini:
    command: "gemini"
  claude:
    command: "claude"

gemini_mode:
  agents:
    gemini:
      command: "gemini"
      allowed_tools: []
  substitutions:
    claude: gemini

workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]

implementation:
  enabled: false
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.transform_to_gemini_only().unwrap();

    // Planning agent should be substituted
    assert_eq!(config.workflow.planning.agent, "gemini");
}

#[test]
fn test_gemini_mode_substitution_preserves_extended_agents() {
    let yaml = r#"
agents:
  gemini:
    command: "gemini"
  claude:
    command: "claude"

gemini_mode:
  substitutions:
    claude: gemini

workflow:
  planning:
    agent: gemini
  reviewing:
    agents:
      - agent: claude
        id: claude-security
        prompt: "Focus on security"

implementation:
  enabled: false
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.transform_to_gemini_only().unwrap();

    // Extended agent ref should have agent name substituted but preserve id and prompt
    match &config.workflow.reviewing.agents[0] {
        AgentRef::Extended(inst) => {
            assert_eq!(inst.agent, "gemini"); // substituted
            assert_eq!(inst.id, Some("claude-security".to_string())); // preserved
            assert_eq!(inst.prompt, Some("Focus on security".to_string())); // preserved
        }
        _ => panic!("Expected extended AgentRef"),
    }
}

#[test]
fn test_gemini_mode_reviewing_override() {
    let yaml = r#"
agents:
  gemini:
    command: "gemini"
  claude:
    command: "claude"

gemini_mode:
  substitutions:
    claude: gemini
  reviewing:
    agents:
      - gemini
    aggregation: all_reject

workflow:
  planning:
    agent: gemini
  reviewing:
    agents: [claude]
    aggregation: any_rejects

implementation:
  enabled: false
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.transform_to_gemini_only().unwrap();

    // Reviewing phase should be completely replaced, not merged
    assert_eq!(config.workflow.reviewing.agents.len(), 1);
    assert_eq!(
        config.workflow.reviewing.aggregation,
        AggregationMode::AllReject
    );
}

#[test]
fn test_gemini_mode_invalid_substitution_target() {
    let yaml = r#"
agents:
  gemini:
    command: "gemini"
  claude:
    command: "claude"

gemini_mode:
  substitutions:
    claude: nonexistent

workflow:
  planning:
    agent: gemini
  reviewing:
    agents: [gemini]

implementation:
  enabled: false
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    let result = config.transform_to_gemini_only();
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("substitution target 'nonexistent' not found"));
}

#[test]
fn test_empty_gemini_mode_is_noop() {
    let yaml = r#"
agents:
  gemini:
    command: "gemini"
  claude:
    command: "claude"

workflow:
  planning:
    agent: gemini
  reviewing:
    agents: [claude]

implementation:
  enabled: false
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    let original_planning_agent = config.workflow.planning.agent.clone();
    config.transform_to_gemini_only().unwrap();

    // Config should be unchanged (no substitutions defined)
    assert_eq!(config.workflow.planning.agent, original_planning_agent);
}

#[test]
fn test_gemini_only_config_uses_gemini_for_all_phases() {
    let config = WorkflowConfig::gemini_only_config();

    // Planning should use gemini
    assert_eq!(config.workflow.planning.agent, "gemini");

    // Reviewing should use gemini (from gemini_mode override)
    assert!(config
        .workflow
        .reviewing
        .agents
        .iter()
        .all(|a| a.agent_name() == "gemini"));

    // Implementation should use gemini for implementing
    assert_eq!(config.implementation.implementing_agent(), Some("gemini"));

    // Implementation review should use gemini-reviewer (different agent for validation)
    assert_eq!(
        config.implementation.reviewing_agent(),
        Some("gemini-reviewer")
    );
}

#[test]
fn test_gemini_only_config_has_gemini_reviewer_agent() {
    let config = WorkflowConfig::gemini_only_config();

    assert!(
        config.agents.contains_key("gemini-reviewer"),
        "gemini-reviewer agent should exist"
    );

    // gemini-reviewer should have the same command as gemini
    let gemini = config.agents.get("gemini").unwrap();
    let gemini_reviewer = config.agents.get("gemini-reviewer").unwrap();
    assert_eq!(gemini.command, gemini_reviewer.command);
}

#[test]
fn test_gemini_only_config_implementation_enabled() {
    let config = WorkflowConfig::gemini_only_config();
    assert!(
        config.implementation.enabled,
        "Implementation should be enabled for gemini-only"
    );
}

#[test]
fn test_gemini_only_config_validates_successfully() {
    // This should not panic - gemini_only_config() calls validate() internally
    let config = WorkflowConfig::gemini_only_config();
    // If we get here without panic, validation passed
    assert!(config.implementation.enabled);
}

#[test]
fn test_gemini_mode_implementation_override() {
    let yaml = r#"
agents:
  gemini:
    command: "gemini"
  claude:
    command: "claude"

gemini_mode:
  agents:
    gemini-reviewer:
      command: "gemini"
  substitutions:
    claude: gemini
  implementation:
    implementing:
      agent: gemini
      max_turns: 50
    reviewing:
      agent: gemini-reviewer

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
    agent: gemini
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.transform_to_gemini_only().unwrap();

    // Implementation should use overrides from gemini_mode
    assert_eq!(
        config.implementation.implementing.as_ref().unwrap().agent,
        "gemini"
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
        "gemini-reviewer"
    );
}

#[test]
fn test_gemini_mode_implementation_conflict_resolution() {
    let yaml = r#"
agents:
  gemini:
    command: "gemini"
  claude:
    command: "claude"

gemini_mode:
  agents:
    gemini-reviewer:
      command: "gemini"
  substitutions:
    claude: gemini

workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]

implementation:
  enabled: true
  implementing:
    agent: gemini
  reviewing:
    agent: claude
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.transform_to_gemini_only().unwrap();

    // Both implementing and reviewing would map to gemini after substitution
    // But since gemini-reviewer exists, reviewing should use it instead
    assert_eq!(
        config.implementation.implementing.as_ref().unwrap().agent,
        "gemini"
    );
    assert_eq!(
        config.implementation.reviewing.as_ref().unwrap().agent,
        "gemini-reviewer"
    );
}
