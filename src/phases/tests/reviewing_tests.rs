//! Integration tests for AgentRef / multi-instance support in reviewing phase

use super::*;

// Aggregate reviews tests (moved here to keep reviewing.rs under line limit)

#[test]
fn test_aggregate_any_rejects_none() {
    let reviews = vec![
        ReviewResult {
            agent_name: "claude".to_string(),
            needs_revision: false,
            feedback: "APPROVED".to_string(),
            summary: "Plan looks good".to_string(),
        },
        ReviewResult {
            agent_name: "codex".to_string(),
            needs_revision: false,
            feedback: "APPROVED".to_string(),
            summary: "No issues found".to_string(),
        },
    ];
    assert_eq!(
        aggregate_reviews(&reviews, &AggregationMode::AnyRejects),
        FeedbackStatus::Approved
    );
}

#[test]
fn test_aggregate_any_rejects_one() {
    let reviews = vec![
        ReviewResult {
            agent_name: "claude".to_string(),
            needs_revision: false,
            feedback: "APPROVED".to_string(),
            summary: "Plan looks good".to_string(),
        },
        ReviewResult {
            agent_name: "codex".to_string(),
            needs_revision: true,
            feedback: "NEEDS REVISION".to_string(),
            summary: "Missing error handling".to_string(),
        },
    ];
    assert_eq!(
        aggregate_reviews(&reviews, &AggregationMode::AnyRejects),
        FeedbackStatus::NeedsRevision
    );
}

#[test]
fn test_aggregate_all_reject_partial() {
    let reviews = vec![
        ReviewResult {
            agent_name: "claude".to_string(),
            needs_revision: false,
            feedback: "APPROVED".to_string(),
            summary: "Plan looks good".to_string(),
        },
        ReviewResult {
            agent_name: "codex".to_string(),
            needs_revision: true,
            feedback: "NEEDS REVISION".to_string(),
            summary: "Missing error handling".to_string(),
        },
    ];
    assert_eq!(
        aggregate_reviews(&reviews, &AggregationMode::AllReject),
        FeedbackStatus::Approved
    );
}

#[test]
fn test_aggregate_all_reject_full() {
    let reviews = vec![
        ReviewResult {
            agent_name: "claude".to_string(),
            needs_revision: true,
            feedback: "NEEDS REVISION".to_string(),
            summary: "Architecture concerns".to_string(),
        },
        ReviewResult {
            agent_name: "codex".to_string(),
            needs_revision: true,
            feedback: "NEEDS REVISION".to_string(),
            summary: "Missing error handling".to_string(),
        },
    ];
    assert_eq!(
        aggregate_reviews(&reviews, &AggregationMode::AllReject),
        FeedbackStatus::NeedsRevision
    );
}

#[test]
fn test_aggregate_majority_one_of_three() {
    let reviews = vec![
        ReviewResult {
            agent_name: "claude".to_string(),
            needs_revision: false,
            feedback: "APPROVED".to_string(),
            summary: "Plan looks good".to_string(),
        },
        ReviewResult {
            agent_name: "codex".to_string(),
            needs_revision: false,
            feedback: "APPROVED".to_string(),
            summary: "No issues found".to_string(),
        },
        ReviewResult {
            agent_name: "gemini".to_string(),
            needs_revision: true,
            feedback: "NEEDS REVISION".to_string(),
            summary: "Minor issues found".to_string(),
        },
    ];

    assert_eq!(
        aggregate_reviews(&reviews, &AggregationMode::Majority),
        FeedbackStatus::Approved
    );
}

#[test]
fn test_aggregate_majority_two_of_three() {
    let reviews = vec![
        ReviewResult {
            agent_name: "claude".to_string(),
            needs_revision: true,
            feedback: "NEEDS REVISION".to_string(),
            summary: "Architecture concerns".to_string(),
        },
        ReviewResult {
            agent_name: "codex".to_string(),
            needs_revision: true,
            feedback: "NEEDS REVISION".to_string(),
            summary: "Missing error handling".to_string(),
        },
        ReviewResult {
            agent_name: "gemini".to_string(),
            needs_revision: false,
            feedback: "APPROVED".to_string(),
            summary: "Plan looks good".to_string(),
        },
    ];

    assert_eq!(
        aggregate_reviews(&reviews, &AggregationMode::Majority),
        FeedbackStatus::NeedsRevision
    );
}

