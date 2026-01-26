use super::*;

#[test]
fn test_parse_verdict_approved() {
    let feedback = "## Review Summary\nLooks good!\n\n## Overall Assessment:** APPROVED";
    assert_eq!(parse_verdict(feedback), VerdictParseResult::Approved);
}

#[test]
fn test_parse_verdict_needs_revision() {
    let feedback = "## Issues Found\nSome problems.\n\n## Overall Assessment:** NEEDS REVISION";
    assert_eq!(parse_verdict(feedback), VerdictParseResult::NeedsRevision);
}

#[test]
fn test_parse_verdict_needs_revision_underscore() {
    let feedback = "## Overall Assessment:** NEEDS_REVISION";
    assert_eq!(parse_verdict(feedback), VerdictParseResult::NeedsRevision);
}

#[test]
fn test_parse_verdict_major_issues() {
    let feedback = "## Overall Assessment: MAJOR ISSUES\n\nSevere problems found.";
    assert_eq!(parse_verdict(feedback), VerdictParseResult::NeedsRevision);
}

#[test]
fn test_parse_verdict_case_insensitive() {
    let feedback = "overall assessment: approved";
    assert_eq!(parse_verdict(feedback), VerdictParseResult::Approved);
}

#[test]
fn test_parse_verdict_malformed_no_verdict() {
    let feedback = "## Overall Assessment:\nSome text but no verdict keyword.";
    assert!(matches!(
        parse_verdict(feedback),
        VerdictParseResult::ParseFailure(_)
    ));
}

#[test]
fn test_parse_verdict_missing_overall_assessment() {
    let feedback = "This feedback has no overall assessment line at all.\nJust random content.";
    assert!(matches!(
        parse_verdict(feedback),
        VerdictParseResult::ParseFailure(_)
    ));
}

#[test]
fn test_parse_verdict_conflicting_content() {
    let feedback = "The plan is APPROVED in some areas but has issues.\n\n## Overall Assessment:** NEEDS REVISION";
    assert_eq!(parse_verdict(feedback), VerdictParseResult::NeedsRevision);
}

#[test]
fn test_parse_verdict_no_major_issues_in_body() {
    let feedback = "I found no major issues in this plan.\n\n## Overall Assessment:** APPROVED";
    assert_eq!(parse_verdict(feedback), VerdictParseResult::Approved);
}

#[test]
fn test_parse_verdict_with_markdown_formatting() {
    let feedback = "### Overall Assessment: **APPROVED**\n\nReady for implementation.";
    assert_eq!(parse_verdict(feedback), VerdictParseResult::Approved);
}

#[test]
fn test_extract_plan_feedback_with_tags() {
    let output = "Some preamble\n<plan-feedback>\n## Summary\nThis is good\n</plan-feedback>\nEnd";
    let feedback = extract_plan_feedback(output);
    assert!(feedback.contains("## Summary"));
    assert!(feedback.contains("This is good"));
    assert!(!feedback.contains("preamble"));
}

#[test]
fn test_extract_plan_feedback_without_tags() {
    let output = "## Summary\nThis is good\n\nNo tags here";
    let feedback = extract_plan_feedback(output);
    assert_eq!(feedback, output);
}

#[test]
fn test_parse_review_feedback_without_tags_when_not_required() {
    let content = "## Summary\nLooks good!\n\n## Overall Assessment: APPROVED";
    let result = parse_review_feedback(content, false);
    assert!(result.is_ok());
    let review = result.unwrap();
    assert_eq!(review.verdict, ReviewVerdict::Approved);
}

#[test]
fn test_parse_review_feedback_without_tags_when_required() {
    let content = "## Summary\nLooks good!\n\n## Overall Assessment: APPROVED";
    let result = parse_review_feedback(content, true);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.error.contains("tags not found"));
}

#[test]
fn test_parse_review_feedback_with_tags_when_required() {
    let content = "<plan-feedback>\n## Summary\nLooks good!\n\n## Overall Assessment: APPROVED\n</plan-feedback>";
    let result = parse_review_feedback(content, true);
    assert!(result.is_ok());
    let review = result.unwrap();
    assert_eq!(review.verdict, ReviewVerdict::Approved);
}
