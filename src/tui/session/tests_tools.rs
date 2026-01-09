//! Tool tracking tests for the Session struct.
//!
//! Tests for ID-aware matching with FIFO fallback, completed tools tracking,
//! and related tool lifecycle management.

use super::*;

#[test]
fn test_tool_started_with_id() {
    let mut session = Session::new(0);

    session.tool_started(
        Some("tool_123".to_string()),
        "Read".to_string(),
        "src/main.rs".to_string(),
        "claude".to_string(),
    );

    assert_eq!(session.active_tools_by_agent.len(), 1);
    let tools = session.active_tools_by_agent.get("claude").unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].tool_id, Some("tool_123".to_string()));
    assert_eq!(tools[0].display_name, "Read");
    assert_eq!(tools[0].input_preview, "src/main.rs");
}

#[test]
fn test_tool_started_without_id() {
    let mut session = Session::new(0);

    session.tool_started(
        None,
        "Bash".to_string(),
        "npm test".to_string(),
        "gemini".to_string(),
    );

    let tools = session.active_tools_by_agent.get("gemini").unwrap();
    assert_eq!(tools[0].tool_id, None);
    assert_eq!(tools[0].display_name, "Bash");
}

#[test]
fn test_tool_result_id_matched_completion() {
    let mut session = Session::new(0);

    // Start two tools with IDs
    session.tool_started(
        Some("tool_1".to_string()),
        "Read".to_string(),
        "file1.rs".to_string(),
        "claude".to_string(),
    );
    session.tool_started(
        Some("tool_2".to_string()),
        "Write".to_string(),
        "file2.rs".to_string(),
        "claude".to_string(),
    );

    // Complete the second by ID
    let duration = session.tool_result_received_for_agent(Some("tool_2"), false, "claude");
    assert!(duration.is_some());

    // First tool should still be active
    let active = session.active_tools_by_agent.get("claude").unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].tool_id, Some("tool_1".to_string()));

    // Completed tool should be in completed list
    let completed = session.completed_tools_by_agent.get("claude").unwrap();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].display_name, "Write");
    assert_eq!(completed[0].input_preview, "file2.rs");
    assert!(!completed[0].is_error);
}

#[test]
fn test_tool_result_fifo_fallback_when_no_ids() {
    let mut session = Session::new(0);

    // Start tools without IDs
    session.tool_started(None, "Read".to_string(), "first.rs".to_string(), "gemini".to_string());
    session.tool_started(None, "Write".to_string(), "second.rs".to_string(), "gemini".to_string());

    // Result without ID should complete the FIRST tool (FIFO)
    let duration = session.tool_result_received_for_agent(None, false, "gemini");
    assert!(duration.is_some());

    let active = session.active_tools_by_agent.get("gemini").unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].display_name, "Write"); // Second tool still active

    let completed = session.completed_tools_by_agent.get("gemini").unwrap();
    assert_eq!(completed[0].display_name, "Read"); // First tool completed
}

#[test]
fn test_tool_result_fifo_fallback_when_id_not_matched() {
    // Critical test case: Gemini scenario where starts have None but results have function name
    let mut session = Session::new(0);

    // Start tool with None ID (like Gemini does)
    session.tool_started(None, "search_file".to_string(), "query".to_string(), "gemini".to_string());

    // Result with an ID that doesn't match any active tool (like Gemini provides function name)
    let duration = session.tool_result_received_for_agent(Some("search_file"), false, "gemini");
    assert!(duration.is_some());

    // Tool should be completed via FIFO fallback
    assert!(!session.active_tools_by_agent.contains_key("gemini"));
    let completed = session.completed_tools_by_agent.get("gemini").unwrap();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].display_name, "search_file");
}

#[test]
fn test_tool_result_empty_string_id_treated_as_none() {
    let mut session = Session::new(0);

    session.tool_started(None, "Tool".to_string(), "preview".to_string(), "agent".to_string());

    // Empty string ID should be treated as None and use FIFO
    let duration = session.tool_result_received_for_agent(Some(""), false, "agent");
    assert!(duration.is_some());

    assert!(session.active_tools_by_agent.is_empty());
    assert_eq!(session.completed_tools_by_agent.get("agent").unwrap().len(), 1);
}