#[test]
fn test_aggregate_empty_reviews() {
    let reviews: Vec<ReviewResult> = vec![];
    assert_eq!(
        aggregate_reviews(&reviews, &AggregationMode::AnyRejects),
        FeedbackStatus::NeedsRevision
    );
}

// Multi-instance / AgentRef specific tests

#[test]
fn test_feedback_path_for_agent_with_display_id() {
    use std::path::Path;

    // Test with display_id style names (hyphenated instance IDs)
    let base = Path::new("/tmp/feedback.md");

    // Single reviewer uses base path
    let path = super::feedback_path_for_agent(base, "claude-security", 1);
    assert_eq!(path.to_str().unwrap(), "/tmp/feedback.md");

    // Multiple reviewers get agent-specific paths
    let path = super::feedback_path_for_agent(base, "claude-security", 2);
    assert_eq!(path.to_str().unwrap(), "/tmp/feedback_claude-security.md");

    let path = super::feedback_path_for_agent(base, "claude-architecture", 2);
    assert_eq!(
        path.to_str().unwrap(),
        "/tmp/feedback_claude-architecture.md"
    );
}

#[test]
fn test_write_feedback_files_with_display_ids() {
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let base_path = dir.path().join("feedback.md");

    // Create reviews with display_id style names (simulating multi-instance)
    let reviews = vec![
        ReviewResult {
            agent_name: "claude-security".to_string(),
            needs_revision: true,
            feedback: "Security concerns found".to_string(),
            summary: "Security review".to_string(),
        },
        ReviewResult {
            agent_name: "claude-architecture".to_string(),
            needs_revision: false,
            feedback: "Architecture looks good".to_string(),
            summary: "Architecture review".to_string(),
        },
    ];

    let paths = super::write_feedback_files(&reviews, &base_path).unwrap();

    assert_eq!(paths.len(), 2);
    assert!(paths[0].to_str().unwrap().contains("claude-security"));
    assert!(paths[1].to_str().unwrap().contains("claude-architecture"));

    // Verify file contents
    let content1 = std::fs::read_to_string(&paths[0]).unwrap();
    assert_eq!(content1, "Security concerns found");

    let content2 = std::fs::read_to_string(&paths[1]).unwrap();
    assert_eq!(content2, "Architecture looks good");
}

#[test]
fn test_merge_feedback_with_display_ids() {
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let output_path = dir.path().join("merged.md");

    // Create reviews with display_id style names
    let reviews = vec![
        ReviewResult {
            agent_name: "claude-security".to_string(),
            needs_revision: true,
            feedback: "Found SQL injection vulnerability".to_string(),
            summary: "Security issues".to_string(),
        },
        ReviewResult {
            agent_name: "claude-architecture".to_string(),
            needs_revision: false,
            feedback: "Clean separation of concerns".to_string(),
            summary: "Good architecture".to_string(),
        },
    ];

    super::merge_feedback(&reviews, &output_path).unwrap();

    let content = std::fs::read_to_string(&output_path).unwrap();

    // Verify header includes display_ids
    assert!(content.contains("2 reviewer(s): claude-security, claude-architecture"));

    // Verify each review section uses display_id
    assert!(content.contains("## CLAUDE-SECURITY Review"));
    assert!(content.contains("## CLAUDE-ARCHITECTURE Review"));

    // Verify feedback content is present
    assert!(content.contains("Found SQL injection vulnerability"));
    assert!(content.contains("Clean separation of concerns"));
}

