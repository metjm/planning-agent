use super::overlays::{build_phase_spans, PhaseDisplayMode};
use super::theme::Theme;
use crate::domain::types::{
    FeatureName, FeedbackPath, ImplementationPhase, ImplementationVerdict, Iteration,
    MaxIterations, Objective, Phase, PlanPath, TimestampUtc, WorkingDir,
};
use crate::domain::view::WorkflowView;
use crate::domain::WorkflowEvent;
use crate::tui::session::Session;
use std::path::PathBuf;
use uuid::Uuid;

fn make_test_view(phase: Phase) -> WorkflowView {
    let mut view = WorkflowView::default();
    let agg_id = Uuid::new_v4().to_string();

    // Create workflow
    view.apply_event(
        &agg_id,
        &WorkflowEvent::WorkflowCreated {
            feature_name: FeatureName::from("test-feature"),
            objective: Objective::from("Test objective"),
            working_dir: WorkingDir::from(PathBuf::from("/tmp/test").as_path()),
            max_iterations: MaxIterations(3),
            plan_path: PlanPath::from(PathBuf::from("/tmp/test/plan.md")),
            feedback_path: FeedbackPath::from(PathBuf::from("/tmp/test/feedback.md")),
            created_at: TimestampUtc::now(),
        },
        1,
    );

    // Transition to the desired phase
    let mut seq = 2;
    match phase {
        Phase::Planning => {
            // Already in Planning after creation
        }
        Phase::Reviewing => {
            view.apply_event(
                &agg_id,
                &WorkflowEvent::PlanningCompleted {
                    plan_path: PlanPath::from(PathBuf::from("/tmp/test/plan.md")),
                    completed_at: TimestampUtc::now(),
                },
                seq,
            );
        }
        Phase::Revising => {
            view.apply_event(
                &agg_id,
                &WorkflowEvent::PlanningCompleted {
                    plan_path: PlanPath::from(PathBuf::from("/tmp/test/plan.md")),
                    completed_at: TimestampUtc::now(),
                },
                seq,
            );
            seq += 1;
            view.apply_event(
                &agg_id,
                &WorkflowEvent::ReviewCycleCompleted {
                    approved: false,
                    completed_at: TimestampUtc::now(),
                },
                seq,
            );
        }
        Phase::AwaitingPlanningDecision => {
            view.apply_event(
                &agg_id,
                &WorkflowEvent::PlanningCompleted {
                    plan_path: PlanPath::from(PathBuf::from("/tmp/test/plan.md")),
                    completed_at: TimestampUtc::now(),
                },
                seq,
            );
            seq += 1;
            // Use PlanningMaxIterationsReached to enter AwaitingPlanningDecision
            view.apply_event(
                &agg_id,
                &WorkflowEvent::PlanningMaxIterationsReached {
                    reached_at: TimestampUtc::now(),
                },
                seq,
            );
        }
        Phase::Complete => {
            view.apply_event(
                &agg_id,
                &WorkflowEvent::PlanningCompleted {
                    plan_path: PlanPath::from(PathBuf::from("/tmp/test/plan.md")),
                    completed_at: TimestampUtc::now(),
                },
                seq,
            );
            seq += 1;
            view.apply_event(
                &agg_id,
                &WorkflowEvent::ReviewCycleCompleted {
                    approved: true,
                    completed_at: TimestampUtc::now(),
                },
                seq,
            );
        }
    }

    view
}

fn make_test_session_with_phase(phase: Phase) -> Session {
    let mut session = Session::default();
    session.workflow_view = Some(make_test_view(phase));
    session
}

