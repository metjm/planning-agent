//! Integration tests for AgentRef / multi-instance agent configuration

use super::*;

#[test]
fn test_agent_ref_simple_parsing() {
    let yaml = r#"
agents:
  claude:
    command: "claude"
  codex:
    command: "codex"

workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude, codex]
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.workflow.reviewing.agents.len(), 2);

    // Both should parse as Simple variants
    assert!(matches!(
        &config.workflow.reviewing.agents[0],
        AgentRef::Simple(name) if name == "claude"
    ));
    assert!(matches!(
        &config.workflow.reviewing.agents[1],
        AgentRef::Simple(name) if name == "codex"
    ));
}

#[test]
fn test_agent_ref_extended_parsing() {
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
        id: claude-security
        prompt: "Focus on security"
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.workflow.reviewing.agents.len(), 1);

    match &config.workflow.reviewing.agents[0] {
        AgentRef::Extended(inst) => {
            assert_eq!(inst.agent, "claude");
            assert_eq!(inst.id, Some("claude-security".to_string()));
            assert_eq!(inst.prompt, Some("Focus on security".to_string()));
        }
        AgentRef::Simple(_) => panic!("Expected Extended variant"),
    }
}

#[test]
fn test_agent_ref_mixed_parsing() {
    let yaml = r#"
agents:
  claude:
    command: "claude"
  codex:
    command: "codex"

workflow:
  planning:
    agent: claude
  reviewing:
    agents:
      - codex
      - agent: claude
        id: claude-arch
        prompt: "Focus on architecture"
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.workflow.reviewing.agents.len(), 2);

    // First should be Simple
    assert!(matches!(
        &config.workflow.reviewing.agents[0],
        AgentRef::Simple(name) if name == "codex"
    ));

    // Second should be Extended
    match &config.workflow.reviewing.agents[1] {
        AgentRef::Extended(inst) => {
            assert_eq!(inst.agent, "claude");
            assert_eq!(inst.id, Some("claude-arch".to_string()));
            assert_eq!(inst.prompt, Some("Focus on architecture".to_string()));
        }
        AgentRef::Simple(_) => panic!("Expected Extended variant"),
    }
}

#[test]
fn test_agent_ref_extended_optional_fields() {
    // Test that id and prompt are truly optional
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
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.workflow.reviewing.agents.len(), 1);

    match &config.workflow.reviewing.agents[0] {
        AgentRef::Extended(inst) => {
            assert_eq!(inst.agent, "claude");
            assert_eq!(inst.id, None);
            assert_eq!(inst.prompt, None);
        }
        AgentRef::Simple(_) => panic!("Expected Extended variant"),
    }
}

#[test]
fn test_agent_ref_methods() {
    let simple = AgentRef::Simple("claude".to_string());
    assert_eq!(simple.agent_name(), "claude");
    assert_eq!(simple.display_id(), "claude");
    assert_eq!(simple.custom_prompt(), None);

    let extended_with_id = AgentRef::Extended(AgentInstance {
        agent: "claude".to_string(),
        id: Some("claude-security".to_string()),
        prompt: Some("Focus on security".to_string()),
        skill: None,
    });
    assert_eq!(extended_with_id.agent_name(), "claude");
    assert_eq!(extended_with_id.display_id(), "claude-security");
    assert_eq!(extended_with_id.custom_prompt(), Some("Focus on security"));

    let extended_without_id = AgentRef::Extended(AgentInstance {
        agent: "claude".to_string(),
        id: None,
        prompt: Some("Focus on security".to_string()),
        skill: None,
    });
    assert_eq!(extended_without_id.agent_name(), "claude");
    assert_eq!(extended_without_id.display_id(), "claude"); // Falls back to agent name
    assert_eq!(
        extended_without_id.custom_prompt(),
        Some("Focus on security")
    );
}

#[test]
fn test_validation_extended_missing_agent() {
    let yaml = r#"
agents:
  claude:
    command: "claude"

workflow:
  planning:
    agent: claude
  reviewing:
    agents:
      - agent: nonexistent
        id: test-instance
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();

    // Validation should fail because referenced agent doesn't exist
    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("nonexistent"));
}

