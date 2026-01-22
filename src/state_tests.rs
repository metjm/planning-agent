//! Tests for state module.

use super::*;
use std::path::PathBuf;

#[test]
fn test_new_state() {
    let state = State::new("user-auth", "Implement authentication", 3).unwrap();
    assert_eq!(state.phase, Phase::Planning);
    assert_eq!(state.iteration, 1);

    // Plan file should be in session directory: ~/.planning-agent/sessions/<session-id>/plan.md
    let plan_file_str = state.plan_file.to_string_lossy();
    assert!(
        plan_file_str.contains(".planning-agent/sessions/"),
        "got: {}",
        plan_file_str
    );
    assert!(plan_file_str.ends_with("/plan.md"), "got: {}", plan_file_str);
    // Verify session ID is in the path
    assert!(
        plan_file_str.contains(&state.workflow_session_id),
        "got: {}",
        plan_file_str
    );
}

#[test]
fn test_new_state_feedback_file_has_round_number() {
    let state = State::new("user-auth", "Implement authentication", 3).unwrap();

    // Feedback file should be in session directory: ~/.planning-agent/sessions/<session-id>/feedback_1.md
    let feedback_file_str = state.feedback_file.to_string_lossy();
    assert!(
        feedback_file_str.contains(".planning-agent/sessions/"),
        "got: {}",
        feedback_file_str
    );
    assert!(
        feedback_file_str.ends_with("/feedback_1.md"),
        "got: {}",
        feedback_file_str
    );
}

#[test]
fn test_update_feedback_for_iteration() {
    let mut state = State::new("test-feature", "Test objective", 3).unwrap();

    // Initial feedback file should have round 1
    assert!(state.feedback_file.to_string_lossy().ends_with("/feedback_1.md"));

    // Update to round 2
    state.update_feedback_for_iteration(2);
    assert!(state.feedback_file.to_string_lossy().ends_with("/feedback_2.md"));

    // Update to round 3
    state.update_feedback_for_iteration(3);
    assert!(state.feedback_file.to_string_lossy().ends_with("/feedback_3.md"));
}

#[test]
fn test_extract_plan_folder_new_format() {
    // New format: ~/.planning-agent/plans/YYYYMMDD-HHMMSS-xxxxxxxx_my-feature/plan.md
    let plan_file =
        PathBuf::from("/home/user/.planning-agent/plans/20250101-120000-abcd1234_my-feature/plan.md");
    let folder = extract_plan_folder(&plan_file);
    assert_eq!(folder, Some("20250101-120000-abcd1234_my-feature".to_string()));
}

#[test]
fn test_extract_plan_folder_legacy_format() {
    let plan_file = PathBuf::from("docs/plans/existing-feature.md");
    let folder = extract_plan_folder(&plan_file);
    assert_eq!(folder, None);
}

#[test]
fn test_extract_sanitized_name_new_format() {
    // New format: folder contains the feature name
    let plan_file =
        PathBuf::from("/home/user/.planning-agent/plans/20250101-120000-abcd1234_my-feature/plan.md");
    let name = extract_sanitized_name(&plan_file);
    assert_eq!(name, Some("my-feature".to_string()));
}

#[test]
fn test_extract_sanitized_name_legacy_format() {
    let plan_file = PathBuf::from("docs/plans/existing-feature.md");
    let name = extract_sanitized_name(&plan_file);
    assert_eq!(name, Some("existing-feature".to_string()));
}

#[test]
fn test_is_session_centric_path() {
    // Session-centric path (UUID in parent)
    let session_path = PathBuf::from(
        "/home/user/.planning-agent/sessions/550e8400-e29b-41d4-a716-446655440000/plan.md",
    );
    assert!(is_session_centric_path(&session_path));

    // Legacy plan path (timestamp-uuid_feature format)
    let legacy_path =
        PathBuf::from("/home/user/.planning-agent/plans/20250101-120000-abcd1234_my-feature/plan.md");
    assert!(!is_session_centric_path(&legacy_path));

    // Docs path
    let docs_path = PathBuf::from("docs/plans/feature.md");
    assert!(!is_session_centric_path(&docs_path));
}

