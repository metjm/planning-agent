use super::parser::CodexParser;
use crate::agents::log::AgentLogger;
use crate::agents::prompt::PreparedPrompt;
use crate::agents::runner::{
    run_agent_process, ContextEmitter, EventEmitter, RunnerConfig, DEFAULT_ACTIVITY_TIMEOUT,
    DEFAULT_OVERALL_TIMEOUT,
};
use crate::agents::{AgentContext, AgentResult};
use crate::config::AgentConfig;
use crate::domain::types::ResumeStrategy;
use anyhow::Result;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;

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

    /// Execute with a centrally-prepared prompt.
    /// The PreparedPrompt already has system_prompt merged into the prompt for Codex.
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
        self.log_start(&logger, &prepared.prompt, context.is_some());

        let cmd = self.build_command(&prepared.prompt, context);
        let mut config = RunnerConfig::new(self.name.clone(), self.working_dir.clone())
            .with_activity_timeout(self.activity_timeout)
            .with_overall_timeout(self.overall_timeout);
        if let Some(ctx) = context {
            config = config.with_session_logger(ctx.session_logger.clone());
            if let Some(cancel_rx) = ctx.cancel_rx.clone() {
                config = config.with_cancel_rx(cancel_rx);
            }
        }
        let mut parser = CodexParser::new();

        let output = run_agent_process(cmd, &config, &mut parser, emitter).await?;
        Ok(output.into())
    }

    fn build_command(&self, prompt: &str, context: Option<&AgentContext>) -> Command {
        let mut cmd = Command::new(&self.config.command);

        // Check if we should resume an existing conversation
        let should_resume = self.config.session_persistence.enabled
            && context.is_some_and(|ctx| {
                ctx.resume_strategy == ResumeStrategy::ConversationResume
                    && ctx.conversation_id.is_some()
            });

        if should_resume {
            // Resume mode: codex exec resume [SESSION_ID] [PROMPT]
            // Find "exec" in args and add "resume" after it
            let conv_id = context.unwrap().conversation_id.as_ref().unwrap();
            for arg in &self.config.args {
                cmd.arg(arg);
                if arg == "exec" {
                    cmd.arg("resume");
                    cmd.arg(conv_id);
                }
            }
        } else {
            // Normal mode: codex exec [args...] [PROMPT]
            for arg in &self.config.args {
                cmd.arg(arg);
            }
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
            let context_suffix = if has_context { " (with context)" } else { "" };
            logger.log_line(
                "start",
                &format!("command: {}{}{}", self.config.command, args, context_suffix),
            );
            logger.log_line("prompt", &prompt.chars().take(200).collect::<String>());
        }
    }
}

#[cfg(test)]
#[path = "tests/agent_tests.rs"]
mod tests;
