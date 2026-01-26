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
    let line =
        r#"{"type": "tool_call", "name": "read_file", "arguments": "{\"path\": \"test.txt\"}"}"#;
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
    let line = r#"{"type":"thread.started","thread_id":"019bc838-8e90-7052-b458-3615bee3647a"}"#;
    let events = parser.parse_line_multi(line).unwrap();
    assert_eq!(events.len(), 1);
    match &events[0] {
        AgentEvent::ConversationIdCaptured(id) => {
            assert_eq!(id, "019bc838-8e90-7052-b458-3615bee3647a");
        }
        _ => panic!("Expected ConversationIdCaptured event"),
    }
}
