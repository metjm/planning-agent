use super::*;

#[test]
fn test_build_review_prompt_includes_paths() {
    let prompt = build_review_prompt_for_agent(
        "Implement feature X",
        Path::new("/home/user/plan.md"),
        Path::new("/home/user/feedback.md"),
        Path::new("/home/user/project"),
        Path::new("/home/user/.planning-agent/sessions/abc123"),
        None, // custom_focus
        None, // skill_name
    );

    assert!(prompt.contains("/home/user/plan.md"));
    assert!(prompt.contains("/home/user/feedback.md"));
    assert!(prompt.contains("/home/user/project"));
    assert!(prompt.contains("Implement feature X"));
}

#[test]
fn test_build_review_prompt_invokes_default_skill() {
    let prompt = build_review_prompt_for_agent(
        "Implement feature X",
        Path::new("/home/user/plan.md"),
        Path::new("/home/user/feedback.md"),
        Path::new("/home/user/project"),
        Path::new("/home/user/.planning-agent/sessions/abc123"),
        None, // custom_focus
        None, // skill_name - defaults to plan-review-adversarial
    );

    // When no skill specified, should use default (adversarial)
    assert!(prompt.contains("plan-review-adversarial"));
    assert!(prompt.ends_with(r#"Run the "plan-review-adversarial" skill to perform the review."#));
}

#[test]
fn test_build_review_prompt_with_specified_skill() {
    let prompt = build_review_prompt_for_agent(
        "Implement feature X",
        Path::new("/home/user/plan.md"),
        Path::new("/home/user/feedback.md"),
        Path::new("/home/user/project"),
        Path::new("/home/user/.planning-agent/sessions/abc123"),
        None,
        Some("plan-review-operational"),
    );

    assert!(prompt.contains("plan-review-operational"));
    assert!(prompt.ends_with(r#"Run the "plan-review-operational" skill to perform the review."#));
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
        None,
    );

    assert!(prompt.contains("PLAN GOAL"));
    assert!(prompt.contains("###"));
}

#[test]
fn test_build_review_prompt_with_custom_focus() {
    // custom_focus is additional context, inserted as REVIEW FOCUS section
    let prompt = build_review_prompt_for_agent(
        "Implement feature X",
        Path::new("/home/user/plan.md"),
        Path::new("/home/user/feedback.md"),
        Path::new("/home/user/project"),
        Path::new("/home/user/.planning-agent/sessions/abc123"),
        Some("Focus on security and performance."),
        None,
    );

    // custom_focus appears in REVIEW FOCUS section
    assert!(prompt.contains("REVIEW FOCUS"));
    assert!(prompt.contains("Focus on security and performance."));
    // Skill invocation is still at the end
    assert!(prompt.ends_with(r#"Run the "plan-review-adversarial" skill to perform the review."#));
}

#[test]
fn test_build_review_prompt_with_both_focus_and_skill() {
    // Both custom_focus and skill_name can be provided
    let prompt = build_review_prompt_for_agent(
        "Implement feature X",
        Path::new("/home/user/plan.md"),
        Path::new("/home/user/feedback.md"),
        Path::new("/home/user/project"),
        Path::new("/home/user/.planning-agent/sessions/abc123"),
        Some("Focus on security."),
        Some("plan-review-codebase"),
    );

    // custom_focus appears in REVIEW FOCUS section
    assert!(prompt.contains("REVIEW FOCUS"));
    assert!(prompt.contains("Focus on security."));
    // Specified skill is used
    assert!(prompt.ends_with(r#"Run the "plan-review-codebase" skill to perform the review."#));
}

#[test]
fn test_build_recovery_prompt_includes_failure_reason() {
    let prompt = build_review_recovery_prompt_for_agent(
        Path::new("/home/user/plan.md"),
        Path::new("/home/user/feedback.md"),
        "Missing Overall Assessment",
        "Some previous output",
        "plan-review-adversarial",
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
        "plan-review-adversarial",
    );

    assert!(prompt.contains("Summary"));
    assert!(prompt.contains("Critical Issues"));
    assert!(prompt.contains("Recommendations"));
    assert!(prompt.contains("Overall Assessment"));
}

#[test]
fn test_build_recovery_prompt_uses_specified_skill() {
    // Test that recovery uses the skill name passed to it
    let prompt = build_review_recovery_prompt_for_agent(
        Path::new("/home/user/plan.md"),
        Path::new("/home/user/feedback.md"),
        "Parse failure",
        "Previous output",
        "plan-review-operational",
    );

    assert!(prompt.contains("plan-review-operational"));
    assert!(prompt.ends_with(r#"Run the "plan-review-operational" skill to complete the review."#));
}

#[test]
fn test_build_recovery_prompt_with_codebase_skill() {
    let prompt = build_review_recovery_prompt_for_agent(
        Path::new("/home/user/plan.md"),
        Path::new("/home/user/feedback.md"),
        "Parse failure",
        "Previous output",
        "plan-review-codebase",
    );

    assert!(prompt.contains("plan-review-codebase"));
    assert!(prompt.ends_with(r#"Run the "plan-review-codebase" skill to complete the review."#));
}

#[test]
fn test_build_recovery_prompt_truncates_long_output() {
    let long_output = "x".repeat(100_000);
    let prompt = build_review_recovery_prompt_for_agent(
        Path::new("/home/user/plan.md"),
        Path::new("/home/user/feedback.md"),
        "Parse failure",
        &long_output,
        "plan-review-adversarial",
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
        None,
    );

    assert!(prompt.contains("/home/user/.planning-agent/sessions/abc123"));
}
