//! Diagnostics bundle creation for review failure recovery.
//!
//! This module provides functionality to collect relevant artifacts when review
//! feedback parsing fails, package them into a ZIP archive, and surface the bundle path
//! to users for debugging purposes.

use crate::planning_paths;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

/// Maximum file size to include in bundle (100KB)
const MAX_FILE_SIZE: u64 = 100 * 1024;

/// Maximum output size to include in recovery prompt (50KB)
pub const MAX_RECOVERY_PROMPT_OUTPUT_SIZE: usize = 50 * 1024;

/// Timestamp information for a single attempt
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttemptTimestamp {
    pub attempt: u8,
    pub started_at: String,
    pub ended_at: String,
}

/// Information about a file included in the bundle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncludedFile {
    pub name: String,
    pub original_size: u64,
    pub included_size: u64,
    pub truncated: bool,
}

/// Manifest for the diagnostics bundle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticsManifest {
    /// ISO 8601 timestamp when bundle was created
    pub created_at: String,
    /// Timestamps for each attempt
    pub attempt_timestamps: Vec<AttemptTimestamp>,
    /// Name of the agent that failed
    pub agent_name: String,
    /// Reason for failure
    pub failure_reason: String,
    /// Server name identifier (for diagnostics purposes)
    pub server_name: String,
    /// Working directory hash
    pub working_dir_hash: String,
    /// Workflow run ID
    pub run_id: String,
    /// Whether <plan-feedback> tags were found in output
    pub plan_feedback_found: bool,
    /// Whether "Overall Assessment" pattern was found
    pub verdict_found: bool,
    /// Number of attempts (1 or 2)
    pub attempt_count: u8,
    /// Files included in the bundle
    pub included_files: Vec<IncludedFile>,
    /// Optional files that were missing
    pub missing_files: Vec<String>,
}

/// Configuration for creating a diagnostics bundle
pub struct BundleConfig<'a> {
    pub working_dir: &'a Path,
    pub agent_name: &'a str,
    pub failure_reason: &'a str,
    pub server_name: &'a str,
    pub run_id: &'a str,
    pub plan_feedback_found: bool,
    pub verdict_found: bool,
    pub attempt_timestamps: Vec<AttemptTimestamp>,
    pub initial_output: Option<&'a str>,
    pub retry_output: Option<&'a str>,
    pub state_path: Option<&'a Path>,
    pub plan_file: Option<&'a Path>,
    pub feedback_file: Option<&'a Path>,
    pub workflow_session_id: Option<&'a str>,
}

/// Creates a diagnostics bundle for a failed review.
///
/// Returns `Some(PathBuf)` with the bundle path on success, or `None` if bundle
/// creation failed (errors are logged but don't abort the review flow).
pub fn create_review_bundle(config: BundleConfig<'_>) -> Option<PathBuf> {
    match create_bundle_internal(config) {
        Ok(path) => Some(path),
        Err(e) => {
            eprintln!("[diagnostics] Failed to create bundle: {}", e);
            None
        }
    }
}

fn create_bundle_internal(config: BundleConfig<'_>) -> Result<PathBuf> {
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let uuid_str = uuid::Uuid::new_v4().to_string();
    // UUID is ASCII hex, so byte indexing is safe
    #[allow(clippy::string_slice)]
    let suffix = &uuid_str[..8];

    let bundle_path = planning_paths::review_bundle_path(
        config.working_dir,
        config.agent_name,
        &timestamp,
        suffix,
    )?;

    let file = File::create(&bundle_path)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut included_files = Vec::new();
    let mut missing_files = Vec::new();

    // Add initial attempt output
    if let Some(output) = config.initial_output {
        let info = add_content_to_zip(&mut zip, "attempt_1_output.txt", output, options)?;
        included_files.push(info);
    }

    // Add retry attempt output
    if let Some(output) = config.retry_output {
        let info = add_content_to_zip(&mut zip, "attempt_2_output.txt", output, options)?;
        included_files.push(info);
    }

    // Add state file
    if let Some(state_path) = config.state_path {
        match add_file_to_zip(&mut zip, state_path, "state.json", options) {
            Ok(info) => included_files.push(info),
            Err(_) => missing_files.push("state.json".to_string()),
        }
    }

    // Add plan file
    if let Some(plan_file) = config.plan_file {
        match add_file_to_zip(&mut zip, plan_file, "plan.md", options) {
            Ok(info) => included_files.push(info),
            Err(_) => missing_files.push("plan.md".to_string()),
        }
    }

    // Add feedback file
    if let Some(feedback_file) = config.feedback_file {
        match add_file_to_zip(&mut zip, feedback_file, "feedback.md", options) {
            Ok(info) => included_files.push(info),
            Err(_) => missing_files.push("feedback.md".to_string()),
        }
    }

    // Add debug log (optional)
    if let Ok(debug_log) = planning_paths::debug_log_path() {
        match add_file_to_zip(&mut zip, &debug_log, "debug.log", options) {
            Ok(info) => included_files.push(info),
            Err(_) => missing_files.push("debug.log".to_string()),
        }
    }

    // Add session snapshot (optional)
    if let Some(session_id) = config.workflow_session_id {
        if let Ok(snapshot_path) = planning_paths::session_snapshot_path(session_id) {
            match add_file_to_zip(&mut zip, &snapshot_path, "session_snapshot.json", options) {
                Ok(info) => included_files.push(info),
                Err(_) => missing_files.push("session_snapshot.json".to_string()),
            }
        }
    }

    // Add workflow logs
    if let Ok(logs_dir) = planning_paths::logs_dir(config.working_dir) {
        // Add recent workflow log
        let workflow_log = logs_dir.join(format!("workflow-{}.log", config.run_id));
        match add_file_to_zip(&mut zip, &workflow_log, "workflow.log", options) {
            Ok(info) => included_files.push(info),
            Err(_) => missing_files.push("workflow.log".to_string()),
        }

        // Add agent stream log
        let stream_log = logs_dir.join(format!("agent-stream-{}.log", config.run_id));
        match add_file_to_zip(&mut zip, &stream_log, "agent-stream.log", options) {
            Ok(info) => included_files.push(info),
            Err(_) => missing_files.push("agent-stream.log".to_string()),
        }
    }

    // Create and add manifest
    let manifest = DiagnosticsManifest {
        created_at: chrono::Utc::now().to_rfc3339(),
        attempt_timestamps: config.attempt_timestamps,
        agent_name: config.agent_name.to_string(),
        failure_reason: config.failure_reason.to_string(),
        server_name: config.server_name.to_string(),
        working_dir_hash: planning_paths::working_dir_hash(config.working_dir),
        run_id: config.run_id.to_string(),
        plan_feedback_found: config.plan_feedback_found,
        verdict_found: config.verdict_found,
        attempt_count: if config.retry_output.is_some() { 2 } else { 1 },
        included_files: included_files.clone(),
        missing_files,
    };

    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    zip.start_file("manifest.json", options)?;
    zip.write_all(manifest_json.as_bytes())?;

    zip.finish()?;

    Ok(bundle_path)
}

