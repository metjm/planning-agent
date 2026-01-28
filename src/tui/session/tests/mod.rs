use super::*;
use crate::config::WorkflowConfig;
use crate::domain::types::{
    AgentId, ConversationId, ImplementationVerdict, Iteration, MaxIterations, ResumeStrategy,
    TimestampUtc,
};
use crate::domain::view::WorkflowView;
use crate::phases::implementing_conversation_key;
use crate::tui::{Event, SessionEventSender};
use std::path::PathBuf;
use tokio::sync::mpsc;

#[test]
fn test_scroll_up_disables_follow_mode() {
    let mut session = Session::new(0);
    session.add_output("line1".to_string());
    session.add_output("line2".to_string());
    assert!(session.output_scroll.follow);

    session.scroll_up();
    assert!(!session.output_scroll.follow);
}

#[test]
fn test_scroll_to_bottom_enables_follow_mode() {
    let mut session = Session::new(0);
    session.output_scroll.follow = false;
    session.scroll_to_bottom();
    assert!(session.output_scroll.follow);
}

#[test]
fn test_add_output_enables_follow_mode() {
    let mut session = Session::new(0);
    session.output_scroll.follow = false;
    session.add_output("new line".to_string());
    assert!(session.output_scroll.follow);
}

#[test]
fn test_input_mode_transitions() {
    let mut session = Session::new(0);
    assert_eq!(session.input_mode, InputMode::Normal);

    session.input_mode = InputMode::NamingTab;
    assert_eq!(session.input_mode, InputMode::NamingTab);

    session.input_mode = InputMode::Normal;
    assert_eq!(session.input_mode, InputMode::Normal);
}

#[test]
fn test_tab_input_buffer() {
    let mut session = Session::new(0);
    session.input_mode = InputMode::NamingTab;

    session.insert_tab_input_char('h');
    session.insert_tab_input_char('e');
    session.insert_tab_input_char('l');
    session.insert_tab_input_char('l');
    session.insert_tab_input_char('o');

    assert_eq!(session.tab_input, "hello");
    assert_eq!(session.tab_input_cursor, 5);

    session.delete_tab_input_char();
    assert_eq!(session.tab_input, "hell");
    assert_eq!(session.tab_input_cursor, 4);
}

#[test]
fn test_session_with_name() {
    let session = Session::with_name(1, "test-feature".to_string());
    assert_eq!(session.id, 1);
    assert_eq!(session.name, "test-feature");
    assert_eq!(session.input_mode, InputMode::Normal);
    assert_eq!(session.status, SessionStatus::Planning);
}

#[test]
fn test_insert_newline() {
    let mut session = Session::new(0);
    session.tab_input = "hello".to_string();
    session.tab_input_cursor = 5;

    session.insert_tab_input_newline();

    assert_eq!(session.tab_input, "hello\n");
    assert_eq!(session.tab_input_cursor, 6);

    session.tab_input = "hello world".to_string();
    session.tab_input_cursor = 5;
    session.insert_tab_input_newline();

    assert_eq!(session.tab_input, "hello\n world");
    assert_eq!(session.tab_input_cursor, 6);
}

#[test]
fn test_cursor_up_movement() {
    let mut session = Session::new(0);
    session.tab_input = "line1\nline2\nline3".to_string();
    session.tab_input_cursor = 14;

    session.move_tab_input_cursor_up();
    assert_eq!(session.tab_input_cursor, 8);

    session.move_tab_input_cursor_up();
    assert_eq!(session.tab_input_cursor, 2);
}

#[test]
fn test_cursor_down_movement() {
    let mut session = Session::new(0);
    session.tab_input = "line1\nline2\nline3".to_string();
    session.tab_input_cursor = 2;

    session.move_tab_input_cursor_down();
    assert_eq!(session.tab_input_cursor, 8);

    session.move_tab_input_cursor_down();
    assert_eq!(session.tab_input_cursor, 14);
}

#[test]
fn test_cursor_up_at_first_line() {
    let mut session = Session::new(0);
    session.tab_input = "line1\nline2".to_string();
    session.tab_input_cursor = 2;

    session.move_tab_input_cursor_up();
    assert_eq!(session.tab_input_cursor, 2);
}

