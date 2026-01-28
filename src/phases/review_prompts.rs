//! Review prompt builders for file-based review workflow.
//!
//! This module contains the prompt construction functions used to instruct
//! review agents to read plans and write feedback to files.

use crate::app::truncate_for_recovery_prompt;
use std::path::Path;

/// System prompt for file-based review - kept minimal since skill handles the details
pub const REVIEW_SYSTEM_PROMPT: &str = "You are a technical plan reviewer.";

/// Default skill to use when no skill is specified.
pub const DEFAULT_REVIEW_SKILL: &str = "plan-review-adversarial";

/// Build the review prompt that instructs the agent to use a review skill.
///
/// # Arguments
///
/// * `objective` - The plan goal/objective
/// * `plan_path_abs` - Absolute path to the plan file
/// * `feedback_path_abs` - Absolute path to write feedback
/// * `working_dir` - The workspace directory
/// * `session_folder_abs` - The session folder path
/// * `custom_focus` - Optional additional review context (inserted as REVIEW FOCUS section)
/// * `skill_name` - Optional skill to invoke (defaults to DEFAULT_REVIEW_SKILL)
///
/// The skill invocation is always last in the prompt. Custom focus, if provided,
/// appears before the skill invocation as additional context.
pub fn build_review_prompt_for_agent(
    objective: &str,
    plan_path_abs: &Path,
    feedback_path_abs: &Path,
    working_dir: &Path,
    session_folder_abs: &Path,
    custom_focus: Option<&str>,
    skill_name: Option<&str>,
) -> String {
    let skill = skill_name.unwrap_or(DEFAULT_REVIEW_SKILL);

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
Run the "{skill}" skill to perform the review."#,
        objective = objective,
        workspace = working_dir.display(),
        plan = plan_path_abs.display(),
        feedback = feedback_path_abs.display(),
        session = session_folder_abs.display(),
        focus_section = focus_section,
        skill = skill,
    )
}

/// Build a follow-up review prompt for when the planner has addressed previous feedback.
///
/// This prompt is used when resuming a reviewer conversation after the planner has revised
/// the plan. It instructs the reviewer to:
/// 1. Re-evaluate the plan from scratch with a fresh perspective
/// 2. Verify whether previous feedback was properly addressed
/// 3. Use the same skill for consistency
///
/// # Arguments
///
/// * `objective` - The plan goal/objective
/// * `plan_path_abs` - Absolute path to the plan file
/// * `feedback_path_abs` - Absolute path to write feedback
/// * `working_dir` - The workspace directory
/// * `session_folder_abs` - The session folder path
/// * `custom_focus` - Optional additional review context
/// * `skill_name` - The skill to invoke (should match the original review)
pub fn build_review_follow_up_prompt_for_agent(
    objective: &str,
    plan_path_abs: &Path,
    feedback_path_abs: &Path,
    working_dir: &Path,
    session_folder_abs: &Path,
    custom_focus: Option<&str>,
    skill_name: Option<&str>,
) -> String {
    let skill = skill_name.unwrap_or(DEFAULT_REVIEW_SKILL);

    let focus_section = match custom_focus {
        Some(focus) => format!(
            "\n########################## REVIEW FOCUS ##########################\n{}\n##################################################################\n",
            focus
        ),
        None => String::new(),
    };

    format!(
        r#"The planner has addressed your previous feedback and revised the plan.

####################### IMPORTANT INSTRUCTIONS #######################
1. Re-evaluate the plan FROM SCRATCH with a fresh perspective
2. Do NOT simply check if your previous feedback was addressed
3. Look for NEW issues that may have been introduced or overlooked
4. Your previous conversation context is available for reference only
######################################################################

########################### PLAN GOAL ###########################
{objective}
#################################################################

Paths:
- Workspace: {workspace}
- Plan file: {plan}
- Feedback output: {feedback}
- Session folder: {session}
{focus_section}
IMPORTANT: You MUST run the "{skill}" skill again to perform this review. Do not skip invoking the skill."#,
        objective = objective,
        workspace = working_dir.display(),
        plan = plan_path_abs.display(),
        feedback = feedback_path_abs.display(),
        session = session_folder_abs.display(),
        focus_section = focus_section,
        skill = skill,
    )
}

/// Build a recovery prompt for when the initial review attempt fails to produce valid feedback.
/// This is used when the skill ran but didn't produce a parseable feedback file.
///
/// # Arguments
///
/// * `plan_path_abs` - Absolute path to the plan file
/// * `feedback_path_abs` - Absolute path to write feedback
/// * `failure_reason` - Description of why the previous attempt failed
/// * `previous_output` - The output from the failed attempt
/// * `skill_name` - The skill to invoke (should match the original attempt)
pub fn build_review_recovery_prompt_for_agent(
    plan_path_abs: &Path,
    feedback_path_abs: &Path,
    failure_reason: &str,
    previous_output: &str,
    skill_name: &str,
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

Run the "{skill_name}" skill to complete the review."###,
        failure_reason = failure_reason,
        previous_output = truncated_output,
        plan = plan_path_abs.display(),
        feedback = feedback_path_abs.display(),
        skill_name = skill_name,
    )
}

#[cfg(test)]
#[path = "tests/review_prompts_tests.rs"]
mod tests;
