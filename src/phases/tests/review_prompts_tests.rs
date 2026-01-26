use super::*;

#[test]
fn test_build_review_prompt_includes_paths() {
    let prompt = build_review_prompt_for_agent(
        "Implement feature X",
        Path::new("/home/user/plan.md"),
        Path::new("/home/user/feedback.md"),
        Path::new("/home/user/project"),
        Path::new("/home/user/.planning-agent/sessions/abc123"),
        None,
    );

    assert!(prompt.contains("/home/user/plan.md"));
    assert!(prompt.contains("/home/user/feedback.md"));
    assert!(prompt.contains("/home/user/project"));
    assert!(prompt.contains("Implement feature X"));
}

#[test]
fn test_build_review_prompt_invokes_skill() {
    let prompt = build_review_prompt_for_agent(
        "Implement feature X",
        Path::new("/home/user/plan.md"),
        Path::new("/home/user/feedback.md"),
        Path::new("/home/user/project"),
        Path::new("/home/user/.planning-agent/sessions/abc123"),
        None,
    );

    assert!(prompt.contains("plan-review"));
    assert!(prompt.ends_with(r#"Run the "plan-review" skill to perform the review."#));
}

#[test]
fn test_build_review_prompt_demarcates_goal() {
    let prompt = build_review_prompt_for_agent(
        "Implement feature X",
        Path::new("/home/user/plan.md"),
        Path::new("/home/user/feedback.md"),
        Path::new("/home/user/project"),
        Path::new("/home/user/.planning-agent/sessions/abc123"),
        None,
    );

    assert!(prompt.contains("PLAN GOAL"));
    assert!(prompt.contains("###"));
}

#[test]
fn test_build_review_prompt_with_custom_focus() {
    let prompt = build_review_prompt_for_agent(
        "Implement feature X",
        Path::new("/home/user/plan.md"),
        Path::new("/home/user/feedback.md"),
        Path::new("/home/user/project"),
        Path::new("/home/user/.planning-agent/sessions/abc123"),
        Some("Focus on security and performance."),
    );

    assert!(prompt.contains("REVIEW FOCUS"));
    assert!(prompt.contains("Focus on security and performance."));
    // Skill instruction should still be last
    assert!(prompt.ends_with(r#"Run the "plan-review" skill to perform the review."#));
}

#[test]
fn test_build_recovery_prompt_includes_failure_reason() {
    let prompt = build_review_recovery_prompt_for_agent(
        Path::new("/home/user/plan.md"),
        Path::new("/home/user/feedback.md"),
        "Missing Overall Assessment",
        "Some previous output",
    );

    assert!(prompt.contains("Missing Overall Assessment"));
    assert!(prompt.contains("/home/user/plan.md"));
    assert!(prompt.contains("/home/user/feedback.md"));
    assert!(prompt.contains("Some previous output"));
}

#[test]
fn test_build_recovery_prompt_includes_template() {
    let prompt = build_review_recovery_prompt_for_agent(
        Path::new("/home/user/plan.md"),
        Path::new("/home/user/feedback.md"),
        "Parse failure",
        "Previous output",
    );

    assert!(prompt.contains("Summary"));
    assert!(prompt.contains("Critical Issues"));
    assert!(prompt.contains("Recommendations"));
    assert!(prompt.contains("Overall Assessment"));
}

#[test]
fn test_build_recovery_prompt_ends_with_skill() {
    let prompt = build_review_recovery_prompt_for_agent(
        Path::new("/home/user/plan.md"),
        Path::new("/home/user/feedback.md"),
        "Parse failure",
        "Previous output",
    );

    assert!(prompt.ends_with(r#"Run the "plan-review" skill to complete the review."#));
}

#[test]
fn test_build_recovery_prompt_truncates_long_output() {
    let long_output = "x".repeat(100_000);
    let prompt = build_review_recovery_prompt_for_agent(
        Path::new("/home/user/plan.md"),
        Path::new("/home/user/feedback.md"),
        "Parse failure",
        &long_output,
    );

    // Should be truncated
    assert!(prompt.len() < long_output.len());
    assert!(prompt.contains("TRUNCATED") || prompt.len() < 60_000);
}

#[test]
fn test_review_system_prompt_minimal() {
    // System prompt is minimal - skill handles details
    assert!(REVIEW_SYSTEM_PROMPT.contains("reviewer"));
}

#[test]
fn test_build_review_prompt_includes_session_folder() {
    let prompt = build_review_prompt_for_agent(
        "Implement feature X",
        Path::new("/home/user/plan.md"),
        Path::new("/home/user/feedback.md"),
        Path::new("/home/user/project"),
        Path::new("/home/user/.planning-agent/sessions/abc123"),
        None,
    );

    assert!(prompt.contains("/home/user/.planning-agent/sessions/abc123"));
}
