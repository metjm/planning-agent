use super::parser::CodexParser;
use crate::agents::log::AgentLogger;
use crate::agents::runner::{run_agent_process, ContextEmitter, EventEmitter, RunnerConfig};
use crate::agents::{AgentContext, AgentResult};
use crate::config::AgentConfig;
use anyhow::Result;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;

const DEFAULT_ACTIVITY_TIMEOUT: Duration = Duration::from_secs(300);
const DEFAULT_OVERALL_TIMEOUT: Duration = Duration::from_secs(1800);

#[derive(Debug, Clone)]
pub struct CodexAgent {
    name: String,
    config: AgentConfig,
    working_dir: PathBuf,
    activity_timeout: Duration,
    overall_timeout: Duration,
}

impl CodexAgent {
    pub fn new(name: String, config: AgentConfig, working_dir: PathBuf) -> Self {
        Self {
            name,
            config,
            working_dir,
            activity_timeout: DEFAULT_ACTIVITY_TIMEOUT,
            overall_timeout: DEFAULT_OVERALL_TIMEOUT,
        }
    }

    #[cfg(test)]
    pub fn name(&self) -> &str {
        &self.name
    }

    pub async fn execute_streaming_with_context(
        &self,
        prompt: String,
        _system_prompt: Option<String>,
        _max_turns: Option<u32>,
        context: AgentContext,
    ) -> Result<AgentResult> {
        let emitter = ContextEmitter::new(context.clone(), self.name.clone());
        self.execute_streaming_internal(prompt, &emitter, true).await
    }

    async fn execute_streaming_internal(
        &self,
        prompt: String,
        emitter: &dyn EventEmitter,
        has_context: bool,
    ) -> Result<AgentResult> {
        let logger = AgentLogger::new(&self.name, &self.working_dir);
        self.log_start(&logger, &prompt, has_context);

        let cmd = self.build_command(&prompt);
        let config = RunnerConfig::new(self.name.clone(), self.working_dir.clone())
            .with_activity_timeout(self.activity_timeout)
            .with_overall_timeout(self.overall_timeout);
        let mut parser = CodexParser::new();

        let output = run_agent_process(cmd, &config, &mut parser, emitter).await?;
        Ok(output.into())
    }

    fn build_command(&self, prompt: &str) -> Command {
        let mut cmd = Command::new(&self.config.command);
        for arg in &self.config.args {
            cmd.arg(arg);
        }
        cmd.arg(prompt);
        cmd
    }

    fn log_start(&self, logger: &Option<AgentLogger>, prompt: &str, has_context: bool) {
        if let Some(ref logger) = logger {
            let args = if self.config.args.is_empty() {
                String::new()
            } else {
                format!(" {}", self.config.args.join(" "))
            };
            let suffix = if has_context { " (with context)" } else { "" };
            logger.log_line("start", &format!("command: {}{}{}", self.config.command, args, suffix));
            logger.log_line("prompt", &prompt.chars().take(200).collect::<String>());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SessionPersistenceConfig;

    #[test]
    fn test_codex_agent_new() {
        let config = AgentConfig {
            command: "codex".to_string(),
            args: vec!["exec".to_string(), "--json".to_string()],
            allowed_tools: vec![],
            session_persistence: SessionPersistenceConfig::default(),
        };
        let agent = CodexAgent::new("codex".to_string(), config, PathBuf::from("."));
        assert_eq!(agent.activity_timeout, DEFAULT_ACTIVITY_TIMEOUT);
        assert_eq!(agent.overall_timeout, DEFAULT_OVERALL_TIMEOUT);
    }
}