#[test]
fn test_update_feedback_for_iteration_with_legacy_plan_file() {
    // Simulate loading a state with legacy plan file format
    let mut state = State::new("test", "test", 3).unwrap();
    let session_id = state.workflow_session_id.clone();
    // Manually set to legacy format
    state.plan_file = PathBuf::from("docs/plans/existing-feature.md");
    state.feedback_file = PathBuf::from("docs/plans/existing-feature_feedback.md");

    // Update to round 2 - should use session-centric path since session_id is set
    state.update_feedback_for_iteration(2);

    // Feedback file should use session directory since workflow_session_id is present
    let feedback_str = state.feedback_file.to_string_lossy();
    assert!(
        feedback_str.contains(".planning-agent/sessions/"),
        "got: {}",
        feedback_str
    );
    assert!(feedback_str.ends_with("/feedback_2.md"), "got: {}", feedback_str);
    assert!(feedback_str.contains(&session_id), "got: {}", feedback_str);
}

#[test]
fn test_update_feedback_for_iteration_with_legacy_plan_file_no_session_id() {
    // Simulate loading a very old state with no session_id
    let mut state = State::new("test", "test", 3).unwrap();
    // Clear session ID to simulate legacy state
    state.workflow_session_id = String::new();
    // Manually set to legacy format
    state.plan_file = PathBuf::from("docs/plans/existing-feature.md");
    state.feedback_file = PathBuf::from("docs/plans/existing-feature_feedback.md");

    // Update to round 2 - should generate a new folder for feedback (legacy path)
    state.update_feedback_for_iteration(2);

    // Feedback file should be in a new folder with the proper format
    let feedback_str = state.feedback_file.to_string_lossy();
    assert!(
        feedback_str.contains(".planning-agent/plans/"),
        "got: {}",
        feedback_str
    );
    assert!(feedback_str.ends_with("/feedback_2.md"), "got: {}", feedback_str);
    assert!(feedback_str.contains("_existing-feature/"), "got: {}", feedback_str);
}

#[test]
fn test_valid_transitions() {
    let mut state = State::new("test", "test", 3).unwrap();

    assert!(state.transition(Phase::Reviewing).is_ok());
    assert_eq!(state.phase, Phase::Reviewing);

    assert!(state.transition(Phase::Revising).is_ok());
    assert_eq!(state.phase, Phase::Revising);

    assert!(state.transition(Phase::Reviewing).is_ok());
    assert!(state.transition(Phase::Complete).is_ok());
}

#[test]
fn test_invalid_transition() {
    let mut state = State::new("test", "test", 3).unwrap();
    assert!(state.transition(Phase::Complete).is_err());
}

#[test]
fn test_should_continue() {
    let mut state = State::new("test", "test", 2).unwrap();
    assert!(state.should_continue());

    state.iteration = 3;
    assert!(!state.should_continue());

    state.iteration = 1;
    state.phase = Phase::Complete;
    assert!(!state.should_continue());
}

#[test]
fn test_new_state_has_workflow_session_id() {
    let state = State::new("test", "test objective", 3).unwrap();
    assert!(!state.workflow_session_id.is_empty());
    assert!(state.agent_conversations.is_empty());
    assert!(state.invocations.is_empty());
}

#[test]
fn test_workflow_session_id_is_stable() {
    let state = State::new("test", "test objective", 3).unwrap();
    let session_id = state.workflow_session_id.clone();
    assert_eq!(state.workflow_session_id, session_id);
}

#[test]
fn test_get_or_create_agent_session_stateless() {
    let mut state = State::new("test", "test objective", 3).unwrap();
    let session = state.get_or_create_agent_session("claude", ResumeStrategy::Stateless);

    assert_eq!(session.resume_strategy, ResumeStrategy::Stateless);
    assert!(session.conversation_id.is_none());
    assert!(!session.last_used_at.is_empty());
}

#[test]
fn test_get_or_create_agent_session_with_conversation_resume() {
    let mut state = State::new("test", "test objective", 3).unwrap();
    let session = state.get_or_create_agent_session("claude", ResumeStrategy::ConversationResume);

    assert_eq!(session.resume_strategy, ResumeStrategy::ConversationResume);
    // Initially None - will be captured from agent output after first run
    assert!(session.conversation_id.is_none());
}