#[test]
fn test_cursor_down_at_last_line() {
    let mut session = Session::new(0);
    session.tab_input = "line1\nline2".to_string();
    session.tab_input_cursor = 8;

    session.move_tab_input_cursor_down();
    assert_eq!(session.tab_input_cursor, 8);
}

#[test]
fn test_cursor_up_clamps_to_shorter_line() {
    let mut session = Session::new(0);
    session.tab_input = "hi\nworld".to_string();
    session.tab_input_cursor = 7;

    session.move_tab_input_cursor_up();
    assert_eq!(session.tab_input_cursor, 2);
}

#[test]
fn test_cursor_down_clamps_to_shorter_line() {
    let mut session = Session::new(0);
    session.tab_input = "world\nhi".to_string();
    session.tab_input_cursor = 4;

    session.move_tab_input_cursor_down();
    assert_eq!(session.tab_input_cursor, 8);
}

#[test]
fn test_get_tab_input_cursor_position() {
    let mut session = Session::new(0);
    session.tab_input = "line1\nline2\nline3".to_string();

    session.tab_input_cursor = 0;
    assert_eq!(session.get_tab_input_cursor_position(), (0, 0));

    session.tab_input_cursor = 3;
    assert_eq!(session.get_tab_input_cursor_position(), (0, 3));

    session.tab_input_cursor = 6;
    assert_eq!(session.get_tab_input_cursor_position(), (1, 0));

    session.tab_input_cursor = 14;
    assert_eq!(session.get_tab_input_cursor_position(), (2, 2));
}

#[test]
fn test_get_tab_input_line_count() {
    let mut session = Session::new(0);

    session.tab_input = "single line".to_string();
    assert_eq!(session.get_tab_input_line_count(), 1);

    session.tab_input = "line1\nline2".to_string();
    assert_eq!(session.get_tab_input_line_count(), 2);

    session.tab_input = "line1\nline2\nline3".to_string();
    assert_eq!(session.get_tab_input_line_count(), 3);

    session.tab_input = "".to_string();
    assert_eq!(session.get_tab_input_line_count(), 1);
}

#[test]
fn test_display_cost_returns_api_cost() {
    let mut session = Session::new(0);
    session.total_cost = 0.1234;
    session.total_input_tokens = 1000;
    session.total_output_tokens = 500;

    let cost = session.display_cost();
    assert_eq!(cost, 0.1234);
}

#[test]
fn test_display_cost_returns_zero_when_no_api_cost() {
    let mut session = Session::new(0);
    session.total_cost = 0.0;
    session.total_input_tokens = 1000;
    session.total_output_tokens = 500;
    session.model_name = Some("claude-3.5-sonnet".to_string());

    let cost = session.display_cost();

    assert_eq!(cost, 0.0);
}

#[test]
fn test_feedback_cursor_position_basic() {
    let mut session = Session::new(0);
    session.user_feedback = "hello".to_string();
    session.cursor_position = 5;

    let (row, col) = session.get_feedback_cursor_position(10);
    assert_eq!(row, 0);
    assert_eq!(col, 5);
}

#[test]
fn test_feedback_cursor_position_with_wrap() {
    let mut session = Session::new(0);
    session.user_feedback = "hello world".to_string();
    session.cursor_position = 11;

    let (row, col) = session.get_feedback_cursor_position(5);

    assert_eq!(row, 2);
    assert_eq!(col, 1);
}

#[test]
fn test_feedback_cursor_position_empty() {
    let mut session = Session::new(0);
    session.user_feedback = "".to_string();
    session.cursor_position = 0;

    let (row, col) = session.get_feedback_cursor_position(10);
    assert_eq!(row, 0);
    assert_eq!(col, 0);
}

#[test]
fn test_feedback_cursor_with_newline() {
    let mut session = Session::new(0);
    session.user_feedback = "hello\nworld".to_string();
    session.cursor_position = 8;

    let (row, col) = session.get_feedback_cursor_position(20);
    assert_eq!(row, 1);
    assert_eq!(col, 2);
}

