//! Unit tests for WorkflowAggregate command handling and event application.

use crate::domain::aggregate::{WorkflowAggregate, WorkflowData, WorkflowState};
use crate::domain::commands::WorkflowCommand;
use crate::domain::events::WorkflowEvent;
use crate::domain::failure::{FailureContext, FailureKind, MAX_FAILURE_HISTORY};
use crate::domain::services::WorkflowServices;
use crate::domain::types::{
    ConversationId, FeedbackStatus, ImplementationPhase, ImplementationVerdict, Iteration,
    MaxIterations, PhaseLabel, PlanningPhase, ResumeStrategy, WorktreeState,
};
use cqrs_es::Aggregate;
use std::path::PathBuf;

/// Create default services for testing.
fn test_services() -> WorkflowServices {
    WorkflowServices::default()
}

/// Create a CreateWorkflow command with test defaults.
fn create_workflow_cmd() -> WorkflowCommand {
    WorkflowCommand::CreateWorkflow {
        feature_name: "test-feature".into(),
        objective: "Test objective".into(),
        working_dir: PathBuf::from("/test/dir").into(),
        max_iterations: MaxIterations(3),
        plan_path: PathBuf::from("/test/plan.md").into(),
        feedback_path: PathBuf::from("/test/feedback.md").into(),
    }
}

/// Apply CreateWorkflow to get an initialized aggregate in Planning phase.
fn initialized_aggregate() -> WorkflowAggregate {
    let mut agg = WorkflowAggregate::default();
    let event = WorkflowEvent::WorkflowCreated {
        feature_name: "test-feature".into(),
        objective: "Test objective".into(),
        working_dir: PathBuf::from("/test/dir").into(),
        max_iterations: MaxIterations(3),
        plan_path: PathBuf::from("/test/plan.md").into(),
        feedback_path: PathBuf::from("/test/feedback.md").into(),
        created_at: crate::domain::types::TimestampUtc::now(),
    };
    agg.apply(event);
    agg
}

/// Get mutable data from an active aggregate (panics if not active).
fn get_data_mut(agg: &mut WorkflowAggregate) -> &mut WorkflowData {
    match &mut agg.state {
        WorkflowState::Active(data) => data,
        _ => panic!("Expected Active state"),
    }
}

// ============================================================================
// CreateWorkflow Tests
// ============================================================================

#[tokio::test]
async fn create_workflow_on_uninitialized_succeeds() {
    let agg = WorkflowAggregate::default();
    let services = test_services();

    let events = agg.handle(create_workflow_cmd(), &services).await.unwrap();

    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], WorkflowEvent::WorkflowCreated { .. }));
}

#[tokio::test]
async fn create_workflow_on_active_fails() {
    let agg = initialized_aggregate();
    let services = test_services();

    let result = agg.handle(create_workflow_cmd(), &services).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn apply_workflow_created_initializes_state() {
    let mut agg = WorkflowAggregate::default();
    assert!(matches!(agg.state, WorkflowState::Uninitialized));

    let event = WorkflowEvent::WorkflowCreated {
        feature_name: "my-feature".into(),
        objective: "My objective".into(),
        working_dir: PathBuf::from("/work").into(),
        max_iterations: MaxIterations(5),
        plan_path: PathBuf::from("/plan.md").into(),
        feedback_path: PathBuf::from("/feedback.md").into(),
        created_at: crate::domain::types::TimestampUtc::now(),
    };

    agg.apply(event);

    match &agg.state {
        WorkflowState::Active(data) => {
            assert_eq!(data.feature_name.as_str(), "my-feature");
            assert_eq!(data.planning_phase, PlanningPhase::Planning);
            assert_eq!(data.iteration.0, 1);
        }
        _ => panic!("Expected Active state"),
    }
}

// ============================================================================
// Planning Phase Tests
// ============================================================================

#[tokio::test]
async fn start_planning_in_planning_phase_succeeds() {
    let agg = initialized_aggregate();
    let services = test_services();

    let events = agg
        .handle(WorkflowCommand::StartPlanning, &services)
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], WorkflowEvent::PlanningStarted { .. }));
}

#[tokio::test]
async fn planning_completed_transitions_to_reviewing() {
    let mut agg = initialized_aggregate();

    let event = WorkflowEvent::PlanningCompleted {
        plan_path: PathBuf::from("/new/plan.md").into(),
        completed_at: crate::domain::types::TimestampUtc::now(),
    };
    agg.apply(event);

    let data = get_data_mut(&mut agg);
    assert_eq!(data.planning_phase, PlanningPhase::Reviewing);
}