#[test]
fn test_update_agent_conversation_id() {
    let mut state = State::new("test", "test objective", 3).unwrap();
    state.get_or_create_agent_session("claude", ResumeStrategy::ConversationResume);

    // Initially None
    assert!(state
        .agent_conversations
        .get("claude")
        .unwrap()
        .conversation_id
        .is_none());

    // Update with captured ID
    state.update_agent_conversation_id("claude", "captured-uuid-123".to_string());

    // Now it should be set
    let session = state.agent_conversations.get("claude").unwrap();
    assert_eq!(session.conversation_id, Some("captured-uuid-123".to_string()));
}

#[test]
fn test_agent_session_is_reused() {
    let mut state = State::new("test", "test objective", 3).unwrap();

    state.get_or_create_agent_session("claude", ResumeStrategy::ConversationResume);
    state.update_agent_conversation_id("claude", "test-uuid".to_string());

    let session1 = state.get_or_create_agent_session("claude", ResumeStrategy::ConversationResume);
    let key1 = session1.conversation_id.clone();

    let session2 = state.get_or_create_agent_session("claude", ResumeStrategy::ConversationResume);
    let key2 = session2.conversation_id.clone();

    assert_eq!(key1, key2);
    assert_eq!(key1, Some("test-uuid".to_string()));
}

#[test]
fn test_record_invocation() {
    let mut state = State::new("test", "test objective", 3).unwrap();
    state.get_or_create_agent_session("claude", ResumeStrategy::ConversationResume);
    state.update_agent_conversation_id("claude", "test-conv-id".to_string());
    state.record_invocation("claude", "Planning");

    assert_eq!(state.invocations.len(), 1);
    let inv = &state.invocations[0];
    assert_eq!(inv.agent, "claude");
    assert_eq!(inv.phase, "Planning");
    assert!(!inv.timestamp.is_empty());
    assert_eq!(inv.conversation_id, Some("test-conv-id".to_string()));
    assert_eq!(inv.resume_strategy, ResumeStrategy::ConversationResume);
}

#[test]
fn test_record_invocation_without_conversation_id() {
    let mut state = State::new("test", "test objective", 3).unwrap();
    state.get_or_create_agent_session("claude", ResumeStrategy::ConversationResume);
    // Don't update conversation_id - simulating first run before capture
    state.record_invocation("claude", "Planning");

    let inv = &state.invocations[0];
    assert!(inv.conversation_id.is_none());
}

#[test]
fn test_ensure_workflow_session_id() {
    let mut state = State::new("test", "test objective", 3).unwrap();
    state.workflow_session_id = String::new();
    assert!(state.workflow_session_id.is_empty());

    state.ensure_workflow_session_id();
    assert!(!state.workflow_session_id.is_empty());
}

#[test]
fn test_backward_compatibility_with_existing_state() {
    let old_state_json = r#"{
        "phase": "reviewing",
        "iteration": 2,
        "max_iterations": 3,
        "feature_name": "existing-feature",
        "objective": "Some objective",
        "plan_file": "docs/plans/existing-feature.md",
        "feedback_file": "docs/plans/existing-feature_feedback.md",
        "last_feedback_status": "needs_revision",
        "approval_overridden": false
    }"#;

    let state: State = serde_json::from_str(old_state_json).unwrap();
    assert_eq!(state.feature_name, "existing-feature");
    assert!(state.workflow_session_id.is_empty());
    assert!(state.agent_conversations.is_empty());
    assert!(state.invocations.is_empty());
}

#[test]
fn test_state_serialization_with_session_data() {
    let mut state = State::new("test", "test objective", 3).unwrap();
    state.get_or_create_agent_session("claude", ResumeStrategy::ConversationResume);
    state.record_invocation("claude", "Planning");

    let json = serde_json::to_string(&state).unwrap();
    let loaded: State = serde_json::from_str(&json).unwrap();

    assert_eq!(loaded.workflow_session_id, state.workflow_session_id);
    assert_eq!(loaded.agent_conversations.len(), 1);
    assert!(loaded.agent_conversations.contains_key("claude"));
    assert_eq!(loaded.invocations.len(), 1);
}

#[test]
fn test_phase_label_short() {
    assert_eq!(PhaseLabel::Planning.short(), "Plan");
    assert_eq!(PhaseLabel::Reviewing.short(), "Review");
    assert_eq!(PhaseLabel::Revising.short(), "Revise");
    assert_eq!(PhaseLabel::Complete.short(), "Done");
}

