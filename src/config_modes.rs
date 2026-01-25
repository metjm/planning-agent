//! Mode-specific configuration types for workflow transformations.
//!
//! This module contains the configuration structures for Claude-only, Codex-only,
//! and Gemini-only workflow modes. Each mode defines agent definitions, substitution
//! rules, and phase overrides.

use crate::config::{AgentConfig, AgentRef, MultiAgentPhase, SingleAgentPhase, WorkflowConfig};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

/// Configuration for Codex-only mode transformation.
/// Defines Codex-specific agents, substitution rules, and optional phase overrides.
/// Mirrors ClaudeModeConfig pattern for consistency.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct CodexModeConfig {
    /// Codex-specific agent definitions that replace/supplement
    /// the base agents section when codex-only mode is active.
    #[serde(default)]
    pub agents: HashMap<String, AgentConfig>,

    /// Maps non-Codex agent names to their Codex replacements.
    /// Example: { "claude": "codex", "gemini": "codex" }
    #[serde(default)]
    pub substitutions: HashMap<String, String>,

    /// Optional override for the reviewing phase configuration.
    /// When present, replaces workflow.reviewing entirely.
    #[serde(default)]
    pub reviewing: Option<MultiAgentPhase>,

    /// Optional override for the implementation phase configuration.
    /// When present, replaces implementation.implementing and implementation.reviewing.
    #[serde(default)]
    pub implementation: Option<CodexModeImplementation>,
}

/// Implementation phase overrides for codex-only mode.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct CodexModeImplementation {
    /// Agent for implementing (typically "codex")
    pub implementing: Option<SingleAgentPhase>,
    /// Agent for reviewing implementation (typically "codex-reviewer")
    pub reviewing: Option<SingleAgentPhase>,
}

/// Configuration for Gemini-only mode transformation.
/// Defines Gemini-specific agents, substitution rules, and optional phase overrides.
/// Mirrors CodexModeConfig pattern for consistency.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct GeminiModeConfig {
    /// Gemini-specific agent definitions that replace/supplement
    /// the base agents section when gemini-only mode is active.
    #[serde(default)]
    pub agents: HashMap<String, AgentConfig>,

    /// Maps non-Gemini agent names to their Gemini replacements.
    /// Example: { "claude": "gemini", "codex": "gemini" }
    #[serde(default)]
    pub substitutions: HashMap<String, String>,

    /// Optional override for the reviewing phase configuration.
    /// When present, replaces workflow.reviewing entirely.
    #[serde(default)]
    pub reviewing: Option<MultiAgentPhase>,

    /// Optional override for the implementation phase configuration.
    /// When present, replaces implementation.implementing and implementation.reviewing.
    #[serde(default)]
    pub implementation: Option<GeminiModeImplementation>,
}

/// Implementation phase overrides for gemini-only mode.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct GeminiModeImplementation {
    /// Agent for implementing (typically "gemini")
    pub implementing: Option<SingleAgentPhase>,
    /// Agent for reviewing implementation (typically "gemini-reviewer")
    pub reviewing: Option<SingleAgentPhase>,
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

impl WorkflowConfig {
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

    /// Returns a Codex-only workflow configuration.
    /// Transforms the default config by applying codex_mode substitutions.
    /// Uses Codex for ALL phases: planning, plan-review, implementation, implementation-review.
    pub fn codex_only_config() -> Self {
        let mut config = Self::default_config();
        config
            .transform_to_codex_only()
            .expect("Failed to transform config to Codex-only mode - this is a bug");
        // Re-normalize after transformation to update implementation defaults
        config.implementation.normalize(&config.workflow).expect(
            "Failed to normalize implementation config after transformation - this is a bug",
        );
        // Validate the transformed config to catch any configuration errors
        config
            .validate()
            .expect("Transformed Codex-only config failed validation - this is a bug");
        config
    }

    /// Returns a Gemini-only workflow configuration.
    /// Transforms the default config by applying gemini_mode substitutions.
    /// Uses Gemini for ALL phases: planning, plan-review, implementation, implementation-review.
    pub fn gemini_only_config() -> Self {
        let mut config = Self::default_config();
        config
            .transform_to_gemini_only()
            .expect("Failed to transform config to Gemini-only mode - this is a bug");
        // Re-normalize after transformation to update implementation defaults
        config.implementation.normalize(&config.workflow).expect(
            "Failed to normalize implementation config after transformation - this is a bug",
        );
        // Validate the transformed config to catch any configuration errors
        config
            .validate()
            .expect("Transformed Gemini-only config failed validation - this is a bug");
        config
    }

    /// Transforms this configuration for Claude-only mode.
    ///
    /// This method:
    /// 1. Validates all substitution targets exist in claude_mode.agents or base agents
    /// 2. Merges claude_mode.agents into the main agents map
    /// 3. Applies substitutions to planning phase
    /// 4. Replaces reviewing phase if claude_mode.reviewing is specified, otherwise applies substitutions
    /// 5. Applies substitutions to implementation config
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
                apply_substitution_to_agent_ref(agent_ref, &substitutions);
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

