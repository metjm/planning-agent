//! Tests for the workflow state machine.

use super::*;
use crate::state::State;
use tempfile::TempDir;

/// Creates a test state machine with a logger in a temp directory.
fn create_test_machine() -> (WorkflowStateMachine, watch::Receiver<StateSnapshot>, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let logs_dir = temp_dir.path().join("logs");
    std::fs::create_dir_all(&logs_dir).expect("Failed to create logs dir");

    let logger =
        Arc::new(StructuredLogger::new("test-session", &logs_dir).expect("Failed to create logger"));

    let state = State::new("test-feature", "Test objective", 3).expect("Failed to create state");

    let (machine, snapshot_rx) = WorkflowStateMachine::new(state, logger);
    (machine, snapshot_rx, temp_dir)
}

#[test]
fn test_planning_to_reviewing_transition() {
    let (mut machine, snapshot_rx, _temp) = create_test_machine();

    // Initial state should be Planning
    assert_eq!(machine.state().phase, Phase::Planning);

    // Apply CompletePlanning command
    let events = machine
        .apply(StateCommand::CompletePlanning {
            plan_path: "/tmp/test.md".into(),
        })
        .expect("CompletePlanning should succeed");

    // Should have PhaseChanged event
    assert_eq!(events.len(), 1);
    match &events[0] {
        StateEvent::PhaseChanged { from, to } => {
            assert_eq!(*from, Phase::Planning);
            assert_eq!(*to, Phase::Reviewing);
        }
        _ => panic!("Expected PhaseChanged event"),
    }

    // State should be Reviewing
    assert_eq!(machine.state().phase, Phase::Reviewing);

    // Snapshot should be updated
    let snapshot = snapshot_rx.borrow();
    assert_eq!(snapshot.phase, Phase::Reviewing);
}

#[test]
fn test_reviewing_to_revising_transition() {
    let (mut machine, _snapshot_rx, _temp) = create_test_machine();

    // Move to Reviewing first
    machine
        .apply(StateCommand::CompletePlanning {
            plan_path: "/tmp/test.md".into(),
        })
        .unwrap();

    // Apply AllReviewersComplete with rejected
    let events = machine
        .apply(StateCommand::AllReviewersComplete { approved: false })
        .expect("AllReviewersComplete should succeed");

    assert_eq!(events.len(), 1);
    match &events[0] {
        StateEvent::PhaseChanged { from, to } => {
            assert_eq!(*from, Phase::Reviewing);
            assert_eq!(*to, Phase::Revising);
        }
        _ => panic!("Expected PhaseChanged event"),
    }

    assert_eq!(machine.state().phase, Phase::Revising);
}

#[test]
fn test_reviewing_to_complete_transition() {
    let (mut machine, _snapshot_rx, _temp) = create_test_machine();

    // Move to Reviewing first
    machine
        .apply(StateCommand::CompletePlanning {
            plan_path: "/tmp/test.md".into(),
        })
        .unwrap();

    // Apply AllReviewersComplete with approved
    let events = machine
        .apply(StateCommand::AllReviewersComplete { approved: true })
        .expect("AllReviewersComplete should succeed");

    assert_eq!(events.len(), 1);
    match &events[0] {
        StateEvent::PhaseChanged { from, to } => {
            assert_eq!(*from, Phase::Reviewing);
            assert_eq!(*to, Phase::Complete);
        }
        _ => panic!("Expected PhaseChanged event"),
    }

    assert_eq!(machine.state().phase, Phase::Complete);
}

#[test]
fn test_revising_to_reviewing_transition() {
    let (mut machine, _snapshot_rx, _temp) = create_test_machine();

    // Move to Reviewing, then Revising
    machine
        .apply(StateCommand::CompletePlanning {
            plan_path: "/tmp/test.md".into(),
        })
        .unwrap();
    machine
        .apply(StateCommand::AllReviewersComplete { approved: false })
        .unwrap();

    let initial_iteration = machine.state().iteration;

    // Apply CompleteRevising
    let events = machine
        .apply(StateCommand::CompleteRevising)
        .expect("CompleteRevising should succeed");

    assert_eq!(events.len(), 2);

    // First event should be IterationIncremented
    match &events[0] {
        StateEvent::IterationIncremented { new_value } => {
            assert_eq!(*new_value, initial_iteration + 1);
        }
        _ => panic!("Expected IterationIncremented event"),
    }

    // Second event should be PhaseChanged
    match &events[1] {
        StateEvent::PhaseChanged { from, to } => {
            assert_eq!(*from, Phase::Revising);
            assert_eq!(*to, Phase::Reviewing);
        }
        _ => panic!("Expected PhaseChanged event"),
    }

    assert_eq!(machine.state().phase, Phase::Reviewing);
    assert_eq!(machine.state().iteration, initial_iteration + 1);
}