#[test]
fn test_feedback_insert_char_utf8() {
    let mut session = Session::new(0);
    session.user_feedback = "".to_string();
    session.cursor_position = 0;

    session.insert_char('你');
    assert_eq!(session.user_feedback, "你");
    assert_eq!(session.cursor_position, 3);

    session.insert_char('好');
    assert_eq!(session.user_feedback, "你好");
    assert_eq!(session.cursor_position, 6);
}

#[test]
fn test_feedback_delete_char_utf8() {
    let mut session = Session::new(0);
    session.user_feedback = "你好".to_string();
    session.cursor_position = 6;

    session.delete_char();
    assert_eq!(session.user_feedback, "你");
    assert_eq!(session.cursor_position, 3);

    session.delete_char();
    assert_eq!(session.user_feedback, "");
    assert_eq!(session.cursor_position, 0);
}

#[test]
fn test_add_output_syncs_scroll() {
    let mut session = Session::new(0);
    session.output_scroll.follow = false;
    session.output_scroll.position = 0;

    session.add_output("line1".to_string());
    assert!(session.output_scroll.follow);
    assert_eq!(session.output_scroll.position, 0);

    session.add_output("line2".to_string());
    assert_eq!(session.output_scroll.position, 1);
}

#[test]
fn test_scroll_to_bottom_syncs_position() {
    let mut session = Session::new(0);
    session.output_lines = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    session.output_scroll.position = 0;
    session.output_scroll.follow = false;

    session.scroll_to_bottom();
    assert!(session.output_scroll.follow);
    assert_eq!(session.output_scroll.position, 2);
}

#[test]
fn test_insert_paste_basic() {
    let mut session = Session::new(0);
    session.insert_paste_tab_input("hello world".to_string());

    assert!(session.has_tab_input_pastes());
    assert_eq!(session.tab_input_pastes.len(), 1);
    assert_eq!(session.tab_input, "[Pasted]");
    assert_eq!(session.tab_input_cursor, 8);
}

#[test]
fn test_insert_paste_multiline() {
    let mut session = Session::new(0);
    session.insert_paste_tab_input("line1\nline2\nline3".to_string());

    assert_eq!(session.tab_input_pastes.len(), 1);
    assert_eq!(session.tab_input_pastes[0].line_count, 3);
    assert_eq!(session.tab_input, "[Pasted +2 lines]");
}

#[test]
fn test_get_display_text_with_paste() {
    let mut session = Session::new(0);
    session.tab_input = "prefix ".to_string();
    session.tab_input_cursor = 7;
    session.insert_paste_tab_input("pasted content\nmore content".to_string());

    let display = session.get_display_text_tab();
    assert_eq!(display, "prefix [Pasted +1 lines]");
}

#[test]
fn test_get_submit_text_expands_paste() {
    let mut session = Session::new(0);
    session.insert_paste_tab_input("pasted content".to_string());

    let submit = session.get_submit_text_tab();
    assert_eq!(submit, "pasted content");
}

#[test]
fn test_get_submit_text_with_surrounding_text() {
    let mut session = Session::new(0);
    session.tab_input = "before ".to_string();
    session.tab_input_cursor = 7;
    session.insert_paste_tab_input("pasted".to_string());
    session.tab_input.push_str(" after");

    let submit = session.get_submit_text_tab();
    assert_eq!(submit, "before pasted after");
}

#[test]
fn test_delete_paste_block() {
    let mut session = Session::new(0);
    session.insert_paste_tab_input("content".to_string());

    assert!(session.has_tab_input_pastes());
    assert!(session.delete_paste_at_cursor_tab());
    assert!(!session.has_tab_input_pastes());
    assert!(session.tab_input.is_empty());
    assert_eq!(session.tab_input_cursor, 0);
}

#[test]
fn test_multiple_pastes() {
    let mut session = Session::new(0);

    session.insert_paste_tab_input("first".to_string());
    session.tab_input.push(' ');
    session.tab_input_cursor = session.tab_input.len();
    session.insert_paste_tab_input("second".to_string());

    assert_eq!(session.tab_input_pastes.len(), 2);
    assert_eq!(session.tab_input, "[Pasted] [Pasted]");

    let submit = session.get_submit_text_tab();
    assert_eq!(submit, "first second");
}

