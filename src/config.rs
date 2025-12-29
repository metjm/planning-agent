use crate::state::ResumeStrategy;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkflowConfig {
    pub agents: HashMap<String, AgentConfig>,
    pub workflow: PhaseConfigs,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct SessionPersistenceConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub strategy: ResumeStrategy,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub session_persistence: SessionPersistenceConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PhaseConfigs {
    pub planning: SingleAgentPhase,
    pub reviewing: MultiAgentPhase,
    pub revising: SingleAgentPhase,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SingleAgentPhase {
    pub agent: String,
    #[serde(default)]
    pub max_turns: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MultiAgentPhase {
    pub agents: Vec<String>,
    #[serde(default)]
    pub aggregation: AggregationMode,
    /// If true, reviews without `<plan-feedback>` tags are treated as parse failures.
    /// If false (default), raw output is used when tags are missing.
    #[serde(default)]
    pub require_plan_feedback_tags: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AggregationMode {
    #[default]
    AnyRejects, 
    AllReject,  
    Majority,   
}

impl WorkflowConfig {

    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        let config: Self = serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse config file as YAML: {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn default_config() -> Self {

        const DEFAULT_WORKFLOW_YAML: &str = include_str!("../workflow.yaml");

        serde_yaml::from_str(DEFAULT_WORKFLOW_YAML)
            .expect("Failed to parse embedded workflow.yaml - this is a bug in the workflow.yaml file")
    }

    fn validate(&self) -> Result<()> {

        if !self.agents.contains_key(&self.workflow.planning.agent) {
            anyhow::bail!(
                "Planning agent '{}' not found in agents configuration",
                self.workflow.planning.agent
            );
        }

        for agent in &self.workflow.reviewing.agents {
            if !self.agents.contains_key(agent) {
                anyhow::bail!(
                    "Review agent '{}' not found in agents configuration",
                    agent
                );
            }
        }

        if !self.agents.contains_key(&self.workflow.revising.agent) {
            anyhow::bail!(
                "Revising agent '{}' not found in agents configuration",
                self.workflow.revising.agent
            );
        }

        if self.workflow.reviewing.agents.is_empty() {
            anyhow::bail!("At least one review agent must be configured");
        }

        Ok(())
    }

    pub fn get_agent(&self, name: &str) -> Option<&AgentConfig> {
        self.agents.get(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = WorkflowConfig::default_config();

        assert!(config.agents.contains_key("claude"));
        assert!(config.agents.contains_key("codex"));
        assert!(config.agents.contains_key("gemini"));

        assert!(config.agents.contains_key(&config.workflow.planning.agent));
        assert!(config.agents.contains_key(&config.workflow.revising.agent));
        for agent in &config.workflow.reviewing.agents {
            assert!(config.agents.contains_key(agent));
        }
    }

    #[test]
    fn test_default_config_validates() {
        let config = WorkflowConfig::default_config();
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

  revising:
    agent: claude
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

  revising:
    agent: claude
"#;
        let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.workflow.reviewing.agents.len(), 2);
        assert!(config.workflow.reviewing.agents.contains(&"claude".to_string()));
        assert!(config.workflow.reviewing.agents.contains(&"codex".to_string()));
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

  revising:
    agent: claude
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

  revising:
    agent: claude
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
  revising:
    agent: claude
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
  revising:
    agent: claude
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
  revising:
    agent: claude
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
  revising:
    agent: claude
"#;
        let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
        let claude_config = config.get_agent("claude").unwrap();

        // session_persistence should default to disabled with Stateless strategy
        assert!(!claude_config.session_persistence.enabled);
        assert_eq!(
            claude_config.session_persistence.strategy,
            crate::state::ResumeStrategy::Stateless
        );
    }
}
