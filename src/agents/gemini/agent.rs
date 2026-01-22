use super::parser::GeminiParser;
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
pub struct GeminiAgent {
    name: String,
    config: AgentConfig,
    working_dir: PathBuf,
    activity_timeout: Duration,
    overall_timeout: Duration,
}

impl GeminiAgent {
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
    /// The PreparedPrompt already has system_prompt merged into the prompt for Gemini.
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
        let mut parser = GeminiParser::new();

        let output = run_agent_process(cmd, &config, &mut parser, emitter).await?;
        Ok(output.into())
    }

    fn build_command(&self, prompt: &str, context: Option<&AgentContext>) -> Command {
        let mut cmd = Command::new(&self.config.command);

        // Add --resume if we have a conversation ID and session persistence is enabled
        if self.config.session_persistence.enabled {
            if let Some(ctx) = context {
                if ctx.resume_strategy == ResumeStrategy::ConversationResume {
                    if let Some(ref conv_id) = ctx.conversation_id {
                        // Gemini accepts UUID directly for --resume
                        cmd.arg("--resume").arg(conv_id);
                    }
                }
            }
        }

        // Add the regular config args
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
mod tests {
    use super::*;
    use crate::agents::AgentContext;
    use crate::config::SessionPersistenceConfig;
    use crate::session_logger::SessionLogger;
    use crate::tui::SessionEventSender;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    fn make_agent(session_persistence_enabled: bool) -> GeminiAgent {
        let config = AgentConfig {
            command: "gemini".to_string(),
            args: vec!["-o".to_string(), "json".to_string()],
            allowed_tools: vec![],
            session_persistence: SessionPersistenceConfig {
                enabled: session_persistence_enabled,
                strategy: ResumeStrategy::ConversationResume,
            },
        };
        GeminiAgent::new("gemini".to_string(), config, PathBuf::from("."))
    }

    fn make_context(
        conversation_id: Option<String>,
        resume_strategy: ResumeStrategy,
    ) -> AgentContext {
        let session_id = format!("test-{}", uuid::Uuid::new_v4());
        let session_logger = Arc::new(SessionLogger::new(&session_id).expect("test logger"));
        let (tx, _rx) = mpsc::unbounded_channel();
        let session_sender = SessionEventSender::new(0, 0, tx);

        AgentContext {
            session_sender,
            phase: "Testing".to_string(),
            conversation_id,
            resume_strategy,
            cancel_rx: None,
            session_logger,
        }
    }

    fn get_args(cmd: &Command) -> Vec<String> {
        let cmd_debug = format!("{:?}", cmd);
        cmd_debug
            .split('"')
            .filter(|s| !s.is_empty() && !s.contains('=') && !s.contains('{') && !s.contains('}'))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != "," && s != " ")
            .collect()
    }

    #[test]
    fn test_gemini_agent_new() {
        let config = AgentConfig {
            command: "gemini".to_string(),
            args: vec![
                "-p".to_string(),
                "--output-format".to_string(),
                "json".to_string(),
            ],
            allowed_tools: vec![],
            session_persistence: SessionPersistenceConfig::default(),
        };
        let agent = GeminiAgent::new("gemini".to_string(), config, PathBuf::from("."));
        assert_eq!(agent.activity_timeout, DEFAULT_ACTIVITY_TIMEOUT);
        assert_eq!(agent.overall_timeout, DEFAULT_OVERALL_TIMEOUT);
    }

    #[test]
    fn test_build_command_with_resume_when_conversation_id_present() {
        let agent = make_agent(true);
        let ctx = make_context(
            Some("4e2f5f4f-c181-417a-855f-291bf3e9e515".to_string()),
            ResumeStrategy::ConversationResume,
        );
        let cmd = agent.build_command("test prompt", Some(&ctx));
        let args = get_args(&cmd);

        assert!(
            args.contains(&"--resume".to_string()),
            "Command should include --resume flag. Args: {:?}",
            args
        );
        assert!(
            args.contains(&"4e2f5f4f-c181-417a-855f-291bf3e9e515".to_string()),
            "Command should include conversation ID. Args: {:?}",
            args
        );
    }

    #[test]
    fn test_build_command_no_resume_when_stateless() {
        let agent = make_agent(true);
        let ctx = make_context(
            Some("4e2f5f4f-c181-417a-855f-291bf3e9e515".to_string()),
            ResumeStrategy::Stateless,
        );
        let cmd = agent.build_command("test prompt", Some(&ctx));
        let args = get_args(&cmd);

        assert!(
            !args.contains(&"--resume".to_string()),
            "Command should NOT include --resume with Stateless strategy. Args: {:?}",
            args
        );
    }

    #[test]
    fn test_build_command_no_resume_when_no_conversation_id() {
        let agent = make_agent(true);
        let ctx = make_context(None, ResumeStrategy::ConversationResume);
        let cmd = agent.build_command("test prompt", Some(&ctx));
        let args = get_args(&cmd);

        assert!(
            !args.contains(&"--resume".to_string()),
            "Command should NOT include --resume without conversation ID. Args: {:?}",
            args
        );
    }

    #[test]
    fn test_build_command_no_resume_when_persistence_disabled() {
        let agent = make_agent(false);
        let ctx = make_context(
            Some("4e2f5f4f-c181-417a-855f-291bf3e9e515".to_string()),
            ResumeStrategy::ConversationResume,
        );
        let cmd = agent.build_command("test prompt", Some(&ctx));
        let args = get_args(&cmd);

        assert!(
            !args.contains(&"--resume".to_string()),
            "Command should NOT include --resume when persistence disabled. Args: {:?}",
            args
        );
    }
}
