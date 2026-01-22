//! Change fingerprinting for implementation loop circuit breaker.
//!
//! This module provides helpers to detect if repository changes have occurred
//! between implementation iterations. If no changes are detected after a
//! `NeedsRevision` verdict, the implementation loop should stop to avoid
//! infinite loops.

use anyhow::Result;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Directories to exclude from fingerprinting.
const EXCLUDED_DIRS: &[&str] = &[".git", "target", "node_modules", ".planning-agent"];

/// Computes a fingerprint of repository changes.
///
/// For git repositories, uses `git status --porcelain` and `git diff --name-only`
/// to build a change set, then hashes those files.
///
/// For non-git directories, computes a lightweight fingerprint from
/// `(relative path, size, mtime)` while excluding common build directories.
///
/// # Arguments
/// * `working_dir` - The directory to fingerprint
///
/// # Returns
/// A u64 hash representing the current state of changes.
pub fn compute_change_fingerprint(working_dir: &Path) -> Result<u64> {
    if is_git_repo(working_dir) {
        compute_git_fingerprint(working_dir)
    } else {
        compute_filesystem_fingerprint(working_dir)
    }
}

/// Checks if a directory is a git repository.
fn is_git_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// Computes a fingerprint for a git repository using git status and diff.
fn compute_git_fingerprint(working_dir: &Path) -> Result<u64> {
    // Get changed files from git status (includes untracked)
    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(working_dir)
        .output()?;

    // Get modified files from git diff
    let diff_output = std::process::Command::new("git")
        .args(["diff", "--name-only", "--diff-filter=ACDMRT"])
        .current_dir(working_dir)
        .output()?;

    // Collect unique file paths
    let mut changed_files: BTreeSet<String> = BTreeSet::new();

    // Parse git status output
    if status_output.status.success() {
        let status_str = String::from_utf8_lossy(&status_output.stdout);
        for line in status_str.lines() {
            // Format: "XY filename" where XY is status, space at position 2
            // Use safe slicing: skip first 3 ASCII chars (status + space)
            if let Some(filename) = line.get(3..).map(str::trim) {
                if !filename.is_empty() {
                    changed_files.insert(filename.to_string());
                }
            }
        }
    }

    // Parse git diff output
    if diff_output.status.success() {
        let diff_str = String::from_utf8_lossy(&diff_output.stdout);
        for line in diff_str.lines() {
            let filename = line.trim();
            if !filename.is_empty() {
                changed_files.insert(filename.to_string());
            }
        }
    }

    // Hash the file contents and metadata
    let mut hasher = Sha256::new();

    for file in &changed_files {
        let file_path = working_dir.join(file);
        hasher.update(file.as_bytes());
        hasher.update(b"\0");

        if file_path.exists() {
            // Include file size and content hash
            if let Ok(metadata) = fs::metadata(&file_path) {
                hasher.update(metadata.len().to_le_bytes());
            }
            if let Ok(content) = fs::read(&file_path) {
                hasher.update(&content);
            }
        } else {
            // File was deleted
            hasher.update(b"DELETED");
        }
        hasher.update(b"\n");
    }

    // Convert first 8 bytes of SHA256 to u64
    let result = hasher.finalize();
    Ok(u64::from_le_bytes(result[..8].try_into().unwrap()))
}

/// Computes a fingerprint for a non-git directory using filesystem metadata.
fn compute_filesystem_fingerprint(working_dir: &Path) -> Result<u64> {
    let mut hasher = Sha256::new();
    let mut entries: BTreeSet<String> = BTreeSet::new();

    collect_files(working_dir, working_dir, &mut entries)?;

    for rel_path in &entries {
        let file_path = working_dir.join(rel_path);
        hasher.update(rel_path.as_bytes());
        hasher.update(b"\0");

        if let Ok(metadata) = fs::metadata(&file_path) {
            // Include size
            hasher.update(metadata.len().to_le_bytes());

            // Include mtime if available
            if let Ok(modified) = metadata.modified() {
                if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
                    hasher.update(duration.as_secs().to_le_bytes());
                }
            }
        }
        hasher.update(b"\n");
    }

    let result = hasher.finalize();
    Ok(u64::from_le_bytes(result[..8].try_into().unwrap()))
}

/// Recursively collects file paths, excluding certain directories.
fn collect_files(base: &Path, dir: &Path, entries: &mut BTreeSet<String>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        // Check if this directory should be excluded
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if EXCLUDED_DIRS.contains(&name) {
                continue;
            }
        }

        if path.is_dir() {
            collect_files(base, &path, entries)?;
        } else if path.is_file() {
            if let Ok(rel_path) = path.strip_prefix(base) {
                if let Some(rel_str) = rel_path.to_str() {
                    entries.insert(rel_str.to_string());
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_compute_filesystem_fingerprint_empty_dir() {
        let dir = TempDir::new().unwrap();
        let fp1 = compute_filesystem_fingerprint(dir.path()).unwrap();

        // Same empty dir should have same fingerprint
        let fp2 = compute_filesystem_fingerprint(dir.path()).unwrap();
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn test_compute_filesystem_fingerprint_changes_with_content() {
        let dir = TempDir::new().unwrap();

        let fp1 = compute_filesystem_fingerprint(dir.path()).unwrap();

        // Add a file
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "hello").unwrap();

        let fp2 = compute_filesystem_fingerprint(dir.path()).unwrap();
        assert_ne!(fp1, fp2, "Fingerprint should change when file is added");

        // Modify the file
        fs::write(&file_path, "hello world").unwrap();

        let fp3 = compute_filesystem_fingerprint(dir.path()).unwrap();
        assert_ne!(fp2, fp3, "Fingerprint should change when file is modified");
    }

    #[test]
    fn test_compute_filesystem_fingerprint_excludes_target() {
        let dir = TempDir::new().unwrap();

        // Add a regular file
        fs::write(dir.path().join("src.txt"), "source").unwrap();
        let fp1 = compute_filesystem_fingerprint(dir.path()).unwrap();

        // Add files in excluded directories
        let target_dir = dir.path().join("target");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join("build.txt"), "build output").unwrap();

        let node_modules = dir.path().join("node_modules");
        fs::create_dir_all(&node_modules).unwrap();
        fs::write(node_modules.join("package.json"), "{}").unwrap();

        let fp2 = compute_filesystem_fingerprint(dir.path()).unwrap();
        assert_eq!(
            fp1, fp2,
            "Fingerprint should not change for excluded directories"
        );
    }

    #[test]
    fn test_is_git_repo() {
        let dir = TempDir::new().unwrap();
        assert!(!is_git_repo(dir.path()));

        // Create .git directory
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        assert!(is_git_repo(dir.path()));
    }

    #[test]
    fn test_collect_files_excludes_properly() {
        let dir = TempDir::new().unwrap();

        // Create files in various locations
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();

        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("lib.rs"), "// lib").unwrap();

        let target = dir.path().join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("debug.txt"), "debug").unwrap();

        let mut entries = BTreeSet::new();
        collect_files(dir.path(), dir.path(), &mut entries).unwrap();

        assert!(entries.contains("main.rs"));
        assert!(entries.contains("src/lib.rs") || entries.contains("src\\lib.rs"));
        // target should be excluded
        assert!(!entries.iter().any(|e| e.contains("target")));
    }
}
