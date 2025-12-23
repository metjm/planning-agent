pub mod claude;
pub mod codex;
pub mod gemini;
mod log;

use crate::config::AgentConfig;
use crate::tui::{Event, SessionEventSender};
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct AgentContext {
    pub session_sender: SessionEventSender,
    pub phase: String,  
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AgentResult {

    pub output: String,

    pub is_error: bool,

    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone)]
pub enum AgentType {
    Claude(claude::ClaudeAgent),
    Codex(codex::CodexAgent),
    Gemini(gemini::GeminiAgent),
}

impl AgentType {

    pub fn from_config(
        name: &str,
        config: &AgentConfig,
        working_dir: PathBuf,
    ) -> Result<Self> {
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

    #[allow(dead_code)]
    pub fn name(&self) -> &str {
        match self {
            Self::Claude(agent) => agent.name(),
            Self::Codex(agent) => agent.name(),
            Self::Gemini(agent) => agent.name(),
        }
    }

    #[allow(dead_code)] 
    pub async fn execute_streaming(
        &self,
        prompt: String,
        system_prompt: Option<String>,
        max_turns: Option<u32>,
        output_tx: mpsc::UnboundedSender<Event>,
    ) -> Result<AgentResult> {
        match self {
            Self::Claude(agent) => {
                agent
                    .execute_streaming(prompt, system_prompt, max_turns, output_tx)
                    .await
            }
            Self::Codex(agent) => {
                agent
                    .execute_streaming(prompt, system_prompt, max_turns, output_tx)
                    .await
            }
            Self::Gemini(agent) => {
                agent
                    .execute_streaming(prompt, system_prompt, max_turns, output_tx)
                    .await
            }
        }
    }

    pub async fn execute_streaming_with_context(
        &self,
        prompt: String,
        system_prompt: Option<String>,
        max_turns: Option<u32>,
        context: AgentContext,
    ) -> Result<AgentResult> {
        match self {
            Self::Claude(agent) => {
                agent
                    .execute_streaming_with_context(prompt, system_prompt, max_turns, context)
                    .await
            }
            Self::Codex(agent) => {
                agent
                    .execute_streaming_with_context(prompt, system_prompt, max_turns, context)
                    .await
            }
            Self::Gemini(agent) => {
                agent
                    .execute_streaming_with_context(prompt, system_prompt, max_turns, context)
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_type_from_config_claude() {
        let config = AgentConfig {
            command: "claude".to_string(),
            args: vec!["-p".to_string()],
            allowed_tools: vec![],
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
        };
        let result = AgentType::from_config("unknown", &config, PathBuf::from("."));
        assert!(result.is_err());
    }
}
