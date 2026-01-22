//! Tests for planning_paths module.

use super::*;
use std::env;
use std::path::Path;
use tempfile::tempdir;

#[test]
fn test_working_dir_hash_consistency() {
    let dir = tempdir().unwrap();
    let path = dir.path();

    let hash1 = working_dir_hash(path);
    let hash2 = working_dir_hash(path);

    assert_eq!(hash1, hash2, "Hash should be consistent across calls");
    assert_eq!(hash1.len(), 12, "Hash should be 12 hex characters");
}

#[test]
fn test_working_dir_hash_different_paths() {
    let dir1 = tempdir().unwrap();
    let dir2 = tempdir().unwrap();

    let hash1 = working_dir_hash(dir1.path());
    let hash2 = working_dir_hash(dir2.path());

    assert_ne!(
        hash1, hash2,
        "Different paths should produce different hashes"
    );
}

#[test]
fn test_hex_encode() {
    assert_eq!(hex_encode(&[0x00, 0xff, 0x10]), "00ff10");
    assert_eq!(hex_encode(&[0xab, 0xcd, 0xef]), "abcdef");
}

#[test]
fn test_planning_agent_home_dir() {
    if env::var("HOME").is_err() {
        return;
    }

    let result = planning_agent_home_dir();
    assert!(result.is_ok());
    let path = result.unwrap();
    assert!(path.ends_with(".planning-agent"));
}

#[test]
fn test_sessions_dir() {
    if env::var("HOME").is_err() {
        return;
    }

    let result = sessions_dir();
    assert!(result.is_ok());
    let path = result.unwrap();
    assert!(path.ends_with("sessions"));
}

#[test]
fn test_state_path() {
    if env::var("HOME").is_err() {
        return;
    }

    let dir = tempdir().unwrap();
    let result = state_path(dir.path(), "my-feature");
    assert!(result.is_ok());
    let path = result.unwrap();
    assert!(path.ends_with("my-feature.json"));
    assert!(path.to_string_lossy().contains("state"));
}

#[test]
fn test_debug_log_path() {
    if env::var("HOME").is_err() {
        return;
    }

    let result = debug_log_path();
    assert!(result.is_ok());
    let path = result.unwrap();
    assert!(path.ends_with("debug.log"));
    assert!(path.to_string_lossy().contains("logs"));
}

#[test]
fn test_update_marker_path() {
    if env::var("HOME").is_err() {
        return;
    }

    let result = update_marker_path();
    assert!(result.is_ok());
    let path = result.unwrap();
    assert!(path.ends_with("update-installed"));
}

#[test]
fn test_session_dir() {
    if env::var("HOME").is_err() {
        return;
    }

    let session_id = format!("test-session-{}", uuid::Uuid::new_v4());
    let result = session_dir(&session_id);
    assert!(result.is_ok());
    let path = result.unwrap();
    assert!(path.ends_with(&session_id));
    assert!(path.to_string_lossy().contains("sessions"));
    assert!(path.exists());
}

#[test]
fn test_session_state_path() {
    if env::var("HOME").is_err() {
        return;
    }

    let session_id = format!("test-session-{}", uuid::Uuid::new_v4());
    let result = session_state_path(&session_id);
    assert!(result.is_ok());
    let path = result.unwrap();
    assert!(path.ends_with("state.json"));
    assert!(path.to_string_lossy().contains(&session_id));
}

#[test]
fn test_session_plan_path() {
    if env::var("HOME").is_err() {
        return;
    }

    let session_id = format!("test-session-{}", uuid::Uuid::new_v4());
    let result = session_plan_path(&session_id);
    assert!(result.is_ok());
    let path = result.unwrap();
    assert!(path.ends_with("plan.md"));
    assert!(path.to_string_lossy().contains(&session_id));
}

#[test]
fn test_session_feedback_path() {
    if env::var("HOME").is_err() {
        return;
    }

    let session_id = format!("test-session-{}", uuid::Uuid::new_v4());
    let result = session_feedback_path(&session_id, 1);
    assert!(result.is_ok());
    let path = result.unwrap();
    assert!(path.ends_with("feedback_1.md"));

    let result2 = session_feedback_path(&session_id, 3);
    assert!(result2.is_ok());
    let path2 = result2.unwrap();
    assert!(path2.ends_with("feedback_3.md"));
}