        Ok(())
    }

    /// Transforms this configuration for Codex-only mode.
    ///
    /// This method mirrors transform_to_claude_only():
    /// 1. Validates all substitution targets exist in codex_mode.agents or base agents
    /// 2. Merges codex_mode.agents into the main agents map
    /// 3. Applies substitutions to planning phase
    /// 4. Replaces reviewing phase if codex_mode.reviewing is specified
    /// 5. Applies implementation overrides if codex_mode.implementation is specified
    ///
    /// Returns an error if a substitution target doesn't exist.
    pub fn transform_to_codex_only(&mut self) -> Result<()> {
        // Clone substitutions map upfront to avoid borrowing conflicts
        let substitutions = self.codex_mode.substitutions.clone();

        // Validate substitution targets exist before proceeding
        for (from, to) in &substitutions {
            let target_exists =
                self.codex_mode.agents.contains_key(to) || self.agents.contains_key(to);
            if !target_exists {
                anyhow::bail!(
                    "Codex-mode substitution target '{}' not found. \
                     Substitution '{}' -> '{}' is invalid. \
                     Ensure codex_mode.agents defines '{}' or it exists in the base agents.",
                    to,
                    from,
                    to,
                    to
                );
            }
        }

        // Merge codex_mode agents into main agents map
        for (name, config) in std::mem::take(&mut self.codex_mode.agents) {
            self.agents.insert(name, config);
        }

        // Apply substitutions to planning phase
        if let Some(target) = substitutions.get(&self.workflow.planning.agent) {
            self.workflow.planning.agent = target.clone();
        }

        // Handle reviewing phase: use override if present, otherwise apply substitutions
        if let Some(reviewing_override) = std::mem::take(&mut self.codex_mode.reviewing) {
            self.workflow.reviewing = reviewing_override;
        } else {
            // Apply substitutions to reviewing agents
            for agent_ref in &mut self.workflow.reviewing.agents {
                apply_substitution_to_agent_ref(agent_ref, &substitutions);
            }
        }

        // Handle implementation overrides
        if let Some(impl_override) = std::mem::take(&mut self.codex_mode.implementation) {
            if let Some(implementing) = impl_override.implementing {
                self.implementation.implementing = Some(implementing);
            }
            if let Some(reviewing) = impl_override.reviewing {
                self.implementation.reviewing = Some(reviewing);
            }
        } else {
            // Apply substitutions to implementation config with conflict resolution
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

                // If substitution would create conflict, use codex-reviewer if available
                if substituted == impl_agent && self.agents.contains_key("codex-reviewer") {
                    review_phase.agent = "codex-reviewer".to_string();
                } else {
                    review_phase.agent = substituted;
                }
            }
        }

        Ok(())
    }

    /// Transforms this configuration for Gemini-only mode.
    ///
    /// This method mirrors transform_to_codex_only():
    /// 1. Validates all substitution targets exist in gemini_mode.agents or base agents
    /// 2. Merges gemini_mode.agents into the main agents map
    /// 3. Applies substitutions to planning phase
    /// 4. Replaces reviewing phase if gemini_mode.reviewing is specified
    /// 5. Applies implementation overrides if gemini_mode.implementation is specified
    ///
    /// Returns an error if a substitution target doesn't exist.
    pub fn transform_to_gemini_only(&mut self) -> Result<()> {
        // Clone substitutions map upfront to avoid borrowing conflicts
        let substitutions = self.gemini_mode.substitutions.clone();

        // Validate substitution targets exist before proceeding
        for (from, to) in &substitutions {
            let target_exists =
                self.gemini_mode.agents.contains_key(to) || self.agents.contains_key(to);
            if !target_exists {
                anyhow::bail!(
                    "Gemini-mode substitution target '{}' not found. \
                     Substitution '{}' -> '{}' is invalid. \
                     Ensure gemini_mode.agents defines '{}' or it exists in the base agents.",
                    to,
                    from,
                    to,
                    to
                );
            }
        }

        // Merge gemini_mode agents into main agents map
        for (name, config) in std::mem::take(&mut self.gemini_mode.agents) {
            self.agents.insert(name, config);
        }

        // Apply substitutions to planning phase
        if let Some(target) = substitutions.get(&self.workflow.planning.agent) {
            self.workflow.planning.agent = target.clone();
        }

        // Handle reviewing phase: use override if present, otherwise apply substitutions
        if let Some(reviewing_override) = std::mem::take(&mut self.gemini_mode.reviewing) {
            self.workflow.reviewing = reviewing_override;
        } else {
            // Apply substitutions to reviewing agents
            for agent_ref in &mut self.workflow.reviewing.agents {
                apply_substitution_to_agent_ref(agent_ref, &substitutions);
            }
        }

        // Handle implementation overrides
        if let Some(impl_override) = std::mem::take(&mut self.gemini_mode.implementation) {
            if let Some(implementing) = impl_override.implementing {
                self.implementation.implementing = Some(implementing);
            }
            if let Some(reviewing) = impl_override.reviewing {
                self.implementation.reviewing = Some(reviewing);
            }
        } else {
            // Apply substitutions to implementation config with conflict resolution
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

                // If substitution would create conflict, use gemini-reviewer if available
                if substituted == impl_agent && self.agents.contains_key("gemini-reviewer") {
                    review_phase.agent = "gemini-reviewer".to_string();
                } else {
                    review_phase.agent = substituted;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
#[path = "config_claude_mode_tests.rs"]
mod config_claude_mode_tests;

#[cfg(test)]
#[path = "config_codex_mode_tests.rs"]
mod config_codex_mode_tests;

#[cfg(test)]
#[path = "config_gemini_mode_tests.rs"]
mod config_gemini_mode_tests;
