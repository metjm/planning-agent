//! Review prompt builders for file-based review workflow.
//!
//! This module contains the prompt construction functions used to instruct
//! review agents to read plans and write feedback to files.

use crate::diagnostics::truncate_for_recovery_prompt;
use crate::prompt_format::PromptBuilder;
use std::path::Path;

/// System prompt for file-based review
pub const REVIEW_SYSTEM_PROMPT: &str = r#"Use the "plan-review" skill to review the plan. Write your review to the feedback-output-path file."#;

/// Build the review prompt that instructs the agent to read the plan and write feedback to a file.
/// This is the primary prompt builder for initial review attempts.
pub fn build_review_prompt_for_agent(
    objective: &str,
    plan_path_abs: &Path,
    feedback_path_abs: &Path,
    working_dir: &Path,
    session_folder_abs: &Path,
    require_tags: bool,
) -> String {
    let output_format = if require_tags {
        r###"CRITICAL: You MUST write your complete review to the feedback-output-path file.

The file content MUST be wrapped in <plan-feedback> tags and include these required sections:

<plan-feedback>
## Summary
[Your review summary - 2-3 sentences]

## Critical Issues
[List blocking issues that must be fixed, or "None." if there are no critical issues]

## Recommendations
[Non-blocking suggestions for improvement]

## Overall Assessment: APPROVED
(or "## Overall Assessment: NEEDS REVISION" if the plan needs changes)
</plan-feedback>

Your review will ONLY be read from the feedback-output-path file. Do NOT rely on stdout for your review."###
    } else {
        r###"CRITICAL: You MUST write your complete review to the feedback-output-path file.

The file content MUST include these required sections:

## Summary
[Your review summary - 2-3 sentences]

## Critical Issues
[List blocking issues that must be fixed, or "None." if there are no critical issues]

## Recommendations
[Non-blocking suggestions for improvement]

## Overall Assessment: APPROVED
(or "## Overall Assessment: NEEDS REVISION" if the plan needs changes)

Your review will ONLY be read from the feedback-output-path file. Do NOT rely on stdout for your review."###
    };

    PromptBuilder::new()
        .phase("reviewing")
        .instructions(r#"Use the "plan-review" skill to review the plan. Write your review to the feedback-output-path file."#)
        .input("workspace-root", &working_dir.display().to_string())
        .input("objective", objective)
        .input("plan-path", &plan_path_abs.display().to_string())
        .input("feedback-output-path", &feedback_path_abs.display().to_string())
        .input("session-folder-path", &session_folder_abs.display().to_string())
        .output_format(output_format)
        .build()
}

/// Build a recovery prompt for when the initial review attempt fails to produce valid feedback.
/// Uses the same feedback file path (stable across attempts) and includes explicit template.
pub fn build_review_recovery_prompt_for_agent(
    plan_path_abs: &Path,
    feedback_path_abs: &Path,
    session_folder_abs: &Path,
    failure_reason: &str,
    previous_output: &str,
    require_tags: bool,
) -> String {
    let truncated_output = truncate_for_recovery_prompt(previous_output);

    let template = if require_tags {
        r###"<plan-feedback>
## Summary
[Your review summary - 2-3 sentences]

## Critical Issues
[List blocking issues, or "None." if there are no critical issues]

## Recommendations
[Non-blocking suggestions]

## Overall Assessment: APPROVED
</plan-feedback>

(Use "## Overall Assessment: NEEDS REVISION" instead if the plan needs changes)"###
    } else {
        r###"## Summary
[Your review summary - 2-3 sentences]

## Critical Issues
[List blocking issues, or "None." if there are no critical issues]

## Recommendations
[Non-blocking suggestions]

## Overall Assessment: APPROVED

(Use "## Overall Assessment: NEEDS REVISION" instead if the plan needs changes)"###
    };

    PromptBuilder::new()
        .phase("reviewing-recovery")
        .instructions(&format!(
            r###"RECOVERY ATTEMPT: Your previous review attempt failed to produce valid feedback.

FAILURE REASON: {}

You MUST complete your review by writing valid feedback to the feedback file.

Steps to complete:
1. If needed, re-read the plan from: {}
2. Write your complete review to: {}
3. Session folder for reference: {}

CRITICAL REQUIREMENTS:
- The feedback file MUST contain an "Overall Assessment: APPROVED" or "Overall Assessment: NEEDS REVISION"
- You MUST write to the exact file path specified

Use this exact template for your feedback file content:

{}

Previous output (for context):
---
{}
---

Please complete your review now by writing the feedback file."###,
            failure_reason,
            plan_path_abs.display(),
            feedback_path_abs.display(),
            session_folder_abs.display(),
            template,
            truncated_output
        ))
        .constraint("You MUST write your review to the feedback-output-path file")
        .constraint("Your review MUST include 'Overall Assessment: APPROVED' or 'Overall Assessment: NEEDS REVISION'")
        .build()
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
            false,
        );

        assert!(prompt.contains("/home/user/plan.md"));
        assert!(prompt.contains("/home/user/feedback.md"));
        assert!(prompt.contains("/home/user/project"));
        assert!(prompt.contains("Implement feature X"));
        assert!(prompt.contains("feedback-output-path"));
        assert!(prompt.contains("plan-path"));
    }

    #[test]
    fn test_build_review_prompt_requires_skill_and_file() {
        let prompt = build_review_prompt_for_agent(
            "Implement feature X",
            Path::new("/home/user/plan.md"),
            Path::new("/home/user/feedback.md"),
            Path::new("/home/user/project"),
            Path::new("/home/user/.planning-agent/sessions/abc123"),
            false,
        );

        assert!(prompt.contains("plan-review"));
        assert!(prompt.contains("feedback-output-path"));
    }

    #[test]
    fn test_build_review_prompt_with_tags_required() {
        let prompt = build_review_prompt_for_agent(
            "Implement feature X",
            Path::new("/home/user/plan.md"),
            Path::new("/home/user/feedback.md"),
            Path::new("/home/user/project"),
            Path::new("/home/user/.planning-agent/sessions/abc123"),
            true,
        );

        assert!(prompt.contains("<plan-feedback>"));
        assert!(prompt.contains("</plan-feedback>"));
    }

    #[test]
    fn test_build_review_prompt_without_tags_required() {
        let prompt = build_review_prompt_for_agent(
            "Implement feature X",
            Path::new("/home/user/plan.md"),
            Path::new("/home/user/feedback.md"),
            Path::new("/home/user/project"),
            Path::new("/home/user/.planning-agent/sessions/abc123"),
            false,
        );

        // Should still explain the format but not require tags
        assert!(prompt.contains("## Summary"));
        assert!(prompt.contains("## Overall Assessment"));
    }

    #[test]
    fn test_build_recovery_prompt_includes_failure_reason() {
        let prompt = build_review_recovery_prompt_for_agent(
            Path::new("/home/user/plan.md"),
            Path::new("/home/user/feedback.md"),
            Path::new("/home/user/.planning-agent/sessions/abc123"),
            "Missing Overall Assessment",
            "Some previous output",
            false,
        );

        assert!(prompt.contains("RECOVERY ATTEMPT"));
        assert!(prompt.contains("Missing Overall Assessment"));
        assert!(prompt.contains("/home/user/plan.md"));
        assert!(prompt.contains("/home/user/feedback.md"));
        assert!(prompt.contains("Some previous output"));
    }

    #[test]
    fn test_build_recovery_prompt_with_tags() {
        let prompt = build_review_recovery_prompt_for_agent(
            Path::new("/home/user/plan.md"),
            Path::new("/home/user/feedback.md"),
            Path::new("/home/user/.planning-agent/sessions/abc123"),
            "Missing tags",
            "Previous output",
            true,
        );

        assert!(prompt.contains("<plan-feedback>"));
        assert!(prompt.contains("</plan-feedback>"));
    }

    #[test]
    fn test_build_recovery_prompt_truncates_long_output() {
        let long_output = "x".repeat(100_000);
        let prompt = build_review_recovery_prompt_for_agent(
            Path::new("/home/user/plan.md"),
            Path::new("/home/user/feedback.md"),
            Path::new("/home/user/.planning-agent/sessions/abc123"),
            "Parse failure",
            &long_output,
            false,
        );

        // Should be truncated
        assert!(prompt.len() < long_output.len());
        assert!(prompt.contains("TRUNCATED") || prompt.len() < 60_000);
    }

    #[test]
    fn test_review_system_prompt_file_based() {
        assert!(REVIEW_SYSTEM_PROMPT.contains("plan-review"));
        assert!(REVIEW_SYSTEM_PROMPT.contains("feedback-output-path"));
    }

    #[test]
    fn test_build_review_prompt_includes_session_folder() {
        let prompt = build_review_prompt_for_agent(
            "Implement feature X",
            Path::new("/home/user/plan.md"),
            Path::new("/home/user/feedback.md"),
            Path::new("/home/user/project"),
            Path::new("/home/user/.planning-agent/sessions/abc123"),
            false,
        );

        assert!(prompt.contains("<session-folder-path>"));
        assert!(prompt.contains("/home/user/.planning-agent/sessions/abc123"));
    }

    #[test]
    fn test_build_recovery_prompt_includes_session_folder() {
        let prompt = build_review_recovery_prompt_for_agent(
            Path::new("/home/user/plan.md"),
            Path::new("/home/user/feedback.md"),
            Path::new("/home/user/.planning-agent/sessions/abc123"),
            "Missing Overall Assessment",
            "Some previous output",
            false,
        );

        assert!(prompt.contains("/home/user/.planning-agent/sessions/abc123"));
        assert!(prompt.contains("Session folder for reference"));
    }
}