fn make_test_session_with_impl_phase(impl_phase: ImplementationPhase) -> Session {
    let mut session = Session::default();
    let mut view = make_test_view(Phase::Complete);

    // Apply events to set up implementation state (proper CQRS approach)
    view.apply_event(
        "test-id",
        &WorkflowEvent::ImplementationStarted {
            max_iterations: MaxIterations(3),
            started_at: TimestampUtc::default(),
        },
        1,
    );
    view.apply_event(
        "test-id",
        &WorkflowEvent::ImplementationRoundStarted {
            iteration: Iteration(1),
            started_at: TimestampUtc::default(),
        },
        2,
    );

    // Set the desired phase by applying appropriate events
    match impl_phase {
        ImplementationPhase::Implementing => {
            // Already in Implementing from ImplementationRoundStarted
        }
        ImplementationPhase::ImplementationReview => {
            view.apply_event(
                "test-id",
                &WorkflowEvent::ImplementationReviewCompleted {
                    iteration: Iteration(1),
                    verdict: ImplementationVerdict::NeedsChanges,
                    feedback: None,
                    completed_at: TimestampUtc::default(),
                },
                3,
            );
        }
        ImplementationPhase::AwaitingDecision => {
            view.apply_event(
                "test-id",
                &WorkflowEvent::ImplementationMaxIterationsReached {
                    reached_at: TimestampUtc::default(),
                },
                3,
            );
        }
        ImplementationPhase::Complete => {
            view.apply_event(
                "test-id",
                &WorkflowEvent::ImplementationAccepted {
                    approved_at: TimestampUtc::default(),
                },
                3,
            );
        }
    }

    session.workflow_view = Some(view);
    session
}

#[test]
fn test_chips_mode_planning_phase_active() {
    let session = make_test_session_with_phase(Phase::Planning);
    let theme = Theme::planning();
    let spans = build_phase_spans(
        &session,
        &theme,
        PhaseDisplayMode::Chips { spinner_frame: 0 },
    );

    // First span should contain spinner character for active Planning
    let first_content = spans[0].content.to_string();
    assert!(
        first_content.contains('⠋'),
        "Expected spinner for active phase, got: {}",
        first_content
    );
}

#[test]
fn test_chips_mode_reviewing_shows_planning_complete() {
    let session = make_test_session_with_phase(Phase::Reviewing);
    let theme = Theme::planning();
    let spans = build_phase_spans(
        &session,
        &theme,
        PhaseDisplayMode::Chips { spinner_frame: 0 },
    );

    // First span should contain checkmark for completed Planning
    let first_content = spans[0].content.to_string();
    assert!(
        first_content.contains('✓'),
        "Expected checkmark for completed phase, got: {}",
        first_content
    );
}

#[test]
fn test_arrows_mode_has_separators() {
    let session = make_test_session_with_phase(Phase::Planning);
    let theme = Theme::planning();
    let spans = build_phase_spans(&session, &theme, PhaseDisplayMode::Arrows);

    // Should have arrow separators
    let all_text: String = spans.iter().map(|s| s.content.to_string()).collect();
    assert!(
        all_text.contains(" → "),
        "Expected arrow separators in arrows mode, got: {}",
        all_text
    );
}

#[test]
fn test_awaiting_planning_decision_maps_to_reviewing_current() {
    let session = make_test_session_with_phase(Phase::AwaitingPlanningDecision);
    let theme = Theme::planning();
    let spans = build_phase_spans(
        &session,
        &theme,
        PhaseDisplayMode::Chips { spinner_frame: 0 },
    );

    // Span structure: [symbol, "Plan", " ", symbol, "Review", " ", symbol, "Revise", " ", symbol, "Done"]
    let all_text: String = spans.iter().map(|s| s.content.to_string()).collect();

    // Planning should be complete (checkmark)
    assert!(
        all_text.contains("✓"),
        "Expected checkmark for completed Planning, got: {}",
        all_text
    );

    // Reviewing should be active (spinner) - because AwaitingPlanningDecision maps to Reviewing
    assert!(
        all_text.contains("⠋"),
        "Expected spinner for active Reviewing, got: {}",
        all_text
    );

    // Revise and Done should be pending (circles)
    let circle_count = all_text.matches('○').count();
    assert_eq!(
        circle_count, 2,
        "Expected 2 pending phases (Revise, Done), got {} in: {}",
        circle_count, all_text
    );
}

