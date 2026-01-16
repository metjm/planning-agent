use crate::agents::{AgentContext, AgentType};
use crate::config::WorkflowConfig;
use crate::prompt_format::PromptBuilder;
use crate::session_logger::SessionLogger;
use crate::state::ResumeStrategy;
use crate::tui::SessionEventSender;
use crate::verification_state::VerificationState;
use anyhow::Result;
use regex::Regex;
use std::fs;
use std::sync::Arc;

const VERIFICATION_SYSTEM_PROMPT: &str = r#"You are a verification agent that compares an implementation against its approved plan.

Your task is to:
1. Read the implementation plan from the plan file
2. Inspect the repository state (file contents, structure, changes)
3. Compare each requirement in the plan against the actual implementation
4. Generate a structured verification report

Be thorough but fair - minor differences in implementation approach are acceptable if they achieve the plan's goals.
Focus on functional correctness and completeness.
IMPORTANT: Use absolute paths for all file references in your verification report."#;

/// Result of parsing a verification verdict from the report
#[derive(Debug, Clone, PartialEq)]
pub enum VerificationVerdictResult {
    Approved,
    NeedsRevision,
    ParseFailure(String),
}

/// Parses the verification verdict from a verification report.
/// Looks for "Verdict: APPROVED" or "Verdict: NEEDS REVISION" patterns.
pub fn parse_verification_verdict(report: &str) -> VerificationVerdictResult {
    let re = Regex::new(r"(?i)(?:##\s*)?Verdict[:\*\s]*\**\s*(APPROVED|NEEDS\s*_?\s*REVISION)")
        .unwrap();

    if let Some(captures) = re.captures(report) {
        if let Some(verdict_match) = captures.get(1) {
            let verdict = verdict_match.as_str().to_uppercase();
            let normalized = verdict.replace('_', " ").replace("  ", " ");

            if normalized == "APPROVED" {
                return VerificationVerdictResult::Approved;
            } else if normalized.contains("NEEDS") && normalized.contains("REVISION") {
                return VerificationVerdictResult::NeedsRevision;
            }
        }
    }

    VerificationVerdictResult::ParseFailure("No valid Verdict found in verification report".to_string())
}

/// Extracts feedback content from <verification-feedback> tags.
pub fn extract_verification_feedback(report: &str) -> Option<String> {
    let re = Regex::new(r"(?s)<verification-feedback>\s*(.*?)\s*</verification-feedback>").unwrap();
    if let Some(captures) = re.captures(report) {
        if let Some(content) = captures.get(1) {
            return Some(content.as_str().to_string());
        }
    }
    None
}

/// Runs the verification phase, comparing implementation against plan.
pub async fn run_verification_phase(
    verification_state: &mut VerificationState,
    config: &WorkflowConfig,
    session_sender: SessionEventSender,
    session_logger: Arc<SessionLogger>,
) -> Result<String> {
    let verification_config = config
        .verification
        .verifying
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No verifying agent configured"))?;

    let agent_name = &verification_config.agent;
    let max_turns = verification_config.max_turns;

    let agent_config = config
        .get_agent(agent_name)
        .ok_or_else(|| anyhow::anyhow!("Verifying agent '{}' not found in config", agent_name))?;

    session_sender.send_output(format!(
        "[verification] Starting verification round {} using agent: {}",
        verification_state.iteration, agent_name
    ));

    let agent = AgentType::from_config(
        agent_name,
        agent_config,
        verification_state.working_dir.clone(),
    )?;

    let prompt = build_verification_prompt(verification_state);
    let report_path = verification_state.verification_report_path();

    let phase_name = format!("Verifying #{}", verification_state.iteration);

    let context = AgentContext {
        session_sender: session_sender.clone(),
        phase: phase_name,
        conversation_id: None, // Verification is stateless per round
        resume_strategy: ResumeStrategy::Stateless,
        session_logger,
    };

    let result = agent
        .execute_streaming_with_context(
            prompt,
            Some(VERIFICATION_SYSTEM_PROMPT.to_string()),
            max_turns,
            context,
        )
        .await?;

    // Extract report from output or read from file if agent wrote it there
    let mut report = result.output.clone();

    // If output is empty or doesn't contain the verdict, try reading from report file
    if (report.trim().is_empty() || !report.contains("Verdict")) && report_path.exists() {
        if let Ok(file_content) = fs::read_to_string(&report_path) {
            if !file_content.trim().is_empty() {
                report = file_content;
                session_sender.send_output(format!(
                    "[verification] Loaded report from {}",
                    report_path.display()
                ));
            }
        }
    }

    // Save report to file
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&report_path, &report)?;

    session_sender.send_output(format!(
        "[verification] Report saved to {}",
        report_path.display()
    ));

    // Parse verdict and update state
    let verdict = parse_verification_verdict(&report);
    match verdict {
        VerificationVerdictResult::Approved => {
            session_sender.send_output("[verification] Verdict: APPROVED".to_string());
            verification_state.last_verdict = Some("APPROVED".to_string());
        }
        VerificationVerdictResult::NeedsRevision => {
            session_sender.send_output("[verification] Verdict: NEEDS REVISION".to_string());
            verification_state.last_verdict = Some("NEEDS_REVISION".to_string());
        }
        VerificationVerdictResult::ParseFailure(ref err) => {
            session_sender.send_output(format!(
                "[verification] WARNING: Could not parse verdict: {}",
                err
            ));
            // Treat parse failure as needs revision to be safe
            verification_state.last_verdict = Some("NEEDS_REVISION".to_string());
        }
    }

    Ok(report)
}