#[test]
fn test_feedback_paste_insert_and_expand() {
    let mut session = Session::new(0);
    session.insert_paste_feedback("feedback content".to_string());

    assert!(session.has_feedback_pastes());
    assert_eq!(session.get_display_text_feedback(), "[Pasted]");
    assert_eq!(session.get_submit_text_feedback(), "feedback content");
}

#[test]
fn test_clear_pastes() {
    let mut session = Session::new(0);
    session.insert_paste_tab_input("content".to_string());
    session.insert_paste_feedback("feedback".to_string());

    assert!(session.has_tab_input_pastes());
    assert!(session.has_feedback_pastes());

    session.clear_tab_input_pastes();
    session.clear_feedback_pastes();

    assert!(!session.has_tab_input_pastes());
    assert!(!session.has_feedback_pastes());
}

#[test]
fn test_empty_paste_ignored() {
    let mut session = Session::new(0);
    session.insert_paste_tab_input("".to_string());

    assert!(!session.has_tab_input_pastes());
    assert!(session.tab_input.is_empty());
}

#[test]
fn test_insert_feedback_newline() {
    let mut session = Session::new(0);
    session.user_feedback = "hello".to_string();
    session.cursor_position = 5;

    session.insert_feedback_newline();

    assert_eq!(session.user_feedback, "hello\n");
    assert_eq!(session.cursor_position, 6);
}

#[test]
fn test_insert_feedback_newline_middle() {
    let mut session = Session::new(0);
    session.user_feedback = "hello world".to_string();
    session.cursor_position = 5;

    session.insert_feedback_newline();

    assert_eq!(session.user_feedback, "hello\n world");
    assert_eq!(session.cursor_position, 6);
}

#[test]
fn test_chat_message_with_summary_suffix_routes_to_existing_tab() {
    let mut session = Session::new(0);

    // Create the base Planning tab
    session.add_run_tab("Planning".to_string());
    assert_eq!(session.run_tabs.len(), 1);
    assert_eq!(session.run_tabs[0].phase, "Planning");

    // Add a message with " Summary" suffix - should route to existing Planning tab
    session.add_chat_message(
        "summarizer",
        "Planning Summary",
        "Summary content".to_string(),
    );

    // Should still have only one tab
    assert_eq!(session.run_tabs.len(), 1);
    assert_eq!(session.run_tabs[0].phase, "Planning");

    // Message should be added to the Planning tab
    assert_eq!(session.run_tabs[0].entries.len(), 1);
    match &session.run_tabs[0].entries[0] {
        RunTabEntry::Text(message) => assert_eq!(message.message, "Summary content"),
        _ => panic!("Expected text entry"),
    }
}

#[test]
fn test_chat_message_summary_suffix_creates_normalized_tab_if_missing() {
    let mut session = Session::new(0);

    // Add message with Summary suffix when no tab exists
    session.add_chat_message("summarizer", "Planning Summary", "Test".to_string());

    // Should create a tab with the normalized name (without suffix)
    assert_eq!(session.run_tabs.len(), 1);
    assert_eq!(session.run_tabs[0].phase, "Planning");
}

#[test]
fn test_review_iteration_phase_consistency() {
    let mut session = Session::new(0);

    // Create Reviewing #1 tab (matches reviewing.rs format)
    session.add_run_tab("Reviewing #1".to_string());

    // Add message to the same tab
    session.add_chat_message("reviewer", "Reviewing #1", "Review content".to_string());

    // Should still have only one tab
    assert_eq!(session.run_tabs.len(), 1);
    assert_eq!(session.run_tabs[0].entries.len(), 1);
}

#[test]
fn test_add_run_tab() {
    let mut session = Session::new(0);
    assert!(session.run_tabs.is_empty());

    session.add_run_tab("Planning".to_string());
    assert_eq!(session.run_tabs.len(), 1);
    assert_eq!(session.run_tabs[0].phase, "Planning");
    assert_eq!(session.active_run_tab, 0);

    session.add_run_tab("Reviewing".to_string());
    assert_eq!(session.run_tabs.len(), 2);
    assert_eq!(session.active_run_tab, 1);
}

