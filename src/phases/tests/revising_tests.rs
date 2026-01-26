use super::*;
use crate::domain::types::{
    FeatureName, FeedbackPath, Iteration, MaxIterations, Objective, Phase, PlanPath, WorkflowId,
    WorkingDir,
};
use crate::domain::view::WorkflowView;
use std::path::{Path, PathBuf};
use uuid::Uuid;

fn test_reviews() -> Vec<ReviewResult> {
    vec![
        ReviewResult {
            agent_name: "claude".to_string(),
            needs_revision: true,
            feedback: "Issue 1: Missing tests".to_string(),
            summary: "Missing test coverage".to_string(),
        },
        ReviewResult {
            agent_name: "codex".to_string(),
            needs_revision: true,
            feedback: "Issue 2: Unclear architecture".to_string(),
            summary: "Architecture needs clarification".to_string(),
        },
    ]
}

fn minimal_view() -> WorkflowView {
    WorkflowView {
        workflow_id: Some(WorkflowId(Uuid::new_v4())),
        feature_name: Some(FeatureName("test".to_string())),
        objective: Some(Objective("test objective".to_string())),
        working_dir: Some(WorkingDir(PathBuf::from("/workspaces/myproject"))),
        plan_path: Some(PlanPath(PathBuf::from(
            "/home/user/.planning-agent/sessions/abc123/plan.md",
        ))),
        feedback_path: Some(FeedbackPath(PathBuf::from(
            "/home/user/.planning-agent/sessions/abc123/feedback.md",
        ))),
        planning_phase: Some(Phase::Revising),
        iteration: Some(Iteration(1)),
        max_iterations: Some(MaxIterations(3)),
        ..Default::default()
    }
}

#[test]
fn test_revision_prompt_includes_plan_path() {
    let view = minimal_view();

    let reviews = test_reviews();
    let working_dir = std::path::Path::new("/workspaces/myproject");
    let session_folder = Path::new("/home/user/.planning-agent/sessions/abc123");

    // Test with session_resume_active = false (full context prompt)
    let prompt =
        build_revision_prompt_with_reviews(&view, &reviews, working_dir, session_folder, false, 1);

    eprintln!("Generated revision prompt:\n{}", prompt);

    // The full context prompt should include plan-output-path
    assert!(
        prompt.contains("<plan-output-path>"),
        "Revision prompt should contain <plan-output-path> tag"
    );
    assert!(
        prompt.contains("/home/user/.planning-agent/sessions/abc123/plan.md"),
        "Revision prompt should contain the plan file path"
    );
}

#[test]
fn test_build_revision_prompt_full_context() {
    let view = minimal_view();

    let reviews = test_reviews();
    let working_dir = Path::new("/workspaces/myproject");
    let session_folder = Path::new("/home/user/.planning-agent/sessions/abc123");

    // Test with session_resume_active = false (full context prompt)
    let prompt =
        build_revision_prompt_with_reviews(&view, &reviews, working_dir, session_folder, false, 1);

    // Check XML structure
    assert!(prompt.starts_with("<user-prompt>"));
    assert!(prompt.ends_with("</user-prompt>"));
    assert!(prompt.contains("<phase>revising</phase>"));
    // Check summary table is present
    assert!(prompt.contains("| Reviewer | Verdict | Summary |"));
    assert!(prompt.contains("| claude | NEEDS REVISION | Missing test coverage |"));
    assert!(prompt.contains("| codex | NEEDS REVISION | Architecture needs clarification |"));
    // Check feedback file paths are present
    assert!(prompt.contains("feedback_1_claude.md"));
    assert!(prompt.contains("feedback_1_codex.md"));
    // Check inputs
    assert!(prompt.contains("<workspace-root>/workspaces/myproject</workspace-root>"));
    // Check constraints
    assert!(prompt.contains("Use absolute paths"));
}

