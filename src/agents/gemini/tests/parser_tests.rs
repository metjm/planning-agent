use super::*;

#[test]
fn test_parse_direct_response() {
    let mut parser = GeminiParser::new();
    let line = r#"{"response": "Hello from Gemini!"}"#;
    let events = parser.parse_line_multi(line).unwrap();
    assert_eq!(events.len(), 1);
    match &events[0] {
        AgentEvent::TextContent(content) => assert_eq!(content, "Hello from Gemini!"),
        _ => panic!("Expected TextContent event"),
    }
}

#[test]
fn test_parse_candidates_structure() {
    let mut parser = GeminiParser::new();
    let line =
        r#"{"candidates": [{"content": {"parts": [{"text": "Part 1"}, {"text": "Part 2"}]}}]}"#;
    let events = parser.parse_line_multi(line).unwrap();
    assert_eq!(events.len(), 2);
    match &events[0] {
        AgentEvent::TextContent(content) => assert_eq!(content, "Part 1"),
        _ => panic!("Expected TextContent event"),
    }
    match &events[1] {
        AgentEvent::TextContent(content) => assert_eq!(content, "Part 2"),
        _ => panic!("Expected TextContent event"),
    }
}

#[test]
fn test_parse_function_call() {
    let mut parser = GeminiParser::new();
    let line = r#"{"functionCall": {"name": "search", "args": {"query": "test"}}}"#;
    let events = parser.parse_line_multi(line).unwrap();
    assert_eq!(events.len(), 1);
    match &events[0] {
        AgentEvent::ToolStarted { display_name, .. } => {
            assert_eq!(display_name, "search");
        }
        _ => panic!("Expected ToolStarted event"),
    }
}

#[test]
fn test_parse_function_response() {
    let mut parser = GeminiParser::new();
    let line = r#"{"functionResponse": {"name": "search", "response": "search results"}}"#;
    let events = parser.parse_line_multi(line).unwrap();
    assert_eq!(events.len(), 1);
    match &events[0] {
        AgentEvent::ToolResult {
            tool_use_id,
            is_error,
            content_lines,
            ..
        } => {
            assert_eq!(tool_use_id, "search");
            assert!(!is_error);
            assert_eq!(content_lines, &["search results"]);
        }
        _ => panic!("Expected ToolResult event"),
    }
}

#[test]
fn test_parse_error() {
    let mut parser = GeminiParser::new();
    let line = r#"{"error": {"message": "Rate limit exceeded"}}"#;
    let events = parser.parse_line_multi(line).unwrap();
    assert_eq!(events.len(), 1);
    match &events[0] {
        AgentEvent::Error(msg) => assert_eq!(msg, "Rate limit exceeded"),
        _ => panic!("Expected Error event"),
    }
}

#[test]
fn test_parse_usage_metadata() {
    let mut parser = GeminiParser::new();
    let line = r#"{"usageMetadata": {"promptTokenCount": 100, "candidatesTokenCount": 50}}"#;
    let events = parser.parse_line_multi(line).unwrap();
    assert_eq!(events.len(), 1);
    match &events[0] {
        AgentEvent::TokenUsage(usage) => {
            assert_eq!(usage.input_tokens, 100);
            assert_eq!(usage.output_tokens, 50);
        }
        _ => panic!("Expected TokenUsage event"),
    }
}

#[test]
fn test_parse_raw_text() {
    let mut parser = GeminiParser::new();
    let line = "Plain text output";
    let events = parser.parse_line_multi(line).unwrap();
    assert_eq!(events.len(), 1);
    match &events[0] {
        AgentEvent::TextContent(content) => assert_eq!(content, "Plain text output"),
        _ => panic!("Expected TextContent event"),
    }
}

#[test]
fn test_parse_empty_line() {
    let mut parser = GeminiParser::new();
    let events = parser.parse_line_multi("").unwrap();
    assert!(events.is_empty());
}

#[test]
fn test_parse_session_id_captures_conversation_id() {
    let mut parser = GeminiParser::new();
    let line = r#"{"session_id": "4e2f5f4f-c181-417a-855f-291bf3e9e515", "response": "Hello"}"#;
    let events = parser.parse_line_multi(line).unwrap();
    // Should have both ConversationIdCaptured and TextContent
    assert_eq!(events.len(), 2);
    match &events[0] {
        AgentEvent::ConversationIdCaptured(id) => {
            assert_eq!(id, "4e2f5f4f-c181-417a-855f-291bf3e9e515");
        }
        _ => panic!("Expected ConversationIdCaptured event first"),
    }
    match &events[1] {
        AgentEvent::TextContent(content) => {
            assert_eq!(content, "Hello");
        }
        _ => panic!("Expected TextContent event second"),
    }
}