#[test]
fn test_reviews_with_same_base_agent_different_display_ids() {
    // Test that multiple instances of same agent produce distinct results
    let reviews = vec![
        ReviewResult {
            agent_name: "claude-security".to_string(),
            needs_revision: true,
            feedback: "NEEDS REVISION: security issues".to_string(),
            summary: "Security concerns".to_string(),
        },
        ReviewResult {
            agent_name: "claude-architecture".to_string(),
            needs_revision: true,
            feedback: "NEEDS REVISION: architecture issues".to_string(),
            summary: "Architecture concerns".to_string(),
        },
        ReviewResult {
            agent_name: "codex".to_string(),
            needs_revision: false,
            feedback: "APPROVED".to_string(),
            summary: "Looks good".to_string(),
        },
    ];

    // With any_rejects, even one rejection means needs revision
    assert_eq!(
        aggregate_reviews(&reviews, &AggregationMode::AnyRejects),
        FeedbackStatus::NeedsRevision
    );

    // With majority (2/3 reject), needs revision
    assert_eq!(
        aggregate_reviews(&reviews, &AggregationMode::Majority),
        FeedbackStatus::NeedsRevision
    );

    // Verify each review is counted separately
    assert_eq!(reviews.len(), 3);
    assert_eq!(reviews.iter().filter(|r| r.needs_revision).count(), 2);
}

#[test]
fn test_system_prompt_combination() {
    // Test the system prompt combination logic used in run_multi_agent_review_with_context
    let base_prompt = REVIEW_SYSTEM_PROMPT;
    let custom_prompt = "Focus on security:\n- Check for SQL injection\n- Verify auth";

    // Without custom prompt
    let system_prompt_none: Option<&str> = None;
    let combined = match system_prompt_none {
        Some(custom) => format!("{}\n\n{}", base_prompt, custom),
        None => base_prompt.to_string(),
    };
    assert_eq!(combined, base_prompt);

    // With custom prompt
    let combined_with_custom = format!("{}\n\n{}", base_prompt, custom_prompt);
    assert!(combined_with_custom.starts_with(base_prompt));
    assert!(combined_with_custom.ends_with(custom_prompt));
    assert!(combined_with_custom.contains("\n\n"));
}

#[test]
fn test_five_agent_review_aggregation() {
    // Test aggregation with 5 reviewers - simulating multi-instance scenario
    let reviews = vec![
        // Agent 1: codex (simple) - approves
        ReviewResult {
            agent_name: "codex".to_string(),
            needs_revision: false,
            feedback: "APPROVED: Code looks correct".to_string(),
            summary: "No issues found".to_string(),
        },
        // Agent 2: gemini (simple) - rejects
        ReviewResult {
            agent_name: "gemini".to_string(),
            needs_revision: true,
            feedback: "NEEDS REVISION: Missing tests".to_string(),
            summary: "Test coverage needed".to_string(),
        },
        // Agent 3: claude-security (extended) - rejects
        ReviewResult {
            agent_name: "claude-security".to_string(),
            needs_revision: true,
            feedback: "NEEDS REVISION: SQL injection vulnerability".to_string(),
            summary: "Security issues found".to_string(),
        },
        // Agent 4: claude-architecture (extended) - approves
        ReviewResult {
            agent_name: "claude-architecture".to_string(),
            needs_revision: false,
            feedback: "APPROVED: Good separation of concerns".to_string(),
            summary: "Architecture is solid".to_string(),
        },
        // Agent 5: claude (extended, no custom id) - approves
        ReviewResult {
            agent_name: "claude".to_string(),
            needs_revision: false,
            feedback: "APPROVED: Logic is correct".to_string(),
            summary: "Correctness verified".to_string(),
        },
    ];

    assert_eq!(reviews.len(), 5, "Should have 5 reviews");

    // Count approvals and rejections
    let rejections = reviews.iter().filter(|r| r.needs_revision).count();
    let approvals = reviews.iter().filter(|r| !r.needs_revision).count();
    assert_eq!(rejections, 2, "Should have 2 rejections");
    assert_eq!(approvals, 3, "Should have 3 approvals");

    // Test different aggregation modes with 5 agents

    // AnyRejects: 2 rejections -> needs revision
    assert_eq!(
        aggregate_reviews(&reviews, &AggregationMode::AnyRejects),
        FeedbackStatus::NeedsRevision,
        "AnyRejects should return NeedsRevision with 2 rejections"
    );

    // AllReject: Not all reject (only 2/5) -> approved
    assert_eq!(
        aggregate_reviews(&reviews, &AggregationMode::AllReject),
        FeedbackStatus::Approved,
        "AllReject should return Approved when not all reject"
    );

    // Majority: 3/5 approve -> approved (majority approves)
    assert_eq!(
        aggregate_reviews(&reviews, &AggregationMode::Majority),
        FeedbackStatus::Approved,
        "Majority should return Approved with 3/5 approvals"
    );
}

