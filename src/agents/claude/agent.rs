
use super::log::AgentLogger;
use super::parser::{parse_json_line, ParsedEvent};
use super::{AgentContext, AgentResult};
use crate::config::AgentConfig;
use crate::state::ResumeStrategy;
use crate::tui::Event;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::Instant;

const DEFAULT_ACTIVITY_TIMEOUT: Duration = Duration::from_secs(300);

const DEFAULT_OVERALL_TIMEOUT: Duration = Duration::from_secs(1800);

const PROCESS_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

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

    pub fn name(&self) -> &str {
        &self.name
    }

    #[allow(dead_code)]
    pub fn with_activity_timeout(mut self, timeout: Duration) -> Self {
        self.activity_timeout = timeout;
        self
    }

    #[allow(dead_code)]
    pub fn with_overall_timeout(mut self, timeout: Duration) -> Self {
        self.overall_timeout = timeout;
        self
    }

    #[allow(dead_code)]
    pub async fn execute_streaming(
        &self,
        prompt: String,
        system_prompt: Option<String>,
        max_turns: Option<u32>,
        output_tx: mpsc::UnboundedSender<Event>,
    ) -> Result<AgentResult> {
        let sender = LegacyEventSender { tx: output_tx };
        self.execute_streaming_internal(prompt, system_prompt, max_turns, &sender, None::<&AgentContext>)
            .await
    }

    pub async fn execute_streaming_with_context(
        &self,
        prompt: String,
        system_prompt: Option<String>,
        max_turns: Option<u32>,
        context: AgentContext,
    ) -> Result<AgentResult> {
        let sender = ContextEventSender {
            context: context.clone(),
            agent_name: self.name.clone(),
        };
        self.execute_streaming_internal(prompt, system_prompt, max_turns, &sender, Some(&context))
            .await
    }

    async fn execute_streaming_internal(
        &self,
        prompt: String,
        system_prompt: Option<String>,
        max_turns: Option<u32>,
        sender: &dyn EventSender,
        context: Option<&AgentContext>,
    ) -> Result<AgentResult> {
        let logger = AgentLogger::new(&self.name, &self.working_dir);
        self.log_start(&logger, &prompt, &system_prompt, context.is_some());

        let mut cmd = self.build_command(&prompt, &system_prompt, max_turns, context);
        let mut child = cmd.spawn().context("Failed to spawn claude process")?;

        let stdout = child.stdout.take().context("Failed to get stdout")?;
        let stderr = child.stderr.take().context("Failed to get stderr")?;

        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        let mut final_result: Option<String> = None;
        let mut total_cost: Option<f64> = None;
        let mut is_error = false;
        let mut last_message_type: Option<String> = None;

        let start_time = Instant::now();
        let mut last_activity = Instant::now();

        self.log_timeout(&logger);
        sender.send_output("[agent:claude] Starting...".to_string());

        loop {
            if start_time.elapsed() > self.overall_timeout {
                self.handle_overall_timeout(&logger, sender, &mut child).await?;
            }

            let activity_deadline = last_activity + self.activity_timeout;

            tokio::select! {
                line = stdout_reader.next_line() => {
                    last_activity = Instant::now();
                    match line {
                        Ok(Some(line)) => {
                            if let Some(ref logger) = logger {
                                logger.log_line("stdout", &line);
                            }
                            sender.send_bytes_received(line.len());

                            let events = parse_json_line(&line, &mut last_message_type);
                            for event in events {
                                match event {
                                    ParsedEvent::TurnCompleted => sender.send_turn_completed(),
                                    ParsedEvent::ModelDetected(m) => sender.send_model_detected(m),
                                    ParsedEvent::StopReason(r) => sender.send_stop_reason(r),
                                    ParsedEvent::TokenUsage(u) => sender.send_token_usage(u),
                                    ParsedEvent::TextContent(t) => {
                                        sender.send_streaming(t.clone());
                                        sender.send_agent_message(t);
                                    }
                                    ParsedEvent::ToolStarted { display_name, input_preview, .. } => {
                                        sender.send_tool_started(display_name.clone());
                                        sender.send_streaming(format!("[Tool: {}] {}", display_name, input_preview));
                                    }
                                    ParsedEvent::ToolResult { tool_use_id, is_error: is_err, content_lines, has_more } => {
                                        sender.send_tool_result_received(tool_use_id.clone(), is_err);
                                        sender.send_tool_finished(tool_use_id);
                                        for (i, line) in content_lines.iter().enumerate() {
                                            let prefix = if i == 0 { "[Result] " } else { "         " };
                                            sender.send_streaming(format!("{}{}", prefix, line));
                                        }
                                        if has_more {
                                            sender.send_streaming("         ...".to_string());
                                        }
                                    }
                                    ParsedEvent::TodosUpdate(items) => sender.send_todos_update(items),
                                    ParsedEvent::ContentBlockStart { name } => {
                                        sender.send_tool_started(name.clone());
                                        sender.send_streaming(format!("[Tool: {}] starting...", name));
                                    }
                                    ParsedEvent::ContentDelta(t) => {
                                        sender.send_streaming(t.clone());
                                        sender.send_agent_message(t);
                                    }
                                    ParsedEvent::Result { output, cost, is_error: is_err } => {
                                        final_result = output;
                                        total_cost = cost;
                                        is_error = is_err;
                                    }
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(e) => {
                            sender.send_output(format!("[error] Failed to read stdout: {}", e));
                            break;
                        }
                    }
                }
                line = stderr_reader.next_line() => {
                    last_activity = Instant::now();
                    if let Ok(Some(line)) = line {
                        if let Some(ref logger) = logger {
                            logger.log_line("stderr", &line);
                        }
                        sender.send_streaming(format!("[stderr] {}", line));
                    }
                }
                _ = tokio::time::sleep_until(activity_deadline) => {
                    self.handle_activity_timeout(&logger, sender, &mut child).await?;
                }
            }
        }

        let status = self.wait_for_process(&logger, sender, &mut child).await?;

        if let Some(ref logger) = logger {
            logger.log_line("exit", &format!("status: {}", status));
        }

        if !status.success() {
            anyhow::bail!("Claude process exited with status {}", status);
        }

        if let Some(cost) = total_cost {
            sender.send_output(format!("[agent:claude] Cost: ${:.4}", cost));
        }

        Ok(AgentResult {
            output: final_result.unwrap_or_default(),
            is_error,
            cost_usd: total_cost,
        })
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

        cmd.current_dir(&self.working_dir);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

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

    async fn handle_overall_timeout(
        &self,
        logger: &Option<AgentLogger>,
        sender: &dyn EventSender,
        child: &mut tokio::process::Child,
    ) -> Result<()> {
        if let Some(ref logger) = logger {
            logger.log_line("timeout", "overall timeout triggered");
        }
        sender.send_output(format!(
            "[agent:claude] ERROR: Exceeded overall timeout of {:?}",
            self.overall_timeout
        ));
        let _ = child.kill().await;
        anyhow::bail!(
            "Claude invocation exceeded overall timeout of {:?}",
            self.overall_timeout
        );
    }

    async fn handle_activity_timeout(
        &self,
        logger: &Option<AgentLogger>,
        sender: &dyn EventSender,
        child: &mut tokio::process::Child,
    ) -> Result<()> {
        if let Some(ref logger) = logger {
            logger.log_line("timeout", "activity timeout triggered");
        }
        sender.send_output(format!(
            "[agent:claude] WARNING: No activity for {:?}, terminating...",
            self.activity_timeout
        ));
        let _ = child.kill().await;
        anyhow::bail!(
            "Claude subprocess became unresponsive (no output for {:?})",
            self.activity_timeout
        );
    }

    async fn wait_for_process(
        &self,
        logger: &Option<AgentLogger>,
        sender: &dyn EventSender,
        child: &mut tokio::process::Child,
    ) -> Result<std::process::ExitStatus> {
        match tokio::time::timeout(PROCESS_WAIT_TIMEOUT, child.wait()).await {
            Ok(Ok(status)) => Ok(status),
            Ok(Err(e)) => {
                if let Some(ref logger) = logger {
                    logger.log_line("timeout", &format!("failed to wait for process: {}", e));
                }
                anyhow::bail!("Failed to wait for claude process: {}", e);
            }
            Err(_) => {
                if let Some(ref logger) = logger {
                    logger.log_line(
                        "timeout",
                        &format!(
                            "process did not exit within {:?} after stream closed, force killing",
                            PROCESS_WAIT_TIMEOUT
                        ),
                    );
                }
                sender.send_output(format!(
                    "[agent:claude] WARNING: Process did not exit within {:?}, force killing...",
                    PROCESS_WAIT_TIMEOUT
                ));
                let _ = child.kill().await;
                anyhow::bail!(
                    "Claude process did not exit within {:?} after stream closed",
                    PROCESS_WAIT_TIMEOUT
                );
            }
        }
    }
}

trait EventSender: Send + Sync {
    fn send_output(&self, msg: String);
    fn send_streaming(&self, msg: String);
    fn send_bytes_received(&self, bytes: usize);
    fn send_turn_completed(&self);
    fn send_model_detected(&self, model: String);
    fn send_stop_reason(&self, reason: String);
    fn send_token_usage(&self, usage: crate::tui::TokenUsage);
    fn send_tool_started(&self, name: String);
    fn send_tool_finished(&self, id: String);
    fn send_tool_result_received(&self, id: String, is_error: bool);
    fn send_agent_message(&self, msg: String);
    fn send_todos_update(&self, items: Vec<crate::tui::TodoItem>);
}

struct LegacyEventSender {
    tx: mpsc::UnboundedSender<Event>,
}

impl EventSender for LegacyEventSender {
    fn send_output(&self, msg: String) {
        let _ = self.tx.send(Event::Output(msg));
    }
    fn send_streaming(&self, msg: String) {
        let _ = self.tx.send(Event::Streaming(msg));
    }
    fn send_bytes_received(&self, bytes: usize) {
        let _ = self.tx.send(Event::BytesReceived(bytes));
    }
    fn send_turn_completed(&self) {
        let _ = self.tx.send(Event::TurnCompleted);
    }
    fn send_model_detected(&self, model: String) {
        let _ = self.tx.send(Event::ModelDetected(model));
    }
    fn send_stop_reason(&self, reason: String) {
        let _ = self.tx.send(Event::StopReason(reason));
    }
    fn send_token_usage(&self, usage: crate::tui::TokenUsage) {
        let _ = self.tx.send(Event::TokenUsage(usage));
    }
    fn send_tool_started(&self, name: String) {
        let _ = self.tx.send(Event::ToolStarted(name));
    }
    fn send_tool_finished(&self, id: String) {
        let _ = self.tx.send(Event::ToolFinished(id));
    }
    fn send_tool_result_received(&self, id: String, is_error: bool) {
        let _ = self.tx.send(Event::ToolResultReceived {
            tool_id: id,
            is_error,
        });
    }
    fn send_agent_message(&self, _msg: String) {

    }
    fn send_todos_update(&self, _items: Vec<crate::tui::TodoItem>) {

    }
}

struct ContextEventSender {
    context: AgentContext,
    agent_name: String,
}

impl EventSender for ContextEventSender {
    fn send_output(&self, msg: String) {
        self.context.session_sender.send_output(msg);
    }
    fn send_streaming(&self, msg: String) {
        self.context.session_sender.send_streaming(msg);
    }
    fn send_bytes_received(&self, bytes: usize) {
        self.context.session_sender.send_bytes_received(bytes);
    }
    fn send_turn_completed(&self) {
        self.context.session_sender.send_turn_completed();
    }
    fn send_model_detected(&self, model: String) {
        self.context.session_sender.send_model_detected(model);
    }
    fn send_stop_reason(&self, reason: String) {
        self.context.session_sender.send_stop_reason(reason);
    }
    fn send_token_usage(&self, usage: crate::tui::TokenUsage) {
        self.context.session_sender.send_token_usage(usage);
    }
    fn send_tool_started(&self, name: String) {
        self.context.session_sender.send_tool_started(name);
    }
    fn send_tool_finished(&self, id: String) {
        self.context.session_sender.send_tool_finished(id);
    }
    fn send_tool_result_received(&self, id: String, is_error: bool) {
        self.context
            .session_sender
            .send_tool_result_received(id, is_error);
    }
    fn send_agent_message(&self, msg: String) {
        self.context.session_sender.send_agent_message(
            self.agent_name.clone(),
            self.context.phase.clone(),
            msg,
        );
    }
    fn send_todos_update(&self, items: Vec<crate::tui::TodoItem>) {
        self.context
            .session_sender
            .send_todos_update(self.agent_name.clone(), items);
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
