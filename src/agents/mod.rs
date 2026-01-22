pub mod claude;
pub mod codex;
pub mod gemini;
pub(crate) mod log;
pub mod prompt;
pub mod protocol;
pub mod runner;

use crate::config::AgentConfig;
use crate::session_logger::SessionLogger;
use crate::state::ResumeStrategy;
use crate::tui::SessionEventSender;
use anyhow::Result;
use prompt::{prepare_prompt, AgentCapabilities, PreparedPrompt, PromptRequest};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::watch;

#[derive(Clone)]
pub struct AgentContext {
    pub session_sender: SessionEventSender,
    pub phase: String,
    /// The AI agent's conversation ID for session resume (e.g., Claude's session UUID)
    pub conversation_id: Option<String>,
    pub resume_strategy: ResumeStrategy,
    /// Optional cancellation signal receiver for cooperative cancellation.
    pub cancel_rx: Option<watch::Receiver<bool>>,
    /// Session logger for agent events.
    #[allow(dead_code)]
    pub session_logger: Arc<SessionLogger>,
}

#[derive(Debug, Clone)]
pub struct AgentResult {
    pub output: String,
    pub is_error: bool,
    /// Cost in USD (stored for potential display/logging, currently unused)
    #[allow(dead_code)]
    pub cost_usd: Option<f64>,
    /// Captured conversation ID for future resume (from agent's init message)
    pub conversation_id: Option<String>,
    /// Stop reason if agent was stopped (max_turns, max_tokens, cancelled, etc.)
    #[allow(dead_code)]
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub enum AgentType {
    Claude(claude::ClaudeAgent),
    Codex(codex::CodexAgent),
    Gemini(gemini::GeminiAgent),
}

impl AgentType {
    pub fn from_config(name: &str, config: &AgentConfig, working_dir: PathBuf) -> Result<Self> {
        match config.command.as_str() {
            "claude" => Ok(Self::Claude(claude::ClaudeAgent::new(
                name.to_string(),
                config.clone(),
                working_dir,
            ))),
            "codex" => Ok(Self::Codex(codex::CodexAgent::new(
                name.to_string(),
                config.clone(),
                working_dir,
            ))),
            "gemini" => Ok(Self::Gemini(gemini::GeminiAgent::new(
                name.to_string(),
                config.clone(),
                working_dir,
            ))),
            other => anyhow::bail!("Unknown agent command: {}", other),
        }
    }

    /// Returns the capabilities of this agent type.
    fn capabilities(&self) -> AgentCapabilities {
        match self {
            Self::Claude(_) => AgentCapabilities::Claude,
            Self::Codex(_) => AgentCapabilities::Codex,
            Self::Gemini(_) => AgentCapabilities::Gemini,
        }
    }

    /// Returns true if this agent type supports conversation resume.
    /// All agents (Claude, Codex, Gemini) support this feature:
    /// - Claude: uses --resume <conversation_id>
    /// - Codex: uses exec resume <thread_id> <prompt>
    /// - Gemini: uses --resume <session_id>
    pub fn supports_session_resume(&self) -> bool {
        true
    }

    #[cfg(test)]
    pub fn name(&self) -> &str {
        match self {
            Self::Claude(agent) => agent.name(),
            Self::Codex(agent) => agent.name(),
            Self::Gemini(agent) => agent.name(),
        }
    }

    /// Prepare a prompt request for this agent type.
    /// Handles merging system prompt into user prompt for agents that don't support it.
    fn prepare_prompt(
        &self,
        user_prompt: String,
        system_prompt: Option<String>,
        max_turns: Option<u32>,
    ) -> PreparedPrompt {
        let mut request = PromptRequest::new(user_prompt);
        if let Some(sys) = system_prompt {
            request = request.with_system_prompt(sys);
        }
        if let Some(turns) = max_turns {
            request = request.with_max_turns(turns);
        }
        prepare_prompt(request, self.capabilities())
    }

    pub async fn execute_streaming_with_context(
        &self,
        prompt: String,
        system_prompt: Option<String>,
        max_turns: Option<u32>,
        context: AgentContext,
    ) -> Result<AgentResult> {
        // Prepare prompt centrally - handles system prompt merging for non-Claude agents
        let prepared = self.prepare_prompt(prompt, system_prompt, max_turns);

        match self {
            Self::Claude(agent) => {
                agent
                    .execute_streaming_with_prepared(prepared, context)
                    .await
            }
            Self::Codex(agent) => {
                agent
                    .execute_streaming_with_prepared(prepared, context)
                    .await
            }
            Self::Gemini(agent) => {
                agent
                    .execute_streaming_with_prepared(prepared, context)
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SessionPersistenceConfig;

    #[test]
    fn test_agent_type_from_config_claude() {
        let config = AgentConfig {
            command: "claude".to_string(),
            args: vec!["-p".to_string()],
            allowed_tools: vec![],
            session_persistence: SessionPersistenceConfig::default(),
        };
        let agent = AgentType::from_config("claude", &config, PathBuf::from(".")).unwrap();
        assert_eq!(agent.name(), "claude");
    }

    #[test]
    fn test_agent_type_from_config_codex() {
        let config = AgentConfig {
            command: "codex".to_string(),
            args: vec!["exec".to_string()],
            allowed_tools: vec![],
            session_persistence: SessionPersistenceConfig::default(),
        };
        let agent = AgentType::from_config("codex", &config, PathBuf::from(".")).unwrap();
        assert_eq!(agent.name(), "codex");
    }

    #[test]
    fn test_agent_type_from_config_gemini() {
        let config = AgentConfig {
            command: "gemini".to_string(),
            args: vec!["-p".to_string()],
            allowed_tools: vec![],
            session_persistence: SessionPersistenceConfig::default(),
        };
        let agent = AgentType::from_config("gemini", &config, PathBuf::from(".")).unwrap();
        assert_eq!(agent.name(), "gemini");
    }

    #[test]
    fn test_agent_type_from_config_unknown() {
        let config = AgentConfig {
            command: "unknown".to_string(),
            args: vec![],
            allowed_tools: vec![],
            session_persistence: SessionPersistenceConfig::default(),
        };
        let result = AgentType::from_config("unknown", &config, PathBuf::from("."));
        assert!(result.is_err());
    }
}