#[test]
fn test_five_agent_feedback_files() {
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let base_path = dir.path().join("feedback.md");

    // 5 reviews with different display_ids
    let reviews = vec![
        ReviewResult {
            agent_name: "codex".to_string(),
            needs_revision: false,
            feedback: "Codex feedback content".to_string(),
            summary: "Codex summary".to_string(),
        },
        ReviewResult {
            agent_name: "gemini".to_string(),
            needs_revision: true,
            feedback: "Gemini feedback content".to_string(),
            summary: "Gemini summary".to_string(),
        },
        ReviewResult {
            agent_name: "claude-security".to_string(),
            needs_revision: true,
            feedback: "Security review feedback".to_string(),
            summary: "Security summary".to_string(),
        },
        ReviewResult {
            agent_name: "claude-architecture".to_string(),
            needs_revision: false,
            feedback: "Architecture review feedback".to_string(),
            summary: "Architecture summary".to_string(),
        },
        ReviewResult {
            agent_name: "claude".to_string(),
            needs_revision: false,
            feedback: "Correctness review feedback".to_string(),
            summary: "Correctness summary".to_string(),
        },
    ];

    // Write individual feedback files
    let paths = super::write_feedback_files(&reviews, &base_path).unwrap();
    assert_eq!(paths.len(), 5, "Should create 5 feedback files");

    // Verify each file exists and has correct content
    for (i, path) in paths.iter().enumerate() {
        assert!(path.exists(), "Feedback file {} should exist", i);
        let content = std::fs::read_to_string(path).unwrap();
        assert_eq!(
            content, reviews[i].feedback,
            "File {} content should match review feedback",
            i
        );
    }

    // Verify file paths contain the correct display_ids
    assert!(paths[0].to_str().unwrap().contains("codex"));
    assert!(paths[1].to_str().unwrap().contains("gemini"));
    assert!(paths[2].to_str().unwrap().contains("claude-security"));
    assert!(paths[3].to_str().unwrap().contains("claude-architecture"));
    // Note: paths[4] will contain "claude" but since it's a substring of others,
    // we verify it's the last one specifically
    let path4_str = paths[4].to_str().unwrap();
    assert!(
        path4_str.ends_with("feedback_claude.md"),
        "Last file should be feedback_claude.md, got: {}",
        path4_str
    );
}

#[test]
fn test_five_agent_merge_feedback() {
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let output_path = dir.path().join("merged.md");

    // 5 reviews
    let reviews = vec![
        ReviewResult {
            agent_name: "codex".to_string(),
            needs_revision: false,
            feedback: "All tests pass".to_string(),
            summary: "Tests OK".to_string(),
        },
        ReviewResult {
            agent_name: "gemini".to_string(),
            needs_revision: true,
            feedback: "Missing edge case handling".to_string(),
            summary: "Edge cases".to_string(),
        },
        ReviewResult {
            agent_name: "claude-security".to_string(),
            needs_revision: true,
            feedback: "Found XSS vulnerability in input handler".to_string(),
            summary: "XSS found".to_string(),
        },
        ReviewResult {
            agent_name: "claude-architecture".to_string(),
            needs_revision: false,
            feedback: "Good use of dependency injection".to_string(),
            summary: "Good DI".to_string(),
        },
        ReviewResult {
            agent_name: "claude".to_string(),
            needs_revision: false,
            feedback: "Logic verified correct".to_string(),
            summary: "Logic OK".to_string(),
        },
    ];

    super::merge_feedback(&reviews, &output_path).unwrap();

    let content = std::fs::read_to_string(&output_path).unwrap();

    // Verify header
    assert!(
        content.contains("5 reviewer(s)"),
        "Should mention 5 reviewers"
    );
    assert!(
        content.contains("codex, gemini, claude-security, claude-architecture, claude"),
        "Should list all reviewer display_ids"
    );

    // Verify each review section exists
    assert!(content.contains("## CODEX Review"));
    assert!(content.contains("## GEMINI Review"));
    assert!(content.contains("## CLAUDE-SECURITY Review"));
    assert!(content.contains("## CLAUDE-ARCHITECTURE Review"));
    assert!(content.contains("## CLAUDE Review"));

    // Verify feedback content
    assert!(content.contains("All tests pass"));
    assert!(content.contains("Missing edge case handling"));
    assert!(content.contains("Found XSS vulnerability"));
    assert!(content.contains("Good use of dependency injection"));
    assert!(content.contains("Logic verified correct"));
}

