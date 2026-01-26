//! Claude CLI output parser implementing the unified AgentStreamParser trait.
//!
//! This is the reference parser implementation that other agent parsers follow.
//! It parses the Claude CLI's stream-json output format.

use crate::agents::protocol::{AgentEvent, AgentStreamParser, AgentTokenUsage, ParseError};
use crate::tui::{TodoItem, TodoStatus};
use serde_json::Value;

use super::util::extract_bash_command;

/// Parser for Claude CLI JSON output implementing the unified AgentStreamParser trait.
#[derive(Debug, Clone, Default)]
pub struct ClaudeParser {
    last_message_type: Option<String>,
}

impl ClaudeParser {
    pub fn new() -> Self {
        Self {
            last_message_type: None,
        }
    }

    fn parse_json(&mut self, line: &str) -> Vec<AgentEvent> {
        let mut events = Vec::new();

        let json: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return events,
        };

        let msg_type = match json.get("type").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => return events,
        };

        match msg_type {
            "assistant" | "user" => {
                if msg_type == "assistant" && self.last_message_type.as_deref() == Some("user") {
                    events.push(AgentEvent::TurnCompleted);
                }
                self.last_message_type = Some(msg_type.to_string());

                if let Some(message) = json.get("message") {
                    if let Some(model) = message.get("model").and_then(|m| m.as_str()) {
                        if !model.is_empty() {
                            events.push(AgentEvent::ModelDetected(model.to_string()));
                        }
                    }

                    if let Some(stop) = message.get("stop_reason").and_then(|s| s.as_str()) {
                        events.push(AgentEvent::StopReason(stop.to_string()));
                    }

                    if let Some(usage) = message.get("usage") {
                        let token_usage = AgentTokenUsage {
                            input_tokens: usage
                                .get("input_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0),
                            output_tokens: usage
                                .get("output_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0),
                            cache_creation_tokens: usage
                                .get("cache_creation_input_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0),
                            cache_read_tokens: usage
                                .get("cache_read_input_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0),
                        };
                        events.push(AgentEvent::TokenUsage(token_usage));
                    }

                    if let Some(content) = message.get("content") {
                        if let Some(arr) = content.as_array() {
                            events.extend(Self::parse_content_array(arr));
                        }
                    }
                }
            }
            "content_block_start" => {
                if let Some(content_block) = json.get("content_block") {
                    if let Some(block_type) = content_block.get("type").and_then(|t| t.as_str()) {
                        if block_type == "tool_use" {
                            if let Some(name) = content_block.get("name").and_then(|n| n.as_str()) {
                                events.push(AgentEvent::ContentBlockStart {
                                    name: name.to_string(),
                                });
                            }
                        }
                    }
                }
            }
            "content_block_delta" => {
                if let Some(delta) = json.get("delta") {
                    if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                        for chunk in text.lines() {
                            if !chunk.trim().is_empty() {
                                events.push(AgentEvent::ContentDelta(chunk.to_string()));
                            }
                        }
                    }
                }
            }
            "result" => {
                let output = json.get("result").and_then(|r| r.as_str()).map(Into::into);
                let cost = json.get("total_cost_usd").and_then(|c| c.as_f64());
                let is_error = json
                    .get("is_error")
                    .and_then(|e| e.as_bool())
                    .unwrap_or(false);
                events.push(AgentEvent::Result {
                    output,
                    cost,
                    is_error,
                });
            }
            "system" => {
                // Capture conversation ID from init message
                // Format: {"type":"system","subtype":"init","session_id":"uuid",...}
                if json.get("subtype").and_then(|s| s.as_str()) == Some("init") {
                    if let Some(session_id) = json.get("session_id").and_then(|s| s.as_str()) {
                        events.push(AgentEvent::ConversationIdCaptured(session_id.to_string()));
                    }
                }
            }
            _ => {}
        }

        events
    }

    fn parse_content_array(arr: &[Value]) -> Vec<AgentEvent> {
        let mut events = Vec::new();

        for item in arr {
            if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                for chunk in text.lines() {
                    if !chunk.trim().is_empty() {
                        events.push(AgentEvent::TextContent(chunk.to_string()));
                    }
                }
            }

            if let Some(tool_type) = item.get("type").and_then(|t| t.as_str()) {
                if tool_type == "tool_use" {
                    if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                        if name == "TodoWrite" {
                            if let Some(input) = item.get("input") {
                                if let Some(todos) = input.get("todos").and_then(|t| t.as_array()) {
                                    let items: Vec<TodoItem> = todos
                                        .iter()
                                        .filter_map(|t| {
                                            Some(TodoItem {
                                                status: match t.get("status")?.as_str()? {
                                                    "pending" => TodoStatus::Pending,
                                                    "in_progress" => TodoStatus::InProgress,
                                                    "completed" => TodoStatus::Completed,
                                                    _ => return None,
                                                },
                                                active_form: t
                                                    .get("activeForm")?
                                                    .as_str()?
                                                    .to_string(),
                                            })
                                        })
                                        .collect();
                                    events.push(AgentEvent::TodosUpdate(items));
                                }
                            }
                        }

                        let display_name = if name == "Bash" {
                            item.get("input")
                                .and_then(|i| i.get("command"))
                                .and_then(|c| c.as_str())
                                .map(extract_bash_command)
                                .unwrap_or_else(|| "Bash".to_string())
                        } else {
                            name.to_string()
                        };

                        let input_preview = item
                            .get("input")
                            .map(|i| {
                                let s = i.to_string();
                                let preview: String = s.chars().take(100).collect();
                                if s.len() > 100 {
                                    format!("{}...", preview)
                                } else {
                                    preview
                                }
                            })
                            .unwrap_or_default();

                        // Extract tool_use_id from the content block if available
                        let tool_use_id = item
                            .get("id")
                            .and_then(|id| id.as_str())
                            .map(|s| s.to_string());

                        events.push(AgentEvent::ToolStarted {
                            display_name,
                            input_preview,
                            tool_use_id,
                        });
                    }
                }
                if tool_type == "tool_result" {
                    let tool_use_id = item
                        .get("tool_use_id")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    let is_error = item
                        .get("is_error")
                        .and_then(|e| e.as_bool())
                        .unwrap_or(false);

                    let (content_lines, has_more) =
                        if let Some(content) = item.get("content").and_then(|c| c.as_str()) {
                            let lines: Vec<String> =
                                content.lines().take(5).map(|s| s.to_string()).collect();
                            let has_more = content.lines().count() > 5;
                            (lines, has_more)
                        } else {
                            (Vec::new(), false)
                        };

                    events.push(AgentEvent::ToolResult {
                        tool_use_id,
                        is_error,
                        content_lines,
                        has_more,
                    });
                }
            }
        }

        events
    }
}

impl AgentStreamParser for ClaudeParser {
    fn parse_line(&mut self, line: &str) -> Result<Option<AgentEvent>, ParseError> {
        let events = self.parse_line_multi(line)?;
        Ok(events.into_iter().next())
    }

    fn parse_line_multi(&mut self, line: &str) -> Result<Vec<AgentEvent>, ParseError> {
        Ok(self.parse_json(line))
    }
}

#[cfg(test)]
#[path = "tests/parser_tests.rs"]
mod tests;