#[test]
fn test_invalid_transition_from_planning_to_complete() {
    let (mut machine, _snapshot_rx, _temp) = create_test_machine();

    // Try to mark complete from Planning phase (invalid)
    let result = machine.apply(StateCommand::MarkComplete);

    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Can only mark complete from Reviewing phase"));

    // State should be unchanged
    assert_eq!(machine.state().phase, Phase::Planning);
}

#[test]
fn test_invalid_transition_from_revising_to_complete() {
    let (mut machine, _snapshot_rx, _temp) = create_test_machine();

    // Move to Revising
    machine
        .apply(StateCommand::CompletePlanning {
            plan_path: "/tmp/test.md".into(),
        })
        .unwrap();
    machine
        .apply(StateCommand::AllReviewersComplete { approved: false })
        .unwrap();

    // Try to mark complete from Revising phase (invalid)
    let result = machine.apply(StateCommand::MarkComplete);

    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Can only mark complete from Reviewing phase"));

    // State should be unchanged
    assert_eq!(machine.state().phase, Phase::Revising);
}

#[test]
fn test_restart_with_feedback() {
    let (mut machine, snapshot_rx, _temp) = create_test_machine();

    let original_objective = machine.state().objective.clone();

    // Move to Complete
    machine
        .apply(StateCommand::CompletePlanning {
            plan_path: "/tmp/test.md".into(),
        })
        .unwrap();
    machine
        .apply(StateCommand::AllReviewersComplete { approved: true })
        .unwrap();

    assert_eq!(machine.state().phase, Phase::Complete);

    // Restart with feedback
    let events = machine
        .apply(StateCommand::RestartWithFeedback {
            feedback: "Please improve error handling".to_string(),
        })
        .expect("RestartWithFeedback should succeed");

    assert_eq!(events.len(), 3);

    // Should have PhaseChanged, IterationReset, and WorkflowRestarted events
    match &events[0] {
        StateEvent::PhaseChanged { from, to } => {
            assert_eq!(*from, Phase::Complete);
            assert_eq!(*to, Phase::Planning);
        }
        _ => panic!("Expected PhaseChanged event"),
    }

    match &events[1] {
        StateEvent::IterationReset => {}
        _ => panic!("Expected IterationReset event"),
    }

    match &events[2] {
        StateEvent::WorkflowRestarted { feedback_preview } => {
            assert!(feedback_preview.contains("Please improve error handling"));
        }
        _ => panic!("Expected WorkflowRestarted event"),
    }

    // State should be reset
    assert_eq!(machine.state().phase, Phase::Planning);
    assert_eq!(machine.state().iteration, 1);
    assert!(machine
        .state()
        .objective
        .contains("Please improve error handling"));
    assert!(machine.state().objective.starts_with(&original_objective));

    // Snapshot should be updated
    let snapshot = snapshot_rx.borrow();
    assert_eq!(snapshot.phase, Phase::Planning);
}

#[test]
fn test_agent_failed_records_failure() {
    let (mut machine, _snapshot_rx, _temp) = create_test_machine();

    let events = machine
        .apply(StateCommand::AgentFailed {
            agent_id: "claude".to_string(),
            error: "Connection timeout".to_string(),
        })
        .expect("AgentFailed should succeed");

    assert_eq!(events.len(), 1);
    match &events[0] {
        StateEvent::ErrorOccurred { error } => {
            assert_eq!(error, "Connection timeout");
        }
        _ => panic!("Expected ErrorOccurred event"),
    }

    // State should have failure recorded
    assert!(machine.state().has_failure());
    let failure = machine.state().last_failure.as_ref().unwrap();
    assert_eq!(failure.agent_name, Some("claude".to_string()));
}

#[test]
fn test_clear_failure() {
    let (mut machine, _snapshot_rx, _temp) = create_test_machine();

    // First set a failure
    machine
        .apply(StateCommand::AgentFailed {
            agent_id: "claude".to_string(),
            error: "Test error".to_string(),
        })
        .unwrap();

    assert!(machine.state().has_failure());

    // Clear the failure
    let events = machine
        .apply(StateCommand::ClearFailure)
        .expect("ClearFailure should succeed");

    assert_eq!(events.len(), 1);
    match &events[0] {
        StateEvent::ErrorCleared => {}
        _ => panic!("Expected ErrorCleared event"),
    }

    assert!(!machine.state().has_failure());
}

#[test]
fn test_snapshot_broadcast() {
    let (mut machine, snapshot_rx, _temp) = create_test_machine();

    // Initial snapshot
    let initial_snapshot = snapshot_rx.borrow().clone();
    assert_eq!(initial_snapshot.phase, Phase::Planning);

    // Apply a command
    machine
        .apply(StateCommand::CompletePlanning {
            plan_path: "/tmp/test.md".into(),
        })
        .unwrap();

    // Snapshot should be updated
    let updated_snapshot = snapshot_rx.borrow().clone();
    assert_eq!(updated_snapshot.phase, Phase::Reviewing);
}