#[test]
fn test_session_snapshot_path() {
    if env::var("HOME").is_err() {
        return;
    }

    let session_id = format!("test-session-{}", uuid::Uuid::new_v4());
    let result = session_snapshot_path(&session_id);
    assert!(result.is_ok());
    let path = result.unwrap();
    assert!(path.ends_with("session.json"));
    assert!(path.to_string_lossy().contains(&session_id));
}

#[test]
fn test_session_logs_dir() {
    if env::var("HOME").is_err() {
        return;
    }

    let session_id = format!("test-session-{}", uuid::Uuid::new_v4());
    let result = session_logs_dir(&session_id);
    assert!(result.is_ok());
    let path = result.unwrap();
    assert!(path.ends_with("logs"));
    assert!(path.to_string_lossy().contains(&session_id));
    assert!(path.exists());
}

#[test]
fn test_session_info_path() {
    if env::var("HOME").is_err() {
        return;
    }

    let session_id = format!("test-session-{}", uuid::Uuid::new_v4());
    let result = session_info_path(&session_id);
    assert!(result.is_ok());
    let path = result.unwrap();
    assert!(path.ends_with("session_info.json"));
}

#[test]
fn test_session_info_save_and_load() {
    if env::var("HOME").is_err() {
        return;
    }

    let session_id = format!("test-session-{}", uuid::Uuid::new_v4());
    let info = SessionInfo::new(
        &session_id,
        "test-feature",
        "Test objective",
        Path::new("/tmp/test"),
        "Planning",
        1,
    );

    let save_result = info.save(&session_id);
    assert!(save_result.is_ok());

    let load_result = SessionInfo::load(&session_id);
    assert!(load_result.is_ok());
    let loaded = load_result.unwrap();

    assert_eq!(loaded.session_id, session_id);
    assert_eq!(loaded.feature_name, "test-feature");
    assert_eq!(loaded.objective, "Test objective");
    assert_eq!(loaded.phase, "Planning");
    assert_eq!(loaded.iteration, 1);
}

#[test]
fn test_convert_rfc3339_to_timestamp() {
    let result = convert_rfc3339_to_timestamp("2026-01-15T14:30:00.123Z");
    assert!(result.is_some());
    assert_eq!(result.unwrap(), "20260115-143000");

    let result_with_tz = convert_rfc3339_to_timestamp("2026-01-15T14:30:00+00:00");
    assert!(result_with_tz.is_some());
    assert_eq!(result_with_tz.unwrap(), "20260115-143000");

    let invalid = convert_rfc3339_to_timestamp("not-a-timestamp");
    assert!(invalid.is_none());
}

#[test]
fn test_session_implementation_log_path() {
    if env::var("HOME").is_err() {
        return;
    }

    let session_id = format!("test-session-{}", uuid::Uuid::new_v4());
    let result = session_implementation_log_path(&session_id, 1);
    assert!(result.is_ok());
    let path = result.unwrap();
    assert!(path.ends_with("implementation_1.log"));
    assert!(path.to_string_lossy().contains(&session_id));

    let result2 = session_implementation_log_path(&session_id, 3);
    assert!(result2.is_ok());
    let path2 = result2.unwrap();
    assert!(path2.ends_with("implementation_3.log"));
}

#[test]
fn test_session_implementation_review_path() {
    if env::var("HOME").is_err() {
        return;
    }

    let session_id = format!("test-session-{}", uuid::Uuid::new_v4());
    let result = session_implementation_review_path(&session_id, 1);
    assert!(result.is_ok());
    let path = result.unwrap();
    assert!(path.ends_with("implementation_review_1.md"));
    assert!(path.to_string_lossy().contains(&session_id));

    let result2 = session_implementation_review_path(&session_id, 2);
    assert!(result2.is_ok());
    let path2 = result2.unwrap();
    assert!(path2.ends_with("implementation_review_2.md"));
}
