//! Codex CLI output parser implementing the unified AgentStreamParser trait.
//!
//! Parses JSON line output from the Codex CLI tool and converts it to
//! unified AgentEvent types.

use crate::agents::protocol::{AgentEvent, AgentStreamParser, ParseError};
use serde_json::Value;

/// Parser for Codex CLI JSON output.
///
/// The Codex CLI emits JSON lines with various event types including:
/// - `message`, `content`, `text` - text content events
/// - `item.completed`, `item.delta` - structured item events
/// - `function_call`, `tool_call` - tool invocation events
/// - `function_result`, `tool_result` - tool result events
/// - `done`, `complete`, `finished` - completion events
/// - `error` - error events
#[derive(Debug, Clone, Default)]
pub struct CodexParser {
    // Track if we've seen any events for potential multi-turn support
    _event_count: usize,
}

impl CodexParser {
    pub fn new() -> Self {
        Self { _event_count: 0 }
    }

    /// Parse a JSON value into AgentEvents
    fn parse_json(&mut self, json: &Value) -> Vec<AgentEvent> {
        let mut events = Vec::new();

        // Try to get event type
        if let Some(event_type) = json.get("type").and_then(|v| v.as_str()) {
            match event_type {
                "thread.started" => {
                    // Capture thread_id for conversation resume
                    if let Some(thread_id) = json.get("thread_id").and_then(|v| v.as_str()) {
                        events.push(AgentEvent::ConversationIdCaptured(thread_id.to_string()));
                    }
                }
                "message" | "content" | "text" => {
                    if let Some(content) = self.extract_text_content(json) {
                        events.push(AgentEvent::TextContent(content));
                    }
                }
                "item.started" => {
                    // Handle command_execution tool start events
                    if let Some(item) = json.get("item") {
                        let item_type = item.get("type").and_then(|v| v.as_str());
                        if item_type == Some("command_execution") {
                            // Extract command for display name
                            let command = item
                                .get("command")
                                .and_then(|c| c.as_str())
                                .unwrap_or("command");
                            let display_name = Self::truncate_command(command, 50);
                            let input_preview = Self::truncate_command(command, 100);

                            // Extract item.id for tool correlation
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
                }
                "item.completed" | "item.delta" => {
                    if let Some(item) = json.get("item") {
                        let item_type = item.get("type").and_then(|v| v.as_str());

                        // Handle command_execution completion events
                        if item_type == Some("command_execution") {
                            let tool_use_id = item
                                .get("id")
                                .and_then(|id| id.as_str())
                                .unwrap_or("")
                                .to_string();

                            // Check for error: exit_code != 0 or status == "failed"
                            let exit_code = item.get("exit_code").and_then(|c| c.as_i64());
                            let status = item.get("status").and_then(|s| s.as_str());
                            let is_error = exit_code.map(|c| c != 0).unwrap_or(false)
                                || status == Some("failed");

                            // Extract output content
                            let content_lines = if let Some(output) =
                                item.get("aggregated_output").and_then(|o| o.as_str())
                            {
                                output.lines().take(5).map(|l| l.to_string()).collect()
                            } else {
                                vec![]
                            };

                            let has_more = item
                                .get("aggregated_output")
                                .and_then(|o| o.as_str())
                                .map(|s| s.lines().count() > 5)
                                .unwrap_or(false);

                            events.push(AgentEvent::ToolResult {
                                tool_use_id,
                                is_error,
                                content_lines,
                                has_more,
                            });
                        }

                        // Handle message-type items (existing logic)
                        let is_message = matches!(
                            item_type,
                            Some("agent_message")
                                | Some("message")
                                | Some("assistant_message")
                                | Some("final_message")
                                | Some("text")
                                | Some("reasoning")
                        );
                        if is_message {
                            if let Some(content) = item
                                .get("text")
                                .or_else(|| item.get("delta"))
                                .or_else(|| item.get("content"))
                                .and_then(|c| c.as_str())
                            {
                                events.push(AgentEvent::TextContent(content.to_string()));
                            }
                        }
                    }
                }
                "function_call" | "tool_call" => {
                    if let Some(name) = json
                        .get("name")
                        .or_else(|| json.get("function").and_then(|f| f.get("name")))
                        .and_then(|n| n.as_str())
                    {
                        // Extract input preview if available
                        let input_preview = json
                            .get("arguments")
                            .or_else(|| json.get("input"))
                            .map(|v| {
                                let s = v.to_string();
                                if s.len() > 100 {
                                    format!("{}...", s.get(..100).unwrap_or(&s))
                                } else {
                                    s
                                }
                            })
                            .unwrap_or_default();

                        // Extract tool_use_id from call_id or id field
                        let tool_use_id = json
                            .get("call_id")
                            .or_else(|| json.get("id"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        events.push(AgentEvent::ToolStarted {
                            display_name: name.to_string(),
                            input_preview,
                            tool_use_id,
                        });
                    }
                }
                "function_result" | "tool_result" => {
                    let tool_id = json
                        .get("call_id")
                        .or_else(|| json.get("id"))
                        .and_then(|i| i.as_str())
                        .unwrap_or("")
                        .to_string();

                    let is_error = json
                        .get("is_error")
                        .and_then(|e| e.as_bool())
                        .unwrap_or(false);

                    // Extract result content
                    let content_lines =
                        if let Some(content) = json.get("output").or_else(|| json.get("result")) {
                            if let Some(s) = content.as_str() {
                                s.lines().take(5).map(|l| l.to_string()).collect()
                            } else {
                                vec![content.to_string()]
                            }
                        } else {
                            vec![]
                        };

                    let has_more = json
                        .get("output")
                        .or_else(|| json.get("result"))
                        .and_then(|c| c.as_str())
                        .map(|s| s.lines().count() > 5)
                        .unwrap_or(false);

                    events.push(AgentEvent::ToolResult {
                        tool_use_id: tool_id,
                        is_error,
                        content_lines,
                        has_more,
                    });
                }
                "done" | "complete" | "finished" => {
                    let output = json
                        .get("message")
                        .or_else(|| json.get("result"))
                        .or_else(|| json.get("output"))
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string());

                    let cost = json.get("cost_usd").and_then(|c| c.as_f64());

                    events.push(AgentEvent::Result {
                        output,
                        cost,
                        is_error: false,
                    });
                }
                "error" => {
                    let message = json
                        .get("message")
                        .or_else(|| json.get("error"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("Unknown error")
                        .to_string();

                    events.push(AgentEvent::Error(message));
                }
                _ => {
                    // Unknown event type - try to extract content anyway
                    if let Some(content) = self.extract_text_content(json) {
                        events.push(AgentEvent::TextContent(content));
                    }
                }
            }
        } else {
            // No type field - try to extract content directly
            if let Some(content) = self.extract_text_content(json) {
                events.push(AgentEvent::TextContent(content));
            }
        }

        events
    }

    /// Extract text content from various possible JSON field names
    fn extract_text_content(&self, json: &Value) -> Option<String> {
        json.get("content")
            .or_else(|| json.get("text"))
            .or_else(|| json.get("message"))
            .or_else(|| json.get("output"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string())
    }

    /// Truncate a command string to a maximum length for display
    /// Extracts the core command from bash wrapper if present
    fn truncate_command(command: &str, max_len: usize) -> String {
        // Strip common bash wrapper patterns like "/bin/bash -lc 'cmd'" or "/bin/bash -c 'cmd'"
        let core_command = command
            .strip_prefix("/bin/bash -lc ")
            .or_else(|| command.strip_prefix("/bin/bash -c "))
            .or_else(|| command.strip_prefix("bash -lc "))
            .or_else(|| command.strip_prefix("bash -c "))
            .unwrap_or(command)
            .trim_matches('\'')
            .trim_matches('"');

        if core_command.len() > max_len {
            format!(
                "{}...",
                core_command.get(..max_len.saturating_sub(3)).unwrap_or("")
            )
        } else {
            core_command.to_string()
        }
    }
}

impl AgentStreamParser for CodexParser {
    fn parse_line(&mut self, line: &str) -> Result<Option<AgentEvent>, ParseError> {
        // Try to parse multiple events and return the first one
        let events = self.parse_line_multi(line)?;
        Ok(events.into_iter().next())
    }

    fn parse_line_multi(&mut self, line: &str) -> Result<Vec<AgentEvent>, ParseError> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(vec![]);
        }

        // Try to parse as JSON
        match serde_json::from_str::<Value>(trimmed) {
            Ok(json) => {
                self._event_count += 1;
                Ok(self.parse_json(&json))
            }
            Err(_) => {
                // Not valid JSON - treat as raw text content
                // This handles non-JSON output from the CLI
                Ok(vec![AgentEvent::TextContent(line.to_string())])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_message_event() {
        let mut parser = CodexParser::new();
        let line = r#"{"type": "message", "content": "Hello, world!"}"#;
        let events = parser.parse_line_multi(line).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::TextContent(content) => assert_eq!(content, "Hello, world!"),
            _ => panic!("Expected TextContent event"),
        }
    }

    #[test]
    fn test_parse_item_completed_event() {
        let mut parser = CodexParser::new();
        let line = r#"{"type": "item.completed", "item": {"type": "agent_message", "text": "Done processing"}}"#;
        let events = parser.parse_line_multi(line).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::TextContent(content) => assert_eq!(content, "Done processing"),
            _ => panic!("Expected TextContent event"),
        }
    }

    #[test]
    fn test_parse_tool_call_event() {
        let mut parser = CodexParser::new();
        let line = r#"{"type": "tool_call", "name": "read_file", "arguments": "{\"path\": \"test.txt\"}"}"#;
        let events = parser.parse_line_multi(line).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::ToolStarted { display_name, .. } => {
                assert_eq!(display_name, "read_file");
            }
            _ => panic!("Expected ToolStarted event"),
        }
    }

    #[test]
    fn test_parse_tool_result_event() {
        let mut parser = CodexParser::new();
        let line = r#"{"type": "tool_result", "call_id": "call_123", "output": "file contents"}"#;
        let events = parser.parse_line_multi(line).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::ToolResult {
                tool_use_id,
                is_error,
                content_lines,
                ..
            } => {
                assert_eq!(tool_use_id, "call_123");
                assert!(!is_error);
                assert_eq!(content_lines, &["file contents"]);
            }
            _ => panic!("Expected ToolResult event"),
        }
    }

    #[test]
    fn test_parse_done_event() {
        let mut parser = CodexParser::new();
        let line = r#"{"type": "done", "result": "Task completed"}"#;
        let events = parser.parse_line_multi(line).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::Result {
                output,
                cost,
                is_error,
            } => {
                assert_eq!(output, &Some("Task completed".to_string()));
                assert!(cost.is_none());
                assert!(!is_error);
            }
            _ => panic!("Expected Result event"),
        }
    }

    #[test]
    fn test_parse_error_event() {
        let mut parser = CodexParser::new();
        let line = r#"{"type": "error", "message": "Something went wrong"}"#;
        let events = parser.parse_line_multi(line).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::Error(msg) => assert_eq!(msg, "Something went wrong"),
            _ => panic!("Expected Error event"),
        }
    }

    #[test]
    fn test_parse_raw_text() {
        let mut parser = CodexParser::new();
        let line = "Just some plain text output";
        let events = parser.parse_line_multi(line).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::TextContent(content) => assert_eq!(content, "Just some plain text output"),
            _ => panic!("Expected TextContent event"),
        }
    }

