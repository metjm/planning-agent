use crate::config_modes::{ClaudeModeConfig, CodexModeConfig, GeminiModeConfig};
use crate::domain::failure::FailurePolicy;
use crate::domain::types::ResumeStrategy;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkflowConfig {
    /// Name of this workflow (e.g., "claude-only", "default", "my-custom").
    /// Used to persist and restore the correct workflow across session resume.
    #[serde(default)]
    pub name: String,
    pub agents: HashMap<String, AgentConfig>,
    pub workflow: PhaseConfigs,
    /// Failure handling policy for transient failures and recovery.
    #[serde(default)]
    pub failure_policy: FailurePolicy,
    /// Optional implementation workflow configuration.
    /// When present and enabled, allows JSON-mode implementation after plan approval.
    #[serde(default)]
    pub implementation: ImplementationConfig,
    /// Claude-mode configuration for --claude flag transformation.
    #[serde(default)]
    pub claude_mode: ClaudeModeConfig,
    /// Codex-mode configuration for codex-only workflow transformation.
    #[serde(default)]
    pub codex_mode: CodexModeConfig,
    /// Gemini-mode configuration for gemini-only workflow transformation.
    #[serde(default)]
    pub gemini_mode: GeminiModeConfig,
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
            let implementing_agent = self
                .implementing
                .as_ref()
                .map(|p| p.agent.as_str())
                .unwrap_or("");

            // Find first reviewer that differs from the implementing agent
            let reviewer = workflow
                .reviewing
                .agents
                .iter()
                .map(|r| r.agent_name())
                .find(|name| *name != implementing_agent);

            if let Some(reviewer_name) = reviewer {
                self.reviewing = Some(SingleAgentPhase {
                    agent: reviewer_name.to_string(),
                    max_turns: None, // Use agent default
                });
            } else if workflow.reviewing.agents.len() == 1
                && workflow.reviewing.agents[0].agent_name() != implementing_agent
            {
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
    /// If true, run reviewers sequentially with immediate revision on rejection.
    /// When any reviewer rejects, the plan is revised and all reviewers must
    /// re-review from the beginning. Default: false (parallel execution).
    #[serde(default)]
    pub sequential: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
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

        let mut config: Self = serde_yaml::from_str(DEFAULT_WORKFLOW_YAML).expect(
            "Failed to parse embedded workflow.yaml - this is a bug in the workflow.yaml file",
        );
        // Normalize implementation config defaults
        config
            .implementation
            .normalize(&config.workflow)
            .expect("Failed to normalize implementation config - this is a bug");
        config
    }

    pub(crate) fn validate(&self) -> Result<()> {
        // Validate all agent configs have non-empty commands
        for (name, config) in &self.agents {
            if config.command.trim().is_empty() {
                anyhow::bail!(
                    "Agent '{}' has an empty command. Each agent must specify a valid command.",
                    name
                );
            }
        }

        // Validate max_turns is not zero (which would prevent any work)
        if let Some(max_turns) = self.workflow.planning.max_turns {
            if max_turns == 0 {
                anyhow::bail!(
                    "Planning phase has max_turns=0, which would prevent any work. \
                     Either remove max_turns to use the default, or set a positive value."
                );
            }
        }

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

        // Check for duplicate display IDs in reviewers - each reviewer must write to a unique file
        let mut seen_ids = std::collections::HashSet::new();
        for agent_ref in &self.workflow.reviewing.agents {
            let display_id = agent_ref.display_id();
            if !seen_ids.insert(display_id) {
                anyhow::bail!(
                    "Duplicate reviewer display ID '{}' detected in workflow.reviewing.agents. \
                     Each reviewer must have a unique 'id' field because feedback files are named \
                     using the display ID (e.g., feedback_1_{}.md). Having duplicate IDs causes \
                     one reviewer's feedback to overwrite another's.\n\n\
                     Fix: Change one of the duplicate reviewers to use a distinct 'id', for example:\n\
                     - agent: claude\n  id: {}-2  # or a descriptive name like '{}-completeness'\n  prompt: ...",
                    display_id, display_id, display_id, display_id
                );
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
#[path = "tests/config_tests/config_tests.rs"]
mod config_tests;

#[cfg(test)]
#[path = "tests/config_tests/config_inline_tests.rs"]
mod tests;
