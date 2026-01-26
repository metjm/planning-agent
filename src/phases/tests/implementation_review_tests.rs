use super::*;
use std::path::PathBuf;

fn minimal_state() -> State {
    use crate::state::Phase;
    use std::collections::HashMap;

    State {
        phase: Phase::Complete,
        iteration: 1,
        max_iterations: 3,
        feature_name: "test-feature".to_string(),
        objective: "Test objective".to_string(),
        plan_file: PathBuf::from("/tmp/test-plan/plan.md"),
        feedback_file: PathBuf::from("/tmp/test-feedback.md"),
        last_feedback_status: None,
        approval_overridden: false,
        workflow_session_id: "test-session-id".to_string(),
        agent_conversations: HashMap::new(),
        invocations: Vec::new(),
        updated_at: String::new(),
        last_failure: None,
        failure_history: Vec::new(),
        worktree_info: None,
        implementation_state: None,
        sequential_review: None,
    }
}

#[test]
fn test_build_implementation_review_prompt_basic() {
    let state = minimal_state();
    let working_dir = PathBuf::from("/tmp/workspace");
    let prompt = build_implementation_review_prompt(&state, &working_dir, 1, None);

    // Check paths are included
    assert!(prompt.contains("/tmp/workspace"));
    assert!(prompt.contains("/tmp/test-plan/plan.md"));

    // Check iteration
    assert!(prompt.contains("IMPLEMENTATION REVIEW #1"));

    // Check skill instruction is last
    assert!(prompt.ends_with(r#"Run the "implementation-review" skill to perform the review."#));
}

#[test]
fn test_build_implementation_review_prompt_with_log_path() {
    let state = minimal_state();
    let working_dir = PathBuf::from("/tmp/workspace");
    let log_path = PathBuf::from("/tmp/session/implementation_1.log");
    let prompt = build_implementation_review_prompt(&state, &working_dir, 1, Some(&log_path));

    // Should include the implementation log path
    assert!(prompt.contains("Implementation log:"));
    assert!(prompt.contains("implementation_1.log"));

    // Skill instruction still last
    assert!(prompt.ends_with(r#"Run the "implementation-review" skill to perform the review."#));
}

#[test]
fn test_implementation_review_result_with_needs_revision_verdict() {
    let result = ImplementationReviewResult {
        verdict: VerificationVerdictResult::NeedsRevision,
        feedback: Some("Fix this".to_string()),
    };

    assert!(result.verdict.needs_revision());
    assert_eq!(result.feedback, Some("Fix this".to_string()));
}
