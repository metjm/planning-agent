//! Tests for session browser input handling, specifically ZIP export functionality.

use super::*;
use std::io::Read;
use tempfile::tempdir;

/// Test successful ZIP export of a session directory.
#[tokio::test]
async fn test_export_session_zip_success() {
    // Create a temp directory structure mimicking a session
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let home_dir = temp_dir.path();

    // Create the session directory structure
    let session_id = "test-session-12345";
    let session_dir = home_dir
        .join(".planning-agent")
        .join("sessions")
        .join(session_id);
    std::fs::create_dir_all(&session_dir).expect("Failed to create session dir");

    // Create test files
    std::fs::write(session_dir.join("events.jsonl"), b"test event data\n")
        .expect("Failed to write events.jsonl");
    std::fs::write(
        session_dir.join("plan.md"),
        b"# Test Plan\n\nThis is a test plan.",
    )
    .expect("Failed to write plan.md");

    // Create logs subdirectory with a file
    let logs_dir = session_dir.join("logs");
    std::fs::create_dir(&logs_dir).expect("Failed to create logs dir");
    std::fs::write(
        logs_dir.join("session.log"),
        b"[2024-01-01] Session started\n",
    )
    .expect("Failed to write session.log");

    // Create output directory
    let output_dir = tempdir().expect("Failed to create output dir");

    // Mock home_dir by directly testing the add_directory_to_zip function
    // since export_session_zip_async uses dirs::home_dir() which we can't mock
    let zip_name = format!(
        "{}_{}.zip",
        chrono::Local::now().format("%Y%m%d"),
        session_id
    );
    let zip_path = output_dir.path().join(&zip_name);

    // Create the ZIP using the helper function
    {
        let file = std::fs::File::create(&zip_path).expect("Failed to create zip file");
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        add_directory_to_zip(&mut zip, &session_dir, &session_dir, options)
            .expect("Failed to add directory to zip");

        zip.finish().expect("Failed to finish zip");
    }

    // Verify the ZIP was created
    assert!(zip_path.exists(), "ZIP file should exist");

    // Extract and verify contents
    let file = std::fs::File::open(&zip_path).expect("Failed to open zip file");
    let mut archive = zip::ZipArchive::new(file).expect("Failed to read zip archive");

    // Check for expected files
    let mut found_events = false;
    let mut found_plan = false;
    let mut found_session_log = false;
    let mut found_logs_dir = false;

    for i in 0..archive.len() {
        let entry = archive.by_index(i).expect("Failed to get archive entry");
        let name = entry.name();

        if name == "events.jsonl" {
            found_events = true;
        } else if name == "plan.md" {
            found_plan = true;
        } else if name == "logs/session.log" {
            found_session_log = true;
        } else if name == "logs/" {
            found_logs_dir = true;
        }
    }

    assert!(found_events, "ZIP should contain events.jsonl");
    assert!(found_plan, "ZIP should contain plan.md");
    assert!(found_logs_dir, "ZIP should contain logs/ directory");
    assert!(found_session_log, "ZIP should contain logs/session.log");

    // Verify file content
    let mut plan_file = archive.by_name("plan.md").expect("Failed to get plan.md");
    let mut content = String::new();
    plan_file
        .read_to_string(&mut content)
        .expect("Failed to read plan.md");
    assert!(
        content.contains("Test Plan"),
        "plan.md should have correct content"
    );
}

/// Test ZIP export with nested directory structure.
#[tokio::test]
async fn test_export_session_zip_nested_directories() {
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let session_dir = temp_dir.path().join("session");
    std::fs::create_dir(&session_dir).expect("Failed to create session dir");

    // Create nested structure: logs/sub/deep.log
    let deep_dir = session_dir.join("logs").join("sub");
    std::fs::create_dir_all(&deep_dir).expect("Failed to create deep dir");
    std::fs::write(deep_dir.join("deep.log"), b"deep log content")
        .expect("Failed to write deep.log");

    // Create the ZIP
    let zip_path = temp_dir.path().join("nested.zip");
    {
        let file = std::fs::File::create(&zip_path).expect("Failed to create zip file");
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        add_directory_to_zip(&mut zip, &session_dir, &session_dir, options)
            .expect("Failed to add directory to zip");

        zip.finish().expect("Failed to finish zip");
    }

    // Verify nested structure is preserved
    let file = std::fs::File::open(&zip_path).expect("Failed to open zip");
    let mut archive = zip::ZipArchive::new(file).expect("Failed to read archive");

    let mut found_deep = false;
    for i in 0..archive.len() {
        let entry = archive.by_index(i).expect("Failed to get entry");
        if entry.name() == "logs/sub/deep.log" {
            found_deep = true;
            break;
        }
    }

    assert!(
        found_deep,
        "ZIP should preserve nested directory structure (logs/sub/deep.log)"
    );
}

/// Test ZIP export of empty session directory.
#[tokio::test]
async fn test_export_session_zip_empty_directory() {
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let session_dir = temp_dir.path().join("empty_session");
    std::fs::create_dir(&session_dir).expect("Failed to create session dir");

    // Create the ZIP
    let zip_path = temp_dir.path().join("empty.zip");
    {
        let file = std::fs::File::create(&zip_path).expect("Failed to create zip file");
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        add_directory_to_zip(&mut zip, &session_dir, &session_dir, options)
            .expect("Failed to add directory to zip");

        zip.finish().expect("Failed to finish zip");
    }

    // Verify empty ZIP is valid
    let file = std::fs::File::open(&zip_path).expect("Failed to open zip");
    let archive = zip::ZipArchive::new(file).expect("Failed to read archive");

    assert_eq!(archive.len(), 0, "Empty directory should create empty ZIP");
}
