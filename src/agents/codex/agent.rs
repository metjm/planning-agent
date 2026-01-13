use super::parser::CodexParser;
use crate::agents::log::AgentLogger;
use crate::agents::runner::{run_agent_process, ContextEmitter, EventEmitter, RunnerConfig};
use crate::agents::{AgentContext, AgentResult};
use crate::config::AgentConfig;
use crate::mcp::McpServerConfig;
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

    pub async fn execute_streaming_with_context(
        &self,
        prompt: String,
        _system_prompt: Option<String>,
        _max_turns: Option<u32>,
        context: AgentContext,
    ) -> Result<AgentResult> {
        let emitter = ContextEmitter::new(context.clone(), self.name.clone());
        self.execute_streaming_internal(prompt, &emitter, true, None).await
    }

    /// Execute with MCP config for review feedback collection
    pub async fn execute_streaming_with_mcp(
        &self,
        prompt: String,
        _system_prompt: Option<String>,
        _max_turns: Option<u32>,
        context: AgentContext,
        mcp_config: &McpServerConfig,
    ) -> Result<AgentResult> {
        let emitter = ContextEmitter::new(context.clone(), self.name.clone());
        self.execute_streaming_internal(prompt, &emitter, true, Some(mcp_config)).await
    }

    async fn execute_streaming_internal(
        &self,
        prompt: String,
        emitter: &dyn EventEmitter,
        has_context: bool,
        mcp_config: Option<&McpServerConfig>,
    ) -> Result<AgentResult> {
        let logger = AgentLogger::new(&self.name, &self.working_dir);
        self.log_start(&logger, &prompt, has_context, mcp_config.is_some());

        // Create temp config dir if using MCP (will be cleaned up when dropped)
        let _temp_config = match mcp_config {
            Some(mcp) => Some(TempCodexConfigDir::new(mcp)?),
            None => None,
        };

        let cmd = self.build_command(&prompt, mcp_config, _temp_config.as_ref());
        let config = RunnerConfig::new(self.name.clone(), self.working_dir.clone())
            .with_activity_timeout(self.activity_timeout)
            .with_overall_timeout(self.overall_timeout);
        let mut parser = CodexParser::new();

        let output = run_agent_process(cmd, &config, &mut parser, emitter).await?;
        // _temp_config dropped here, cleaning up the temp directory
        Ok(output.into())
    }

    fn build_command(
        &self,
        prompt: &str,
        mcp_config: Option<&McpServerConfig>,
        temp_config: Option<&TempCodexConfigDir>,
    ) -> Command {
        let mut cmd = Command::new(&self.config.command);

        // Set HOME to temp directory if using MCP
        // Codex reads config from ~/.codex/config.toml which contains the MCP server settings
        // No --enable flag needed - MCP is configured via config.toml, not a feature flag
        if let Some(temp) = temp_config {
            cmd.env("HOME", temp.home_dir());
        }
        let _ = mcp_config; // MCP config is used via temp_config's config.toml

        // Add the regular config args (including subcommand like "exec")
        for arg in &self.config.args {
            cmd.arg(arg);
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
