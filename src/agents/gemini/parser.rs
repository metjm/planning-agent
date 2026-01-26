//! Gemini CLI output parser implementing the unified AgentStreamParser trait.
//!
//! Parses JSON line output from the Gemini CLI tool and converts it to
//! unified AgentEvent types.

use crate::agents::protocol::{AgentEvent, AgentStreamParser, ParseError};
use serde_json::Value;

/// Parser for Gemini CLI JSON output.
///
/// The Gemini CLI emits JSON with various structures including:
/// - Direct `response`, `text`, `content`, `output`, or `result` fields
/// - Google AI API style `candidates` array with nested `content.parts`
/// - `functionCall` / `function_call` for tool invocations
/// - `functionResponse` / `function_response` for tool results
/// - `error` object for errors
#[derive(Debug, Clone, Default)]
pub struct GeminiParser {
    // Track if we've seen any events for potential multi-turn support
    _event_count: usize,
}

impl GeminiParser {
    pub fn new() -> Self {
        Self { _event_count: 0 }
    }

    /// Parse a JSON value into AgentEvents
    fn parse_json(&mut self, json: &Value) -> Vec<AgentEvent> {
        let mut events = Vec::new();

        // Capture session_id for conversation resume
        if let Some(session_id) = json.get("session_id").and_then(|v| v.as_str()) {
            events.push(AgentEvent::ConversationIdCaptured(session_id.to_string()));
        }

        // First check for direct text content
        if let Some(response) = json
            .get("response")
            .or_else(|| json.get("text"))
            .or_else(|| json.get("content"))
            .or_else(|| json.get("output"))
            .or_else(|| json.get("result"))
            .and_then(|r| r.as_str())
        {
            events.push(AgentEvent::TextContent(response.to_string()));
        }

        // Check for Google AI API style candidates structure
        if let Some(candidates) = json.get("candidates").and_then(|c| c.as_array()) {
            for candidate in candidates {
                if let Some(content) = candidate.get("content") {
                    if let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
                        for part in parts {
                            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                events.push(AgentEvent::TextContent(text.to_string()));
                            }
                        }
                    }
                }
            }
        }

        // Check for function call (tool invocation)
        if let Some(function_call) = json
            .get("functionCall")
            .or_else(|| json.get("function_call"))
        {
            if let Some(name) = function_call.get("name").and_then(|n| n.as_str()) {
                // Extract arguments preview if available
                let input_preview = function_call
                    .get("args")
                    .or_else(|| function_call.get("arguments"))
                    .map(|v| {
                        let s = v.to_string();
                        if s.len() > 100 {
                            format!("{}...", s.get(..100).unwrap_or(&s))
                        } else {
                            s
                        }
                    })
                    .unwrap_or_default();

                // Gemini doesn't provide a tool_use_id in function calls,
                // so we leave it as None (FIFO matching will be used)
                events.push(AgentEvent::ToolStarted {
                    display_name: name.to_string(),
                    input_preview,
                    tool_use_id: None,
                });
            }
        }

        // Check for function response (tool result)
        if let Some(function_response) = json
            .get("functionResponse")
            .or_else(|| json.get("function_response"))
        {
            let tool_id = function_response
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();

            let is_error = function_response
                .get("error")
                .map(|e| !e.is_null())
                .unwrap_or(false);

            // Extract response content
            let content_lines = if let Some(response) = function_response.get("response") {
                if let Some(s) = response.as_str() {
                    s.lines().take(5).map(|l| l.to_string()).collect()
                } else {
                    vec![response.to_string()]
                }
            } else {
                vec![]
            };

            let has_more = function_response
                .get("response")
                .and_then(|r| r.as_str())
                .map(|s| s.lines().count() > 5)
                .unwrap_or(false);

            events.push(AgentEvent::ToolResult {
                tool_use_id: tool_id,
                is_error,
                content_lines,
                has_more,
            });
        }

        // Check for error
        if let Some(error) = json.get("error") {
            if let Some(message) = error.get("message").and_then(|m| m.as_str()) {
                events.push(AgentEvent::Error(message.to_string()));
            } else if let Some(msg) = error.as_str() {
                events.push(AgentEvent::Error(msg.to_string()));
            }
        }

        // Check for usage/metadata
        if let Some(usage) = json.get("usageMetadata") {
            let input_tokens = usage
                .get("promptTokenCount")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let output_tokens = usage
                .get("candidatesTokenCount")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            events.push(AgentEvent::TokenUsage(
                crate::agents::protocol::AgentTokenUsage {
                    input_tokens,
                    output_tokens,
                    cache_creation_tokens: 0,
                    cache_read_tokens: 0,
                },
            ));
        }

        events
    }
}

impl AgentStreamParser for GeminiParser {
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
#[path = "tests/parser_tests.rs"]
mod tests;
