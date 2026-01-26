use super::*;
use crate::planning_paths::{set_home_for_test, TestHomeGuard};
use tempfile::tempdir;

/// Helper to set up an isolated test home directory.
fn test_env() -> (tempfile::TempDir, TestHomeGuard) {
    let dir = tempdir().expect("Failed to create temp dir");
    let guard = set_home_for_test(dir.path().to_path_buf());
    (dir, guard)
}

#[test]
fn test_format_commit_date() {
    // Date + time format
    assert_eq!(format_commit_date("2024-12-20T10:30:00Z"), "Dec 20 10:30");
    assert_eq!(format_commit_date("2024-01-05T00:00:00Z"), "Jan 5 00:00");
    assert_eq!(format_commit_date("2024-06-15T23:59:59Z"), "Jun 15 23:59");
    // With timezone offset instead of Z
    assert_eq!(
        format_commit_date("2024-03-10T14:25:00+00:00"),
        "Mar 10 14:25"
    );
    // Date only (no time part) falls back to date-only format
    assert_eq!(format_commit_date("2024-07-04"), "Jul 4");
    // Invalid formats
    assert_eq!(format_commit_date("invalid"), "invalid");
    assert_eq!(format_commit_date(""), "");
}

#[test]
fn test_build_sha_is_set() {
    assert!(!BUILD_SHA.is_empty());
}

#[test]
fn test_update_status_default() {
    let status = UpdateStatus::default();
    match status {
        UpdateStatus::Checking | UpdateStatus::VersionUnknown => {}
        _ => panic!("Unexpected default status"),
    }
}

#[test]
fn test_write_and_consume_update_marker() {
    // This test uses home-based storage (~/.planning-agent/update-installed)
    let (_temp_dir, _guard) = test_env();

    // First consume to ensure we start clean
    let _ = consume_update_marker();

    // Should be false when no marker exists
    assert!(!consume_update_marker());

    // Write the marker
    write_update_marker().unwrap();

    // Consume should return true and remove the marker
    assert!(consume_update_marker());

    // Second consume should return false (marker was removed)
    assert!(!consume_update_marker());
}

#[test]
fn test_build_update_args_structure() {
    let (args, features_msg) = build_update_args();

    // Core args are always present
    assert!(args.contains(&"install"));
    assert!(args.contains(&"--git"));
    assert!(args.contains(&"https://github.com/metjm/planning-agent.git"));
    assert!(args.contains(&"--force"));

    // Feature handling depends on BUILD_FEATURES
    if !BUILD_FEATURES.is_empty() {
        assert!(
            args.contains(&"--features"),
            "Args should contain --features when BUILD_FEATURES is non-empty"
        );
        assert!(
            args.contains(&BUILD_FEATURES),
            "Args should contain the BUILD_FEATURES value"
        );
        assert!(
            features_msg.contains(BUILD_FEATURES),
            "Features message should mention the features"
        );
    } else {
        assert!(
            !args.contains(&"--features"),
            "Args should not contain --features when BUILD_FEATURES is empty"
        );
        assert!(
            features_msg.is_empty(),
            "Features message should be empty when no features"
        );
    }
}

#[test]
fn test_build_features_format() {
    // BUILD_FEATURES should be empty or contain valid feature names
    if !BUILD_FEATURES.is_empty() {
        for feature in BUILD_FEATURES.split(',') {
            assert!(
                feature == "host-gui" || feature == "host-gui-tray",
                "Unknown feature in BUILD_FEATURES: '{}'. \
                 If a new feature was added, update this test.",
                feature
            );
        }
    }
}

#[test]
fn test_features_message_format() {
    let (_, features_msg) = build_update_args();

    if !BUILD_FEATURES.is_empty() {
        assert!(
            features_msg.starts_with(" with features: "),
            "Features message should start with ' with features: ', got: '{}'",
            features_msg
        );
    } else {
        assert!(
            features_msg.is_empty(),
            "Features message should be empty when no features"
        );
    }
}