#[test]
fn test_add_chat_message() {
    let mut session = Session::new(0);

    session.add_chat_message("claude", "Planning", "Hello world".to_string());
    assert_eq!(session.run_tabs.len(), 1);
    assert_eq!(session.run_tabs[0].entries.len(), 1);
    match &session.run_tabs[0].entries[0] {
        RunTabEntry::Text(message) => {
            assert_eq!(message.agent_name, "claude");
            assert_eq!(message.message, "Hello world");
        }
        _ => panic!("Expected text entry"),
    }
}

#[test]
fn test_add_chat_message_creates_tab_if_needed() {
    let mut session = Session::new(0);

    session.add_chat_message("codex", "Reviewing", "Test message".to_string());
    assert_eq!(session.run_tabs.len(), 1);
    assert_eq!(session.run_tabs[0].phase, "Reviewing");
}

#[test]
fn test_add_chat_message_filters_empty() {
    let mut session = Session::new(0);
    session.add_run_tab("Planning".to_string());

    session.add_chat_message("claude", "Planning", "".to_string());
    session.add_chat_message("claude", "Planning", "   ".to_string());
    assert!(session.run_tabs[0].entries.is_empty());
}

#[test]
fn test_run_tab_navigation() {
    let mut session = Session::new(0);
    session.add_run_tab("Planning".to_string());
    session.add_run_tab("Reviewing".to_string());
    session.add_run_tab("Revising".to_string());

    assert_eq!(session.active_run_tab, 2);

    session.prev_run_tab();
    assert_eq!(session.active_run_tab, 1);

    session.prev_run_tab();
    assert_eq!(session.active_run_tab, 0);

    session.prev_run_tab();
    assert_eq!(session.active_run_tab, 0);

    session.next_run_tab();
    assert_eq!(session.active_run_tab, 1);

    session.next_run_tab();
    assert_eq!(session.active_run_tab, 2);

    session.next_run_tab();
    assert_eq!(session.active_run_tab, 2);
}

#[test]
fn test_toggle_focus_with_todos_visible() {
    let mut session = Session::new(0);
    assert_eq!(session.focused_panel, FocusedPanel::Output);

    // When todos are visible, Output -> Todos -> Chat -> Output
    session.toggle_focus_with_visibility(true);
    assert_eq!(session.focused_panel, FocusedPanel::Todos);

    session.toggle_focus_with_visibility(true);
    assert_eq!(session.focused_panel, FocusedPanel::Chat);

    session.toggle_focus_with_visibility(true);
    assert_eq!(session.focused_panel, FocusedPanel::Output);
}

#[test]
fn test_toggle_focus_without_todos_visible() {
    let mut session = Session::new(0);
    assert_eq!(session.focused_panel, FocusedPanel::Output);

    // When todos are not visible, Output -> Chat -> Output (skips Todos)
    session.toggle_focus_with_visibility(false);
    assert_eq!(session.focused_panel, FocusedPanel::Chat);

    session.toggle_focus_with_visibility(false);
    assert_eq!(session.focused_panel, FocusedPanel::Output);
}

#[tokio::test]
async fn test_review_failure_events_update_history() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let sender = SessionEventSender::new(0, 0, tx);

    sender.send_review_round_started(ReviewKind::Implementation, 1);
    sender.send_reviewer_started(ReviewKind::Implementation, 1, "impl".to_string());
    sender.send_reviewer_failed(
        ReviewKind::Implementation,
        1,
        "impl".to_string(),
        "boom".to_string(),
    );
    sender.send_review_round_completed(ReviewKind::Implementation, 1, false);

    let mut session = Session::new(0);
    for _ in 0..4 {
        let event = rx.try_recv().expect("event");
        match event {
            Event::SessionReviewRoundStarted { kind, round, .. } => {
                session.start_review_round(kind, round);
            }
            Event::SessionReviewerStarted {
                kind,
                round,
                display_id,
                ..
            } => {
                session.reviewer_started(kind, round, display_id);
            }
            Event::SessionReviewerFailed {
                kind,
                round,
                display_id,
                error,
                ..
            } => {
                session.reviewer_failed(kind, round, display_id, error);
            }
            Event::SessionReviewRoundCompleted {
                kind,
                round,
                approved,
                ..
            } => {
                session.set_round_verdict(kind, round, approved);
            }
            _ => {}
        }
    }

    let round = session
        .review_history
        .iter()
        .find(|r| r.kind == ReviewKind::Implementation && r.round == 1)
        .expect("implementation round");

    assert_eq!(round.aggregate_verdict, Some(false));
    let reviewer = round
        .reviewers
        .iter()
        .find(|r| r.display_id == "impl")
        .expect("reviewer");
    assert!(matches!(reviewer.status, ReviewerStatus::Failed { .. }));
}