    #[test]
    fn test_parse_empty_line() {
        let mut parser = CodexParser::new();
        let events = parser.parse_line_multi("").unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn test_parse_json_no_type() {
        let mut parser = CodexParser::new();
        let line = r#"{"content": "text without type field"}"#;
        let events = parser.parse_line_multi(line).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::TextContent(content) => assert_eq!(content, "text without type field"),
            _ => panic!("Expected TextContent event"),
        }
    }

    #[test]
    fn test_parse_command_execution_started() {
        let mut parser = CodexParser::new();
        let line = r#"{"type":"item.started","item":{"id":"item_1","type":"command_execution","command":"/bin/bash -lc ls","aggregated_output":"","exit_code":null,"status":"in_progress"}}"#;
        let events = parser.parse_line_multi(line).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::ToolStarted {
                display_name,
                input_preview,
                tool_use_id,
            } => {
                assert_eq!(display_name, "ls");
                assert_eq!(input_preview, "ls");
                assert_eq!(tool_use_id, &Some("item_1".to_string()));
            }
            _ => panic!("Expected ToolStarted event"),
        }
    }

    #[test]
    fn test_parse_command_execution_completed_success() {
        let mut parser = CodexParser::new();
        let line = r#"{"type":"item.completed","item":{"id":"item_1","type":"command_execution","command":"/bin/bash -lc ls","aggregated_output":"file1.txt\nfile2.txt\n","exit_code":0,"status":"completed"}}"#;
        let events = parser.parse_line_multi(line).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::ToolResult {
                tool_use_id,
                is_error,
                content_lines,
                has_more,
            } => {
                assert_eq!(tool_use_id, "item_1");
                assert!(!is_error);
                assert_eq!(content_lines, &["file1.txt", "file2.txt"]);
                assert!(!has_more);
            }
            _ => panic!("Expected ToolResult event"),
        }
    }

    #[test]
    fn test_parse_command_execution_completed_error() {
        let mut parser = CodexParser::new();
        let line = r#"{"type":"item.completed","item":{"id":"item_2","type":"command_execution","command":"/bin/bash -lc 'cat nonexistent'","aggregated_output":"cat: nonexistent: No such file or directory","exit_code":1,"status":"completed"}}"#;
        let events = parser.parse_line_multi(line).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::ToolResult {
                tool_use_id,
                is_error,
                content_lines,
                ..
            } => {
                assert_eq!(tool_use_id, "item_2");
                assert!(is_error); // exit_code != 0
                assert_eq!(
                    content_lines,
                    &["cat: nonexistent: No such file or directory"]
                );
            }
            _ => panic!("Expected ToolResult event"),
        }
    }

    #[test]
    fn test_truncate_command() {
        // Test bash wrapper stripping
        assert_eq!(CodexParser::truncate_command("/bin/bash -lc ls", 50), "ls");
        assert_eq!(
            CodexParser::truncate_command("/bin/bash -c 'echo hello'", 50),
            "echo hello"
        );
        assert_eq!(
            CodexParser::truncate_command("bash -lc 'rg pattern'", 50),
            "rg pattern"
        );

        // Test truncation
        let long_cmd = "rg --type rust 'very_long_pattern_that_exceeds_the_limit'";
        let truncated = CodexParser::truncate_command(long_cmd, 20);
        assert!(truncated.ends_with("..."));
        assert!(truncated.len() <= 20);

        // Test no truncation needed
        assert_eq!(CodexParser::truncate_command("ls -la", 50), "ls -la");
    }

    #[test]
    fn test_parse_thread_started_captures_conversation_id() {
        let mut parser = CodexParser::new();
        let line =
            r#"{"type":"thread.started","thread_id":"019bc838-8e90-7052-b458-3615bee3647a"}"#;
        let events = parser.parse_line_multi(line).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::ConversationIdCaptured(id) => {
                assert_eq!(id, "019bc838-8e90-7052-b458-3615bee3647a");
            }
            _ => panic!("Expected ConversationIdCaptured event"),
        }
    }
}