fn build_verification_prompt(state: &VerificationState) -> String {
    let plan_path = state.plan_file_path();
    let report_path = state.verification_report_path();

    let output_format = format!(
        r###"Your report MUST follow this structure:

```markdown
# Verification Report - Round {}

## Plan Summary
[Brief summary of what the plan intended to implement]

## Verification Checklist
- [x] Feature/step that was implemented correctly
- [ ] Feature/step that is missing or incorrect
...

## Discrepancies Found
1. **Issue**: [Description]
   **Location**: [absolute/path/to/file:line]
   **Expected**: [What the plan specified]
   **Actual**: [What was implemented or missing]

## Verdict
APPROVED (if implementation matches plan)
NEEDS REVISION (if there are issues to fix)

<verification-feedback>
[Detailed feedback for the fixer agent if NEEDS REVISION.
Include specific instructions on what needs to be fixed, using absolute paths.]
</verification-feedback>
```

CRITICAL: Your report MUST include "## Verdict" followed by either "APPROVED" or "NEEDS REVISION".
If there are issues, wrap detailed fix instructions in <verification-feedback> tags."###,
        state.iteration
    );

    PromptBuilder::new()
        .phase("verification")
        .instructions(r#"Verify the implementation against the approved plan.

1. Read the plan file to understand what was supposed to be implemented
2. Explore the repository to see what was actually implemented
3. Compare each requirement/step in the plan against the implementation
4. Note any discrepancies, missing features, or deviations"#)
        .input("workspace-root", &state.working_dir.display().to_string())
        .input("plan-path", &plan_path.display().to_string())
        .input("repository-path", &state.working_dir.display().to_string())
        .input("report-output-path", &report_path.display().to_string())
        .input("iteration", &state.iteration.to_string())
        .constraint(&format!("Use absolute paths for all file references in your report (e.g., {}/src/main.rs:45)", state.working_dir.display()))
        .output_format(&output_format)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_verdict_approved() {
        let report = "## Verdict\nAPPROVED\n\nAll requirements met.";
        assert_eq!(
            parse_verification_verdict(report),
            VerificationVerdictResult::Approved
        );
    }

    #[test]
    fn test_parse_verdict_approved_with_colon() {
        let report = "## Verdict: APPROVED";
        assert_eq!(
            parse_verification_verdict(report),
            VerificationVerdictResult::Approved
        );
    }

    #[test]
    fn test_parse_verdict_needs_revision() {
        let report = "## Verdict\nNEEDS REVISION\n\nSome issues found.";
        assert_eq!(
            parse_verification_verdict(report),
            VerificationVerdictResult::NeedsRevision
        );
    }

    #[test]
    fn test_parse_verdict_needs_revision_underscore() {
        let report = "## Verdict: NEEDS_REVISION";
        assert_eq!(
            parse_verification_verdict(report),
            VerificationVerdictResult::NeedsRevision
        );
    }

    #[test]
    fn test_parse_verdict_case_insensitive() {
        let report = "Verdict: approved";
        assert_eq!(
            parse_verification_verdict(report),
            VerificationVerdictResult::Approved
        );
    }

    #[test]
    fn test_parse_verdict_missing() {
        let report = "This report has no verdict section.";
        assert!(matches!(
            parse_verification_verdict(report),
            VerificationVerdictResult::ParseFailure(_)
        ));
    }

    #[test]
    fn test_extract_verification_feedback() {
        let report = r#"
## Verdict
NEEDS REVISION

<verification-feedback>
1. Fix the authentication logic in src/auth.rs
2. Add missing test cases
</verification-feedback>
"#;
        let feedback = extract_verification_feedback(report).unwrap();
        assert!(feedback.contains("authentication logic"));
        assert!(feedback.contains("test cases"));
    }

    #[test]
    fn test_extract_verification_feedback_missing() {
        let report = "## Verdict: APPROVED\nAll good!";
        assert!(extract_verification_feedback(report).is_none());
    }
}