/// Adds content directly to the ZIP, truncating if necessary
fn add_content_to_zip(
    zip: &mut ZipWriter<File>,
    name: &str,
    content: &str,
    options: SimpleFileOptions,
) -> Result<IncludedFile> {
    let original_size = content.len() as u64;
    let (content_to_write, truncated) = if original_size > MAX_FILE_SIZE {
        let truncated_content = content.get(..MAX_FILE_SIZE as usize).unwrap_or(content);
        let notice = format!(
            "\n\n--- TRUNCATED: Original size {} bytes, showing first {} bytes ---",
            original_size, MAX_FILE_SIZE
        );
        (format!("{}{}", truncated_content, notice), true)
    } else {
        (content.to_string(), false)
    };

    let included_size = content_to_write.len() as u64;

    zip.start_file(name, options)?;
    zip.write_all(content_to_write.as_bytes())?;

    Ok(IncludedFile {
        name: name.to_string(),
        original_size,
        included_size,
        truncated,
    })
}

/// Adds a file to the ZIP, truncating if necessary
fn add_file_to_zip(
    zip: &mut ZipWriter<File>,
    path: &Path,
    archive_name: &str,
    options: SimpleFileOptions,
) -> Result<IncludedFile> {
    let metadata = fs::metadata(path)?;
    let original_size = metadata.len();

    let mut file = File::open(path)?;
    let (content, truncated) = if original_size > MAX_FILE_SIZE {
        let mut buffer = vec![0u8; MAX_FILE_SIZE as usize];
        file.read_exact(&mut buffer)?;
        let notice = format!(
            "\n\n--- TRUNCATED: Original size {} bytes, showing first {} bytes ---",
            original_size, MAX_FILE_SIZE
        );
        buffer.extend(notice.as_bytes());
        (buffer, true)
    } else {
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;
        (buffer, false)
    };

    let included_size = content.len() as u64;

    zip.start_file(archive_name, options)?;
    zip.write_all(&content)?;

    Ok(IncludedFile {
        name: archive_name.to_string(),
        original_size,
        included_size,
        truncated,
    })
}

/// Truncates output for inclusion in a recovery prompt (max 50KB)
pub fn truncate_for_recovery_prompt(output: &str) -> String {
    if output.len() <= MAX_RECOVERY_PROMPT_OUTPUT_SIZE {
        output.to_string()
    } else {
        let truncated = output
            .get(..MAX_RECOVERY_PROMPT_OUTPUT_SIZE)
            .unwrap_or(output);
        format!(
            "{}\n\n--- OUTPUT TRUNCATED (showing first {} of {} bytes) ---",
            truncated,
            MAX_RECOVERY_PROMPT_OUTPUT_SIZE,
            output.len()
        )
    }
}

/// Information about a parse failure for diagnostics
#[derive(Debug, Clone)]
pub struct ParseFailureInfo {
    /// The error message
    pub error: String,
    /// Whether <plan-feedback> tags were found in output
    pub plan_feedback_found: bool,
    /// Whether verdict pattern was found
    pub verdict_found: bool,
}

#[cfg(test)]
mod tests {
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
}
