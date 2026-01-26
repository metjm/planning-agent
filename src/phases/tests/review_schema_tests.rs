use super::*;

#[test]
fn test_review_verdict_serialization() {
    let approved = ReviewVerdict::Approved;
    let json = serde_json::to_string(&approved).unwrap();
    assert_eq!(json, "\"APPROVED\"");

    let needs_revision = ReviewVerdict::NeedsRevision;
    let json = serde_json::to_string(&needs_revision).unwrap();
    assert_eq!(json, "\"NEEDS_REVISION\"");
}

#[test]
fn test_review_verdict_deserialization() {
    let approved: ReviewVerdict = serde_json::from_str("\"APPROVED\"").unwrap();
    assert_eq!(approved, ReviewVerdict::Approved);

    let needs_revision: ReviewVerdict = serde_json::from_str("\"NEEDS_REVISION\"").unwrap();
    assert_eq!(needs_revision, ReviewVerdict::NeedsRevision);
}

#[test]
fn test_submitted_review_needs_revision() {
    let approved = SubmittedReview {
        verdict: ReviewVerdict::Approved,
        summary: "Looks good".to_string(),
        critical_issues: vec![],
        recommendations: vec![],
        full_feedback: None,
    };
    assert!(!approved.needs_revision());

    let needs_rev = SubmittedReview {
        verdict: ReviewVerdict::NeedsRevision,
        summary: "Issues found".to_string(),
        critical_issues: vec!["Missing error handling".to_string()],
        recommendations: vec![],
        full_feedback: None,
    };
    assert!(needs_rev.needs_revision());
}

#[test]
fn test_submitted_review_feedback_content() {
    let review = SubmittedReview {
        verdict: ReviewVerdict::NeedsRevision,
        summary: "The plan has some issues".to_string(),
        critical_issues: vec!["Issue 1".to_string(), "Issue 2".to_string()],
        recommendations: vec!["Suggestion 1".to_string()],
        full_feedback: None,
    };

    let content = review.feedback_content();
    assert!(content.contains("## Summary"));
    assert!(content.contains("The plan has some issues"));
    assert!(content.contains("## Critical Issues"));
    assert!(content.contains("- Issue 1"));
    assert!(content.contains("## Recommendations"));
    assert!(content.contains("NEEDS REVISION"));
}

#[test]
fn test_submitted_review_full_feedback_takes_precedence() {
    let review = SubmittedReview {
        verdict: ReviewVerdict::Approved,
        summary: "Looks good".to_string(),
        critical_issues: vec![],
        recommendations: vec![],
        full_feedback: Some("# Custom Review\n\nFull custom content here.".to_string()),
    };

    let content = review.feedback_content();
    assert_eq!(content, "# Custom Review\n\nFull custom content here.");
}