#[test]
fn test_phase_label_full() {
    assert_eq!(PhaseLabel::Planning.full(), "Planning");
    assert_eq!(PhaseLabel::Reviewing.full(), "Reviewing");
    assert_eq!(PhaseLabel::Revising.full(), "Revising");
    assert_eq!(PhaseLabel::Complete.full(), "Complete");
}

#[test]
fn test_phase_label_with_iteration() {
    assert_eq!(PhaseLabel::Planning.with_iteration(1), "Planning");
    assert_eq!(PhaseLabel::Reviewing.with_iteration(1), "Reviewing");
    assert_eq!(PhaseLabel::Reviewing.with_iteration(2), "Reviewing #2");
    assert_eq!(PhaseLabel::Revising.with_iteration(1), "Revising #1");
    assert_eq!(PhaseLabel::Revising.with_iteration(3), "Revising #3");
    assert_eq!(PhaseLabel::Complete.with_iteration(5), "Complete");
}

#[test]
fn test_phase_label_display() {
    assert_eq!(format!("{}", PhaseLabel::Planning), "Planning");
    assert_eq!(format!("{}", PhaseLabel::Reviewing), "Reviewing");
}

#[test]
fn test_phase_to_label() {
    assert_eq!(Phase::Planning.label(), PhaseLabel::Planning);
    assert_eq!(Phase::Reviewing.label(), PhaseLabel::Reviewing);
    assert_eq!(Phase::Revising.label(), PhaseLabel::Revising);
    assert_eq!(Phase::Complete.label(), PhaseLabel::Complete);
}

#[test]
fn test_new_state_has_updated_at() {
    let state = State::new("test", "test objective", 3).unwrap();
    assert!(!state.updated_at.is_empty());
    assert!(state.has_updated_at());
}

#[test]
fn test_set_updated_at() {
    let mut state = State::new("test", "test objective", 3).unwrap();
    let original = state.updated_at.clone();

    // Wait a tiny bit and update
    std::thread::sleep(std::time::Duration::from_millis(10));
    state.set_updated_at();

    // Timestamp should have changed
    assert_ne!(state.updated_at, original);
    assert!(state.has_updated_at());
}

#[test]
fn test_set_updated_at_with() {
    let mut state = State::new("test", "test objective", 3).unwrap();
    let custom_time = "2025-12-29T15:00:00Z";
    state.set_updated_at_with(custom_time);
    assert_eq!(state.updated_at, custom_time);
}

#[test]
fn test_legacy_state_without_updated_at() {
    // Simulate loading a legacy state file without updated_at field
    let old_state_json = r#"{
        "phase": "reviewing",
        "iteration": 2,
        "max_iterations": 3,
        "feature_name": "existing-feature",
        "objective": "Some objective",
        "plan_file": "docs/plans/existing-feature.md",
        "feedback_file": "docs/plans/existing-feature_feedback.md",
        "last_feedback_status": "needs_revision",
        "approval_overridden": false
    }"#;

    let state: State = serde_json::from_str(old_state_json).unwrap();
    // updated_at should default to empty string
    assert!(state.updated_at.is_empty());
    assert!(!state.has_updated_at());
}

#[test]
fn test_set_failure() {
    use crate::app::failure::{FailureContext, FailureKind};

    let mut state = State::new("test", "test objective", 3).unwrap();
    assert!(!state.has_failure());
    assert!(state.last_failure.is_none());
    assert!(state.failure_history.is_empty());

    let failure = FailureContext::new(
        FailureKind::Network,
        Phase::Reviewing,
        Some("codex".to_string()),
        2,
    );
    state.set_failure(failure);

    assert!(state.has_failure());
    assert!(state.last_failure.is_some());
    assert_eq!(state.failure_history.len(), 1);

    let last = state.last_failure.as_ref().unwrap();
    assert_eq!(last.kind, FailureKind::Network);
    assert_eq!(last.phase, Phase::Reviewing);
    assert_eq!(last.agent_name, Some("codex".to_string()));
}

#[test]
fn test_clear_failure() {
    use crate::app::failure::{FailureContext, FailureKind};

    let mut state = State::new("test", "test objective", 3).unwrap();
    let failure = FailureContext::new(FailureKind::Timeout, Phase::Planning, None, 2);
    state.set_failure(failure);

    assert!(state.has_failure());
    state.clear_failure();

    assert!(!state.has_failure());
    assert!(state.last_failure.is_none());
    // History should still have the failure
    assert_eq!(state.failure_history.len(), 1);
}

