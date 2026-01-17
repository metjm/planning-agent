use crate::app::failure::FailurePolicy;
use crate::state::ResumeStrategy;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkflowConfig {
    pub agents: HashMap<String, AgentConfig>,
    pub workflow: PhaseConfigs,
    /// Optional verification workflow configuration.
    /// When present and enabled, allows post-implementation verification.
    #[serde(default)]
    pub verification: VerificationConfig,
    /// Failure handling policy for transient failures and recovery.
    #[serde(default)]
    pub failure_policy: FailurePolicy,
    /// Optional implementation workflow configuration.
    /// When present and enabled, allows JSON-mode implementation after plan approval.
    #[serde(default)]
    pub implementation: ImplementationConfig,
}

/// Configuration for the post-implementation verification workflow.
/// All fields have defaults to ensure backward compatibility with existing configs.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VerificationConfig {
    /// Whether verification is enabled. Default: false (opt-in feature)
    #[serde(default)]
    pub enabled: bool,
    /// Maximum verification/fix iterations before stopping. Default: 3
    #[serde(default = "default_max_verification_iterations")]
    pub max_iterations: u32,
    /// Configuration for the verifying phase agent
    #[serde(default)]
    pub verifying: Option<SingleAgentPhase>,
    /// Configuration for the fixing phase agent
    #[serde(default)]
    pub fixing: Option<SingleAgentPhase>,
}

impl Default for VerificationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_iterations: default_max_verification_iterations(),
            verifying: None,
            fixing: None,
        }
    }
}

fn default_max_verification_iterations() -> u32 {
    3
}

/// Configuration for the JSON-mode implementation workflow.
/// All fields have defaults to ensure backward compatibility with existing configs.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ImplementationConfig {
    /// Whether implementation is enabled. Default: true
    /// Set to false for single-agent configs where implementation-review requires a distinct reviewer.
    #[serde(default = "default_implementation_enabled")]
    pub enabled: bool,
    /// Maximum implementation/review iterations before stopping. Default: 3
    #[serde(default = "default_max_implementation_iterations")]
    pub max_iterations: u32,
    /// Configuration for the implementing phase agent.
    /// Defaults to workflow.planning agent with max_turns: 100.
    #[serde(default)]
    pub implementing: Option<SingleAgentPhase>,
    /// Configuration for the implementation-review phase agent.
    /// Defaults to the first workflow.reviewing agent that differs from implementing agent.
    #[serde(default)]
    pub reviewing: Option<SingleAgentPhase>,
}

fn default_implementation_enabled() -> bool {
    true
}

fn default_max_implementation_iterations() -> u32 {
    3
}

fn default_implementation_max_turns() -> u32 {
    100
}

impl Default for ImplementationConfig {
    fn default() -> Self {
        Self {
            enabled: default_implementation_enabled(),
            max_iterations: default_max_implementation_iterations(),
            implementing: None,
            reviewing: None,
        }
    }
}

impl ImplementationConfig {
    /// Normalizes the implementation config by filling in defaults from the workflow config.
    /// Returns an error if enabled but no valid reviewer can be determined.
    pub fn normalize(&mut self, workflow: &PhaseConfigs) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        // Default implementing to workflow.planning with max_turns: 100
        if self.implementing.is_none() {
            self.implementing = Some(SingleAgentPhase {
                agent: workflow.planning.agent.clone(),
                max_turns: Some(default_implementation_max_turns()),
            });
        }

        // Default reviewing to first workflow.reviewing agent that differs from implementing
        if self.reviewing.is_none() {
            let implementing_agent = self.implementing.as_ref().map(|p| p.agent.as_str()).unwrap_or("");

            // Find first reviewer that differs from the implementing agent
            let reviewer = workflow.reviewing.agents.iter()
                .map(|r| r.agent_name())
                .find(|name| *name != implementing_agent);

            if let Some(reviewer_name) = reviewer {
                self.reviewing = Some(SingleAgentPhase {
                    agent: reviewer_name.to_string(),
                    max_turns: None, // Use agent default
                });
            } else if workflow.reviewing.agents.len() == 1
                && workflow.reviewing.agents[0].agent_name() != implementing_agent {
                // Single reviewer that is different from implementing agent
                self.reviewing = Some(SingleAgentPhase {
                    agent: workflow.reviewing.agents[0].agent_name().to_string(),
                    max_turns: None,
                });
            }
        }

