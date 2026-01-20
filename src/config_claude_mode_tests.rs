//! Tests for Claude mode transformation functionality

use super::*;

#[test]
fn test_claude_mode_transformation_basic() {
    let yaml = r#"
agents:
  codex:
    command: "codex"
  claude:
    command: "claude"

claude_mode:
  agents:
    claude:
      command: "claude"
      allowed_tools: ["Read", "Write"]
  substitutions:
    codex: claude

workflow:
  planning:
    agent: codex
  reviewing:
    agents: [claude]

implementation:
  enabled: false
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.transform_to_claude_only().unwrap();

    // Planning agent should be substituted
    assert_eq!(config.workflow.planning.agent, "claude");
    // Claude agent should have updated allowed_tools from claude_mode
    assert!(config.agents.get("claude").unwrap().allowed_tools.contains(&"Write".to_string()));
}

#[test]
fn test_claude_mode_substitution_preserves_extended_agents() {
    let yaml = r#"
agents:
  codex:
    command: "codex"
  claude:
    command: "claude"

claude_mode:
  substitutions:
    codex: claude

workflow:
  planning:
    agent: codex
  reviewing:
    agents:
      - agent: codex
        id: codex-security
        prompt: "Focus on security"

implementation:
  enabled: false
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.transform_to_claude_only().unwrap();

    // Extended agent ref should have agent name substituted but preserve id and prompt
    match &config.workflow.reviewing.agents[0] {
        AgentRef::Extended(inst) => {
            assert_eq!(inst.agent, "claude"); // substituted
            assert_eq!(inst.id, Some("codex-security".to_string())); // preserved
            assert_eq!(inst.prompt, Some("Focus on security".to_string())); // preserved
        }
        _ => panic!("Expected extended AgentRef"),
    }
}

#[test]
fn test_claude_mode_reviewing_override() {
    let yaml = r#"
agents:
  codex:
    command: "codex"
  claude:
    command: "claude"

claude_mode:
  substitutions:
    codex: claude
  reviewing:
    agents:
      - claude
      - agent: claude
        id: custom-reviewer
        prompt: "Custom review"
    aggregation: all_reject

workflow:
  planning:
    agent: codex
  reviewing:
    agents: [codex]
    aggregation: any_rejects

implementation:
  enabled: false
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.transform_to_claude_only().unwrap();

    // Reviewing phase should be completely replaced, not merged
    assert_eq!(config.workflow.reviewing.agents.len(), 2);
    assert_eq!(config.workflow.reviewing.aggregation, AggregationMode::AllReject);
    match &config.workflow.reviewing.agents[1] {
        AgentRef::Extended(inst) => {
            assert_eq!(inst.id, Some("custom-reviewer".to_string()));
        }
        _ => panic!("Expected extended AgentRef"),
    }
}

#[test]
fn test_claude_mode_invalid_substitution_target() {
    let yaml = r#"
agents:
  codex:
    command: "codex"
  claude:
    command: "claude"

claude_mode:
  substitutions:
    codex: nonexistent

workflow:
  planning:
    agent: codex
  reviewing:
    agents: [claude]

implementation:
  enabled: false
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    let result = config.transform_to_claude_only();
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("substitution target 'nonexistent' not found"));
}

#[test]
fn test_empty_claude_mode_is_noop() {
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
    config.transform_to_claude_only().unwrap();

    // Config should be unchanged (no substitutions defined)
    assert_eq!(config.workflow.planning.agent, original_planning_agent);
}

#[test]
fn test_claude_practices_reviewer_preserved() {
    // Test that the full workflow.yaml transformation preserves claude-practices
    let config = WorkflowConfig::claude_only_config();

    // Find the claude-practices reviewer
    let practices_reviewer = config.workflow.reviewing.agents.iter().find(|r| {
        matches!(r, AgentRef::Extended(inst) if inst.id == Some("claude-practices".to_string()))
    });

    assert!(practices_reviewer.is_some(), "claude-practices reviewer should be present");
    if let Some(AgentRef::Extended(inst)) = practices_reviewer {
        assert!(inst.prompt.as_ref().unwrap().contains("repository practices"));
    }
}

#[test]
fn test_implementation_reviewer_conflict_resolution() {
    let yaml = r#"
agents:
  codex:
    command: "codex"
  claude:
    command: "claude"

claude_mode:
  agents:
    claude-reviewer:
      command: "claude"
  substitutions:
    codex: claude

workflow:
  planning:
    agent: codex
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
    config.transform_to_claude_only().unwrap();

    // Both implementing and reviewing would map to claude after substitution
    // But since claude-reviewer exists, reviewing should use it instead
    assert_eq!(config.implementation.implementing.as_ref().unwrap().agent, "claude");
    assert_eq!(config.implementation.reviewing.as_ref().unwrap().agent, "claude-reviewer");
}
