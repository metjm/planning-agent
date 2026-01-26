use super::parser::ClaudeParser;
use crate::agents::log::AgentLogger;
use crate::agents::prompt::PreparedPrompt;
use crate::agents::runner::{
    run_agent_process, ContextEmitter, EventEmitter, RunnerConfig, DEFAULT_ACTIVITY_TIMEOUT,
    DEFAULT_OVERALL_TIMEOUT,
};
use crate::agents::{AgentContext, AgentResult};
use crate::config::AgentConfig;
use crate::state::ResumeStrategy;
use anyhow::Result;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;

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

    /// Execute with a centrally-prepared prompt.
    /// The PreparedPrompt already has system_prompt and max_turns handled appropriately.
    pub async fn execute_streaming_with_prepared(
        &self,
        prepared: PreparedPrompt,
        context: AgentContext,
    ) -> Result<AgentResult> {
        let emitter = ContextEmitter::new(context.clone(), self.name.clone());
        self.execute_streaming_internal(prepared, &emitter, Some(&context))
            .await
    }

    async fn execute_streaming_internal(
        &self,
        prepared: PreparedPrompt,
        emitter: &dyn EventEmitter,
        context: Option<&AgentContext>,
    ) -> Result<AgentResult> {
        let logger = context.map(|ctx| AgentLogger::new(&self.name, ctx.session_logger.clone()));
        self.log_start(&logger, &prepared, context.is_some());
        self.log_timeout(&logger);

        let cmd = self.build_command(&prepared, context);
        let mut config = RunnerConfig::new(self.name.clone(), self.working_dir.clone())
            .with_activity_timeout(self.activity_timeout)
            .with_overall_timeout(self.overall_timeout);
        if let Some(ctx) = context {
            config = config.with_session_logger(ctx.session_logger.clone());
            if let Some(cancel_rx) = ctx.cancel_rx.clone() {
                config = config.with_cancel_rx(cancel_rx);
            }
        }
        let mut parser = ClaudeParser::new();

        let output = run_agent_process(cmd, &config, &mut parser, emitter).await?;
        Ok(output.into())
    }

    fn build_command(&self, prepared: &PreparedPrompt, context: Option<&AgentContext>) -> Command {
        let mut cmd = Command::new(&self.config.command);

        for arg in &self.config.args {
            cmd.arg(arg);
        }

        cmd.arg(&prepared.prompt);

        if let Some(ref sys_prompt) = prepared.system_prompt_arg {
            cmd.arg("--append-system-prompt").arg(sys_prompt);
        }

        if !self.config.allowed_tools.is_empty() {
            cmd.arg("--allowedTools")
                .arg(self.config.allowed_tools.join(","));
        }

        if let Some(turns) = prepared.max_turns_arg {
            cmd.arg("--max-turns").arg(turns.to_string());
        }

        if self.config.session_persistence.enabled {
            if let Some(ctx) = context {
                if ctx.resume_strategy == ResumeStrategy::ConversationResume {
                    if let Some(ref conv_id) = ctx.conversation_id {
                        // Use --resume to continue an existing conversation
                        // This requires a conversation ID captured from a previous run
                        cmd.arg("--resume").arg(conv_id);
                    }
                }
            }
        }

        cmd
    }

    fn log_start(
        &self,
        logger: &Option<AgentLogger>,
        prepared: &PreparedPrompt,
        has_context: bool,
    ) {
        if let Some(ref logger) = logger {
            let args = if self.config.args.is_empty() {
                String::new()
            } else {
                format!(" {}", self.config.args.join(" "))
            };
            let suffix = if has_context { " (with context)" } else { "" };
            logger.log_line(
                "start",
                &format!("command: {}{}{}", self.config.command, args, suffix),
            );
            logger.log_line(
                "prompt",
                &prepared.prompt.chars().take(200).collect::<String>(),
            );
            if let Some(ref sys_prompt) = prepared.system_prompt_arg {
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
#[path = "tests/agent_tests.rs"]
mod tests;