#[test]
fn test_failure_history_trimming() {
    use crate::app::failure::{FailureContext, FailureKind, MAX_FAILURE_HISTORY};

    let mut state = State::new("test", "test objective", 3).unwrap();

    // Add more failures than the limit
    for i in 0..(MAX_FAILURE_HISTORY + 10) {
        let failure = FailureContext::new(
            FailureKind::Network,
            Phase::Reviewing,
            Some(format!("agent-{}", i)),
            2,
        );
        state.set_failure(failure);
    }

    // History should be trimmed to MAX_FAILURE_HISTORY
    assert_eq!(state.failure_history.len(), MAX_FAILURE_HISTORY);

    // The oldest failures should have been removed
    // The first remaining failure should be agent-10 (since we added 60 and kept 50)
    let first = &state.failure_history[0];
    assert_eq!(first.agent_name, Some("agent-10".to_string()));
}

#[test]
fn test_state_serialization_with_failure() {
    use crate::app::failure::{FailureContext, FailureKind};

    let mut state = State::new("test", "test objective", 3).unwrap();
    let failure = FailureContext::new(FailureKind::AllReviewersFailed, Phase::Reviewing, None, 3);
    state.set_failure(failure);

    let json = serde_json::to_string(&state).unwrap();
    let loaded: State = serde_json::from_str(&json).unwrap();

    assert!(loaded.has_failure());
    assert_eq!(loaded.failure_history.len(), 1);
    let last = loaded.last_failure.as_ref().unwrap();
    assert_eq!(last.kind, FailureKind::AllReviewersFailed);
}

#[test]
fn test_backward_compatibility_without_failure_fields() {
    // Simulate loading a state without failure fields (pre-failure-handling state)
    let old_state_json = r#"{
        "phase": "reviewing",
        "iteration": 2,
        "max_iterations": 3,
        "feature_name": "existing-feature",
        "objective": "Some objective",
        "plan_file": "docs/plans/existing-feature.md",
        "feedback_file": "docs/plans/existing-feature_feedback.md",
        "last_feedback_status": "needs_revision",
        "approval_overridden": false
    }"#;

    let state: State = serde_json::from_str(old_state_json).unwrap();
    // Failure fields should default properly
    assert!(state.last_failure.is_none());
    assert!(state.failure_history.is_empty());
    assert!(!state.has_failure());
}

// Implementation phase state tests
#[test]
fn test_implementation_phase_state_new() {
    let state = ImplementationPhaseState::new(3);
    assert_eq!(state.phase, ImplementationPhase::Implementing);
    assert_eq!(state.iteration, 1);
    assert_eq!(state.max_iterations, 3);
    assert!(state.last_verdict.is_none());
    assert!(state.last_feedback.is_none());
}

#[test]
fn test_implementation_phase_state_can_continue() {
    let mut state = ImplementationPhaseState::new(3);
    assert!(state.can_continue());

    state.iteration = 3;
    assert!(state.can_continue());

    state.iteration = 4;
    assert!(!state.can_continue());

    state.iteration = 1;
    state.phase = ImplementationPhase::Complete;
    assert!(!state.can_continue());
}

#[test]
fn test_implementation_phase_state_is_approved() {
    let mut state = ImplementationPhaseState::new(3);
    assert!(!state.is_approved());

    state.last_verdict = Some("NEEDS_REVISION".to_string());
    assert!(!state.is_approved());

    state.last_verdict = Some("APPROVED".to_string());
    assert!(state.is_approved());
}

#[test]
fn test_implementation_phase_state_transitions() {
    let mut state = ImplementationPhaseState::new(3);
    assert_eq!(state.phase, ImplementationPhase::Implementing);

    state.advance_to_review();
    assert_eq!(state.phase, ImplementationPhase::ImplementationReview);

    state.advance_to_next_iteration();
    assert_eq!(state.phase, ImplementationPhase::Implementing);
    assert_eq!(state.iteration, 2);

    state.mark_complete();
    assert_eq!(state.phase, ImplementationPhase::Complete);
}

#[test]
fn test_implementation_phase_label() {
    assert_eq!(ImplementationPhase::Implementing.label(), "Implementing");
    assert_eq!(
        ImplementationPhase::ImplementationReview.label(),
        "Reviewing Implementation"
    );
    assert_eq!(
        ImplementationPhase::Complete.label(),
        "Implementation Complete"
    );
}

