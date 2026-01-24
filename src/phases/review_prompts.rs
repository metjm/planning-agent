//! Review prompt builders for file-based review workflow.
//!
//! This module contains the prompt construction functions used to instruct
//! review agents to read plans and write feedback to files.

use crate::diagnostics::truncate_for_recovery_prompt;
use std::path::Path;

/// System prompt for file-based review - kept minimal since skill handles the details
pub const REVIEW_SYSTEM_PROMPT: &str = "You are a technical plan reviewer.";

/// Build the review prompt that instructs the agent to use the plan-review skill.
///
/// The skill contains the full review methodology - this prompt just provides context
/// and tells the agent to invoke it. Custom focus (if any) is placed before the skill
/// instruction so it's seen as context, but the skill invocation is always last.
pub fn build_review_prompt_for_agent(
    objective: &str,
    plan_path_abs: &Path,
    feedback_path_abs: &Path,
    working_dir: &Path,
    session_folder_abs: &Path,
    custom_focus: Option<&str>,
) -> String {
    let focus_section = match custom_focus {
        Some(focus) => format!(
            "\n########################## REVIEW FOCUS ##########################\n{}\n##################################################################\n",
            focus
        ),
        None => String::new(),
    };

    format!(
        r#"Review an implementation plan.

########################### PLAN GOAL ###########################
{objective}
#################################################################

Paths:
- Workspace: {workspace}
- Plan file: {plan}
- Feedback output: {feedback}
- Session folder: {session}
{focus_section}
Run the "plan-review" skill to perform the review."#,
        objective = objective,
        workspace = working_dir.display(),
        plan = plan_path_abs.display(),
        feedback = feedback_path_abs.display(),
        session = session_folder_abs.display(),
        focus_section = focus_section,
    )
}

/// Build a recovery prompt for when the initial review attempt fails to produce valid feedback.
/// This is used when the skill ran but didn't produce a parseable feedback file.
pub fn build_review_recovery_prompt_for_agent(
    plan_path_abs: &Path,
    feedback_path_abs: &Path,
    failure_reason: &str,
    previous_output: &str,
) -> String {
    let truncated_output = truncate_for_recovery_prompt(previous_output);

    format!(
        r###"Your review attempt failed: {failure_reason}

Previous output for reference:
---
{previous_output}
---

Plan file: {plan}
Feedback output: {feedback}

The feedback file MUST contain these sections:
## Summary, ## Critical Issues, ## Recommendations, ## Overall Assessment: APPROVED (or NEEDS REVISION)

Run the "plan-review" skill to complete the review."###,
        failure_reason = failure_reason,
        previous_output = truncated_output,
        plan = plan_path_abs.display(),
        feedback = feedback_path_abs.display(),
    )
}

#[cfg(test)]
mod tests {
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
}
