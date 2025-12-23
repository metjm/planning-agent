
use crate::tui::{TodoItem, TodoStatus, TokenUsage};
use serde_json::Value;

use super::util::extract_bash_command;

pub enum ParsedEvent {

    TurnCompleted,

    ModelDetected(String),

    StopReason(String),

    TokenUsage(TokenUsage),

    TextContent(String),

    ToolStarted {
        name: String,
        display_name: String,
        input_preview: String,
    },

    ToolResult {
        tool_use_id: String,
        is_error: bool,
        content_lines: Vec<String>,
        has_more: bool,
    },

    TodosUpdate(Vec<TodoItem>),

    ContentBlockStart { name: String },

    ContentDelta(String),

    Result {
        output: Option<String>,
        cost: Option<f64>,
        is_error: bool,
    },
}

pub fn parse_json_line(line: &str, last_message_type: &mut Option<String>) -> Vec<ParsedEvent> {
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

            if msg_type == "assistant" && last_message_type.as_deref() == Some("user") {
                events.push(ParsedEvent::TurnCompleted);
            }
            *last_message_type = Some(msg_type.to_string());

            if let Some(message) = json.get("message") {

                if let Some(model) = message.get("model").and_then(|m| m.as_str()) {
                    if !model.is_empty() {
                        events.push(ParsedEvent::ModelDetected(model.to_string()));
                    }
                }

                if let Some(stop) = message.get("stop_reason").and_then(|s| s.as_str()) {
                    events.push(ParsedEvent::StopReason(stop.to_string()));
                }

                if let Some(usage) = message.get("usage") {
                    let token_usage = TokenUsage {
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
                    events.push(ParsedEvent::TokenUsage(token_usage));
                }

                if let Some(content) = message.get("content") {
                    if let Some(arr) = content.as_array() {
                        events.extend(parse_content_array(arr));
                    }
                }
            }
        }
        "content_block_start" => {
            if let Some(content_block) = json.get("content_block") {
                if let Some(block_type) = content_block.get("type").and_then(|t| t.as_str()) {
                    if block_type == "tool_use" {
                        if let Some(name) = content_block.get("name").and_then(|n| n.as_str()) {
                            events.push(ParsedEvent::ContentBlockStart {
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
                            events.push(ParsedEvent::ContentDelta(chunk.to_string()));
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
            events.push(ParsedEvent::Result {
                output,
                cost,
                is_error,
            });
        }
        _ => {}
    }

    events
}

fn parse_content_array(arr: &[Value]) -> Vec<ParsedEvent> {
    let mut events = Vec::new();

    for item in arr {

        if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
            for chunk in text.lines() {
                if !chunk.trim().is_empty() {
                    events.push(ParsedEvent::TextContent(chunk.to_string()));
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
                                            content: t.get("content")?.as_str()?.to_string(),
                                            status: match t.get("status")?.as_str()? {
                                                "pending" => TodoStatus::Pending,
                                                "in_progress" => TodoStatus::InProgress,
                                                "completed" => TodoStatus::Completed,
                                                _ => return None,
                                            },
                                            active_form: t.get("activeForm")?.as_str()?.to_string(),
                                        })
                                    })
                                    .collect();
                                events.push(ParsedEvent::TodosUpdate(items));
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

                    events.push(ParsedEvent::ToolStarted {
                        name: name.to_string(),
                        display_name,
                        input_preview,
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

                let (content_lines, has_more) = if let Some(content) =
                    item.get("content").and_then(|c| c.as_str())
                {
                    let lines: Vec<String> =
                        content.lines().take(5).map(|s| s.to_string()).collect();
                    let has_more = content.lines().count() > 5;
                    (lines, has_more)
                } else {
                    (Vec::new(), false)
                };

                events.push(ParsedEvent::ToolResult {
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
