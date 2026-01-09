use crate::planning_paths;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const BUILD_SHA: &str = env!("PLANNING_AGENT_GIT_SHA");

/// Cache TTL for version info (24 hours)
const VERSION_CACHE_TTL_SECS: u64 = 86_400;

/// Version information for the current build, including commit date.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    pub build_sha: String,
    pub short_sha: String,
    pub commit_date: String,
    pub fetched_at_epoch: u64,
}

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub latest_sha: String,
    pub short_sha: String,
    pub commit_date: String,
}

#[derive(Debug, Clone)]
pub enum UpdateStatus {
    Checking,
    UpToDate,
    UpdateAvailable(UpdateInfo),
    CheckFailed(String),
    VersionUnknown,
}

impl Default for UpdateStatus {
    fn default() -> Self {
        if BUILD_SHA == "unknown" {
            UpdateStatus::VersionUnknown
        } else {
            UpdateStatus::Checking
        }
    }
}

pub fn check_for_update() -> UpdateStatus {
    if BUILD_SHA == "unknown" {
        return UpdateStatus::VersionUnknown;
    }

    match fetch_latest_commit() {
        Ok(info) => {
            if info.latest_sha.starts_with(BUILD_SHA) || BUILD_SHA.starts_with(&info.latest_sha) {
                UpdateStatus::UpToDate
            } else {
                UpdateStatus::UpdateAvailable(info)
            }
        }
        Err(e) => UpdateStatus::CheckFailed(e.to_string()),
    }
}

