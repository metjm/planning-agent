use super::*;
use tempfile::tempdir;

#[test]
fn test_truncate_for_recovery_prompt_short() {
    let short = "This is a short output";
    let result = truncate_for_recovery_prompt(short);
    assert_eq!(result, short);
}

#[test]
fn test_truncate_for_recovery_prompt_long() {
    let long = "x".repeat(MAX_RECOVERY_PROMPT_OUTPUT_SIZE + 1000);
    let result = truncate_for_recovery_prompt(&long);
    assert!(result.len() < long.len());
    assert!(result.contains("OUTPUT TRUNCATED"));
    assert!(result.contains(&format!("{}", MAX_RECOVERY_PROMPT_OUTPUT_SIZE)));
}

#[test]
fn test_create_bundle_basic() {
    let dir = tempdir().unwrap();
    let working_dir = dir.path();

    let config = BundleConfig {
        working_dir,
        agent_name: "test-agent",
        failure_reason: "Test failure",
        server_name: "file-review-test",
        run_id: "20260112-120000",
        plan_feedback_found: false,
        verdict_found: false,
        attempt_timestamps: vec![AttemptTimestamp {
            attempt: 1,
            started_at: "2026-01-12T12:00:00Z".to_string(),
            ended_at: "2026-01-12T12:01:00Z".to_string(),
        }],
        initial_output: Some("Test output from attempt 1"),
        retry_output: None,
        state_path: None,
        plan_file: None,
        feedback_file: None,
        workflow_session_id: None,
    };

    let result = create_review_bundle(config);
    assert!(result.is_some());

    let bundle_path = result.unwrap();
    assert!(bundle_path.exists());
    assert!(bundle_path.to_string_lossy().contains("review-test-agent"));
    assert!(bundle_path.extension().map(|e| e == "zip").unwrap_or(false));
}

#[test]
fn test_create_bundle_with_retry() {
    let dir = tempdir().unwrap();
    let working_dir = dir.path();

    let config = BundleConfig {
        working_dir,
        agent_name: "retry-agent",
        failure_reason: "Both attempts failed",
        server_name: "file-review-test",
        run_id: "20260112-120000",
        plan_feedback_found: true,
        verdict_found: false,
        attempt_timestamps: vec![
            AttemptTimestamp {
                attempt: 1,
                started_at: "2026-01-12T12:00:00Z".to_string(),
                ended_at: "2026-01-12T12:01:00Z".to_string(),
            },
            AttemptTimestamp {
                attempt: 2,
                started_at: "2026-01-12T12:01:30Z".to_string(),
                ended_at: "2026-01-12T12:02:30Z".to_string(),
            },
        ],
        initial_output: Some("Output from attempt 1"),
        retry_output: Some("Output from attempt 2"),
        state_path: None,
        plan_file: None,
        feedback_file: None,
        workflow_session_id: None,
    };

    let result = create_review_bundle(config);
    assert!(result.is_some());

    // Verify bundle contains both outputs by reading and checking manifest
    let bundle_path = result.unwrap();
    let file = File::open(&bundle_path).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();

    // Check manifest
    let mut manifest_file = archive.by_name("manifest.json").unwrap();
    let mut manifest_content = String::new();
    manifest_file.read_to_string(&mut manifest_content).unwrap();
    let manifest: DiagnosticsManifest = serde_json::from_str(&manifest_content).unwrap();

    assert_eq!(manifest.attempt_count, 2);
    assert!(manifest.plan_feedback_found);
    assert!(!manifest.verdict_found);
    assert_eq!(manifest.attempt_timestamps.len(), 2);
}

#[test]
fn test_add_content_truncation() {
    let dir = tempdir().unwrap();
    let bundle_path = dir.path().join("test.zip");
    let file = File::create(&bundle_path).unwrap();
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default();

    // Create content larger than MAX_FILE_SIZE
    let large_content = "x".repeat((MAX_FILE_SIZE + 1000) as usize);
    let info = add_content_to_zip(&mut zip, "large.txt", &large_content, options).unwrap();

    assert!(info.truncated);
    assert_eq!(info.original_size, large_content.len() as u64);
    assert!(info.included_size < info.original_size);
}

#[test]
fn test_parse_failure_info() {
    let info = ParseFailureInfo {
        error: "No valid Overall Assessment found".to_string(),
        plan_feedback_found: true,
        verdict_found: false,
    };

    assert!(info.plan_feedback_found);
    assert!(!info.verdict_found);
    assert!(info.error.contains("Overall Assessment"));
}

#[test]
fn test_bundle_path_format() {
    let dir = tempdir().unwrap();
    let result =
        planning_paths::review_bundle_path(dir.path(), "claude", "20260112-120000", "abcd1234");

    assert!(result.is_ok());
    let path = result.unwrap();
    assert!(path.to_string_lossy().contains("diagnostics"));
    assert!(path
        .to_string_lossy()
        .contains("review-claude-20260112-120000-abcd1234.zip"));
}
