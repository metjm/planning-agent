//! Centralized prompt preparation for all agent types.
//!
//! This module handles the differences between agent CLI capabilities:
//! - Claude: supports --append-system-prompt and --max-turns
//! - Codex: no system prompt flag, no max turns flag
//! - Gemini: no system prompt flag, no max turns flag
//!
//! For agents without system prompt support, the system prompt is merged
//! into the user prompt to ensure consistent behavior.

/// Represents a prompt request before agent-specific preparation.
#[derive(Debug, Clone)]
pub struct PromptRequest {
    /// The main user prompt (may be XML-structured)
    pub user_prompt: String,
    /// Optional system prompt to guide agent behavior
    pub system_prompt: Option<String>,
    /// Maximum turns/iterations for the agent
    pub max_turns: Option<u32>,
}

impl PromptRequest {
    pub fn new(user_prompt: String) -> Self {
        Self {
            user_prompt,
            system_prompt: None,
            max_turns: None,
        }
    }

    pub fn with_system_prompt(mut self, system_prompt: String) -> Self {
        self.system_prompt = Some(system_prompt);
        self
    }

    pub fn with_max_turns(mut self, max_turns: u32) -> Self {
        self.max_turns = Some(max_turns);
        self
    }
}

/// Prepared prompt ready for a specific agent type.
#[derive(Debug, Clone)]
pub struct PreparedPrompt {
    /// The prompt to pass as the main argument
    pub prompt: String,
    /// System prompt to pass via --append-system-prompt (Claude only)
    pub system_prompt_arg: Option<String>,
    /// Max turns to pass via --max-turns (Claude only)
    pub max_turns_arg: Option<u32>,
}

/// Agent capabilities for prompt handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentCapabilities {
    /// Full support: system prompt flag, max turns flag
    Claude,
    /// No system prompt or max turns flags - must merge into prompt
    Codex,
    /// No system prompt or max turns flags - must merge into prompt
    Gemini,
}

impl AgentCapabilities {
    /// Whether this agent supports a separate system prompt argument.
    pub fn supports_system_prompt_arg(&self) -> bool {
        matches!(self, Self::Claude)
    }

    /// Whether this agent supports a max turns argument.
    pub fn supports_max_turns_arg(&self) -> bool {
        matches!(self, Self::Claude)
    }
}

/// Prepare a prompt for a specific agent type.
///
/// For agents that don't support system prompts, the system prompt
/// is prepended to the user prompt within a <system-context> tag.
pub fn prepare_prompt(request: PromptRequest, capabilities: AgentCapabilities) -> PreparedPrompt {
    if capabilities.supports_system_prompt_arg() {
        // Claude: pass system prompt separately
        PreparedPrompt {
            prompt: request.user_prompt,
            system_prompt_arg: request.system_prompt,
            max_turns_arg: if capabilities.supports_max_turns_arg() {
                request.max_turns
            } else {
                None
            },
        }
    } else {
        // Codex/Gemini: merge system prompt into user prompt
        let prompt = match request.system_prompt {
            Some(sys) => format!(
                "<system-context>\n{}\n</system-context>\n\n{}",
                sys, request.user_prompt
            ),
            None => request.user_prompt,
        };

        PreparedPrompt {
            prompt,
            system_prompt_arg: None,
            max_turns_arg: None,
        }
    }
}

#[cfg(test)]
#[path = "tests/prompt_tests.rs"]
mod tests;