#[test]
fn test_awaiting_max_iterations_decision_shows_deciding_active() {
    let session = make_test_session_with_impl_phase(ImplementationPhase::AwaitingDecision);
    let theme = Theme::implementation();
    let spans = build_phase_spans(
        &session,
        &theme,
        PhaseDisplayMode::Chips { spinner_frame: 0 },
    );

    let all_text: String = spans.iter().map(|s| s.content.to_string()).collect();

    // Impl and Review should be complete (2 checkmarks)
    let checkmark_count = all_text.matches('✓').count();
    assert_eq!(
        checkmark_count, 2,
        "Expected Impl and Review to be complete, got {} checkmarks in: {}",
        checkmark_count, all_text
    );

    // Decide should be active (1 spinner)
    assert!(
        all_text.contains("⠋"),
        "Expected spinner for active Deciding, got: {}",
        all_text
    );

    // Done should be pending (1 circle)
    let circle_count = all_text.matches('○').count();
    assert_eq!(
        circle_count, 1,
        "Expected 1 pending phase (Done), got {} in: {}",
        circle_count, all_text
    );
}

#[test]
fn test_chips_mode_complete_phase() {
    let session = make_test_session_with_phase(Phase::Complete);
    let theme = Theme::planning();
    let spans = build_phase_spans(
        &session,
        &theme,
        PhaseDisplayMode::Chips { spinner_frame: 0 },
    );

    let all_text: String = spans.iter().map(|s| s.content.to_string()).collect();

    // All phases should show as complete (checkmarks)
    let checkmark_count = all_text.matches('✓').count();
    assert_eq!(
        checkmark_count, 4,
        "Expected all 4 phases to be complete, got {} checkmarks in: {}",
        checkmark_count, all_text
    );

    // No pending phases
    let circle_count = all_text.matches('○').count();
    assert_eq!(
        circle_count, 0,
        "Expected no pending phases, got {} in: {}",
        circle_count, all_text
    );
}

#[test]
fn test_arrows_mode_no_symbols() {
    let session = make_test_session_with_phase(Phase::Planning);
    let theme = Theme::planning();
    let spans = build_phase_spans(&session, &theme, PhaseDisplayMode::Arrows);

    let all_text: String = spans.iter().map(|s| s.content.to_string()).collect();

    // Should not contain chip-mode symbols
    assert!(
        !all_text.contains('✓'),
        "Arrows mode should not contain checkmarks, got: {}",
        all_text
    );
    assert!(
        !all_text.contains('○'),
        "Arrows mode should not contain circles, got: {}",
        all_text
    );
    assert!(
        !all_text.contains('⠋'),
        "Arrows mode should not contain spinners, got: {}",
        all_text
    );

    // Should contain full phase names
    assert!(
        all_text.contains("Planning"),
        "Expected 'Planning' in arrows mode, got: {}",
        all_text
    );
    assert!(
        all_text.contains("Reviewing"),
        "Expected 'Reviewing' in arrows mode, got: {}",
        all_text
    );
    assert!(
        all_text.contains("Revising"),
        "Expected 'Revising' in arrows mode, got: {}",
        all_text
    );
    assert!(
        all_text.contains("Complete"),
        "Expected 'Complete' in arrows mode, got: {}",
        all_text
    );
}

#[test]
fn test_chips_mode_short_names() {
    let session = make_test_session_with_phase(Phase::Planning);
    let theme = Theme::planning();
    let spans = build_phase_spans(
        &session,
        &theme,
        PhaseDisplayMode::Chips { spinner_frame: 0 },
    );

    let all_text: String = spans.iter().map(|s| s.content.to_string()).collect();

    // Should contain short phase names
    assert!(
        all_text.contains("Plan"),
        "Expected 'Plan' in chips mode, got: {}",
        all_text
    );
    assert!(
        all_text.contains("Review"),
        "Expected 'Review' in chips mode, got: {}",
        all_text
    );
    assert!(
        all_text.contains("Revise"),
        "Expected 'Revise' in chips mode, got: {}",
        all_text
    );
    assert!(
        all_text.contains("Done"),
        "Expected 'Done' in chips mode, got: {}",
        all_text
    );
}