        Ok(())
    }

    /// Returns the implementing agent name, or None if not configured.
    pub fn implementing_agent(&self) -> Option<&str> {
        self.implementing.as_ref().map(|p| p.agent.as_str())
    }

    /// Returns the reviewing agent name, or None if not configured.
    pub fn reviewing_agent(&self) -> Option<&str> {
        self.reviewing.as_ref().map(|p| p.agent.as_str())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SessionPersistenceConfig {
    #[serde(default = "default_session_persistence_enabled")]
    pub enabled: bool,
    #[serde(default = "default_session_persistence_strategy")]
    pub strategy: ResumeStrategy,
}

fn default_session_persistence_enabled() -> bool {
    true
}

fn default_session_persistence_strategy() -> ResumeStrategy {
    ResumeStrategy::ConversationResume
}

impl Default for SessionPersistenceConfig {
    fn default() -> Self {
        Self {
            enabled: default_session_persistence_enabled(),
            strategy: default_session_persistence_strategy(),
        }
    }
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
#[serde(deny_unknown_fields)]
pub struct PhaseConfigs {
    pub planning: SingleAgentPhase,
    pub reviewing: MultiAgentPhase,
    // Note: `revising` field was removed - revision now uses the planning agent
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SingleAgentPhase {
    pub agent: String,
    #[serde(default)]
    pub max_turns: Option<u32>,
}

/// A reference to an agent instance, supporting both simple string references
/// and extended configurations with custom prompts.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum AgentRef {
    /// Simple string reference to a pre-defined agent
    Simple(String),
    /// Extended configuration with optional customization
    Extended(AgentInstance),
}

/// Extended agent instance configuration
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct AgentInstance {
    /// Name of the base agent (must exist in `agents` section)
    pub agent: String,
    /// Optional unique identifier for this instance (for logging/display)
    /// Defaults to agent name if not specified
    #[serde(default)]
    pub id: Option<String>,
    /// Optional prompt text appended to the system prompt for this instance
    #[serde(default)]
    pub prompt: Option<String>,
}

impl AgentRef {
    /// Returns the base agent name
    pub fn agent_name(&self) -> &str {
        match self {
            AgentRef::Simple(name) => name,
            AgentRef::Extended(inst) => &inst.agent,
        }
    }

    /// Returns the display ID (instance id or agent name)
    pub fn display_id(&self) -> &str {
        match self {
            AgentRef::Simple(name) => name,
            AgentRef::Extended(inst) => inst.id.as_deref().unwrap_or(&inst.agent),
        }
    }

    /// Returns the optional custom prompt
    pub fn custom_prompt(&self) -> Option<&str> {
        match self {
            AgentRef::Simple(_) => None,
            AgentRef::Extended(inst) => inst.prompt.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MultiAgentPhase {
    pub agents: Vec<AgentRef>,
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
        let mut config: Self = serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse config file as YAML: {}", path.display()))?;
        // Normalize implementation config defaults before validation
        config.implementation.normalize(&config.workflow)?;
        config.validate()?;
        Ok(config)
    }

    pub fn default_config() -> Self {
        const DEFAULT_WORKFLOW_YAML: &str = include_str!("../workflow.yaml");

        let mut config: Self = serde_yaml::from_str(DEFAULT_WORKFLOW_YAML)
            .expect("Failed to parse embedded workflow.yaml - this is a bug in the workflow.yaml file");
        // Normalize implementation config defaults
        config.implementation.normalize(&config.workflow)
            .expect("Failed to normalize implementation config - this is a bug");
        config
    }

    /// Returns a Claude-only workflow configuration (no Codex or other agents)
    pub fn claude_only_config() -> Self {
        const CLAUDE_ONLY_YAML: &str = include_str!("../workflow-claude-only.yaml");

        let mut config: Self = serde_yaml::from_str(CLAUDE_ONLY_YAML)
            .expect("Failed to parse embedded workflow-claude-only.yaml - this is a bug");
        // Normalize implementation config defaults
        config.implementation.normalize(&config.workflow)
            .expect("Failed to normalize implementation config - this is a bug");
        config
    }

    fn validate(&self) -> Result<()> {

        if !self.agents.contains_key(&self.workflow.planning.agent) {
            anyhow::bail!(
                "Planning agent '{}' not found in agents configuration",
                self.workflow.planning.agent
            );
        }

        for agent_ref in &self.workflow.reviewing.agents {
            let agent_name = agent_ref.agent_name();
            if !self.agents.contains_key(agent_name) {
                anyhow::bail!(
                    "Review agent '{}' not found in agents configuration",
                    agent_name
                );
            }
        }

        if self.workflow.reviewing.agents.is_empty() {
            anyhow::bail!("At least one review agent must be configured");
        }

        // Only validate verification agents if verification is enabled
        if self.verification.enabled {
            if let Some(ref verifying) = self.verification.verifying {
                if !self.agents.contains_key(&verifying.agent) {
                    anyhow::bail!(
                        "Verifying agent '{}' not found in agents configuration",
                        verifying.agent
                    );
                }
            }
            if let Some(ref fixing) = self.verification.fixing {
                if !self.agents.contains_key(&fixing.agent) {
                    anyhow::bail!(
                        "Fixing agent '{}' not found in agents configuration",
                        fixing.agent
                    );
                }
            }
        }

        // Validate failure policy
        self.failure_policy.validate()?;

        // Only validate implementation agents if implementation is enabled
        if self.implementation.enabled {
            // Ensure implementing agent exists
            if let Some(ref implementing) = self.implementation.implementing {
                if !self.agents.contains_key(&implementing.agent) {
                    anyhow::bail!(
                        "Implementation implementing agent '{}' not found in agents configuration",
                        implementing.agent
                    );
                }
            }

            // Ensure reviewing agent exists
            if let Some(ref reviewing) = self.implementation.reviewing {
                if !self.agents.contains_key(&reviewing.agent) {
                    anyhow::bail!(
                        "Implementation reviewing agent '{}' not found in agents configuration",
                        reviewing.agent
                    );
                }
            }

            // Ensure implementing and reviewing agents are different
            let impl_agent = self.implementation.implementing_agent();
            let review_agent = self.implementation.reviewing_agent();
            if let (Some(impl_a), Some(rev_a)) = (impl_agent, review_agent) {
                if impl_a == rev_a {
                    anyhow::bail!(
                        "Implementation review requires a different agent than the implementing agent. \
                        Both are set to '{}'. Either configure a distinct reviewer or set implementation.enabled: false.",
                        impl_a
                    );
                }
            }

            // If enabled but no reviewer could be determined, error
            if self.implementation.reviewing.is_none() {
                anyhow::bail!(
                    "Implementation is enabled but no distinct reviewing agent could be determined. \
                    Configure implementation.reviewing.agent explicitly or set implementation.enabled: false."
                );
            }
        }

        Ok(())
    }

    pub fn get_agent(&self, name: &str) -> Option<&AgentConfig> {
        self.agents.get(name)
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod config_tests;

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

        // Should only have claude agents
        assert!(config.agents.contains_key("claude"));
        assert!(config.agents.contains_key("claude-reviewer"));
        assert!(!config.agents.contains_key("codex"));
        assert!(!config.agents.contains_key("gemini"));

        // Planning and reviewing phases should use claude
        // Note: revision now uses the planning agent automatically
        assert_eq!(config.workflow.planning.agent, "claude");
        assert_eq!(
            config.workflow.reviewing.agents,
            vec![AgentRef::Simple("claude".to_string())]
        );

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

}