#[test]
fn test_five_agent_majority_edge_cases() {
    // Test majority voting edge cases with 5 reviewers

    // Case 1: 3 approve, 2 reject -> Approved (majority)
    let reviews_3_approve = vec![
        ReviewResult {
            agent_name: "a".to_string(),
            needs_revision: false,
            feedback: "".to_string(),
            summary: "".to_string(),
        },
        ReviewResult {
            agent_name: "b".to_string(),
            needs_revision: false,
            feedback: "".to_string(),
            summary: "".to_string(),
        },
        ReviewResult {
            agent_name: "c".to_string(),
            needs_revision: false,
            feedback: "".to_string(),
            summary: "".to_string(),
        },
        ReviewResult {
            agent_name: "d".to_string(),
            needs_revision: true,
            feedback: "".to_string(),
            summary: "".to_string(),
        },
        ReviewResult {
            agent_name: "e".to_string(),
            needs_revision: true,
            feedback: "".to_string(),
            summary: "".to_string(),
        },
    ];
    assert_eq!(
        aggregate_reviews(&reviews_3_approve, &AggregationMode::Majority),
        FeedbackStatus::Approved
    );

    // Case 2: 2 approve, 3 reject -> NeedsRevision (majority rejects)
    let reviews_3_reject = vec![
        ReviewResult {
            agent_name: "a".to_string(),
            needs_revision: false,
            feedback: "".to_string(),
            summary: "".to_string(),
        },
        ReviewResult {
            agent_name: "b".to_string(),
            needs_revision: false,
            feedback: "".to_string(),
            summary: "".to_string(),
        },
        ReviewResult {
            agent_name: "c".to_string(),
            needs_revision: true,
            feedback: "".to_string(),
            summary: "".to_string(),
        },
        ReviewResult {
            agent_name: "d".to_string(),
            needs_revision: true,
            feedback: "".to_string(),
            summary: "".to_string(),
        },
        ReviewResult {
            agent_name: "e".to_string(),
            needs_revision: true,
            feedback: "".to_string(),
            summary: "".to_string(),
        },
    ];
    assert_eq!(
        aggregate_reviews(&reviews_3_reject, &AggregationMode::Majority),
        FeedbackStatus::NeedsRevision
    );

    // Case 3: All 5 approve -> Approved
    let reviews_all_approve: Vec<ReviewResult> = (0..5)
        .map(|i| ReviewResult {
            agent_name: format!("agent{}", i),
            needs_revision: false,
            feedback: "".to_string(),
            summary: "".to_string(),
        })
        .collect();
    assert_eq!(
        aggregate_reviews(&reviews_all_approve, &AggregationMode::Majority),
        FeedbackStatus::Approved
    );

    // Case 4: All 5 reject -> NeedsRevision
    let reviews_all_reject: Vec<ReviewResult> = (0..5)
        .map(|i| ReviewResult {
            agent_name: format!("agent{}", i),
            needs_revision: true,
            feedback: "".to_string(),
            summary: "".to_string(),
        })
        .collect();
    assert_eq!(
        aggregate_reviews(&reviews_all_reject, &AggregationMode::Majority),
        FeedbackStatus::NeedsRevision
    );
}
