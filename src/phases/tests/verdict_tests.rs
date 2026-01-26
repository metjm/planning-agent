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
        VerificationVerdictResult::ParseFailure { .. }
    ));
}

#[test]
fn test_extract_implementation_feedback() {
    let report = r#"
## Verdict
NEEDS REVISION

<implementation-feedback>
The config loading function needs error handling.
</implementation-feedback>
"#;
    let feedback = extract_implementation_feedback(report).unwrap();
    assert!(feedback.contains("config loading"));
    assert!(feedback.contains("error handling"));
}

#[test]
fn test_extract_feedback_tag_custom() {
    let report = r#"
<custom-tag>
Some custom content here.
</custom-tag>
"#;
    let feedback = extract_feedback_tag("custom-tag", report).unwrap();
    assert!(feedback.contains("custom content"));
}

#[test]
fn test_verdict_result_serialization() {
    let approved = VerificationVerdictResult::Approved;
    let json = serde_json::to_string(&approved).unwrap();
    assert!(json.contains("approved"));

    let parsed: VerificationVerdictResult = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, approved);

    let needs_revision = VerificationVerdictResult::NeedsRevision;
    let json = serde_json::to_string(&needs_revision).unwrap();
    assert!(json.contains("needs_revision"));

    let parse_failure = VerificationVerdictResult::ParseFailure {
        reason: "test error".to_string(),
    };
    let json = serde_json::to_string(&parse_failure).unwrap();
    assert!(json.contains("parse_failure"));
    assert!(json.contains("test error"));
}
