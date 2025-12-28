use super::log::AgentLogger;
use super::parser::ClaudeParser;
use super::{AgentContext, AgentResult};
use crate::agents::runner::{run_agent_process, ContextEmitter, EventEmitter, RunnerConfig};
use crate::config::AgentConfig;
use crate::state::ResumeStrategy;
use anyhow::Result;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;

const DEFAULT_ACTIVITY_TIMEOUT: Duration = Duration::from_secs(300);
const DEFAULT_OVERALL_TIMEOUT: Duration = Duration::from_secs(1800);

#[derive(Debug, Clone)]
pub struct ClaudeAgent {
    name: String,
    config: AgentConfig,
    working_dir: PathBuf,
    activity_timeout: Duration,
    overall_timeout: Duration,
}

impl ClaudeAgent {
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
        system_prompt: Option<String>,
        max_turns: Option<u32>,
        context: AgentContext,
    ) -> Result<AgentResult> {
        let emitter = ContextEmitter::new(context.clone(), self.name.clone());
        self.execute_streaming_internal(prompt, system_prompt, max_turns, &emitter, Some(&context))
            .await
    }

    async fn execute_streaming_internal(
        &self,
        prompt: String,
        system_prompt: Option<String>,
        max_turns: Option<u32>,
        emitter: &dyn EventEmitter,
        context: Option<&AgentContext>,
    ) -> Result<AgentResult> {
        let logger = AgentLogger::new(&self.name, &self.working_dir);
        self.log_start(&logger, &prompt, &system_prompt, context.is_some());
        self.log_timeout(&logger);

        let cmd = self.build_command(&prompt, &system_prompt, max_turns, context);
        let config = RunnerConfig::new(self.name.clone(), self.working_dir.clone())
            .with_activity_timeout(self.activity_timeout)
            .with_overall_timeout(self.overall_timeout);
        let mut parser = ClaudeParser::new();

        let output = run_agent_process(cmd, &config, &mut parser, emitter).await?;
        Ok(output.into())
    }

    fn build_command(
        &self,
        prompt: &str,
        system_prompt: &Option<String>,
        max_turns: Option<u32>,
        context: Option<&AgentContext>,
    ) -> Command {
        let mut cmd = Command::new(&self.config.command);

        for arg in &self.config.args {
            cmd.arg(arg);
        }

        cmd.arg(prompt);

        if let Some(ref sys_prompt) = system_prompt {
            cmd.arg("--append-system-prompt").arg(sys_prompt);
        }

        if !self.config.allowed_tools.is_empty() {
            cmd.arg("--allowedTools")
                .arg(self.config.allowed_tools.join(","));
        }

        if let Some(turns) = max_turns {
            cmd.arg("--max-turns").arg(turns.to_string());
        }

        if self.config.session_persistence.enabled {
            if let Some(ctx) = context {
                if ctx.resume_strategy == ResumeStrategy::SessionId {
                    if let Some(ref session_id) = ctx.session_key {
                        cmd.arg("--session-id").arg(session_id);
                    }
                }
            }
        }

        cmd
    }

    fn log_start(
        &self,
        logger: &Option<AgentLogger>,
        prompt: &str,
        system_prompt: &Option<String>,
        has_context: bool,
    ) {
        if let Some(ref logger) = logger {
            let args = if self.config.args.is_empty() {
                String::new()
            } else {
                format!(" {}", self.config.args.join(" "))
            };
            let suffix = if has_context { " (with context)" } else { "" };
            logger.log_line("start", &format!("command: {}{}{}", self.config.command, args, suffix));
            logger.log_line("prompt", &prompt.chars().take(200).collect::<String>());
            if let Some(ref sys_prompt) = system_prompt {
                logger.log_line(
                    "system_prompt",
                    &sys_prompt.chars().take(200).collect::<String>(),
                );
            }
        }
    }

    fn log_timeout(&self, logger: &Option<AgentLogger>) {
        if let Some(ref logger) = logger {
            logger.log_line(
                "timeout",
                &format!(
                    "activity_timeout={:?}, overall_timeout={:?}",
                    self.activity_timeout, self.overall_timeout
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SessionPersistenceConfig;

    #[test]
    fn test_claude_agent_new() {
        let config = AgentConfig {
            command: "claude".to_string(),
            args: vec!["-p".to_string()],
            allowed_tools: vec!["Read".to_string()],
            session_persistence: SessionPersistenceConfig::default(),
        };
        let agent = ClaudeAgent::new("claude".to_string(), config, PathBuf::from("."));
        assert_eq!(agent.activity_timeout, DEFAULT_ACTIVITY_TIMEOUT);
        assert_eq!(agent.overall_timeout, DEFAULT_OVERALL_TIMEOUT);
    }
}
