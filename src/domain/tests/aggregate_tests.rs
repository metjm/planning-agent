//! Unit tests for WorkflowAggregate command handling and event application.

use crate::domain::services::WorkflowServices;
use crate::domain::types::{FeedbackStatus, MaxIterations, Phase};
use crate::domain::WorkflowCommand;
use crate::domain::WorkflowEvent;
use crate::domain::{WorkflowAggregate, WorkflowData, WorkflowState};
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
            assert_eq!(data.planning_phase, Phase::Planning);
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
    assert_eq!(data.planning_phase, Phase::Reviewing);
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
    assert_eq!(get_data_mut(&mut agg).planning_phase, Phase::Reviewing);

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
    assert_eq!(get_data_mut(&mut agg).planning_phase, Phase::Reviewing);

    // Complete review with approval
    agg.apply(WorkflowEvent::ReviewCycleCompleted {
        approved: true,
        completed_at: crate::domain::types::TimestampUtc::now(),
    });

    let data = get_data_mut(&mut agg);
    assert_eq!(data.planning_phase, Phase::Complete);
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
    assert_eq!(data.planning_phase, Phase::Revising);
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
    assert_eq!(get_data_mut(&mut agg).planning_phase, Phase::Revising);

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
    assert_eq!(data.planning_phase, Phase::Reviewing);
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
    assert_eq!(data.planning_phase, Phase::Complete);
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
    assert_eq!(data.planning_phase, Phase::Complete);
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
    assert_eq!(data.planning_phase, Phase::AwaitingPlanningDecision);
}