#[test]
fn test_toggle_focus_with_summary() {
    let mut session = Session::new(0);
    session.add_run_tab("Planning".to_string());
    session.run_tabs[0].summary_state = SummaryState::Ready;

    assert_eq!(session.focused_panel, FocusedPanel::Output);

    // With summary visible: Output -> Todos -> Chat -> Summary -> Output
    session.toggle_focus_with_visibility(true);
    assert_eq!(session.focused_panel, FocusedPanel::Todos);

    session.toggle_focus_with_visibility(true);
    assert_eq!(session.focused_panel, FocusedPanel::Chat);

    session.toggle_focus_with_visibility(true);
    assert_eq!(session.focused_panel, FocusedPanel::Summary);

    session.toggle_focus_with_visibility(true);
    assert_eq!(session.focused_panel, FocusedPanel::Output);
}

fn build_interactive_session() -> Session {
    use crate::domain::WorkflowEvent;

    let mut session = Session::new(0);
    let config = WorkflowConfig::claude_only_config();
    let agent_name = config
        .implementation
        .implementing_agent()
        .expect("implementing agent");
    let conversation_key = implementing_conversation_key(agent_name);
    let agent_id = AgentId::from(conversation_key.as_str());

    // Build a WorkflowView by applying events (proper CQRS approach)
    let mut view = WorkflowView::default();
    let aggregate_id = "test-workflow-id";

    // Apply events to construct the desired state
    view.apply_event(
        aggregate_id,
        &WorkflowEvent::ImplementationStarted {
            max_iterations: MaxIterations(1),
            started_at: TimestampUtc::default(),
        },
        1,
    );
    view.apply_event(
        aggregate_id,
        &WorkflowEvent::ImplementationRoundStarted {
            iteration: Iteration(1),
            started_at: TimestampUtc::default(),
        },
        2,
    );
    view.apply_event(
        aggregate_id,
        &WorkflowEvent::ImplementationReviewCompleted {
            iteration: Iteration(1),
            verdict: ImplementationVerdict::Approved,
            feedback: None,
            completed_at: TimestampUtc::default(),
        },
        3,
    );
    view.apply_event(
        aggregate_id,
        &WorkflowEvent::ImplementationAccepted {
            approved_at: TimestampUtc::default(),
        },
        4,
    );
    view.apply_event(
        aggregate_id,
        &WorkflowEvent::AgentConversationRecorded {
            agent_id: agent_id.clone(),
            resume_strategy: ResumeStrategy::ConversationResume,
            conversation_id: Some(ConversationId("conv-id".to_string())),
            updated_at: TimestampUtc::default(),
        },
        5,
    );

    session.workflow_view = Some(view);
    session.context = Some(SessionContext::new(
        PathBuf::from("/tmp"),
        Some(PathBuf::from("/tmp")),
        PathBuf::from("/tmp/state.json"),
        config,
    ));

    session
}

#[test]
fn test_can_interact_with_implementation() {
    let session = build_interactive_session();
    assert!(session.can_interact_with_implementation());
}

#[test]
fn test_toggle_focus_with_interaction() {
    let mut session = build_interactive_session();
    assert_eq!(session.focused_panel, FocusedPanel::Output);

    session.toggle_focus_with_visibility(false);
    assert_eq!(session.focused_panel, FocusedPanel::Chat);

    session.toggle_focus_with_visibility(false);
    assert_eq!(session.focused_panel, FocusedPanel::ChatInput);

    session.toggle_focus_with_visibility(false);
    assert_eq!(session.focused_panel, FocusedPanel::Output);
}

