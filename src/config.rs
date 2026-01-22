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
    /// Claude-mode configuration for --claude flag transformation.
    #[serde(default)]
    pub claude_mode: ClaudeModeConfig,
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

/// Configuration for Claude-only mode transformation.
/// Defines Claude-specific agents, substitution rules, and optional phase overrides.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ClaudeModeConfig {
    /// Claude-specific agent definitions that replace/supplement
    /// the base agents section when --claude is passed.
    #[serde(default)]
    pub agents: HashMap<String, AgentConfig>,

    /// Maps non-Claude agent names to their Claude replacements.
    /// Example: { "codex": "claude", "gemini": "claude" }
    #[serde(default)]
    pub substitutions: HashMap<String, String>,

    /// Optional override for the reviewing phase configuration.
    /// When present, replaces workflow.reviewing entirely instead of
    /// applying agent substitutions. This preserves extended AgentRef
    /// configurations like custom prompts.
    #[serde(default)]
    pub reviewing: Option<MultiAgentPhase>,
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

    /// Returns a Claude-only workflow configuration.
    /// Transforms the default config by applying claude_mode substitutions.
    pub fn claude_only_config() -> Self {
        let mut config = Self::default_config();
        config
            .transform_to_claude_only()
            .expect("Failed to transform config to Claude-only mode - this is a bug");
        // Re-normalize after transformation to update implementation defaults
        config.implementation.normalize(&config.workflow).expect(
            "Failed to normalize implementation config after transformation - this is a bug",
        );
        // Validate the transformed config to catch any configuration errors
        config
            .validate()
            .expect("Transformed Claude-only config failed validation - this is a bug");
        config
    }

    /// Transforms this configuration for Claude-only mode.
    ///
    /// This method:
    /// 1. Validates all substitution targets exist in claude_mode.agents or base agents
    /// 2. Merges claude_mode.agents into the main agents map
    /// 3. Applies substitutions to planning phase
    /// 4. Replaces reviewing phase if claude_mode.reviewing is specified, otherwise applies substitutions
    /// 5. Applies substitutions to implementation and verification configs
    /// 6. Resolves implementation reviewer conflicts (uses claude-reviewer if available)
    ///
    /// Returns an error if a substitution target doesn't exist.
    pub fn transform_to_claude_only(&mut self) -> Result<()> {
        // Clone substitutions map upfront to avoid borrowing conflicts
        let substitutions = self.claude_mode.substitutions.clone();

        // Validate substitution targets exist before proceeding
        for (from, to) in &substitutions {
            let target_exists =
                self.claude_mode.agents.contains_key(to) || self.agents.contains_key(to);
            if !target_exists {
                anyhow::bail!(
                    "Claude-mode substitution target '{}' not found. \
                     Substitution '{}' -> '{}' is invalid. \
                     Ensure claude_mode.agents defines '{}' or it exists in the base agents.",
                    to,
                    from,
                    to,
                    to
                );
            }
        }

        // Merge claude_mode agents into main agents map
        for (name, config) in std::mem::take(&mut self.claude_mode.agents) {
            self.agents.insert(name, config);
        }

        // Apply substitutions to planning phase
        if let Some(target) = substitutions.get(&self.workflow.planning.agent) {
            self.workflow.planning.agent = target.clone();
        }

        // Handle reviewing phase: use override if present, otherwise apply substitutions
        if let Some(reviewing_override) = std::mem::take(&mut self.claude_mode.reviewing) {
            self.workflow.reviewing = reviewing_override;
        } else {
            // Apply substitutions to reviewing agents
            for agent_ref in &mut self.workflow.reviewing.agents {
                Self::apply_substitution_to_agent_ref(agent_ref, &substitutions);
            }
        }

        // Apply to implementation config with conflict resolution
        if let Some(ref mut impl_phase) = self.implementation.implementing {
            if let Some(target) = substitutions.get(&impl_phase.agent) {
                impl_phase.agent = target.clone();
            }
        }
        if let Some(ref mut review_phase) = self.implementation.reviewing {
            let original = &review_phase.agent;
            let substituted = substitutions
                .get(original)
                .cloned()
                .unwrap_or_else(|| original.clone());

            let impl_agent = self
                .implementation
                .implementing
                .as_ref()
                .map(|p| p.agent.as_str())
                .unwrap_or("");

            // If substitution would create conflict (same agent for impl and review),
            // use claude-reviewer if it exists in the agents map
            if substituted == impl_agent && self.agents.contains_key("claude-reviewer") {
                review_phase.agent = "claude-reviewer".to_string();
            } else {
                review_phase.agent = substituted;
            }
        }

        // Apply to verification config
        if let Some(ref mut verify_phase) = self.verification.verifying {
            if let Some(target) = substitutions.get(&verify_phase.agent) {
                verify_phase.agent = target.clone();
            }
        }
        if let Some(ref mut fix_phase) = self.verification.fixing {
            if let Some(target) = substitutions.get(&fix_phase.agent) {
                fix_phase.agent = target.clone();
            }
        }

        Ok(())
    }

    /// Applies agent name substitution to an AgentRef.
    /// For Extended refs, only the agent name is substituted; id and prompt are preserved.
    fn apply_substitution_to_agent_ref(
        agent_ref: &mut AgentRef,
        substitutions: &HashMap<String, String>,
    ) {
        match agent_ref {
            AgentRef::Simple(name) => {
                if let Some(target) = substitutions.get(name) {
                    *name = target.clone();
                }
            }
            AgentRef::Extended(inst) => {
                if let Some(target) = substitutions.get(&inst.agent) {
                    inst.agent = target.clone();
                }
                // Note: id and prompt fields are preserved
            }
        }
    }

    fn validate(&self) -> Result<()> {
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
#[path = "config_claude_mode_tests.rs"]
mod config_claude_mode_tests;

#[cfg(test)]
#[path = "config_inline_tests.rs"]
mod tests;
