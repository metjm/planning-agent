use super::parser::CodexParser;
use crate::agents::log::AgentLogger;
use crate::agents::prompt::PreparedPrompt;
use crate::agents::runner::{run_agent_process, ContextEmitter, EventEmitter, RunnerConfig};
use crate::agents::{AgentContext, AgentResult};
use crate::config::AgentConfig;
use crate::mcp::McpServerConfig;
use crate::state::ResumeStrategy;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

const DEFAULT_ACTIVITY_TIMEOUT: Duration = Duration::from_secs(300);
const DEFAULT_OVERALL_TIMEOUT: Duration = Duration::from_secs(21600); // 6 hours

/// RAII guard for temporary Codex config directory
/// Cleans up the directory when dropped
struct TempCodexConfigDir {
    path: PathBuf,
}

impl TempCodexConfigDir {
    /// Create a new temp config directory with MCP settings
    /// Copies the real ~/.codex directory and adds MCP config on top
    fn new(mcp_config: &McpServerConfig) -> Result<Self> {
        let uuid = mcp_config
            .server_name
            .strip_prefix("planning-agent-review-")
            .unwrap_or(&mcp_config.server_name);
        let base_path = std::env::temp_dir().join(format!("codex-mcp-{}", uuid));
        let codex_dir = base_path.join(".codex");

        // Copy the real ~/.codex directory to preserve auth, skills, etc.
        if let Some(real_home) = std::env::var_os("HOME") {
            let real_codex_dir = PathBuf::from(real_home).join(".codex");
            if real_codex_dir.exists() {
                Self::copy_dir_recursive(&real_codex_dir, &codex_dir)?;
            } else {
                std::fs::create_dir_all(&codex_dir)?;
            }
        } else {
            std::fs::create_dir_all(&codex_dir)?;
        }

        // Append MCP config to the config.toml file
        let config_path = codex_dir.join("config.toml");
        let mut config_content = std::fs::read_to_string(&config_path).unwrap_or_default();
        config_content.push_str("\n\n# MCP server injected by planning-agent\n");
        config_content.push_str(&mcp_config.to_codex_config_toml());
        std::fs::write(&config_path, config_content)?;

        Ok(Self { path: base_path })
    }