fn fetch_latest_commit() -> Result<UpdateInfo> {

    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build();
    let agent: ureq::Agent = config.into();

    let url = "https://api.github.com/repos/metjm/planning-agent/commits?per_page=1";

    let mut request = agent.get(url)
        .header("User-Agent", format!("planning-agent/{}", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json");

    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        request = request.header("Authorization", format!("Bearer {}", token));
    }

    let body = request
        .call()
        .context("Failed to fetch from GitHub")?
        .body_mut()
        .read_to_string()
        .context("Failed to read response body")?;

    let response: serde_json::Value = serde_json::from_str(&body)
        .context("Failed to parse GitHub response")?;

    let commits = response
        .as_array()
        .context("Expected array response from GitHub")?;

    let commit = commits
        .first()
        .context("No commits found")?;

    let sha = commit["sha"]
        .as_str()
        .context("Missing sha field")?
        .to_string();

    let short_sha = sha.chars().take(7).collect();

    let commit_date = commit["commit"]["author"]["date"]
        .as_str()
        .map(format_commit_date)
        .unwrap_or_else(|| "Unknown".to_string());

    Ok(UpdateInfo {
        latest_sha: sha,
        short_sha,
        commit_date,
    })
}

fn format_commit_date(iso_date: &str) -> String {

    if let Some(date_part) = iso_date.split('T').next() {
        let parts: Vec<&str> = date_part.split('-').collect();
        if parts.len() == 3 {
            let month = match parts[1] {
                "01" => "Jan",
                "02" => "Feb",
                "03" => "Mar",
                "04" => "Apr",
                "05" => "May",
                "06" => "Jun",
                "07" => "Jul",
                "08" => "Aug",
                "09" => "Sep",
                "10" => "Oct",
                "11" => "Nov",
                "12" => "Dec",
                _ => return iso_date.to_string(),
            };
            let day = parts[2].trim_start_matches('0');
            return format!("{} {}", month, day);
        }
    }
    iso_date.to_string()
}

/// Read version cache from disk. Returns None if cache is missing, corrupt, stale, or for a different build.
fn read_version_cache() -> Option<VersionInfo> {
    let cache_path = planning_paths::version_cache_path().ok()?;
    let content = std::fs::read_to_string(&cache_path).ok()?;
    let info: VersionInfo = serde_json::from_str(&content).ok()?;

    // Check if cache is for the current build
    if info.build_sha != BUILD_SHA {
        return None;
    }

    // Check if cache is still fresh
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs();
    if now.saturating_sub(info.fetched_at_epoch) > VERSION_CACHE_TTL_SECS {
        return None;
    }

    Some(info)
}

/// Write version cache to disk. Errors are silently ignored.
fn write_version_cache(info: &VersionInfo) {
    if let Ok(cache_path) = planning_paths::version_cache_path() {
        if let Ok(content) = serde_json::to_string_pretty(info) {
            let _ = std::fs::write(&cache_path, content);
        }
    }
}

/// Fetch commit info for a specific SHA from GitHub.
fn fetch_commit_info(sha: &str) -> Result<VersionInfo> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build();
    let agent: ureq::Agent = config.into();

    let url = format!("https://api.github.com/repos/metjm/planning-agent/commits/{}", sha);

    let mut request = agent.get(&url)
        .header("User-Agent", format!("planning-agent/{}", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json");

    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        request = request.header("Authorization", format!("Bearer {}", token));
    }

    let body = request
        .call()
        .context("Failed to fetch commit from GitHub")?
        .body_mut()
        .read_to_string()
        .context("Failed to read response body")?;

    let response: serde_json::Value = serde_json::from_str(&body)
        .context("Failed to parse GitHub response")?;

    let full_sha = response["sha"]
        .as_str()
        .context("Missing sha field")?
        .to_string();

    let short_sha: String = full_sha.chars().take(7).collect();

    let commit_date = response["commit"]["author"]["date"]
        .as_str()
        .map(format_commit_date)
        .unwrap_or_else(|| "Unknown".to_string());

    let fetched_at_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    Ok(VersionInfo {
        build_sha: BUILD_SHA.to_string(),
        short_sha,
        commit_date,
        fetched_at_epoch,
    })
}

/// Get version info from cache or fetch from GitHub.
/// Returns None if BUILD_SHA is "unknown" or if fetch fails.
pub fn get_cached_or_fetch_version_info() -> Option<VersionInfo> {
    if BUILD_SHA == "unknown" {
        return None;
    }

    // Try reading from cache first
    if let Some(cached) = read_version_cache() {
        return Some(cached);
    }

    // Fetch from GitHub
    match fetch_commit_info(BUILD_SHA) {
        Ok(info) => {
            write_version_cache(&info);
            Some(info)
        }
        Err(_) => {
            // On error, return a basic version info with "Unknown" date
            let fetched_at_epoch = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            Some(VersionInfo {
                build_sha: BUILD_SHA.to_string(),
                short_sha: BUILD_SHA.chars().take(7).collect(),
                commit_date: "Unknown".to_string(),
                fetched_at_epoch,
            })
        }
    }
}

#[derive(Debug, Clone)]
pub enum UpdateResult {
    Success(std::path::PathBuf),
    GitNotFound,
    CargoNotFound,
    InstallFailed(String),
    BinaryNotFound,
}

/// Writes the update marker to home storage (`~/.planning-agent/update-installed`).
///
/// The `working_dir` parameter is no longer used but kept for API compatibility.
#[allow(unused_variables)]
pub fn write_update_marker(working_dir: &std::path::Path) -> std::io::Result<()> {
    let marker_path = planning_paths::update_marker_path()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    std::fs::write(&marker_path, "")
}

/// Consumes the update marker from home storage (`~/.planning-agent/update-installed`).
///
/// The `working_dir` parameter is no longer used but kept for API compatibility.
#[allow(unused_variables)]
pub fn consume_update_marker(working_dir: &std::path::Path) -> bool {
    let marker_path = match planning_paths::update_marker_path() {
        Ok(p) => p,
        Err(_) => return false,
    };
    if marker_path.exists() {
        let _ = std::fs::remove_file(&marker_path);
        true
    } else {
        false
    }
}

pub fn perform_update() -> UpdateResult {

    if which::which("git").is_err() {
        return UpdateResult::GitNotFound;
    }

    if which::which("cargo").is_err() {
        return UpdateResult::CargoNotFound;
    }

    let output = Command::new("cargo")
        .args([
            "install",
            "--git",
            "https://github.com/metjm/planning-agent.git",
            "--force",
        ])
        .output();

    match output {
        Ok(result) => {
            if result.status.success() {

                match which::which("planning") {
                    Ok(path) => UpdateResult::Success(path),
                    Err(_) => {

                        if let Some(home) = dirs::home_dir() {
                            let fallback = home.join(".cargo/bin/planning");
                            if fallback.exists() {
                                return UpdateResult::Success(fallback);
                            }
                        }
                        UpdateResult::BinaryNotFound
                    }
                }
            } else {
                let stderr = String::from_utf8_lossy(&result.stderr);
                let stdout = String::from_utf8_lossy(&result.stdout);
                UpdateResult::InstallFailed(format!(
                    "cargo install failed:\n{}\n{}",
                    stdout, stderr
                ))
            }
        }
        Err(e) => UpdateResult::InstallFailed(format!("Failed to run cargo: {}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_commit_date() {
        assert_eq!(format_commit_date("2024-12-20T10:30:00Z"), "Dec 20");
        assert_eq!(format_commit_date("2024-01-05T00:00:00Z"), "Jan 5");
        assert_eq!(format_commit_date("invalid"), "invalid");
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
        // This test now uses home-based storage, so we just test the functionality
        // without checking a specific path in a temp directory

        // Skip if HOME is not set
        if std::env::var("HOME").is_err() {
            return;
        }

        let temp_dir = std::env::temp_dir().join(format!(
            "planning-agent-test-{}",
            std::process::id()
        ));

        // First consume to ensure we start clean
        let _ = consume_update_marker(&temp_dir);

        // Should be false when no marker exists
        assert!(!consume_update_marker(&temp_dir));

        // Write the marker
        write_update_marker(&temp_dir).unwrap();

        // Consume should return true and remove the marker
        assert!(consume_update_marker(&temp_dir));

        // Second consume should return false (marker was removed)
        assert!(!consume_update_marker(&temp_dir));
    }
}
