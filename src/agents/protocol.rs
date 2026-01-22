//! Unified agent output protocol types.
//!
//! This module defines a common set of event types that all agent parsers
//! (Claude, Codex, Gemini) emit, enabling consistent handling across the
//! agent execution pipeline.

use crate::tui::{TodoItem, TokenUsage};
use std::fmt;

/// Error type for agent output parsing (currently unused as parsers never fail).
#[derive(Debug, Clone)]
pub struct ParseError;

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Parse error")
    }
}

impl std::error::Error for ParseError {}

/// Unified event type emitted by all agent parsers.
///
/// This enum covers all existing event semantics from the Claude parser's
/// `ParsedEvent` and provides a consistent interface for Codex and Gemini agents.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Signals end of a turn (maps from ParsedEvent::TurnCompleted)
    TurnCompleted,

    /// Detected model name (maps from ParsedEvent::ModelDetected)
    ModelDetected(String),

    /// Reason for stopping (maps from ParsedEvent::StopReason)
    StopReason(String),

    /// Token usage metrics (maps from ParsedEvent::TokenUsage)
    TokenUsage(AgentTokenUsage),

    /// Text content from agent (maps from ParsedEvent::TextContent)
    TextContent(String),

    /// Tool execution start (maps from ParsedEvent::ToolStarted)
    ToolStarted {
        display_name: String,
        input_preview: String,
        /// Optional unique identifier for correlating with ToolResult.
        /// Required for agents that execute tools in parallel (e.g., Codex).
        tool_use_id: Option<String>,
    },

    /// Tool execution result (maps from ParsedEvent::ToolResult)
    ToolResult {
        tool_use_id: String,
        is_error: bool,
        content_lines: Vec<String>,
        has_more: bool,
    },

    /// Todo list updates (maps from ParsedEvent::TodosUpdate)
    TodosUpdate(Vec<TodoItem>),

    /// Start of content block (maps from ParsedEvent::ContentBlockStart)
    ContentBlockStart { name: String },

    /// Incremental content (maps from ParsedEvent::ContentDelta)
    ContentDelta(String),

    /// Final result (maps from ParsedEvent::Result)
    Result {
        output: Option<String>,
        cost: Option<f64>,
        is_error: bool,
    },

    /// Parsing or execution error - NEW
    ///
    /// Provides explicit error event handling for cases like JSON parse failures,
    /// process crashes, and timeout errors that don't fit the existing `Result` structure.
    Error(String),

    /// Agent's conversation/session ID captured from output.
    /// Used for conversation resume on subsequent invocations.
    /// - Claude: from `{"type":"system","subtype":"init","session_id":"..."}`
    /// - Codex: from thread_id in output
    /// - Gemini: from session UUID in output
    ConversationIdCaptured(String),
}

/// Token usage information from agent execution.
#[derive(Debug, Clone, Default)]
pub struct AgentTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
}

impl From<AgentTokenUsage> for TokenUsage {
    fn from(usage: AgentTokenUsage) -> Self {
        TokenUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_creation_tokens: usage.cache_creation_tokens,
            cache_read_tokens: usage.cache_read_tokens,
        }
    }
}

impl From<TokenUsage> for AgentTokenUsage {
    fn from(usage: TokenUsage) -> Self {
        AgentTokenUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_creation_tokens: usage.cache_creation_tokens,
            cache_read_tokens: usage.cache_read_tokens,
        }
    }
}

/// Final output from an agent execution.
#[derive(Debug, Clone)]
pub struct AgentOutput {
    /// The textual output from the agent
    pub output: String,
    /// Whether the execution resulted in an error
    pub is_error: bool,
    /// Captured conversation ID for resume (from agent's init/start message)
    pub conversation_id: Option<String>,
    /// Stop reason if agent was stopped (max_turns, max_tokens, cancelled, etc.)
    pub stop_reason: Option<String>,
}

/// Trait for parsing agent-specific output formats into unified AgentEvent types.
///
/// Each agent implementation (Claude, Codex, Gemini) implements this trait
/// to convert their CLI tool's JSON output format into AgentEvent types.
///
/// Note: We use a concrete trait approach rather than dynamic dispatch since
/// there are only 3 agent types. The existing `AgentType` enum handles dispatch.
pub trait AgentStreamParser {
    /// Parse a single line of output into zero or more AgentEvents.
    ///
    /// Returns `Ok(Some(event))` if an event was parsed,
    /// `Ok(None)` if the line was valid but produced no event (e.g., whitespace),
    /// or `Err(ParseError)` if parsing failed.
    ///
    /// Note: Some lines may produce multiple events, but this interface returns
    /// one at a time. Use `parse_line_multi` for batch parsing.
    fn parse_line(&mut self, line: &str) -> Result<Option<AgentEvent>, ParseError>;

    /// Parse a single line of output into multiple AgentEvents.
    ///
    /// This is the primary parsing method - some agent output lines can
    /// produce multiple events (e.g., a message with token usage and content).
    fn parse_line_multi(&mut self, line: &str) -> Result<Vec<AgentEvent>, ParseError> {
        match self.parse_line(line) {
            Ok(Some(event)) => Ok(vec![event]),
            Ok(None) => Ok(vec![]),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_error_display() {
        let err = ParseError;
        assert_eq!(format!("{}", err), "Parse error");
    }

    #[test]
    fn test_token_usage_conversion() {
        let agent_usage = AgentTokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_creation_tokens: 10,
            cache_read_tokens: 5,
        };

        let tui_usage: TokenUsage = agent_usage.clone().into();
        assert_eq!(tui_usage.input_tokens, 100);
        assert_eq!(tui_usage.output_tokens, 50);
        assert_eq!(tui_usage.cache_creation_tokens, 10);
        assert_eq!(tui_usage.cache_read_tokens, 5);

        let back: AgentTokenUsage = tui_usage.into();
        assert_eq!(back.input_tokens, 100);
        assert_eq!(back.output_tokens, 50);
    }

    #[test]
    fn test_agent_output_default() {
        let output = AgentOutput {
            output: "test".to_string(),
            is_error: false,
            conversation_id: None,
            stop_reason: None,
        };
        assert_eq!(output.output, "test");
        assert!(!output.is_error);
    }

    #[test]
    fn test_agent_output_with_conversation_id() {
        let output = AgentOutput {
            output: "test".to_string(),
            is_error: false,
            conversation_id: Some("abc-123".to_string()),
            stop_reason: None,
        };
        assert_eq!(output.conversation_id, Some("abc-123".to_string()));
    }
}