#[test]
fn test_focus_changes_to_chat_input_after_success_modal_dismiss_with_enter() {
    let mut session = build_interactive_session();
    assert_eq!(session.focused_panel, FocusedPanel::Output);

    // Open implementation success modal
    session.open_implementation_success(3);
    assert!(session.implementation_success_modal.is_some());

    // Simulate Enter key dismiss behavior: close modal and set focus
    session.close_implementation_success();
    if session.can_interact_with_implementation() {
        session.focused_panel = FocusedPanel::ChatInput;
    }

    // Verify modal is closed and focus moved to ChatInput
    assert!(session.implementation_success_modal.is_none());
    assert_eq!(session.focused_panel, FocusedPanel::ChatInput);
}

#[test]
fn test_focus_unchanged_after_success_modal_dismiss_with_esc() {
    let mut session = build_interactive_session();
    assert_eq!(session.focused_panel, FocusedPanel::Output);

    // Open implementation success modal
    session.open_implementation_success(3);
    assert!(session.implementation_success_modal.is_some());

    // Simulate Esc key dismiss behavior: close modal without changing focus
    session.close_implementation_success();
    // Note: Esc does NOT set focus to ChatInput

    // Verify modal is closed but focus remains unchanged
    assert!(session.implementation_success_modal.is_none());
    assert_eq!(session.focused_panel, FocusedPanel::Output);
}

#[test]
fn test_focus_unchanged_when_cannot_interact_with_implementation() {
    // Use basic session that cannot interact (no workflow view, no context)
    let mut session = Session::new(0);
    assert_eq!(session.focused_panel, FocusedPanel::Output);
    assert!(!session.can_interact_with_implementation());

    // Open implementation success modal
    session.open_implementation_success(3);
    assert!(session.implementation_success_modal.is_some());

    // Simulate Enter key dismiss behavior
    session.close_implementation_success();
    if session.can_interact_with_implementation() {
        session.focused_panel = FocusedPanel::ChatInput;
    }

    // Verify modal is closed but focus unchanged (can't interact)
    assert!(session.implementation_success_modal.is_none());
    assert_eq!(session.focused_panel, FocusedPanel::Output);
}

#[test]
fn test_hotkey_guard_blocks_when_chat_input_focused() {
    let mut session = Session::new(0);

    // Default state: not in text input mode
    let in_text_input_default = session.input_mode != InputMode::Normal
        || session.approval_mode == ApprovalMode::EnteringFeedback
        || session.approval_mode == ApprovalMode::EnteringIterations
        || session.focused_panel == FocusedPanel::ChatInput;
    assert!(!in_text_input_default);

    // When ChatInput is focused: should block hotkeys
    session.focused_panel = FocusedPanel::ChatInput;
    let in_text_input_chat = session.input_mode != InputMode::Normal
        || session.approval_mode == ApprovalMode::EnteringFeedback
        || session.approval_mode == ApprovalMode::EnteringIterations
        || session.focused_panel == FocusedPanel::ChatInput;
    assert!(in_text_input_chat);

    // When Chat (read-only) is focused: should NOT block hotkeys
    session.focused_panel = FocusedPanel::Chat;
    let in_text_input_readonly = session.input_mode != InputMode::Normal
        || session.approval_mode == ApprovalMode::EnteringFeedback
        || session.approval_mode == ApprovalMode::EnteringIterations
        || session.focused_panel == FocusedPanel::ChatInput;
    assert!(!in_text_input_readonly);
}

#[test]
fn test_hotkey_guard_blocks_for_existing_input_modes() {
    let mut session = Session::new(0);

    // EnteringFeedback should block
    session.approval_mode = ApprovalMode::EnteringFeedback;
    let in_text_feedback = session.input_mode != InputMode::Normal
        || session.approval_mode == ApprovalMode::EnteringFeedback
        || session.approval_mode == ApprovalMode::EnteringIterations
        || session.focused_panel == FocusedPanel::ChatInput;
    assert!(in_text_feedback);

    // EnteringIterations should block
    session.approval_mode = ApprovalMode::EnteringIterations;
    let in_text_iterations = session.input_mode != InputMode::Normal
        || session.approval_mode == ApprovalMode::EnteringFeedback
        || session.approval_mode == ApprovalMode::EnteringIterations
        || session.focused_panel == FocusedPanel::ChatInput;
    assert!(in_text_iterations);

    // NamingTab mode should block
    session.approval_mode = ApprovalMode::None;
    session.input_mode = InputMode::NamingTab;
    let in_text_naming = session.input_mode != InputMode::Normal
        || session.approval_mode == ApprovalMode::EnteringFeedback
        || session.approval_mode == ApprovalMode::EnteringIterations
        || session.focused_panel == FocusedPanel::ChatInput;
    assert!(in_text_naming);
}

