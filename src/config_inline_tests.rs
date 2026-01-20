//! Inline tests for config module (extracted to comply with line limit)

use super::*;

#[test]
fn test_default_config() {
    let config = WorkflowConfig::default_config();

    assert!(config.agents.contains_key("claude"));
    assert!(config.agents.contains_key("codex"));
    assert!(config.agents.contains_key("gemini"));

    assert!(config.agents.contains_key(&config.workflow.planning.agent));
    for agent_ref in &config.workflow.reviewing.agents {
        assert!(config.agents.contains_key(agent_ref.agent_name()));
    }
}

#[test]
fn test_default_config_validates() {
    let config = WorkflowConfig::default_config();
    assert!(config.validate().is_ok());
}

#[test]
fn test_claude_only_config() {
    let config = WorkflowConfig::claude_only_config();

    // Claude agents should be present (claude-reviewer added from claude_mode.agents)
    assert!(config.agents.contains_key("claude"));
    assert!(config.agents.contains_key("claude-reviewer"));
    // Original agents are still in the map, just not used in workflow phases
    assert!(config.agents.contains_key("codex"));
    assert!(config.agents.contains_key("gemini"));

    // Planning phase should use claude (substituted from codex)
    assert_eq!(config.workflow.planning.agent, "claude");

    // Reviewing phase should use the claude_mode.reviewing override
    // with claude, claude-practices, and claude-completeness reviewers
    assert_eq!(config.workflow.reviewing.agents.len(), 3);
    assert_eq!(config.workflow.reviewing.agents[0], AgentRef::Simple("claude".to_string()));
    // Verify extended reviewers have unique IDs
    let ids: Vec<_> = config.workflow.reviewing.agents.iter()
        .map(|a| a.display_id()).collect();
    assert_eq!(ids, vec!["claude", "claude-practices", "claude-completeness"]);

    // Implementation should be enabled with distinct reviewer
    assert!(config.implementation.enabled);
    assert_eq!(config.implementation.implementing_agent(), Some("claude"));
    assert_eq!(
        config.implementation.reviewing_agent(),
        Some("claude-reviewer")
    );

    // Should validate successfully
    assert!(config.validate().is_ok());
}

#[test]
fn test_yaml_parsing() {
    let yaml = r#"
agents:
  claude:
    command: "claude"
    args: ["-p", "--output-format", "stream-json"]
    allowed_tools: ["Read", "Write"]

workflow:
  planning:
    agent: claude
    max_turns: 50

  reviewing:
    agents: [claude]
    aggregation: any_rejects
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.workflow.planning.agent, "claude");
    assert_eq!(config.workflow.planning.max_turns, Some(50));
    assert_eq!(
        config.workflow.reviewing.aggregation,
        AggregationMode::AnyRejects
    );
}

#[test]
fn test_multi_agent_yaml_parsing() {
    let yaml = r#"
agents:
  claude:
    command: "claude"
    args: ["-p"]
  codex:
    command: "codex"
    args: ["exec", "--json"]

workflow:
  planning:
    agent: claude

  reviewing:
    agents: [claude, codex]
    aggregation: any_rejects
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.workflow.reviewing.agents.len(), 2);
    assert!(config
        .workflow
        .reviewing
        .agents
        .contains(&AgentRef::Simple("claude".to_string())));
    assert!(config
        .workflow
        .reviewing
        .agents
        .contains(&AgentRef::Simple("codex".to_string())));
}

#[test]
fn test_validation_missing_agent() {
    let yaml = r#"
agents:
  claude:
    command: "claude"

workflow:
  planning:
    agent: nonexistent

  reviewing:
    agents: [claude]
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(config.validate().is_err());
}

#[test]
fn test_validation_missing_review_agent() {
    let yaml = r#"
agents:
  claude:
    command: "claude"

workflow:
  planning:
    agent: claude

  reviewing:
    agents: [claude, nonexistent]
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(config.validate().is_err());
}

#[test]
fn test_aggregation_modes() {
    let yaml_any = r#"
agents:
  claude:
    command: "claude"
workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]
    aggregation: any_rejects
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml_any).unwrap();
    assert_eq!(
        config.workflow.reviewing.aggregation,
        AggregationMode::AnyRejects
    );

    let yaml_all = r#"
agents:
  claude:
    command: "claude"
workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]
    aggregation: all_reject
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml_all).unwrap();
    assert_eq!(
        config.workflow.reviewing.aggregation,
        AggregationMode::AllReject
    );

    let yaml_majority = r#"