    /// Recursively copy a directory
    fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if src_path.is_dir() {
                Self::copy_dir_recursive(&src_path, &dst_path)?;
            } else {
                std::fs::copy(&src_path, &dst_path)?;
            }
        }
        Ok(())
    }

    /// Get the path to use as HOME for codex
    fn home_dir(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempCodexConfigDir {
    fn drop(&mut self) {
        // Best-effort cleanup
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

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
        mcp_config: Option<&McpServerConfig>,
    ) -> Result<AgentResult> {
        let emitter = ContextEmitter::new(context.clone(), self.name.clone());
        self.execute_streaming_internal(prepared, &emitter, Some(&context), mcp_config).await
    }

    async fn execute_streaming_internal(
        &self,
        prepared: PreparedPrompt,
        emitter: &dyn EventEmitter,
        context: Option<&AgentContext>,
        mcp_config: Option<&McpServerConfig>,
    ) -> Result<AgentResult> {
        let logger = context.map(|ctx| AgentLogger::new(&self.name, ctx.session_logger.clone()));
        self.log_start(&logger, &prepared.prompt, context.is_some(), mcp_config.is_some());

        // Create temp config dir if using MCP (will be cleaned up when dropped)
        let _temp_config = match mcp_config {
            Some(mcp) => Some(TempCodexConfigDir::new(mcp)?),
            None => None,
        };

        let cmd = self.build_command(&prepared.prompt, context, _temp_config.as_ref());
        let mut config = RunnerConfig::new(self.name.clone(), self.working_dir.clone())
            .with_activity_timeout(self.activity_timeout)
            .with_overall_timeout(self.overall_timeout);
        if let Some(ctx) = context {
            config = config.with_session_logger(ctx.session_logger.clone());
        }
        let mut parser = CodexParser::new();

        let output = run_agent_process(cmd, &config, &mut parser, emitter).await?;
        // _temp_config dropped here, cleaning up the temp directory
        Ok(output.into())
    }

    fn build_command(
        &self,
        prompt: &str,
        context: Option<&AgentContext>,
        temp_config: Option<&TempCodexConfigDir>,
    ) -> Command {
        let mut cmd = Command::new(&self.config.command);

        // Set HOME to temp directory if using MCP
        // Codex reads config from ~/.codex/config.toml which contains the MCP server settings
        if let Some(temp) = temp_config {
            cmd.env("HOME", temp.home_dir());
        }

        // Check if we should resume an existing conversation
        let should_resume = self.config.session_persistence.enabled
            && context.map_or(false, |ctx| {
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

    fn log_start(&self, logger: &Option<AgentLogger>, prompt: &str, has_context: bool, has_mcp: bool) {
        if let Some(ref logger) = logger {
            let args = if self.config.args.is_empty() {
                String::new()
            } else {
                format!(" {}", self.config.args.join(" "))
            };
            let context_suffix = if has_context { " (with context)" } else { "" };
            let mcp_suffix = if has_mcp { " (with MCP)" } else { "" };
            logger.log_line("start", &format!("command: {}{}{}{}", self.config.command, args, context_suffix, mcp_suffix));
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

    fn make_agent(session_persistence_enabled: bool) -> CodexAgent {
        let config = AgentConfig {
            command: "codex".to_string(),
            args: vec!["exec".to_string(), "--json".to_string()],
            allowed_tools: vec![],
            session_persistence: SessionPersistenceConfig {
                enabled: session_persistence_enabled,
                strategy: ResumeStrategy::ConversationResume,
            },
        };
        CodexAgent::new("codex".to_string(), config, PathBuf::from("."))
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

    #[test]
    fn test_build_command_with_resume_when_conversation_id_present() {
        let agent = make_agent(true);
        let ctx = make_context(
            Some("019bc838-8e90-7052-b458-3615bee3647a".to_string()),
            ResumeStrategy::ConversationResume,
        );
        let cmd = agent.build_command("test prompt", Some(&ctx), None);
        let args = get_args(&cmd);

        // Should contain "exec resume [session_id]" sequence
        assert!(
            args.contains(&"resume".to_string()),
            "Command should include resume subcommand. Args: {:?}",
            args
        );
        assert!(
            args.contains(&"019bc838-8e90-7052-b458-3615bee3647a".to_string()),
            "Command should include conversation ID. Args: {:?}",
            args
        );
    }

    #[test]
    fn test_build_command_no_resume_when_stateless() {
        let agent = make_agent(true);
        let ctx = make_context(
            Some("019bc838-8e90-7052-b458-3615bee3647a".to_string()),
            ResumeStrategy::Stateless,
        );
        let cmd = agent.build_command("test prompt", Some(&ctx), None);
        let args = get_args(&cmd);

        assert!(
            !args.contains(&"resume".to_string()),
            "Command should NOT include resume with Stateless strategy. Args: {:?}",
            args
        );
    }

    #[test]
    fn test_build_command_no_resume_when_no_conversation_id() {
        let agent = make_agent(true);
        let ctx = make_context(None, ResumeStrategy::ConversationResume);
        let cmd = agent.build_command("test prompt", Some(&ctx), None);
        let args = get_args(&cmd);

        assert!(
            !args.contains(&"resume".to_string()),
            "Command should NOT include resume without conversation ID. Args: {:?}",
            args
        );
    }

    #[test]
    fn test_build_command_no_resume_when_persistence_disabled() {
        let agent = make_agent(false);
        let ctx = make_context(
            Some("019bc838-8e90-7052-b458-3615bee3647a".to_string()),
            ResumeStrategy::ConversationResume,
        );
        let cmd = agent.build_command("test prompt", Some(&ctx), None);
        let args = get_args(&cmd);

        assert!(
            !args.contains(&"resume".to_string()),
            "Command should NOT include resume when persistence disabled. Args: {:?}",
            args
        );
    }
}
