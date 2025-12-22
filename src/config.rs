use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkflowConfig {
    pub agents: HashMap<String, AgentConfig>,
    pub workflow: PhaseConfigs,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
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
}

#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AggregationMode {
    #[default]
    AnyRejects, // If any reviewer rejects, needs revision
    AllReject,  // Only if all reviewers reject
    Majority,   // If majority rejects
}

impl WorkflowConfig {
    /// Load configuration from a YAML file
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        let config: Self = serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse config file as YAML: {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    /// Returns the default configuration for multi-agent workflows
    pub fn default_config() -> Self {
        let mut agents = HashMap::new();
        agents.insert(
            "claude".to_string(),
            AgentConfig {
                command: "claude".to_string(),
                args: vec![
                    "-p".to_string(),
                    "--output-format".to_string(),
                    "stream-json".to_string(),
                    "--verbose".to_string(),
                    "--dangerously-skip-permissions".to_string(),
                ],
                allowed_tools: vec![
                    "Read".to_string(),
                    "Glob".to_string(),
                    "Grep".to_string(),
                    "Write".to_string(),
                    "Edit".to_string(),
                    "WebSearch".to_string(),
                    "WebFetch".to_string(),
                    "Skill".to_string(),
                    "Task".to_string(),
                ],
            },
        );
        agents.insert(
            "codex".to_string(),
            AgentConfig {
                command: "codex".to_string(),
                args: vec!["exec".to_string(), "--json".to_string()],
                allowed_tools: vec![],
            },
        );
        agents.insert(
            "gemini".to_string(),
            AgentConfig {
                command: "gemini".to_string(),
                args: vec![
                    "-p".to_string(),
                    "--output-format".to_string(),
                    "json".to_string(),
                ],
                allowed_tools: vec![],
            },
        );

        Self {
            agents,
            workflow: PhaseConfigs {
                planning: SingleAgentPhase {
                    agent: "claude".to_string(),
                    max_turns: Some(50),
                },
                reviewing: MultiAgentPhase {
                    agents: vec!["claude".to_string(), "codex".to_string()],
                    aggregation: AggregationMode::AnyRejects,
                },
                revising: SingleAgentPhase {
                    agent: "claude".to_string(),
                    max_turns: None,
                },
            },
        }
    }

    /// Validate the configuration
    fn validate(&self) -> Result<()> {
        // Verify planning agent exists
        if !self.agents.contains_key(&self.workflow.planning.agent) {
            anyhow::bail!(
                "Planning agent '{}' not found in agents configuration",
                self.workflow.planning.agent
            );
        }

        // Verify all review agents exist
        for agent in &self.workflow.reviewing.agents {
            if !self.agents.contains_key(agent) {
                anyhow::bail!(
                    "Review agent '{}' not found in agents configuration",
                    agent
                );
            }
        }

        // Verify revising agent exists
        if !self.agents.contains_key(&self.workflow.revising.agent) {
            anyhow::bail!(
                "Revising agent '{}' not found in agents configuration",
                self.workflow.revising.agent
            );
        }

        // Verify at least one review agent is configured
        if self.workflow.reviewing.agents.is_empty() {
            anyhow::bail!("At least one review agent must be configured");
        }

        Ok(())
    }

    /// Get agent config by name
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
        assert_eq!(config.workflow.planning.agent, "claude");
        assert_eq!(
            config.workflow.reviewing.agents,
            vec!["claude", "codex"]
        );
        assert_eq!(config.workflow.revising.agent, "claude");
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
}