#[test]
fn test_tool_finished_no_double_removal() {
    let mut session = Session::new(0);

    session.tool_started(
        Some("tool_1".to_string()),
        "Read".to_string(),
        "file.rs".to_string(),
        "claude".to_string(),
    );

    // ToolResult completes the tool first
    session.tool_result_received_for_agent(Some("tool_1"), false, "claude");
    assert!(session.active_tools_by_agent.is_empty());
    assert_eq!(session.completed_tools_by_agent.get("claude").unwrap().len(), 1);

    // ToolFinished should be a no-op (no second tool to remove)
    session.tool_finished_for_agent(Some("tool_1"), "claude");

    // Still should have only 1 completed tool
    assert_eq!(session.completed_tools_by_agent.get("claude").unwrap().len(), 1);
}

#[test]
fn test_tool_result_with_error_flag() {
    let mut session = Session::new(0);

    session.tool_started(
        Some("tool_1".to_string()),
        "Bash".to_string(),
        "npm test".to_string(),
        "claude".to_string(),
    );

    session.tool_result_received_for_agent(Some("tool_1"), true, "claude");

    let completed = session.completed_tools_by_agent.get("claude").unwrap();
    assert!(completed[0].is_error);
}

#[test]
fn test_completed_tools_ordered_newest_first() {
    let mut session = Session::new(0);

    session.tool_started(None, "First".to_string(), "".to_string(), "agent".to_string());
    session.tool_result_received_for_agent(None, false, "agent");

    session.tool_started(None, "Second".to_string(), "".to_string(), "agent".to_string());
    session.tool_result_received_for_agent(None, false, "agent");

    session.tool_started(None, "Third".to_string(), "".to_string(), "agent".to_string());
    session.tool_result_received_for_agent(None, false, "agent");

    let completed = session.all_completed_tools();
    assert_eq!(completed.len(), 3);
    // Newest first
    assert_eq!(completed[0].1.display_name, "Third");
    assert_eq!(completed[1].1.display_name, "Second");
    assert_eq!(completed[2].1.display_name, "First");
}

#[test]
fn test_orphan_result_creates_synthetic_entry() {
    let mut session = Session::new(0);

    // No active tools - receive a result anyway
    let duration = session.tool_result_received_for_agent(Some("orphan_tool"), true, "agent");

    assert_eq!(duration, Some(0)); // Duration is 0 for synthetic entries
    let completed = session.completed_tools_by_agent.get("agent").unwrap();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].display_name, "orphan_tool");
    assert_eq!(completed[0].duration_ms, 0);
    assert!(completed[0].is_error);
}

#[test]
fn test_completed_tools_retention_cap() {
    let mut session = Session::new(0);

    // Add more than MAX_COMPLETED_TOOLS (100)
    for i in 0..120 {
        session.tool_started(
            None,
            format!("Tool_{}", i),
            "".to_string(),
            "agent".to_string(),
        );
        session.tool_result_received_for_agent(None, false, "agent");
    }

    // Total completed should be capped at 100
    let total: usize = session.completed_tools_by_agent.values().map(|v| v.len()).sum();
    assert!(total <= 100);

    // Newest tools should be retained
    let completed = session.completed_tools_by_agent.get("agent").unwrap();
    assert!(completed[0].display_name.contains("119")); // Most recent
}

#[test]
fn test_content_block_start_flow() {
    let mut session = Session::new(0);

    // ContentBlockStart has None for tool_id and empty input_preview
    session.tool_started(None, "function_name".to_string(), "".to_string(), "claude".to_string());

    let tools = session.active_tools_by_agent.get("claude").unwrap();
    assert_eq!(tools[0].tool_id, None);
    assert_eq!(tools[0].display_name, "function_name");
    assert_eq!(tools[0].input_preview, "");

    // Complete via FIFO
    session.tool_result_received_for_agent(None, false, "claude");

    let completed = session.completed_tools_by_agent.get("claude").unwrap();
    assert_eq!(completed[0].display_name, "function_name");
}
