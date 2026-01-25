use super::overlays::{build_phase_spans, PhaseDisplayMode};
use super::theme::Theme;
use crate::state::{ImplementationPhase, ImplementationPhaseState, Phase, State};
use crate::tui::session::Session;
use std::path::PathBuf;

fn make_test_state(phase: Phase) -> State {
    State {
        phase,
        iteration: 1,
        max_iterations: 3,
        feature_name: "test".to_string(),
        objective: "test".to_string(),
        plan_file: PathBuf::from("/tmp/plan.md"),
        feedback_file: PathBuf::from("/tmp/feedback.md"),
        last_feedback_status: None,
        approval_overridden: false,
        workflow_session_id: "test".to_string(),
        agent_conversations: Default::default(),
        invocations: Default::default(),
        updated_at: Default::default(),
        last_failure: None,
        failure_history: Default::default(),
        worktree_info: None,
        implementation_state: None,
        sequential_review: None,
    }
}

fn make_test_session_with_phase(phase: Phase) -> Session {
    let mut session = Session::default();
    session.workflow_state = Some(make_test_state(phase));
    session
}

fn make_test_session_with_impl_phase(impl_phase: ImplementationPhase) -> Session {
    let mut session = Session::default();
    let mut state = make_test_state(Phase::Complete);
    state.implementation_state = Some(ImplementationPhaseState {
        phase: impl_phase,
        iteration: 1,
        max_iterations: 3,
        last_verdict: None,
        last_feedback: None,
    });
    session.workflow_state = Some(state);
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
    let session =
        make_test_session_with_impl_phase(ImplementationPhase::AwaitingMaxIterationsDecision);
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