#[test]
fn test_build_revision_prompt_session_resume() {
    let view = minimal_view();

    let reviews = test_reviews();
    let working_dir = Path::new("/workspaces/myproject");
    let session_folder = Path::new("/home/user/.planning-agent/sessions/abc123");

    // Test with session_resume_active = true (simplified continuation prompt)
    let prompt =
        build_revision_prompt_with_reviews(&view, &reviews, working_dir, session_folder, true, 1);

    // Should NOT be XML structured
    assert!(!prompt.starts_with("<user-prompt>"));
    assert!(!prompt.contains("<phase>revising</phase>"));

    // Should be a simpler continuation prompt
    assert!(prompt.contains("The reviewers have provided feedback"));
    assert!(prompt.contains("Please revise the plan"));

    // Check summary table is present
    assert!(prompt.contains("| Reviewer | Verdict | Summary |"));
    assert!(prompt.contains("| claude | NEEDS REVISION | Missing test coverage |"));
    assert!(prompt.contains("| codex | NEEDS REVISION | Architecture needs clarification |"));

    // Check feedback file paths are present
    assert!(prompt.contains("feedback_1_claude.md"));
    assert!(prompt.contains("feedback_1_codex.md"));

    // Should reference the plan file
    assert!(prompt.contains("plan.md"));
}

#[test]
fn revision_system_prompt_contains_no_timeline_directive() {
    assert!(
        REVISION_SYSTEM_PROMPT.contains("DO NOT include timelines"),
        "REVISION_SYSTEM_PROMPT must contain the no-timeline directive"
    );
    assert!(
        REVISION_SYSTEM_PROMPT.contains("in two weeks"),
        "REVISION_SYSTEM_PROMPT must contain example phrase 'in two weeks'"
    );
}

#[test]
fn revision_prompt_session_resume_contains_no_timeline_directive() {
    let view = minimal_view();

    let reviews = test_reviews();
    let working_dir = Path::new("/workspaces/myproject");
    let session_folder = Path::new("/home/user/.planning-agent/sessions/abc123");

    // Test with session_resume_active = true (simplified continuation prompt)
    let prompt =
        build_revision_prompt_with_reviews(&view, &reviews, working_dir, session_folder, true, 1);

    assert!(
        prompt.contains("Do not add timelines"),
        "Session resume prompt must contain the no-timeline directive"
    );
}

#[test]
fn revision_prompt_full_context_contains_no_timeline_directive() {
    let view = minimal_view();

    let reviews = test_reviews();
    let working_dir = Path::new("/workspaces/myproject");
    let session_folder = Path::new("/home/user/.planning-agent/sessions/abc123");

    // Test with session_resume_active = false (full context prompt)
    let prompt =
        build_revision_prompt_with_reviews(&view, &reviews, working_dir, session_folder, false, 1);

    assert!(
        prompt.contains("DO NOT include timelines"),
        "Full context prompt must contain the no-timeline directive"
    );
}

#[test]
fn test_revision_prompt_includes_session_folder_full_context() {
    let view = minimal_view();

    let reviews = test_reviews();
    let working_dir = Path::new("/workspaces/myproject");
    let session_folder = Path::new("/home/user/.planning-agent/sessions/abc123");

    let prompt =
        build_revision_prompt_with_reviews(&view, &reviews, working_dir, session_folder, false, 1);

    assert!(prompt.contains("<session-folder-path>"));
    assert!(prompt.contains("/home/user/.planning-agent/sessions/abc123"));
}

#[test]
fn test_revision_prompt_includes_session_folder_session_resume() {
    let view = minimal_view();

    let reviews = test_reviews();
    let working_dir = Path::new("/workspaces/myproject");
    let session_folder = Path::new("/home/user/.planning-agent/sessions/abc123");

    let prompt =
        build_revision_prompt_with_reviews(&view, &reviews, working_dir, session_folder, true, 1);

    assert!(prompt.contains("session folder"));
    assert!(prompt.contains("/home/user/.planning-agent/sessions/abc123"));
}