#[test]
fn test_is_focus_on_invisible_todos() {
    let mut session = Session::new(0);
    session.focused_panel = FocusedPanel::Todos;

    assert!(session.is_focus_on_invisible_todos(false));
    assert!(!session.is_focus_on_invisible_todos(true));

    session.focused_panel = FocusedPanel::Output;
    assert!(!session.is_focus_on_invisible_todos(false));
}

#[test]
fn test_reset_focus_if_todos_invisible() {
    let mut session = Session::new(0);
    session.focused_panel = FocusedPanel::Todos;

    // Should reset to Output when todos not visible
    session.reset_focus_if_todos_invisible(false);
    assert_eq!(session.focused_panel, FocusedPanel::Output);

    // Should not change when todos are visible
    session.focused_panel = FocusedPanel::Todos;
    session.reset_focus_if_todos_invisible(true);
    assert_eq!(session.focused_panel, FocusedPanel::Todos);
}

#[test]
fn test_todo_scroll_up() {
    let mut session = Session::new(0);
    session.todo_scroll.position = 5;

    session.todo_scroll_up();
    assert_eq!(session.todo_scroll.position, 4);

    session.todo_scroll.position = 0;
    session.todo_scroll_up();
    assert_eq!(session.todo_scroll.position, 0); // Should not go negative
}

#[test]
fn test_todo_scroll_down() {
    let mut session = Session::new(0);
    session.todo_scroll.position = 0;

    session.todo_scroll_down(10);
    assert_eq!(session.todo_scroll.position, 1);

    session.todo_scroll.position = 10;
    session.todo_scroll_down(10);
    assert_eq!(session.todo_scroll.position, 10); // Should not exceed max
}

#[test]
fn test_todo_scroll_to_top() {
    let mut session = Session::new(0);
    session.todo_scroll.position = 5;

    session.todo_scroll_to_top();
    assert_eq!(session.todo_scroll.position, 0);
}

#[test]
fn test_todo_scroll_to_bottom() {
    let mut session = Session::new(0);
    session.todo_scroll.position = 0;

    session.todo_scroll_to_bottom(15);
    assert_eq!(session.todo_scroll.position, 15);
}

#[test]
fn test_clear_todos_resets_scroll() {
    let mut session = Session::new(0);
    session.update_todos(
        "agent".to_string(),
        vec![TodoItem {
            status: TodoStatus::Pending,
            active_form: "Task 1".to_string(),
        }],
    );
    session.todo_scroll.position = 5;

    session.clear_todos();

    assert!(session.todos.is_empty());
    assert_eq!(session.todo_scroll.position, 0);
}

#[test]
fn test_open_implementation_success() {
    let mut session = Session::new(0);
    assert!(session.implementation_success_modal.is_none());

    session.open_implementation_success(3);

    assert!(session.implementation_success_modal.is_some());
    let modal = session.implementation_success_modal.as_ref().unwrap();
    assert_eq!(modal.iterations_used, 3);
}

#[test]
fn test_close_implementation_success() {
    let mut session = Session::new(0);
    session.open_implementation_success(2);
    assert!(session.implementation_success_modal.is_some());

    session.close_implementation_success();

    assert!(session.implementation_success_modal.is_none());
}

#[test]
fn test_open_implementation_success_closes_plan_modal() {
    let mut session = Session::new(0);
    session.plan_modal_open = true;
    session.plan_modal_content = "Some plan content".to_string();

    session.open_implementation_success(1);

    // Plan modal should be closed
    assert!(!session.plan_modal_open);
    assert!(session.plan_modal_content.is_empty());
    // Success modal should be open
    assert!(session.implementation_success_modal.is_some());
}