#[test]
fn test_implementation_state_serialization() {
    let mut state = ImplementationPhaseState::new(5);
    state.phase = ImplementationPhase::ImplementationReview;
    state.iteration = 2;
    state.last_verdict = Some("NEEDS_REVISION".to_string());
    state.last_feedback = Some("Fix the bug".to_string());

    let json = serde_json::to_string(&state).unwrap();
    let loaded: ImplementationPhaseState = serde_json::from_str(&json).unwrap();

    assert_eq!(loaded.phase, ImplementationPhase::ImplementationReview);
    assert_eq!(loaded.iteration, 2);
    assert_eq!(loaded.max_iterations, 5);
    assert_eq!(loaded.last_verdict, Some("NEEDS_REVISION".to_string()));
    assert_eq!(loaded.last_feedback, Some("Fix the bug".to_string()));
}

// SequentialReviewState round-robin tests
#[test]
fn test_increment_run_count() {
    let mut state = SequentialReviewState::new();
    assert_eq!(state.get_run_count("A"), 0);

    state.increment_run_count("A");
    assert_eq!(state.get_run_count("A"), 1);

    state.increment_run_count("A");
    assert_eq!(state.get_run_count("A"), 2);

    state.increment_run_count("B");
    assert_eq!(state.get_run_count("B"), 1);
    assert_eq!(state.get_run_count("A"), 2);
}

#[test]
fn test_get_run_count_unknown() {
    let state = SequentialReviewState::new();
    assert_eq!(state.get_run_count("unknown"), 0);
}

#[test]
fn test_start_new_cycle_sorts_by_count() {
    let mut state = SequentialReviewState::new();
    state.reviewer_run_counts.insert("A".to_string(), 5);
    state.reviewer_run_counts.insert("B".to_string(), 2);
    state.reviewer_run_counts.insert("C".to_string(), 8);

    state.start_new_cycle(&["A", "B", "C"]);

    // Should be sorted by run count: B(2), A(5), C(8)
    assert_eq!(state.current_cycle_order, vec!["B", "A", "C"]);
    assert_eq!(state.current_reviewer_index, 0);
}

#[test]
fn test_start_new_cycle_stable_sort() {
    // When run counts are equal, config order should be preserved (stable sort)
    let mut state = SequentialReviewState::new();
    state.reviewer_run_counts.insert("A".to_string(), 2);
    state.reviewer_run_counts.insert("B".to_string(), 2);
    state.reviewer_run_counts.insert("C".to_string(), 2);

    state.start_new_cycle(&["A", "B", "C"]);

    // All have same count, so config order is preserved
    assert_eq!(state.current_cycle_order, vec!["A", "B", "C"]);
}

#[test]
fn test_mid_cycle_order_stability() {
    // This is the critical test: order must remain stable mid-cycle even after counts change
    let mut state = SequentialReviewState::new();
    state.reviewer_run_counts.insert("A".to_string(), 2);
    state.reviewer_run_counts.insert("B".to_string(), 2);
    state.reviewer_run_counts.insert("C".to_string(), 8);

    // Start cycle: order is [A, B, C] (counts 2, 2, 8)
    state.start_new_cycle(&["A", "B", "C"]);
    assert_eq!(state.current_cycle_order, vec!["A", "B", "C"]);

    // Get current reviewer (index 0)
    assert_eq!(state.get_current_reviewer(), Some("A"));

    // Increment A's count (simulating A running)
    state.increment_run_count("A");
    // Now counts are A:3, B:2, C:8

    // Advance to next reviewer
    state.advance_to_next_reviewer();
    assert_eq!(state.current_reviewer_index, 1);

    // CRITICAL: get_current_reviewer should return "B", not "A"
    // If we were re-sorting, "A" would be at index 1 (since B:2 < A:3 < C:8)
    // But since we use stored order, "B" is at index 1
    assert_eq!(state.get_current_reviewer(), Some("B"));
}

#[test]
fn test_reset_clears_cycle_order() {
    let mut state = SequentialReviewState::new();
    state.start_new_cycle(&["A", "B"]);
    assert!(!state.current_cycle_order.is_empty());

    state.reset_to_first_reviewer();

    assert!(state.current_cycle_order.is_empty());
    assert_eq!(state.current_reviewer_index, 0);
}

