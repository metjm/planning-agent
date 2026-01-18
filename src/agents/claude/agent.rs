use super::parser::ClaudeParser;
use crate::agents::log::AgentLogger;
use crate::agents::prompt::PreparedPrompt;
use crate::agents::runner::{run_agent_process, ContextEmitter, EventEmitter, RunnerConfig};
use crate::agents::{AgentContext, AgentResult};
use crate::config::AgentConfig;
use crate::state::ResumeStrategy;
use anyhow::Result;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;

const DEFAULT_ACTIVITY_TIMEOUT: Duration = Duration::from_secs(300);
const DEFAULT_OVERALL_TIMEOUT: Duration = Duration::from_secs(21600); // 6 hours

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

    fn build_command(
        &self,
        prepared: &PreparedPrompt,
        context: Option<&AgentContext>,
    ) -> Command {
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

    fn log_start(&self, logger: &Option<AgentLogger>, prepared: &PreparedPrompt, has_context: bool) {
        if let Some(ref logger) = logger {
            let args = if self.config.args.is_empty() {
                String::new()
            } else {
                format!(" {}", self.config.args.join(" "))
            };
            let suffix = if has_context { " (with context)" } else { "" };
            logger.log_line("start", &format!("command: {}{}{}", self.config.command, args, suffix));
            logger.log_line("prompt", &prepared.prompt.chars().take(200).collect::<String>());
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
mod tests {
    use super::*;
    use crate::agents::AgentContext;
    use crate::config::SessionPersistenceConfig;
    use crate::session_logger::SessionLogger;
    use crate::tui::SessionEventSender;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    fn make_agent(session_persistence_enabled: bool) -> ClaudeAgent {
        let config = AgentConfig {
            command: "claude".to_string(),
            args: vec!["-p".to_string()],
            allowed_tools: vec!["Read".to_string()],
            session_persistence: SessionPersistenceConfig {
                enabled: session_persistence_enabled,
                strategy: ResumeStrategy::ConversationResume,
            },
        };
        ClaudeAgent::new("claude".to_string(), config, PathBuf::from("."))
    }

    fn make_context(
        conversation_id: Option<String>,
        resume_strategy: ResumeStrategy,
    ) -> AgentContext {
        // Create a test session_id in the proper format
        let session_id = format!("test-{}", uuid::Uuid::new_v4());
        let session_logger = Arc::new(SessionLogger::new(&session_id).expect("test logger"));

        // Create a channel for the sender (we won't use it, just need to satisfy types)
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

    fn make_prepared_prompt() -> PreparedPrompt {
        PreparedPrompt {
            prompt: "test prompt".to_string(),
            system_prompt_arg: None,
            max_turns_arg: None,
        }
    }

    fn get_args(cmd: &Command) -> Vec<String> {
        let cmd_debug = format!("{:?}", cmd);
        // Parse args from debug output - crude but works for testing
        cmd_debug
            .split('"')
            .filter(|s| !s.is_empty() && !s.contains('=') && !s.contains('{') && !s.contains('}'))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != "," && s != " ")
            .collect()
    }

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

    #[test]
    fn test_build_command_with_resume_when_conversation_id_present() {
        let agent = make_agent(true);
        let prepared = make_prepared_prompt();
        let ctx = make_context(
            Some("abc-123-def".to_string()),
            ResumeStrategy::ConversationResume,
        );
        let cmd = agent.build_command(&prepared, Some(&ctx));
        let args = get_args(&cmd);

        // Should contain --resume followed by the conversation ID
        assert!(
            args.contains(&"--resume".to_string()),
            "Command should include --resume flag. Args: {:?}",
            args
        );
        assert!(
            args.contains(&"abc-123-def".to_string()),
            "Command should include conversation ID. Args: {:?}",
            args
        );
    }

    #[test]
    fn test_build_command_no_resume_when_stateless() {
        let agent = make_agent(true);
        let prepared = make_prepared_prompt();
        let ctx = make_context(
            Some("abc-123-def".to_string()),
            ResumeStrategy::Stateless, // Stateless strategy
        );
        let cmd = agent.build_command(&prepared, Some(&ctx));
        let args = get_args(&cmd);

        // Should NOT contain --resume
        assert!(
            !args.contains(&"--resume".to_string()),
            "Command should NOT include --resume with Stateless strategy. Args: {:?}",
            args
        );
    }

    #[test]
    fn test_build_command_no_resume_when_no_conversation_id() {
        let agent = make_agent(true);
        let prepared = make_prepared_prompt();
        let ctx = make_context(
            None, // No conversation ID yet
            ResumeStrategy::ConversationResume,
        );
        let cmd = agent.build_command(&prepared, Some(&ctx));
        let args = get_args(&cmd);

        // Should NOT contain --resume (no ID to resume)
        assert!(
            !args.contains(&"--resume".to_string()),
            "Command should NOT include --resume without conversation ID. Args: {:?}",
            args
        );
    }

    #[test]
    fn test_build_command_no_resume_when_persistence_disabled() {
        let agent = make_agent(false); // Persistence disabled
        let prepared = make_prepared_prompt();
        let ctx = make_context(
            Some("abc-123-def".to_string()),
            ResumeStrategy::ConversationResume,
        );
        let cmd = agent.build_command(&prepared, Some(&ctx));
        let args = get_args(&cmd);

        // Should NOT contain --resume (persistence disabled)
        assert!(
            !args.contains(&"--resume".to_string()),
            "Command should NOT include --resume when persistence disabled. Args: {:?}",
            args
        );
    }

    #[test]
    fn test_build_command_no_resume_when_no_context() {
        let agent = make_agent(true);
        let prepared = make_prepared_prompt();
        let cmd = agent.build_command(&prepared, None); // No context
        let args = get_args(&cmd);

        // Should NOT contain --resume (no context)
        assert!(
            !args.contains(&"--resume".to_string()),
            "Command should NOT include --resume without context. Args: {:?}",
            args
        );
    }
}