#[tokio::test]
async fn planning_completed_command_produces_event() {
    let agg = initialized_aggregate();
    let services = test_services();

    let events = agg
        .handle(
            WorkflowCommand::PlanningCompleted {
                plan_path: PathBuf::from("/updated/plan.md").into(),
            },
            &services,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    match &events[0] {
        WorkflowEvent::PlanningCompleted { plan_path, .. } => {
            assert_eq!(plan_path.as_path(), PathBuf::from("/updated/plan.md"));
        }
        _ => panic!("Expected PlanningCompleted event"),
    }
}

// ============================================================================
// Review Phase Tests
// ============================================================================

#[tokio::test]
async fn review_cycle_started_emits_event_in_reviewing_phase() {
    use crate::domain::review::{ReviewMode, SequentialReviewState};

    let mut agg = initialized_aggregate();
    let services = test_services();

    // Transition to Reviewing phase
    agg.apply(WorkflowEvent::PlanningCompleted {
        plan_path: PathBuf::from("/plan.md").into(),
        completed_at: crate::domain::types::TimestampUtc::now(),
    });
    assert_eq!(
        get_data_mut(&mut agg).planning_phase,
        PlanningPhase::Reviewing
    );

    let reviewers = vec!["reviewer-1".into(), "reviewer-2".into()];
    let mode = ReviewMode::Sequential(SequentialReviewState::new());

    let events = agg
        .handle(
            WorkflowCommand::ReviewCycleStarted {
                mode: mode.clone(),
                reviewers: reviewers.clone(),
            },
            &services,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    match &events[0] {
        WorkflowEvent::ReviewCycleStarted {
            mode: event_mode,
            reviewers: event_reviewers,
            ..
        } => {
            assert_eq!(event_mode, &mode);
            assert_eq!(event_reviewers, &reviewers);
        }
        _ => panic!("Expected ReviewCycleStarted event"),
    }

    // Apply and verify review_mode is set
    agg.apply(events.into_iter().next().unwrap());
    let data = get_data_mut(&mut agg);
    assert!(data.review_mode.is_some());
}

#[tokio::test]
async fn reviewer_approved_records_approval_in_sequential_state() {
    use crate::domain::review::{ReviewMode, SequentialReviewState};
    use crate::domain::types::AgentId;

    let mut agg = initialized_aggregate();
    let services = test_services();

    // Transition to Reviewing phase
    agg.apply(WorkflowEvent::PlanningCompleted {
        plan_path: PathBuf::from("/plan.md").into(),
        completed_at: crate::domain::types::TimestampUtc::now(),
    });

    // Set up sequential review mode
    let reviewers: Vec<AgentId> = vec!["reviewer-1".into(), "reviewer-2".into()];
    let mode = ReviewMode::Sequential(SequentialReviewState::new());
    agg.apply(WorkflowEvent::ReviewCycleStarted {
        mode,
        reviewers: reviewers.clone(),
        started_at: crate::domain::types::TimestampUtc::now(),
    });

    let reviewer_id: AgentId = "reviewer-1".into();
    let events = agg
        .handle(
            WorkflowCommand::ReviewerApproved {
                reviewer_id: reviewer_id.clone(),
            },
            &services,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    match &events[0] {
        WorkflowEvent::ReviewerApproved {
            reviewer_id: event_reviewer_id,
            ..
        } => {
            assert_eq!(event_reviewer_id, &reviewer_id);
        }
        _ => panic!("Expected ReviewerApproved event"),
    }

    // Apply and verify approval is recorded in sequential state
    agg.apply(events.into_iter().next().unwrap());
    let data = get_data_mut(&mut agg);
    match &data.review_mode {
        Some(ReviewMode::Sequential(state)) => {
            assert!(state.approvals.contains_key(&reviewer_id));
            assert_eq!(state.approvals.get(&reviewer_id), Some(&1)); // plan_version starts at 1
        }
        _ => panic!("Expected Sequential review mode"),
    }
}

#[tokio::test]
async fn reviewer_rejected_records_rejection_in_sequential_state() {
    use crate::domain::review::{ReviewMode, SequentialReviewState};
    use crate::domain::types::AgentId;

    let mut agg = initialized_aggregate();
    let services = test_services();

    // Transition to Reviewing phase
    agg.apply(WorkflowEvent::PlanningCompleted {
        plan_path: PathBuf::from("/plan.md").into(),
        completed_at: crate::domain::types::TimestampUtc::now(),
    });

    // Set up sequential review mode
    let reviewers: Vec<AgentId> = vec!["reviewer-1".into(), "reviewer-2".into()];
    let mode = ReviewMode::Sequential(SequentialReviewState::new());
    agg.apply(WorkflowEvent::ReviewCycleStarted {
        mode,
        reviewers: reviewers.clone(),
        started_at: crate::domain::types::TimestampUtc::now(),
    });

    let reviewer_id: AgentId = "reviewer-1".into();
    let feedback_path = PathBuf::from("/feedback/reviewer-1.md").into();
    let events = agg
        .handle(
            WorkflowCommand::ReviewerRejected {
                reviewer_id: reviewer_id.clone(),
                feedback_path,
            },
            &services,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    match &events[0] {
        WorkflowEvent::ReviewerRejected {
            reviewer_id: event_reviewer_id,
            feedback_path: event_feedback_path,
            ..
        } => {
            assert_eq!(event_reviewer_id, &reviewer_id);
            assert_eq!(
                event_feedback_path.as_path(),
                PathBuf::from("/feedback/reviewer-1.md")
            );
        }
        _ => panic!("Expected ReviewerRejected event"),
    }

    // Apply and verify rejection is recorded in sequential state
    agg.apply(events.into_iter().next().unwrap());
    let data = get_data_mut(&mut agg);
    match &data.review_mode {
        Some(ReviewMode::Sequential(state)) => {
            assert_eq!(state.last_rejecting_reviewer, Some(reviewer_id));
        }
        _ => panic!("Expected Sequential review mode"),
    }
}

#[tokio::test]
async fn review_cycle_completed_approved_transitions_to_complete() {
    let mut agg = initialized_aggregate();

    // Transition to Reviewing phase
    agg.apply(WorkflowEvent::PlanningCompleted {
        plan_path: PathBuf::from("/plan.md").into(),
        completed_at: crate::domain::types::TimestampUtc::now(),
    });
    assert_eq!(
        get_data_mut(&mut agg).planning_phase,
        PlanningPhase::Reviewing
    );

    // Complete review with approval
    agg.apply(WorkflowEvent::ReviewCycleCompleted {
        approved: true,
        completed_at: crate::domain::types::TimestampUtc::now(),
    });

    let data = get_data_mut(&mut agg);
    assert_eq!(data.planning_phase, PlanningPhase::Complete);
    assert_eq!(data.last_feedback_status, Some(FeedbackStatus::Approved));
}

#[tokio::test]
async fn review_cycle_completed_rejected_transitions_to_revising() {
    let mut agg = initialized_aggregate();

    // Transition to Reviewing phase
    agg.apply(WorkflowEvent::PlanningCompleted {
        plan_path: PathBuf::from("/plan.md").into(),
        completed_at: crate::domain::types::TimestampUtc::now(),
    });

    // Complete review with rejection
    agg.apply(WorkflowEvent::ReviewCycleCompleted {
        approved: false,
        completed_at: crate::domain::types::TimestampUtc::now(),
    });

    let data = get_data_mut(&mut agg);
    assert_eq!(data.planning_phase, PlanningPhase::Revising);
    assert_eq!(
        data.last_feedback_status,
        Some(FeedbackStatus::NeedsRevision)
    );
}

// ============================================================================
// Revision Phase Tests
// ============================================================================

#[tokio::test]
async fn revising_started_emits_event_in_revising_phase() {
    let mut agg = initialized_aggregate();
    let services = test_services();

    // Go through Planning -> Reviewing -> Revising
    agg.apply(WorkflowEvent::PlanningCompleted {
        plan_path: PathBuf::from("/plan.md").into(),
        completed_at: crate::domain::types::TimestampUtc::now(),
    });
    agg.apply(WorkflowEvent::ReviewCycleCompleted {
        approved: false,
        completed_at: crate::domain::types::TimestampUtc::now(),
    });
    assert_eq!(
        get_data_mut(&mut agg).planning_phase,
        PlanningPhase::Revising
    );

    // Now RevisingStarted should succeed
    let events = agg
        .handle(
            WorkflowCommand::RevisingStarted {
                feedback_summary: "Address reviewer comments".to_string(),
            },
            &services,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    match &events[0] {
        WorkflowEvent::RevisingStarted {
            feedback_summary, ..
        } => {
            assert_eq!(feedback_summary, "Address reviewer comments");
        }
        _ => panic!("Expected RevisingStarted event"),
    }
}

#[tokio::test]
async fn revision_completed_increments_iteration() {
    let mut agg = initialized_aggregate();

    // Go through Planning → Reviewing → Revising
    agg.apply(WorkflowEvent::PlanningCompleted {
        plan_path: PathBuf::from("/plan.md").into(),
        completed_at: crate::domain::types::TimestampUtc::now(),
    });
    agg.apply(WorkflowEvent::ReviewCycleCompleted {
        approved: false,
        completed_at: crate::domain::types::TimestampUtc::now(),
    });

    assert_eq!(get_data_mut(&mut agg).iteration.0, 1);

    // Complete revision
    agg.apply(WorkflowEvent::RevisionCompleted {
        plan_path: PathBuf::from("/plan_v2.md").into(),
        completed_at: crate::domain::types::TimestampUtc::now(),
    });

    let data = get_data_mut(&mut agg);
    assert_eq!(data.iteration.0, 2);
    assert_eq!(data.planning_phase, PlanningPhase::Reviewing);
}

// ============================================================================
// User Decision Tests
// ============================================================================

#[tokio::test]
async fn user_override_approval_sets_flag_and_completes() {
    let mut agg = initialized_aggregate();
    let services = test_services();

    let events = agg
        .handle(
            WorkflowCommand::UserOverrideApproval {
                override_reason: "User bypassed review".to_string(),
            },
            &services,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        WorkflowEvent::UserOverrideApproval { .. }
    ));

    // Apply and verify state
    agg.apply(events.into_iter().next().unwrap());
    let data = get_data_mut(&mut agg);
    assert!(data.approval_overridden);
    assert_eq!(data.planning_phase, PlanningPhase::Complete);
}

#[tokio::test]
async fn user_aborted_produces_event() {
    let agg = initialized_aggregate();
    let services = test_services();

    let events = agg
        .handle(
            WorkflowCommand::UserAborted {
                reason: "User cancelled".to_string(),
            },
            &services,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    match &events[0] {
        WorkflowEvent::UserAborted { reason, .. } => {
            assert_eq!(reason, "User cancelled");
        }
        _ => panic!("Expected UserAborted event"),
    }
}

#[tokio::test]
async fn user_approved_sets_phase_to_complete() {
    let mut agg = initialized_aggregate();
    let services = test_services();

    let events = agg
        .handle(WorkflowCommand::UserApproved, &services)
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], WorkflowEvent::UserApproved { .. }));

    // Apply and verify state
    agg.apply(events.into_iter().next().unwrap());
    let data = get_data_mut(&mut agg);
    assert_eq!(data.planning_phase, PlanningPhase::Complete);
}

#[tokio::test]
async fn user_declined_emits_event_with_feedback() {
    let agg = initialized_aggregate();
    let services = test_services();

    let events = agg
        .handle(
            WorkflowCommand::UserDeclined {
                feedback: "Needs more detail on error handling".to_string(),
            },
            &services,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    match &events[0] {
        WorkflowEvent::UserDeclined { feedback, .. } => {
            assert_eq!(feedback, "Needs more detail on error handling");
        }
        _ => panic!("Expected UserDeclined event"),
    }
}

#[tokio::test]
async fn planning_max_iterations_reached_sets_phase_to_awaiting_decision() {
    let mut agg = initialized_aggregate();
    let services = test_services();

    let events = agg
        .handle(WorkflowCommand::PlanningMaxIterationsReached, &services)
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        WorkflowEvent::PlanningMaxIterationsReached { .. }
    ));

    // Apply and verify state
    agg.apply(events.into_iter().next().unwrap());
    let data = get_data_mut(&mut agg);
    assert_eq!(data.planning_phase, PlanningPhase::AwaitingDecision);
}

// ============================================================================
// Implementation Phase Tests
// ============================================================================

#[tokio::test]
async fn user_requested_implementation_emits_two_events() {
    let agg = initialized_aggregate();
    let services = test_services();

    let events = agg
        .handle(WorkflowCommand::UserRequestedImplementation, &services)
        .await
        .unwrap();

    // UserRequestedImplementation command emits both events
    assert_eq!(events.len(), 2);
    assert!(matches!(
        events[0],
        WorkflowEvent::UserRequestedImplementation { .. }
    ));
    assert!(matches!(
        events[1],
        WorkflowEvent::ImplementationStarted { .. }
    ));
}

#[tokio::test]
async fn direct_implementation_started_command_is_rejected() {
    let agg = initialized_aggregate();
    let services = test_services();

    let result = agg
        .handle(
            WorkflowCommand::ImplementationStarted {
                max_iterations: MaxIterations(3),
            },
            &services,
        )
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn implementation_started_event_initializes_implementation_state() {
    let mut agg = initialized_aggregate();

    agg.apply(WorkflowEvent::ImplementationStarted {
        max_iterations: MaxIterations(5),
        started_at: crate::domain::types::TimestampUtc::now(),
    });

    let data = get_data_mut(&mut agg);
    assert!(data.implementation_state.is_some());
    let impl_state = data.implementation_state.as_ref().unwrap();
    assert_eq!(impl_state.max_iterations.0, 5);
}

#[tokio::test]
async fn implementation_review_completed_updates_verdict() {
    let mut agg = initialized_aggregate();

    // Start implementation
    agg.apply(WorkflowEvent::ImplementationStarted {
        max_iterations: MaxIterations(3),
        started_at: crate::domain::types::TimestampUtc::now(),
    });

    // Complete a review
    agg.apply(WorkflowEvent::ImplementationReviewCompleted {
        iteration: Iteration::first(),
        verdict: ImplementationVerdict::Approved,
        feedback: Some("Looks good".to_string()),
        completed_at: crate::domain::types::TimestampUtc::now(),
    });

    let data = get_data_mut(&mut agg);
    let impl_state = data.implementation_state.as_ref().unwrap();
    assert_eq!(
        impl_state.last_verdict,
        Some(ImplementationVerdict::Approved)
    );
    assert_eq!(impl_state.last_feedback, Some("Looks good".to_string()));
}

#[tokio::test]
async fn implementation_accepted_sets_phase_complete() {
    let mut agg = initialized_aggregate();

    agg.apply(WorkflowEvent::ImplementationStarted {
        max_iterations: MaxIterations(3),
        started_at: crate::domain::types::TimestampUtc::now(),
    });
    agg.apply(WorkflowEvent::ImplementationAccepted {
        approved_at: crate::domain::types::TimestampUtc::now(),
    });

    let data = get_data_mut(&mut agg);
    let impl_state = data.implementation_state.as_ref().unwrap();
    assert_eq!(impl_state.phase, ImplementationPhase::Complete);
}

// ============================================================================
// Failure Tracking Tests
// ============================================================================

#[tokio::test]
async fn record_failure_adds_to_history() {
    let mut agg = initialized_aggregate();
    let services = test_services();

    let failure = FailureContext::new(
        FailureKind::Unknown("Test error".to_string()),
        PhaseLabel::Planning,
        Some("test-agent".into()),
        1,
    );

    let events = agg
        .handle(
            WorkflowCommand::RecordFailure {
                failure: failure.clone(),
            },
            &services,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    agg.apply(events.into_iter().next().unwrap());

    let data = get_data_mut(&mut agg);
    assert!(data.last_failure.is_some());
    assert_eq!(data.failure_history.len(), 1);
}

// ============================================================================
// Worktree Tests
// ============================================================================

#[tokio::test]
async fn attach_worktree_stores_state() {
    let mut agg = initialized_aggregate();
    let services = test_services();

    let worktree = WorktreeState {
        worktree_path: PathBuf::from("/worktree"),
        branch_name: "feature-branch".to_string(),
        source_branch: Some("main".to_string()),
        original_dir: PathBuf::from("/original"),
    };

    let events = agg
        .handle(
            WorkflowCommand::AttachWorktree {
                worktree_state: worktree.clone(),
            },
            &services,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    agg.apply(events.into_iter().next().unwrap());

    let data = get_data_mut(&mut agg);
    assert!(data.worktree_info.is_some());
    assert_eq!(
        data.worktree_info.as_ref().unwrap().branch_name,
        "feature-branch"
    );
}

// ============================================================================
// Agent Conversation Tests
// ============================================================================

#[tokio::test]
async fn record_agent_conversation_stores_state() {
    let mut agg = initialized_aggregate();
    let services = test_services();

    let events = agg
        .handle(
            WorkflowCommand::RecordAgentConversation {
                agent_id: "claude".into(),
                resume_strategy: ResumeStrategy::ConversationResume,
                conversation_id: Some(ConversationId::from("conv-123")),
            },
            &services,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    agg.apply(events.into_iter().next().unwrap());

    let data = get_data_mut(&mut agg);
    assert!(data.agent_conversations.contains_key(&"claude".into()));
    let conv = data.agent_conversations.get(&"claude".into()).unwrap();
    assert_eq!(conv.conversation_id, Some(ConversationId::from("conv-123")));
    assert_eq!(conv.resume_strategy, ResumeStrategy::ConversationResume);
}

// ============================================================================
// Invocation Tracking Tests
// ============================================================================

#[tokio::test]
async fn record_invocation_appends_to_invocations_list() {
    let mut agg = initialized_aggregate();
    let services = test_services();

    let events = agg
        .handle(
            WorkflowCommand::RecordInvocation {
                agent_id: "claude".into(),
                phase: PhaseLabel::Planning,
                conversation_id: Some(ConversationId::from("conv-abc")),
                resume_strategy: ResumeStrategy::ConversationResume,
            },
            &services,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        WorkflowEvent::InvocationRecorded { .. }
    ));

    // Apply first invocation
    agg.apply(events.into_iter().next().unwrap());
    assert_eq!(get_data_mut(&mut agg).invocations.len(), 1);

    // Record a second invocation
    let events = agg
        .handle(
            WorkflowCommand::RecordInvocation {
                agent_id: "gemini".into(),
                phase: PhaseLabel::Reviewing,
                conversation_id: None,
                resume_strategy: ResumeStrategy::Stateless,
            },
            &services,
        )
        .await
        .unwrap();

    agg.apply(events.into_iter().next().unwrap());

    let data = get_data_mut(&mut agg);
    assert_eq!(data.invocations.len(), 2);

    // Verify first invocation
    assert_eq!(data.invocations[0].agent.0, "claude");
    assert_eq!(data.invocations[0].phase, PhaseLabel::Planning);
    assert_eq!(
        data.invocations[0].conversation_id,
        Some(ConversationId::from("conv-abc"))
    );
    assert_eq!(
        data.invocations[0].resume_strategy,
        ResumeStrategy::ConversationResume
    );

    // Verify second invocation
    assert_eq!(data.invocations[1].agent.0, "gemini");
    assert_eq!(data.invocations[1].phase, PhaseLabel::Reviewing);
    assert_eq!(data.invocations[1].conversation_id, None);
    assert_eq!(
        data.invocations[1].resume_strategy,
        ResumeStrategy::Stateless
    );
}

// ============================================================================
// Invalid Transition Tests
// ============================================================================

#[tokio::test]
async fn planning_completed_in_wrong_phase_fails() {
    let mut agg = initialized_aggregate();

    // Move to Reviewing phase
    agg.apply(WorkflowEvent::PlanningCompleted {
        plan_path: PathBuf::from("/plan.md").into(),
        completed_at: crate::domain::types::TimestampUtc::now(),
    });

    let services = test_services();

    // Try PlanningCompleted again (should fail - we're in Reviewing)
    let result = agg
        .handle(
            WorkflowCommand::PlanningCompleted {
                plan_path: PathBuf::from("/another.md").into(),
            },
            &services,
        )
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn implementation_commands_fail_without_implementation_state() {
    let agg = initialized_aggregate();
    let services = test_services();

    // Try implementation command without starting implementation
    let result = agg
        .handle(
            WorkflowCommand::ImplementationRoundStarted {
                iteration: Iteration::first(),
            },
            &services,
        )
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn implementation_round_started_succeeds_with_implementation_state() {
    let mut agg = initialized_aggregate();
    let services = test_services();

    // First, start implementation to create implementation state
    agg.apply(WorkflowEvent::ImplementationStarted {
        max_iterations: MaxIterations(3),
        started_at: crate::domain::types::TimestampUtc::now(),
    });

    let events = agg
        .handle(
            WorkflowCommand::ImplementationRoundStarted {
                iteration: Iteration::first(),
            },
            &services,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    match &events[0] {
        WorkflowEvent::ImplementationRoundStarted { iteration, .. } => {
            assert_eq!(*iteration, Iteration::first());
        }
        _ => panic!("Expected ImplementationRoundStarted event"),
    }

    // Apply and verify state changes
    agg.apply(events.into_iter().next().unwrap());
    let data = get_data_mut(&mut agg);
    let impl_state = data.implementation_state.as_ref().unwrap();
    assert_eq!(impl_state.phase, ImplementationPhase::Implementing);
    assert_eq!(impl_state.iteration, Iteration::first());
}

#[tokio::test]
async fn implementation_round_completed_succeeds_with_implementation_state() {
    let mut agg = initialized_aggregate();
    let services = test_services();

    // Start implementation
    agg.apply(WorkflowEvent::ImplementationStarted {
        max_iterations: MaxIterations(3),
        started_at: crate::domain::types::TimestampUtc::now(),
    });

    let events = agg
        .handle(
            WorkflowCommand::ImplementationRoundCompleted {
                iteration: Iteration::first(),
                fingerprint: 12345,
            },
            &services,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    match &events[0] {
        WorkflowEvent::ImplementationRoundCompleted {
            iteration,
            fingerprint,
            ..
        } => {
            assert_eq!(*iteration, Iteration::first());
            assert_eq!(*fingerprint, 12345);
        }
        _ => panic!("Expected ImplementationRoundCompleted event"),
    }
}

#[tokio::test]
async fn implementation_max_iterations_reached_sets_phase_to_awaiting_decision() {
    let mut agg = initialized_aggregate();
    let services = test_services();

    // Start implementation
    agg.apply(WorkflowEvent::ImplementationStarted {
        max_iterations: MaxIterations(3),
        started_at: crate::domain::types::TimestampUtc::now(),
    });

    let events = agg
        .handle(
            WorkflowCommand::ImplementationMaxIterationsReached,
            &services,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        WorkflowEvent::ImplementationMaxIterationsReached { .. }
    ));

    // Apply and verify state changes
    agg.apply(events.into_iter().next().unwrap());
    let data = get_data_mut(&mut agg);
    let impl_state = data.implementation_state.as_ref().unwrap();
    assert_eq!(impl_state.phase, ImplementationPhase::AwaitingDecision);
}

#[tokio::test]
async fn implementation_declined_sets_phase_to_complete() {
    let mut agg = initialized_aggregate();
    let services = test_services();

    // Start implementation
    agg.apply(WorkflowEvent::ImplementationStarted {
        max_iterations: MaxIterations(3),
        started_at: crate::domain::types::TimestampUtc::now(),
    });

    let events = agg
        .handle(
            WorkflowCommand::ImplementationDeclined {
                reason: "Not satisfied with implementation".to_string(),
            },
            &services,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    match &events[0] {
        WorkflowEvent::ImplementationDeclined { reason, .. } => {
            assert_eq!(reason, "Not satisfied with implementation");
        }
        _ => panic!("Expected ImplementationDeclined event"),
    }

    // Apply and verify state changes
    agg.apply(events.into_iter().next().unwrap());
    let data = get_data_mut(&mut agg);
    let impl_state = data.implementation_state.as_ref().unwrap();
    assert_eq!(impl_state.phase, ImplementationPhase::Complete);
}

#[tokio::test]
async fn implementation_cancelled_sets_phase_to_complete() {
    let mut agg = initialized_aggregate();
    let services = test_services();

    // Start implementation
    agg.apply(WorkflowEvent::ImplementationStarted {
        max_iterations: MaxIterations(3),
        started_at: crate::domain::types::TimestampUtc::now(),
    });

    let events = agg
        .handle(
            WorkflowCommand::ImplementationCancelled {
                reason: "User cancelled the implementation".to_string(),
            },
            &services,
        )
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    match &events[0] {
        WorkflowEvent::ImplementationCancelled { reason, .. } => {
            assert_eq!(reason, "User cancelled the implementation");
        }
        _ => panic!("Expected ImplementationCancelled event"),
    }

    // Apply and verify state changes
    agg.apply(events.into_iter().next().unwrap());
    let data = get_data_mut(&mut agg);
    let impl_state = data.implementation_state.as_ref().unwrap();
    assert_eq!(impl_state.phase, ImplementationPhase::Complete);
}

// ============================================================================
// Failure History Limit Tests
// ============================================================================

#[tokio::test]
async fn failure_history_limited_to_max_entries() {
    let mut agg = initialized_aggregate();

    // Add more than MAX_FAILURE_HISTORY failures
    for i in 0..(MAX_FAILURE_HISTORY + 10) {
        let failure = FailureContext::new(
            FailureKind::Unknown(format!("Error {}", i)),
            PhaseLabel::Planning,
            Some("test-agent".into()),
            i as u32,
        );
        agg.apply(WorkflowEvent::FailureRecorded {
            failure,
            recorded_at: crate::domain::types::TimestampUtc::now(),
        });
    }

    let data = get_data_mut(&mut agg);

    // History should be trimmed to MAX_FAILURE_HISTORY
    assert_eq!(data.failure_history.len(), MAX_FAILURE_HISTORY);

    // Oldest failures should be removed (first 10 are gone)
    // The first remaining failure should be "Error 10"
    let first_failure = &data.failure_history[0];
    match &first_failure.kind {
        FailureKind::Unknown(msg) => assert_eq!(msg, "Error 10"),
        _ => panic!("Expected Unknown failure kind"),
    }

    // The last failure should be "Error 59" (MAX_FAILURE_HISTORY + 10 - 1)
    let last_failure = &data.failure_history[MAX_FAILURE_HISTORY - 1];
    match &last_failure.kind {
        FailureKind::Unknown(msg) => {
            assert_eq!(msg, &format!("Error {}", MAX_FAILURE_HISTORY + 10 - 1))
        }
        _ => panic!("Expected Unknown failure kind"),
    }
}

// ============================================================================
// Multiple Revision Iteration Tests
// ============================================================================

#[tokio::test]
async fn multiple_revision_iterations_increment_correctly() {
    let mut agg = initialized_aggregate();

    // Verify initial iteration is 1
    assert_eq!(get_data_mut(&mut agg).iteration.0, 1);

    // First cycle: Planning -> Reviewing -> Revising -> Reviewing
    agg.apply(WorkflowEvent::PlanningCompleted {
        plan_path: PathBuf::from("/plan.md").into(),
        completed_at: crate::domain::types::TimestampUtc::now(),
    });
    agg.apply(WorkflowEvent::ReviewCycleCompleted {
        approved: false,
        completed_at: crate::domain::types::TimestampUtc::now(),
    });
    assert_eq!(
        get_data_mut(&mut agg).planning_phase,
        PlanningPhase::Revising
    );
    assert_eq!(get_data_mut(&mut agg).iteration.0, 1);

    // Complete revision 1 -> iteration becomes 2
    agg.apply(WorkflowEvent::RevisionCompleted {
        plan_path: PathBuf::from("/plan_v2.md").into(),
        completed_at: crate::domain::types::TimestampUtc::now(),
    });
    assert_eq!(get_data_mut(&mut agg).iteration.0, 2);
    assert_eq!(
        get_data_mut(&mut agg).planning_phase,
        PlanningPhase::Reviewing
    );

    // Second cycle: Reviewing -> Revising -> Reviewing
    agg.apply(WorkflowEvent::ReviewCycleCompleted {
        approved: false,
        completed_at: crate::domain::types::TimestampUtc::now(),
    });
    assert_eq!(
        get_data_mut(&mut agg).planning_phase,
        PlanningPhase::Revising
    );
    assert_eq!(get_data_mut(&mut agg).iteration.0, 2);

    // Complete revision 2 -> iteration becomes 3
    agg.apply(WorkflowEvent::RevisionCompleted {
        plan_path: PathBuf::from("/plan_v3.md").into(),
        completed_at: crate::domain::types::TimestampUtc::now(),
    });
    assert_eq!(get_data_mut(&mut agg).iteration.0, 3);
    assert_eq!(
        get_data_mut(&mut agg).planning_phase,
        PlanningPhase::Reviewing
    );

    // Third cycle: Reviewing -> Revising -> Reviewing
    agg.apply(WorkflowEvent::ReviewCycleCompleted {
        approved: false,
        completed_at: crate::domain::types::TimestampUtc::now(),
    });
    assert_eq!(
        get_data_mut(&mut agg).planning_phase,
        PlanningPhase::Revising
    );
    assert_eq!(get_data_mut(&mut agg).iteration.0, 3);

    // Complete revision 3 -> iteration becomes 4
    agg.apply(WorkflowEvent::RevisionCompleted {
        plan_path: PathBuf::from("/plan_v4.md").into(),
        completed_at: crate::domain::types::TimestampUtc::now(),
    });
    assert_eq!(get_data_mut(&mut agg).iteration.0, 4);
    assert_eq!(
        get_data_mut(&mut agg).planning_phase,
        PlanningPhase::Reviewing
    );
}