#[test]
fn test_validate_reviewer_state_detects_removed_reviewer() {
    let mut state = SequentialReviewState::new();
    state.current_cycle_order = vec!["A".to_string(), "B".to_string(), "C".to_string()];
    state.current_reviewer_index = 1;

    // B was removed from config
    let reset = state.validate_reviewer_state(&["A", "C"]);

    assert!(reset);
    assert!(state.current_cycle_order.is_empty());
    assert_eq!(state.current_reviewer_index, 0);
}

#[test]
fn test_validate_reviewer_state_preserves_valid_state() {
    let mut state = SequentialReviewState::new();
    state.current_cycle_order = vec!["A".to_string(), "B".to_string()];
    state.current_reviewer_index = 1;

    // Same config
    let reset = state.validate_reviewer_state(&["A", "B"]);

    assert!(!reset);
    assert_eq!(state.current_cycle_order, vec!["A", "B"]);
    assert_eq!(state.current_reviewer_index, 1);
}

#[test]
fn test_validate_reviewer_state_handles_index_out_of_bounds() {
    let mut state = SequentialReviewState::new();
    state.current_cycle_order = vec!["A".to_string(), "B".to_string(), "C".to_string()];
    state.current_reviewer_index = 5; // Out of bounds

    let reset = state.validate_reviewer_state(&["A", "B", "C"]);

    assert!(reset);
    assert!(state.current_cycle_order.is_empty());
    assert_eq!(state.current_reviewer_index, 0);
}

#[test]
fn test_config_change_adds_new_reviewer() {
    let mut state = SequentialReviewState::new();
    state.current_cycle_order = vec!["A".to_string(), "B".to_string()];
    state.current_reviewer_index = 0;
    state.reviewer_run_counts.insert("A".to_string(), 5);
    state.reviewer_run_counts.insert("B".to_string(), 5);

    // C added to config - existing order is still valid (all members exist)
    let reset = state.validate_reviewer_state(&["A", "B", "C"]);

    assert!(!reset);
    // Order preserved for current cycle
    assert_eq!(state.current_cycle_order, vec!["A", "B"]);

    // But on next cycle, C will be included and sorted to front (count 0)
    state.reset_to_first_reviewer();
    state.start_new_cycle(&["A", "B", "C"]);
    // C has count 0, A and B have count 5
    assert_eq!(state.current_cycle_order, vec!["C", "A", "B"]);
}

#[test]
fn test_needs_cycle_start() {
    let mut state = SequentialReviewState::new();
    assert!(state.needs_cycle_start()); // Empty order

    state.start_new_cycle(&["A", "B"]);
    assert!(!state.needs_cycle_start()); // Order populated

    state.reset_to_first_reviewer();
    assert!(state.needs_cycle_start()); // Cleared after reset
}

#[test]
fn test_new_reviewer_starts_with_count_zero() {
    let mut state = SequentialReviewState::new();
    state.reviewer_run_counts.insert("A".to_string(), 5);

    state.start_new_cycle(&["A", "new_reviewer"]);

    // new_reviewer has count 0, A has count 5
    assert_eq!(state.current_cycle_order, vec!["new_reviewer", "A"]);
}

#[test]
fn test_session_resume_with_empty_cycle_order() {
    let mut state = SequentialReviewState::new();
    state.current_reviewer_index = 1;
    state.current_cycle_order = vec![]; // Simulates old session resume

    assert!(state.needs_cycle_start());
}

#[test]
fn test_serialization_preserves_all_fields() {
    let mut state = SequentialReviewState::new();
    state.current_reviewer_index = 2;
    state.plan_version = 3;
    state.approvals.insert("A".to_string(), 3);
    state.reviewer_run_counts.insert("A".to_string(), 5);
    state.reviewer_run_counts.insert("B".to_string(), 2);
    state.current_cycle_order = vec!["B".to_string(), "A".to_string()];

    let json = serde_json::to_string(&state).unwrap();
    let loaded: SequentialReviewState = serde_json::from_str(&json).unwrap();

    assert_eq!(loaded.current_reviewer_index, 2);
    assert_eq!(loaded.plan_version, 3);
    assert_eq!(loaded.approvals.get("A"), Some(&3));
    assert_eq!(loaded.reviewer_run_counts.get("A"), Some(&5));
    assert_eq!(loaded.reviewer_run_counts.get("B"), Some(&2));
    assert_eq!(loaded.current_cycle_order, vec!["B", "A"]);
}

