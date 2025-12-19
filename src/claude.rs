use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Stdio;
use tokio::process::Command;

#[derive(Debug, Deserialize)]
pub struct ClaudeResult {
    #[serde(rename = "type")]
    pub result_type: Option<String>,
    pub subtype: Option<String>,
    pub result: String,
    pub is_error: bool,
    pub total_cost_usd: Option<f64>,
    pub num_turns: Option<u32>,
    pub session_id: Option<String>,
}

pub struct ClaudeInvocation {
    prompt: String,
    append_system_prompt: Option<String>,
    allowed_tools: Vec<String>,
    max_turns: Option<u32>,
    working_dir: Option<std::path::PathBuf>,
}

impl ClaudeInvocation {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            append_system_prompt: None,
            allowed_tools: Vec::new(),
            max_turns: None,
            working_dir: None,
        }
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.append_system_prompt = Some(prompt.into());
        self
    }

    pub fn with_allowed_tools(mut self, tools: Vec<String>) -> Self {
        self.allowed_tools = tools;
        self
    }

    pub fn with_max_turns(mut self, turns: u32) -> Self {
        self.max_turns = Some(turns);
        self
    }

    pub fn with_working_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.working_dir = Some(dir);
        self
    }

    pub async fn execute(self) -> Result<ClaudeResult> {
        let mut cmd = Command::new("claude");

        // Non-interactive mode with JSON output
        cmd.arg("-p")
            .arg(&self.prompt)
            .arg("--output-format")
            .arg("json");

        // Add system prompt if provided
        if let Some(ref system_prompt) = self.append_system_prompt {
            cmd.arg("--append-system-prompt").arg(system_prompt);
        }

        // Add allowed tools if provided
        if !self.allowed_tools.is_empty() {
            cmd.arg("--allowedTools").arg(self.allowed_tools.join(","));
        }

        // Add max turns if provided
        if let Some(turns) = self.max_turns {
            cmd.arg("--max-turns").arg(turns.to_string());
        }

        // Set working directory if provided
        if let Some(ref dir) = self.working_dir {
            cmd.current_dir(dir);
        }

        // Capture output
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped());

        eprintln!("[planning-agent] Invoking claude...");

        let output = cmd
            .spawn()
            .context("Failed to spawn claude process")?
            .wait_with_output()
            .await
            .context("Failed to wait for claude process")?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if !stderr.is_empty() {
            eprintln!("[planning-agent] Claude stderr: {}", stderr);
        }

        if !output.status.success() {
            anyhow::bail!(
                "Claude process exited with status {}: {}",
                output.status,
                stderr
            );
        }

        // Parse JSON output
        let result: ClaudeResult = serde_json::from_str(&stdout)
            .with_context(|| format!("Failed to parse claude output as JSON: {}", stdout))?;

        if result.is_error {
            anyhow::bail!("Claude returned an error: {}", result.result);
        }

        if let Some(cost) = result.total_cost_usd {
            eprintln!("[planning-agent] Cost: ${:.4}", cost);
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invocation_builder() {
        let inv = ClaudeInvocation::new("test prompt")
            .with_system_prompt("be helpful")
            .with_allowed_tools(vec!["Read".to_string(), "Write".to_string()])
            .with_max_turns(5);

        assert_eq!(inv.prompt, "test prompt");
        assert_eq!(inv.append_system_prompt, Some("be helpful".to_string()));
        assert_eq!(inv.allowed_tools, vec!["Read", "Write"]);
        assert_eq!(inv.max_turns, Some(5));
    }
}