agents:
  claude:
    command: "claude"
workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]
    aggregation: majority
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml_majority).unwrap();
    assert_eq!(
        config.workflow.reviewing.aggregation,
        AggregationMode::Majority
    );
}

#[test]
fn test_config_backward_compatibility_without_session_persistence() {
    // Test that configs without session_persistence field parse correctly
    let yaml = r#"
agents:
  claude:
    command: "claude"
    args: ["-p"]

workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    let claude_config = config.get_agent("claude").unwrap();

    // session_persistence should default to enabled with ConversationResume strategy
    // This enables session continuity for planning/revision phases by default
    assert!(claude_config.session_persistence.enabled);
    assert_eq!(
        claude_config.session_persistence.strategy,
        crate::state::ResumeStrategy::ConversationResume
    );
}

#[test]
fn test_config_backward_compatibility_without_verification() {
    // Test that configs without verification section parse correctly
    let yaml = r#"
agents:
  claude:
    command: "claude"
    args: ["-p"]

workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]

# Single-agent config needs implementation disabled
implementation:
  enabled: false
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();

    // verification should default to disabled
    assert!(!config.verification.enabled);
    assert_eq!(config.verification.max_iterations, 3);
    assert!(config.verification.verifying.is_none());
    assert!(config.verification.fixing.is_none());

    // Validation should pass
    assert!(config.validate().is_ok());
}

#[test]
fn test_verification_config_parsing() {
    let yaml = r#"
agents:
  claude:
    command: "claude"
    args: ["-p"]

workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]

verification:
  enabled: true
  max_iterations: 5
  verifying:
    agent: claude
    max_turns: 10
  fixing:
    agent: claude
    max_turns: 15

# Single-agent config needs implementation disabled
implementation:
  enabled: false
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();

    assert!(config.verification.enabled);
    assert_eq!(config.verification.max_iterations, 5);

    let verifying = config.verification.verifying.as_ref().unwrap();
    assert_eq!(verifying.agent, "claude");
    assert_eq!(verifying.max_turns, Some(10));

    let fixing = config.verification.fixing.as_ref().unwrap();
    assert_eq!(fixing.agent, "claude");
    assert_eq!(fixing.max_turns, Some(15));

    // Validation should pass
    assert!(config.validate().is_ok());
}

#[test]
fn test_verification_validation_missing_agent() {
    let yaml = r#"
agents:
  claude:
    command: "claude"

workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]

verification:
  enabled: true
  verifying:
    agent: nonexistent
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();

    // Validation should fail because verifying agent doesn't exist
    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Verifying agent"));
}

#[test]
fn test_verification_validation_disabled_skips_agent_check() {
    // When verification is disabled, missing agents should not cause validation errors
    let yaml = r#"
agents:
  claude:
    command: "claude"

workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]

verification:
  enabled: false
  verifying:
    agent: nonexistent

# Single-agent config needs implementation disabled
implementation:
  enabled: false
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();

    // Validation should pass because verification is disabled
    assert!(config.validate().is_ok());
}

#[test]
fn test_config_with_revising_field_fails_to_parse() {
    // Configs that include the old `revising` field should fail to parse
    // This is a clean break - no backward compatibility for this removed field
    let yaml = r#"
agents:
  claude:
    command: "claude"

workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]
  revising:
    agent: claude
"#;
    let result: Result<WorkflowConfig, _> = serde_yaml::from_str(yaml);
    assert!(result.is_err(), "Config with revising field should fail to parse");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("revising") || err.contains("unknown field"),
        "Error should mention 'revising' or 'unknown field': {}", err);
}

#[test]
fn test_validation_duplicate_reviewer_display_id() {
    // Duplicate reviewer display IDs should fail validation with clear error
    let yaml = r#"
agents:
  claude:
    command: "claude"

workflow:
  planning:
    agent: claude
  reviewing:
    agents:
      - agent: claude
        id: custom-reviewer
        prompt: "Review 1"
      - agent: claude
        id: custom-reviewer
        prompt: "Review 2"

implementation:
  enabled: false
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    let result = config.validate();
    assert!(result.is_err(), "Config with duplicate reviewer IDs should fail validation");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Duplicate reviewer display ID 'custom-reviewer'"),
        "Error should mention the duplicate ID: {}", err);
    assert!(err.contains("feedback files are named"),
        "Error should explain why duplicates are problematic: {}", err);
}
