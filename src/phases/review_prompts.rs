//! Review prompt builders for MCP-based review workflow.
//!
//! This module contains the prompt construction functions used to instruct
//! review agents and handle recovery scenarios.

use crate::diagnostics::truncate_for_recovery_prompt;
use crate::prompt_format::PromptBuilder;
use crate::state::State;
use std::path::Path;

pub const REVIEW_SYSTEM_PROMPT: &str = r#"You are a technical plan reviewer.
Review the plan for correctness, completeness, and technical accuracy.
Use the "plan-review" skill to review.
IMPORTANT: Use absolute paths for all file references in your feedback.

CRITICAL: You MUST call the `get_plan` MCP tool to retrieve the plan you are reviewing.
Do NOT search for or read plan files from the filesystem - the plan is ONLY available via the `get_plan` MCP tool.
"#;

/// Build the review prompt that will be embedded in the MCP server's get_plan response
pub fn build_mcp_review_prompt(state: &State, working_dir: &Path) -> String {
    PromptBuilder::new()
        .phase("reviewing")
        .instructions(r#"Review the implementation plan above for:
1. Technical correctness and feasibility
2. Completeness (does it address all requirements?)
3. Potential risks or issues
4. Code quality and best practices

After your review, you MUST submit your feedback using the `submit_review` MCP tool with:
- verdict: "APPROVED" or "NEEDS_REVISION"
- summary: A brief one-paragraph summary
- critical_issues: Array of blocking issues (if any)
- recommendations: Array of non-blocking suggestions"#)
        .input("workspace-root", &working_dir.display().to_string())
        .input("objective", &state.objective)
        .constraint("Use absolute paths for all file references in your feedback")
        .build()
}

/// Build the prompt that instructs the agent to use the MCP tools
pub fn build_mcp_agent_prompt(state: &State, working_dir: &Path) -> String {
    PromptBuilder::new()
        .phase("reviewing")
        .instructions(r#"You are reviewing an implementation plan.

CRITICAL - FIRST STEP: Call the `get_plan` MCP tool to retrieve the plan content.
The plan is ONLY available via the `get_plan` MCP tool.
Do NOT search for or read plan files from the filesystem - they are stored in a location you cannot guess.

Follow these steps:
1. Call `get_plan` MCP tool FIRST to retrieve the plan content and review instructions
2. Read and analyze the plan content returned by `get_plan`
3. You may read codebase files to verify the plan's technical claims
4. Submit your review using the `submit_review` MCP tool

You MUST use the MCP tools to complete this review:
- FIRST: Call `get_plan` to get the plan content (this is the ONLY way to get the plan)
- THEN: Call `submit_review` with your verdict and feedback"#)
        .input("workspace-root", &working_dir.display().to_string())
        .input("objective", &state.objective)
        .constraint("Use absolute paths for all file references in your feedback")
        .output_format(r#"After submitting your review via MCP, wrap your final assessment in <plan-feedback> tags:

<plan-feedback>
## Summary
[Your review summary]

## Critical Issues
[List any blocking issues, or "None" if approved]

## Recommendations
[Non-blocking suggestions]

## Overall Assessment: [APPROVED or NEEDS REVISION]
</plan-feedback>"#)
        .build()
}

/// Build a recovery prompt for when the initial review attempt fails to parse
pub fn build_mcp_recovery_prompt(
    mcp_server_name: &str,
    previous_output: &str,
    failure_reason: &str,
) -> String {
    let truncated_output = truncate_for_recovery_prompt(previous_output);

    PromptBuilder::new()
        .phase("reviewing-recovery")
        .instructions(&format!(
            r#"Your previous review attempt failed to produce a parseable verdict.

FAILURE REASON: {}

You MUST complete your review by calling the MCP tools from server "{}":
1. Call `get_plan` to retrieve the plan content
2. Call `submit_review` with your verdict ("APPROVED" or "NEEDS_REVISION") and feedback

CRITICAL: Your response MUST include either:
- A call to the `submit_review` MCP tool, OR
- A <plan-feedback> section with "Overall Assessment: APPROVED" or "Overall Assessment: NEEDS REVISION"

Previous output (for context):
---
{}
---

Please complete your review now by calling the MCP tools."#,
            failure_reason, mcp_server_name, truncated_output
        ))
        .output_format(r#"After submitting your review via MCP, wrap your final assessment in <plan-feedback> tags:

<plan-feedback>
## Summary
[Your review summary]

## Critical Issues
[List any blocking issues, or "None" if approved]

## Recommendations
[Non-blocking suggestions]

## Overall Assessment: [APPROVED or NEEDS REVISION]
</plan-feedback>"#)
        .build()
}

#[allow(dead_code)]
pub fn build_review_prompt_for_agent(
    objective: &str,
    plan_path_abs: &Path,
    feedback_path_abs: &Path,
    working_dir: &Path,
) -> String {
    PromptBuilder::new()
        .phase("reviewing")
        .instructions(r#"Review the implementation plan for technical correctness and completeness.

Read the plan file first, then provide your detailed review.

Provide your assessment with one of these verdicts:
- "APPROVED" - if the plan is ready for implementation
- "NEEDS REVISION" - if the plan has issues that need to be fixed

Include specific feedback about any issues found."#)
        .input("workspace-root", &working_dir.display().to_string())
        .input("objective", objective)
        .input("plan-path", &plan_path_abs.display().to_string())
        .input("feedback-output-path", &feedback_path_abs.display().to_string())
        .constraint("Use absolute paths for all file references in your feedback")
        .constraint("Your feedback MUST include 'Overall Assessment:** APPROVED' or 'Overall Assessment:** NEEDS REVISION'")
        .output_format(r#"CRITICAL: You MUST wrap your final feedback in <plan-feedback> tags. Only the content inside these tags will be saved as the review feedback. Everything outside these tags (thinking, tool calls, intermediate steps) will be ignored.

Example format:
<plan-feedback>
## Review Summary
...your assessment here...

## Issues Found
...specific issues (use absolute paths)...

## Overall Assessment: APPROVED/NEEDS REVISION
</plan-feedback>"#)
        .build()
}
