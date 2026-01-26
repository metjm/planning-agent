use super::*;

#[test]
fn test_parse_assistant_message() {
    let mut parser = ClaudeParser::new();
    let line = r#"{"type": "assistant", "message": {"model": "claude-3", "content": [{"type": "text", "text": "Hello"}]}}"#;
    let events = parser.parse_line_multi(line).unwrap();
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::ModelDetected(m) if m == "claude-3")));
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::TextContent(t) if t == "Hello")));
}

#[test]
fn test_parse_turn_completed() {
    let mut parser = ClaudeParser::new();
    // First a user message
    let _ = parser.parse_line_multi(r#"{"type": "user", "message": {}}"#);
    // Then an assistant message should trigger TurnCompleted
    let events = parser
        .parse_line_multi(r#"{"type": "assistant", "message": {}}"#)
        .unwrap();
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::TurnCompleted)));
}

#[test]
fn test_parse_result() {
    let mut parser = ClaudeParser::new();
    let line =
        r#"{"type": "result", "result": "Done!", "total_cost_usd": 0.05, "is_error": false}"#;
    let events = parser.parse_line_multi(line).unwrap();
    assert_eq!(events.len(), 1);
    match &events[0] {
        AgentEvent::Result {
            output,
            cost,
            is_error,
        } => {
            assert_eq!(output, &Some("Done!".to_string()));
            assert_eq!(*cost, Some(0.05));
            assert!(!is_error);
        }
        _ => panic!("Expected Result event"),
    }
}

#[test]
fn test_parse_tool_use() {
    let mut parser = ClaudeParser::new();
    let line = r#"{"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "toolu_123", "name": "Read", "input": {"path": "test.txt"}}]}}"#;
    let events = parser.parse_line_multi(line).unwrap();
    assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolStarted {
        display_name,
        tool_use_id,
        ..
    } if display_name == "Read" && tool_use_id == &Some("toolu_123".to_string()))));
}

#[test]
fn test_parse_empty_line() {
    let mut parser = ClaudeParser::new();
    let events = parser.parse_line_multi("").unwrap();
    assert!(events.is_empty());
}

#[test]
fn test_parse_invalid_json() {
    let mut parser = ClaudeParser::new();
    let events = parser.parse_line_multi("not json").unwrap();
    assert!(events.is_empty());
}

#[test]
fn test_parse_system_init_captures_conversation_id() {
    let mut parser = ClaudeParser::new();
    let line = r#"{"type":"system","subtype":"init","session_id":"7c4aefbb-b0a5-45d7-bd7a-8494f1d6d47f","cwd":"/workspace","tools":["Read"]}"#;
    let events = parser.parse_line_multi(line).unwrap();
    assert_eq!(events.len(), 1);
    match &events[0] {
        AgentEvent::ConversationIdCaptured(id) => {
            assert_eq!(id, "7c4aefbb-b0a5-45d7-bd7a-8494f1d6d47f");
        }
        _ => panic!("Expected ConversationIdCaptured event"),
    }
}

#[test]
fn test_parse_system_non_init_ignored() {
    let mut parser = ClaudeParser::new();
    // A system message without subtype "init" should not capture anything
    let line = r#"{"type":"system","subtype":"other","session_id":"abc"}"#;
    let events = parser.parse_line_multi(line).unwrap();
    assert!(events.is_empty());
}
