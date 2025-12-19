use crate::tui::{Event, TokenUsage};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs::OpenOptions;
use std::io::Write;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

/// Extract the actual command from a bash command string
/// Handles env vars, command chains, and paths
fn extract_bash_command(cmd: &str) -> String {
    let cmd = cmd.trim();

    // Split by command separators and take the first/main command
    let first_cmd = cmd
        .split("&&")
        .next()
        .unwrap_or(cmd)
        .split("||")
        .next()
        .unwrap_or(cmd)
        .split(';')
        .next()
        .unwrap_or(cmd)
        .split('|')
        .next()
        .unwrap_or(cmd)
        .trim();

    // Split into tokens and skip env vars (FOO=bar)
    let tokens: Vec<&str> = first_cmd.split_whitespace().collect();

    for token in tokens {
        // Skip env var assignments
        if token.contains('=') && !token.starts_with('-') {
            continue;
        }
        // Skip common prefixes
        if token == "sudo" || token == "env" || token == "time" || token == "nice" {
            continue;
        }
        // Extract command name from path
        let cmd_name = token.rsplit('/').next().unwrap_or(token);
        // Return just the command, capitalize first letter for display
        return format!("Bash:{}", cmd_name);
    }

    "Bash".to_string()
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Fields are part of Claude API response, kept for completeness
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
        // Note: --dangerously-skip-permissions is needed for non-interactive use
        cmd.arg("-p")
            .arg(&self.prompt)
            .arg("--output-format")
            .arg("json")
            .arg("--dangerously-skip-permissions");

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

        // Parse JSON output - Claude outputs multiple JSON lines (NDJSON), find the result line
        let result: ClaudeResult = stdout
            .lines()
            .find(|line| line.contains("\"type\":\"result\""))
            .ok_or_else(|| anyhow::anyhow!("No result line found in claude output"))
            .and_then(|line| {
                serde_json::from_str(line)
                    .with_context(|| format!("Failed to parse result line as JSON: {}", line))
            })?;

        if result.is_error {
            anyhow::bail!("Claude returned an error: {}", result.result);
        }

        if let Some(cost) = result.total_cost_usd {
            eprintln!("[planning-agent] Cost: ${:.4}", cost);
        }

        Ok(result)
    }

    /// Execute with streaming output to a channel
    pub async fn execute_streaming(
        self,
        output_tx: mpsc::UnboundedSender<Event>,
    ) -> Result<ClaudeResult> {
        let mut cmd = Command::new("claude");

        // Non-interactive mode with streaming JSON output
        // Note: --verbose is required when using -p with stream-json
        // Note: --dangerously-skip-permissions is needed for non-interactive use
        cmd.arg("-p")
            .arg(&self.prompt)
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--dangerously-skip-permissions");

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

        // Capture output with streaming
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let _ = output_tx.send(Event::Output("[planning-agent] Invoking claude...".to_string()));

        // Set up log file with run ID
        let run_id = {
            use std::sync::OnceLock;
            static RUN_ID: OnceLock<String> = OnceLock::new();
            RUN_ID.get_or_init(|| {
                chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
            }).clone()
        };
        let log_path = std::env::current_dir()
            .unwrap_or_default()
            .join(format!(".planning-agent/claude-stream-{}.log", run_id));
        let _ = std::fs::create_dir_all(log_path.parent().unwrap_or(&std::path::PathBuf::from(".")));
        let mut log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .ok();

        if let Some(ref mut f) = log_file {
            let _ = writeln!(f, "\n=== New Claude invocation at {:?} ===", std::time::SystemTime::now());
            let _ = writeln!(f, "Prompt: {}", self.prompt.chars().take(200).collect::<String>());
        }

        let mut child = cmd
            .spawn()
            .context("Failed to spawn claude process")?;

        let stdout = child.stdout.take().context("Failed to get stdout")?;
        let stderr = child.stderr.take().context("Failed to get stderr")?;

        // Stream stdout lines
        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        let mut final_result: Option<ClaudeResult> = None;
        let mut all_output = String::new();

        // Read stdout and stderr concurrently
        loop {
            tokio::select! {
                line = stdout_reader.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            // Log raw line
                            if let Some(ref mut f) = log_file {
                                let _ = writeln!(f, "[stdout] {}", line);
                            }

                            // Send bytes received stat
                            let _ = output_tx.send(Event::BytesReceived(line.len()));

                            all_output.push_str(&line);
                            all_output.push('\n');

                            // Try to parse as JSON to check for result
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                                // Check if this is a content message
                                if let Some(msg_type) = json.get("type").and_then(|v| v.as_str()) {
                                    match msg_type {
                                        "assistant" | "user" => {
                                            // Extract message content for display
                                            if let Some(message) = json.get("message") {
                                                // Parse token usage
                                                if let Some(usage) = message.get("usage") {
                                                    let token_usage = TokenUsage {
                                                        input_tokens: usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                                                        output_tokens: usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                                                        cache_creation_tokens: usage.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                                                        cache_read_tokens: usage.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                                                    };
                                                    let _ = output_tx.send(Event::TokenUsage(token_usage));
                                                }
                                                if let Some(content) = message.get("content") {
                                                    if let Some(arr) = content.as_array() {
                                                        for item in arr {
                                                            // Handle text content
                                                            if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                                                // Split long text into multiple lines
                                                                for chunk in text.lines() {
                                                                    if !chunk.trim().is_empty() {
                                                                        let _ = output_tx.send(Event::Streaming(chunk.to_string()));
                                                                    }
                                                                }
                                                            }
                                                            // Handle tool_use content
                                                            if let Some(tool_type) = item.get("type").and_then(|t| t.as_str()) {
                                                                if tool_type == "tool_use" {
                                                                    if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                                                                        // For Bash, try to extract the actual command
                                                                        let display_name = if name == "Bash" {
                                                                            item.get("input")
                                                                                .and_then(|i| i.get("command"))
                                                                                .and_then(|c| c.as_str())
                                                                                .map(extract_bash_command)
                                                                                .unwrap_or_else(|| "Bash".to_string())
                                                                        } else {
                                                                            name.to_string()
                                                                        };

                                                                        // Send ToolStarted event
                                                                        let _ = output_tx.send(Event::ToolStarted(display_name.clone()));

                                                                        let input_preview = item.get("input")
                                                                            .map(|i| {
                                                                                let s = i.to_string();
                                                                                let preview: String = s.chars().take(100).collect();
                                                                                if s.len() > 100 { format!("{}...", preview) } else { preview }
                                                                            })
                                                                            .unwrap_or_default();
                                                                        let _ = output_tx.send(Event::Streaming(format!("[Tool: {}] {}", display_name, input_preview)));
                                                                    }
                                                                }
                                                                // Handle tool_result content
                                                                if tool_type == "tool_result" {
                                                                    // Try to get the tool name from tool_use_id or just clear the oldest
                                                                    if let Some(tool_use_id) = item.get("tool_use_id").and_then(|t| t.as_str()) {
                                                                        // Send a generic finish - we'll clear by position
                                                                        let _ = output_tx.send(Event::ToolFinished(tool_use_id.to_string()));
                                                                    }
                                                                    if let Some(content) = item.get("content").and_then(|c| c.as_str()) {
                                                                        // Split result into lines too
                                                                        let lines: Vec<&str> = content.lines().take(5).collect();
                                                                        for (i, line) in lines.iter().enumerate() {
                                                                            let prefix = if i == 0 { "[Result] " } else { "         " };
                                                                            let _ = output_tx.send(Event::Streaming(format!("{}{}", prefix, line)));
                                                                        }
                                                                        if content.lines().count() > 5 {
                                                                            let _ = output_tx.send(Event::Streaming("         ...".to_string()));
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        "content_block_start" => {
                                            // Handle content block start (e.g., tool calls starting)
                                            if let Some(content_block) = json.get("content_block") {
                                                if let Some(block_type) = content_block.get("type").and_then(|t| t.as_str()) {
                                                    if block_type == "tool_use" {
                                                        if let Some(name) = content_block.get("name").and_then(|n| n.as_str()) {
                                                            let _ = output_tx.send(Event::ToolStarted(name.to_string()));
                                                            let _ = output_tx.send(Event::Streaming(format!("[Tool: {}] starting...", name)));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        "content_block_stop" => {
                                            // A content block finished - we'll mark tool as done when we see the result
                                        }
                                        "content_block_delta" => {
                                            // Handle streaming text deltas
                                            if let Some(delta) = json.get("delta") {
                                                if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                                                    // Stream text as it comes in
                                                    for chunk in text.lines() {
                                                        if !chunk.trim().is_empty() {
                                                            let _ = output_tx.send(Event::Streaming(chunk.to_string()));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        "result" => {
                                            // This is the final result
                                            if let Ok(result) = serde_json::from_value::<ClaudeResult>(json) {
                                                final_result = Some(result);
                                            }
                                        }
                                        _ => {
                                            // Log other event types for debugging
                                            // let _ = output_tx.send(Event::Streaming(format!("[event: {}]", msg_type)));
                                        }
                                    }
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(e) => {
                            let _ = output_tx.send(Event::Output(format!("[error] Failed to read stdout: {}", e)));
                            break;
                        }
                    }
                }
                line = stderr_reader.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            if let Some(ref mut f) = log_file {
                                let _ = writeln!(f, "[stderr] {}", line);
                            }
                            let _ = output_tx.send(Event::Streaming(format!("[stderr] {}", line)));
                        }
                        Ok(None) => {}
                        Err(_) => {}
                    }
                }
            }
        }

        let status = child.wait().await.context("Failed to wait for claude process")?;

        if let Some(ref mut f) = log_file {
            let _ = writeln!(f, "[exit] status: {}", status);
        }

        if !status.success() {
            anyhow::bail!("Claude process exited with status {}", status);
        }

        let result = final_result.ok_or_else(|| anyhow::anyhow!("No result received from claude"))?;

        if result.is_error {
            anyhow::bail!("Claude returned an error: {}", result.result);
        }

        if let Some(cost) = result.total_cost_usd {
            let _ = output_tx.send(Event::Output(format!("[planning-agent] Cost: ${:.4}", cost)));
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