#[test]
fn test_backward_compatibility_without_run_counts() {
    // Simulate loading old state without new fields
    let old_json = r#"{
        "current_reviewer_index": 1,
        "plan_version": 2,
        "approvals": {"A": 2},
        "accumulated_reviews": []
    }"#;

    let loaded: SequentialReviewState = serde_json::from_str(old_json).unwrap();

    assert_eq!(loaded.current_reviewer_index, 1);
    assert_eq!(loaded.plan_version, 2);
    assert!(loaded.reviewer_run_counts.is_empty()); // Default empty
    assert!(loaded.current_cycle_order.is_empty()); // Default empty
}

// ============================================================================
// Round-robin tiebreaker tests
// ============================================================================

#[test]
fn test_tiebreaker_prefers_previous_rejecting_reviewer() {
    // When run counts are equal, the previous rejecting reviewer should go first
    let mut state = SequentialReviewState::new();
    state.reviewer_run_counts.insert("A".to_string(), 2);
    state.reviewer_run_counts.insert("B".to_string(), 2);
    state.reviewer_run_counts.insert("C".to_string(), 2);

    // Record that C rejected
    state.record_rejection("C");

    let tiebreaker = state.start_new_cycle(&["A", "B", "C"]);

    // C should be first despite config order putting it last
    assert_eq!(state.current_cycle_order, vec!["C", "A", "B"]);
    // Should return the rejector ID
    assert_eq!(tiebreaker, Some("C".to_string()));
}

#[test]
fn test_tiebreaker_only_applies_when_counts_equal() {
    // Tiebreaker should only affect ordering within the same run count tier
    let mut state = SequentialReviewState::new();
    state.reviewer_run_counts.insert("A".to_string(), 1); // Lowest count
    state.reviewer_run_counts.insert("B".to_string(), 2);
    state.reviewer_run_counts.insert("C".to_string(), 2);

    // Record that C rejected (C is tied with B, not A)
    state.record_rejection("C");

    let tiebreaker = state.start_new_cycle(&["A", "B", "C"]);

    // A should still be first (lowest count), C before B in the tie
    assert_eq!(state.current_cycle_order, vec!["A", "C", "B"]);
    assert_eq!(tiebreaker, Some("C".to_string()));
}

#[test]
fn test_rejection_cleared_after_start_new_cycle() {
    let mut state = SequentialReviewState::new();
    state.record_rejection("A");
    assert!(state.last_rejecting_reviewer.is_some());

    let _ = state.start_new_cycle(&["A", "B"]);

    // The take() call should have cleared it
    assert!(state.last_rejecting_reviewer.is_none());
}

#[test]
fn test_no_tiebreaker_when_no_previous_rejection() {
    // When no reviewer rejected, config order is preserved (existing behavior)
    let mut state = SequentialReviewState::new();
    state.reviewer_run_counts.insert("A".to_string(), 2);
    state.reviewer_run_counts.insert("B".to_string(), 2);
    state.reviewer_run_counts.insert("C".to_string(), 2);

    let tiebreaker = state.start_new_cycle(&["A", "B", "C"]);

    // Config order preserved
    assert_eq!(state.current_cycle_order, vec!["A", "B", "C"]);
    // No tiebreaker used
    assert_eq!(tiebreaker, None);
}

#[test]
fn test_serialization_preserves_last_rejecting_reviewer() {
    let mut state = SequentialReviewState::new();
    state.record_rejection("A");

    let json = serde_json::to_string(&state).unwrap();
    let loaded: SequentialReviewState = serde_json::from_str(&json).unwrap();

    assert_eq!(loaded.last_rejecting_reviewer, Some("A".to_string()));
}

#[test]
fn test_backward_compatibility_without_last_rejecting_reviewer() {
    // Simulate loading old state without the new field
    let old_json = r#"{
        "current_reviewer_index": 0,
        "plan_version": 1,
        "approvals": {},
        "accumulated_reviews": [],
        "reviewer_run_counts": {"A": 1, "B": 1},
        "current_cycle_order": ["A", "B"]
    }"#;

    let loaded: SequentialReviewState = serde_json::from_str(old_json).unwrap();

    // Should default to None
    assert!(loaded.last_rejecting_reviewer.is_none());
}
