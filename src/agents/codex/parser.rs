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
                "message" | "content" | "text" => {
                    if let Some(content) = self.extract_text_content(json) {
                        events.push(AgentEvent::TextContent(content));
                    }
                }
                "item.completed" | "item.delta" => {
                    if let Some(item) = json.get("item") {
                        let item_type = item.get("type").and_then(|v| v.as_str());
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
                                    format!("{}...", &s[..100])
                                } else {
                                    s
                                }
                            })
                            .unwrap_or_default();

                        events.push(AgentEvent::ToolStarted {
                            name: name.to_string(),
                            display_name: name.to_string(),
                            input_preview,
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
                    let content_lines = if let Some(content) =
                        json.get("output").or_else(|| json.get("result"))
                    {
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

    fn reset(&mut self) {
        self._event_count = 0;
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
            AgentEvent::ToolStarted { name, display_name, .. } => {
                assert_eq!(name, "read_file");
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
            AgentEvent::ToolResult { tool_use_id, is_error, content_lines, .. } => {
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
            AgentEvent::Result { output, cost, is_error } => {
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
    fn test_reset() {
        let mut parser = CodexParser::new();
        let _ = parser.parse_line_multi(r#"{"type": "message", "content": "test"}"#);
        parser.reset();
        // After reset, the parser should be in initial state
        assert_eq!(parser._event_count, 0);
    }
}