#[test]
fn test_multi_instance_same_agent_config() {
    // Test running multiple instances of the same agent with different prompts
    let yaml = r#"
agents:
  claude:
    command: "claude"
    args: ["-p"]
  codex:
    command: "codex"
    args: ["exec"]

workflow:
  planning:
    agent: claude
  reviewing:
    agents:
      - codex
      - agent: claude
        id: claude-security
        prompt: |
          Focus on security concerns:
          - Authentication and authorization
          - Input validation
      - agent: claude
        id: claude-architecture
        prompt: |
          Focus on architecture:
          - Code organization
          - Design patterns

# Implementation uses codex for implementing and claude for reviewing
implementation:
  enabled: true
  implementing:
    agent: codex
  reviewing:
    agent: claude
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.implementation.normalize(&config.workflow).unwrap();

    // Should have 3 reviewers
    assert_eq!(config.workflow.reviewing.agents.len(), 3);

    // First is simple codex reference
    match &config.workflow.reviewing.agents[0] {
        AgentRef::Simple(name) => assert_eq!(name, "codex"),
        _ => panic!("Expected Simple variant for codex"),
    }

    // Second is extended claude-security
    match &config.workflow.reviewing.agents[1] {
        AgentRef::Extended(inst) => {
            assert_eq!(inst.agent, "claude");
            assert_eq!(inst.id, Some("claude-security".to_string()));
            assert!(inst.prompt.as_ref().unwrap().contains("security"));
        }
        _ => panic!("Expected Extended variant for claude-security"),
    }

    // Third is extended claude-architecture
    match &config.workflow.reviewing.agents[2] {
        AgentRef::Extended(inst) => {
            assert_eq!(inst.agent, "claude");
            assert_eq!(inst.id, Some("claude-architecture".to_string()));
            assert!(inst.prompt.as_ref().unwrap().contains("architecture"));
        }
        _ => panic!("Expected Extended variant for claude-architecture"),
    }

    // Validation should pass (both claude instances reference valid agent)
    assert!(config.validate().is_ok());
}

#[test]
fn test_agent_ref_display_id_uniqueness() {
    // Verify that display_ids are properly extracted for tracking
    let yaml = r#"
agents:
  claude:
    command: "claude"

workflow:
  planning:
    agent: claude
  reviewing:
    agents:
      - claude
      - agent: claude
        id: claude-security
      - agent: claude
        id: claude-perf
      - agent: claude
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();

    let display_ids: Vec<&str> = config
        .workflow
        .reviewing
        .agents
        .iter()
        .map(|r| r.display_id())
        .collect();

    // Should have 4 display_ids
    assert_eq!(display_ids.len(), 4);
    assert_eq!(display_ids[0], "claude");
    assert_eq!(display_ids[1], "claude-security");
    assert_eq!(display_ids[2], "claude-perf");
    assert_eq!(display_ids[3], "claude"); // Note: duplicate is allowed but not recommended

    // All should reference the same base agent
    for agent_ref in &config.workflow.reviewing.agents {
        assert_eq!(agent_ref.agent_name(), "claude");
    }
}