#[test]
fn test_watch_channel_receiver_dropped() {
    let (mut machine, snapshot_rx, _temp) = create_test_machine();

    // Drop the receiver
    drop(snapshot_rx);

    // Apply commands - should not panic even without receivers
    let result = machine.apply(StateCommand::CompletePlanning {
        plan_path: "/tmp/test.md".into(),
    });

    assert!(result.is_ok());
    assert_eq!(machine.state().phase, Phase::Reviewing);
}

#[test]
fn test_user_override_approval() {
    let (mut machine, _snapshot_rx, _temp) = create_test_machine();

    // From Planning phase (would normally be invalid to go to Complete)
    assert_eq!(machine.state().phase, Phase::Planning);

    // User override should work regardless of phase
    let events = machine
        .apply(StateCommand::UserOverrideApproval)
        .expect("UserOverrideApproval should succeed");

    assert_eq!(events.len(), 2);

    match &events[1] {
        StateEvent::WorkflowComplete {
            approved,
            override_used,
        } => {
            assert!(*approved);
            assert!(*override_used);
        }
        _ => panic!("Expected WorkflowComplete event"),
    }

    assert_eq!(machine.state().phase, Phase::Complete);
    assert!(machine.state().approval_overridden);
}

#[test]
fn test_sequential_review_commands() {
    let (mut machine, _snapshot_rx, _temp) = create_test_machine();

    // Initially no sequential review state
    assert!(machine.state().sequential_review.is_none());

    // Initialize
    machine
        .apply(StateCommand::InitSequentialReview)
        .expect("InitSequentialReview should succeed");

    assert!(machine.state().sequential_review.is_some());
    assert_eq!(
        machine
            .state()
            .sequential_review
            .as_ref()
            .unwrap()
            .current_reviewer_index,
        0
    );

    // Advance
    machine
        .apply(StateCommand::AdvanceSequentialReviewer)
        .expect("AdvanceSequentialReviewer should succeed");

    assert_eq!(
        machine
            .state()
            .sequential_review
            .as_ref()
            .unwrap()
            .current_reviewer_index,
        1
    );

    // Clear
    machine
        .apply(StateCommand::ClearSequentialReview)
        .expect("ClearSequentialReview should succeed");

    assert!(machine.state().sequential_review.is_none());
}

#[test]
fn test_increment_iteration() {
    let (mut machine, _snapshot_rx, _temp) = create_test_machine();

    let initial = machine.state().iteration;

    let events = machine
        .apply(StateCommand::IncrementIteration)
        .expect("IncrementIteration should succeed");

    assert_eq!(events.len(), 1);
    match &events[0] {
        StateEvent::IterationIncremented { new_value } => {
            assert_eq!(*new_value, initial + 1);
        }
        _ => panic!("Expected IterationIncremented event"),
    }

    assert_eq!(machine.state().iteration, initial + 1);
}

#[test]
fn test_extend_max_iterations() {
    let (mut machine, _snapshot_rx, _temp) = create_test_machine();

    let initial = machine.state().max_iterations;

    let events = machine
        .apply(StateCommand::ExtendMaxIterations)
        .expect("ExtendMaxIterations should succeed");

    // No events for this command
    assert!(events.is_empty());

    assert_eq!(machine.state().max_iterations, initial + 1);
}

#[test]
fn test_update_agent_conversation() {
    let (mut machine, _snapshot_rx, _temp) = create_test_machine();

    // First create an agent session
    machine
        .state_mut()
        .get_or_create_agent_session("claude", crate::state::ResumeStrategy::ConversationResume);

    // Update conversation ID
    let events = machine
        .apply(StateCommand::UpdateAgentConversation {
            agent: "claude".to_string(),
            conversation_id: "conv-123".to_string(),
        })
        .expect("UpdateAgentConversation should succeed");

    assert_eq!(events.len(), 1);
    match &events[0] {
        StateEvent::AgentConversationUpdated { agent } => {
            assert_eq!(agent, "claude");
        }
        _ => panic!("Expected AgentConversationUpdated event"),
    }

    let conv_state = machine
        .state()
        .agent_conversations
        .get("claude")
        .expect("Agent should exist");
    assert_eq!(conv_state.conversation_id, Some("conv-123".to_string()));
}

#[test]
fn test_record_invocation() {
    let (mut machine, _snapshot_rx, _temp) = create_test_machine();

    let events = machine
        .apply(StateCommand::RecordInvocation {
            agent: "gemini".to_string(),
            phase: "reviewing".to_string(),
        })
        .expect("RecordInvocation should succeed");

    assert_eq!(events.len(), 1);
    match &events[0] {
        StateEvent::InvocationRecorded { agent, phase } => {
            assert_eq!(agent, "gemini");
            assert_eq!(phase, "reviewing");
        }
        _ => panic!("Expected InvocationRecorded event"),
    }

    assert_eq!(machine.state().invocations.len(), 1);
    assert_eq!(machine.state().invocations[0].agent, "gemini");
    assert_eq!(machine.state().invocations[0].phase, "reviewing");
}