#[test]
fn test_full_workflow_config_with_multi_instance() {
    // Test a complete realistic workflow configuration
    let yaml = r#"
agents:
  claude:
    command: "claude"
    args: ["-p", "--output-format", "stream-json"]
  codex:
    command: "codex"
    args: ["exec", "--json"]

workflow:
  planning:
    agent: claude
    max_turns: 50

  reviewing:
    agents:
      - codex
      - agent: claude
        id: claude-security
        prompt: "Review for security vulnerabilities"
      - agent: claude
        id: claude-correctness
        prompt: "Review for logical correctness"
    aggregation: any_rejects
    require_plan_feedback_tags: true

failure_policy:
  max_retries: 3
  on_all_reviewers_failed: save_state

# Implementation uses claude for implementing and codex for reviewing
implementation:
  enabled: true
  implementing:
    agent: claude
  reviewing:
    agent: codex
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.implementation.normalize(&config.workflow).unwrap();

    // Validate entire config
    assert!(config.validate().is_ok());

    // Check reviewing configuration
    assert_eq!(config.workflow.reviewing.agents.len(), 3);
    assert_eq!(
        config.workflow.reviewing.aggregation,
        AggregationMode::AnyRejects
    );
    assert!(config.workflow.reviewing.require_plan_feedback_tags);

    // Verify we can get agent configs for all reviewers
    for agent_ref in &config.workflow.reviewing.agents {
        let agent_config = config.get_agent(agent_ref.agent_name());
        assert!(
            agent_config.is_some(),
            "Agent config should exist for {}",
            agent_ref.agent_name()
        );
    }

    // Verify custom prompts are accessible
    let custom_prompts: Vec<Option<&str>> = config
        .workflow
        .reviewing
        .agents
        .iter()
        .map(|r| r.custom_prompt())
        .collect();

    assert_eq!(custom_prompts[0], None); // codex has no custom prompt
    assert_eq!(
        custom_prompts[1],
        Some("Review for security vulnerabilities")
    );
    assert_eq!(custom_prompts[2], Some("Review for logical correctness"));
}

#[test]
fn test_five_agent_config() {
    // Comprehensive test with 5 reviewers - mix of simple and extended formats
    let yaml = r#"
agents:
  claude:
    command: "claude"
    args: ["-p", "--output-format", "stream-json"]
  codex:
    command: "codex"
    args: ["exec", "--json"]
  gemini:
    command: "gemini"
    args: ["-p"]

workflow:
  planning:
    agent: claude
    max_turns: 50

  reviewing:
    agents:
      # Agent 1: Simple codex reference
      - codex
      # Agent 2: Simple gemini reference
      - gemini
      # Agent 3: Claude instance for security review
      - agent: claude
        id: claude-security
        prompt: |
          Focus your review on security concerns:
          - Authentication and authorization vulnerabilities
          - Input validation and sanitization
          - SQL injection, XSS, and other OWASP top 10
          - Secrets and credential handling
      # Agent 4: Claude instance for architecture review
      - agent: claude
        id: claude-architecture
        prompt: |
          Focus your review on architectural concerns:
          - Code organization and modularity
          - Design patterns and best practices
          - Performance implications
          - Maintainability and extensibility
      # Agent 5: Claude instance for correctness (no custom id, just prompt)
      - agent: claude
        prompt: "Review for logical correctness and edge cases"
    aggregation: majority
    require_plan_feedback_tags: true

failure_policy:
  max_retries: 2
  on_all_reviewers_failed: save_state

# Implementation uses claude for implementing and codex for reviewing
implementation:
  enabled: true
  implementing:
    agent: claude
  reviewing:
    agent: codex
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.implementation.normalize(&config.workflow).unwrap();

    // Validate entire config
    let validation_result = config.validate();
    assert!(
        validation_result.is_ok(),
        "Validation failed: {:?}",
        validation_result.err()
    );

    // Check we have exactly 5 reviewers
    assert_eq!(
        config.workflow.reviewing.agents.len(),
        5,
        "Expected 5 reviewers"
    );

    // Verify each agent's properties
    let agents = &config.workflow.reviewing.agents;

    // Agent 1: Simple codex
    assert!(matches!(&agents[0], AgentRef::Simple(name) if name == "codex"));
    assert_eq!(agents[0].agent_name(), "codex");
    assert_eq!(agents[0].display_id(), "codex");
    assert_eq!(agents[0].custom_prompt(), None);

    // Agent 2: Simple gemini
    assert!(matches!(&agents[1], AgentRef::Simple(name) if name == "gemini"));
    assert_eq!(agents[1].agent_name(), "gemini");
    assert_eq!(agents[1].display_id(), "gemini");
    assert_eq!(agents[1].custom_prompt(), None);

    // Agent 3: Extended claude-security
    match &agents[2] {
        AgentRef::Extended(inst) => {
            assert_eq!(inst.agent, "claude");
            assert_eq!(inst.id, Some("claude-security".to_string()));
            assert!(inst.prompt.as_ref().unwrap().contains("security"));
            assert!(inst.prompt.as_ref().unwrap().contains("OWASP"));
        }
        _ => panic!("Agent 3 should be Extended"),
    }
    assert_eq!(agents[2].agent_name(), "claude");
    assert_eq!(agents[2].display_id(), "claude-security");
    assert!(agents[2].custom_prompt().unwrap().contains("security"));

    // Agent 4: Extended claude-architecture
    match &agents[3] {
        AgentRef::Extended(inst) => {
            assert_eq!(inst.agent, "claude");
            assert_eq!(inst.id, Some("claude-architecture".to_string()));
            assert!(inst.prompt.as_ref().unwrap().contains("architectural"));
        }
        _ => panic!("Agent 4 should be Extended"),
    }
    assert_eq!(agents[3].agent_name(), "claude");
    assert_eq!(agents[3].display_id(), "claude-architecture");
    assert!(agents[3].custom_prompt().unwrap().contains("modularity"));

    // Agent 5: Extended claude with prompt but no custom id (falls back to agent name)
    match &agents[4] {
        AgentRef::Extended(inst) => {
            assert_eq!(inst.agent, "claude");
            assert_eq!(inst.id, None); // No custom id
            assert!(inst.prompt.as_ref().unwrap().contains("correctness"));
        }
        _ => panic!("Agent 5 should be Extended"),
    }
    assert_eq!(agents[4].agent_name(), "claude");
    assert_eq!(agents[4].display_id(), "claude"); // Falls back to agent name
    assert!(agents[4].custom_prompt().unwrap().contains("edge cases"));

    // Verify all agent configs exist
    for agent_ref in &config.workflow.reviewing.agents {
        let agent_config = config.get_agent(agent_ref.agent_name());
        assert!(
            agent_config.is_some(),
            "Agent config should exist for base agent '{}'",
            agent_ref.agent_name()
        );
    }

    // Verify display_ids
    let display_ids: Vec<&str> = agents.iter().map(|a| a.display_id()).collect();
    assert_eq!(
        display_ids,
        vec![
            "codex",
            "gemini",
            "claude-security",
            "claude-architecture",
            "claude"
        ]
    );

    // Verify agent_names (base agents)
    let agent_names: Vec<&str> = agents.iter().map(|a| a.agent_name()).collect();
    assert_eq!(
        agent_names,
        vec!["codex", "gemini", "claude", "claude", "claude"]
    );

    // Verify aggregation mode
    assert_eq!(
        config.workflow.reviewing.aggregation,
        AggregationMode::Majority
    );
}

// Implementation config tests

#[test]
fn test_config_backward_compatibility_without_implementation() {
    // Test that configs without implementation section parse correctly
    // and get defaults normalized from workflow section
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
    agent: codex
  reviewing:
    agents: [claude, codex]
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.implementation.normalize(&config.workflow).unwrap();

    // implementation should default to enabled
    assert!(config.implementation.enabled);
    assert_eq!(config.implementation.max_iterations, 3);

    // implementing should default to planning agent (codex) with max_turns: 100
    let implementing = config.implementation.implementing.as_ref().unwrap();
    assert_eq!(implementing.agent, "codex");
    assert_eq!(implementing.max_turns, Some(100));

    // reviewing should default to first distinct reviewer (claude)
    let reviewing = config.implementation.reviewing.as_ref().unwrap();
    assert_eq!(reviewing.agent, "claude");

    // Validation should pass
    assert!(config.validate().is_ok());
}

#[test]
fn test_implementation_config_explicit() {
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
    agent: codex
  reviewing:
    agents: [claude]

implementation:
  enabled: true
  max_iterations: 5
  implementing:
    agent: codex
    max_turns: 150
  reviewing:
    agent: claude
    max_turns: 50
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.implementation.normalize(&config.workflow).unwrap();

    assert!(config.implementation.enabled);
    assert_eq!(config.implementation.max_iterations, 5);

    let implementing = config.implementation.implementing.as_ref().unwrap();
    assert_eq!(implementing.agent, "codex");
    assert_eq!(implementing.max_turns, Some(150));

    let reviewing = config.implementation.reviewing.as_ref().unwrap();
    assert_eq!(reviewing.agent, "claude");
    assert_eq!(reviewing.max_turns, Some(50));

    assert!(config.validate().is_ok());
}

#[test]
fn test_implementation_config_disabled() {
    let yaml = r#"
agents:
  claude:
    command: "claude"

workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]

implementation:
  enabled: false
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.implementation.normalize(&config.workflow).unwrap();

    assert!(!config.implementation.enabled);
    // Validation should pass even without distinct reviewer when disabled
    assert!(config.validate().is_ok());
}

#[test]
fn test_implementation_validation_same_agent_fails() {
    let yaml = r#"
agents:
  claude:
    command: "claude"

workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]

implementation:
  enabled: true
  implementing:
    agent: claude
  reviewing:
    agent: claude
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.implementation.normalize(&config.workflow).unwrap();

    // Validation should fail because implementing and reviewing are the same
    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("different agent"));
}

#[test]
fn test_implementation_validation_missing_agent() {
    let yaml = r#"
agents:
  claude:
    command: "claude"

workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]

implementation:
  enabled: true
  implementing:
    agent: nonexistent
  reviewing:
    agent: claude
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();

    // Validation should fail because implementing agent doesn't exist
    let result = config.validate();
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("implementing agent"));
}

#[test]
fn test_implementation_single_agent_no_distinct_reviewer() {
    // Single agent config without explicit implementation section
    // should fail if enabled because no distinct reviewer can be found
    let yaml = r#"
agents:
  claude:
    command: "claude"

workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]
"#;
    let mut config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    config.implementation.normalize(&config.workflow).unwrap();

    // enabled is true by default, but no distinct reviewer
    assert!(config.implementation.enabled);
    // reviewing should not be set because only claude exists
    assert!(config.implementation.reviewing.is_none());

    // Validation should fail
    let result = config.validate();
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("no distinct reviewing agent"));
}

#[test]
fn test_agent_instance_with_skill_field() {
    let yaml = r#"
        agent: claude
        id: adversarial
        skill: plan-review-adversarial
    "#;
    let instance: AgentInstance = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(instance.agent, "claude");
    assert_eq!(instance.id, Some("adversarial".to_string()));
    assert_eq!(instance.skill, Some("plan-review-adversarial".to_string()));
    assert!(instance.prompt.is_none());
}

#[test]
fn test_agent_instance_with_skill_and_prompt() {
    let yaml = r#"
        agent: claude
        id: reviewer
        skill: plan-review-operational
        prompt: "Additional focus on security"
    "#;
    let instance: AgentInstance = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(instance.skill, Some("plan-review-operational".to_string()));
    assert_eq!(
        instance.prompt,
        Some("Additional focus on security".to_string())
    );
}

#[test]
fn test_agent_instance_without_skill_field() {
    let yaml = r#"
        agent: claude
        id: default-reviewer
        prompt: "Focus on completeness"
    "#;
    let instance: AgentInstance = serde_yaml::from_str(yaml).unwrap();
    assert!(instance.skill.is_none());
    assert!(instance.prompt.is_some());
}

#[test]
fn test_agent_ref_skill_simple_returns_none() {
    let agent_ref = AgentRef::Simple("claude".to_string());
    assert!(agent_ref.skill().is_none());
}

#[test]
fn test_agent_ref_skill_extended_with_skill() {
    let agent_ref = AgentRef::Extended(AgentInstance {
        agent: "claude".to_string(),
        id: Some("adversarial".to_string()),
        prompt: None,
        skill: Some("plan-review-adversarial".to_string()),
    });
    assert_eq!(agent_ref.skill(), Some("plan-review-adversarial"));
}

#[test]
fn test_agent_ref_skill_extended_without_skill() {
    let agent_ref = AgentRef::Extended(AgentInstance {
        agent: "claude".to_string(),
        id: Some("default".to_string()),
        prompt: Some("Some prompt".to_string()),
        skill: None,
    });
    assert!(agent_ref.skill().is_none());
}
