use crate::planning_paths;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const BUILD_SHA: &str = env!("PLANNING_AGENT_GIT_SHA");

/// Build timestamp (Unix epoch seconds from git commit).
/// Used for version comparison to determine if a client is newer than a daemon.
/// A value of 0 means the timestamp couldn't be determined (e.g., not a git repo).
pub const BUILD_TIMESTAMP: u64 = {
    // Parse at compile time - this is a const context so we can't use .parse()
    // The env var is set by build.rs as a decimal string
    let bytes = env!("PLANNING_AGENT_BUILD_TIMESTAMP").as_bytes();
    let mut result: u64 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let digit = bytes[i];
        if digit >= b'0' && digit <= b'9' {
            result = result * 10 + (digit - b'0') as u64;
        }
        i += 1;
    }
    result
};

/// Features enabled at build time (comma-separated, empty if none).
/// Used by perform_update() to preserve features across updates.
pub const BUILD_FEATURES: &str = env!("PLANNING_AGENT_BUILD_FEATURES");

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

    let mut request = agent
        .get(url)
        .header(
            "User-Agent",
            format!("planning-agent/{}", env!("CARGO_PKG_VERSION")),
        )
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

    let response: serde_json::Value =
        serde_json::from_str(&body).context("Failed to parse GitHub response")?;

    let commits = response
        .as_array()
        .context("Expected array response from GitHub")?;

    let commit = commits.first().context("No commits found")?;

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
    // Split by 'T' to get date and time parts
    let mut split = iso_date.split('T');
    let date_part = match split.next() {
        Some(d) => d,
        None => return iso_date.to_string(),
    };
    let time_part = split.next();

    let date_parts: Vec<&str> = date_part.split('-').collect();
    if date_parts.len() != 3 {
        return iso_date.to_string();
    }

    let month = match date_parts[1] {
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
    let day = date_parts[2].trim_start_matches('0');

    // Extract HH:MM from time part if available (e.g., "10:30:00Z" -> "10:30")
    if let Some(time) = time_part {
        // Remove trailing timezone suffix (Z or +00:00 etc) and take HH:MM
        let time_clean = time
            .trim_end_matches('Z')
            .split('+')
            .next()
            .unwrap_or(time)
            .split('-')
            .next()
            .unwrap_or(time);
        // Time format is "HH:MM:SS", all ASCII, so byte index 5 is safe
        // Use .get() for consistency with clippy::string_slice lint
        if let Some(hh_mm) = time_clean.get(..5) {
            return format!("{} {} {}", month, day, hh_mm);
        }
    }

    format!("{} {}", month, day)
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
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
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

    let url = format!(
        "https://api.github.com/repos/metjm/planning-agent/commits/{}",
        sha
    );

    let mut request = agent
        .get(&url)
        .header(
            "User-Agent",
            format!("planning-agent/{}", env!("CARGO_PKG_VERSION")),
        )
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

    let response: serde_json::Value =
        serde_json::from_str(&body).context("Failed to parse GitHub response")?;

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
    /// Update succeeded. Contains (binary_path, features_message).
    /// features_message is empty if no features, otherwise " with features: X,Y"
    Success(std::path::PathBuf, String),
    GitNotFound,
    CargoNotFound,
    /// Install failed. Contains (error_message, is_feature_error).
    /// is_feature_error is true if the failure was due to an unknown feature.
    InstallFailed(String, bool),
    BinaryNotFound,
}

/// Writes the update marker to home storage (`~/.planning-agent/update-installed`).
pub fn write_update_marker() -> std::io::Result<()> {
    let marker_path =
        planning_paths::update_marker_path().map_err(|e| std::io::Error::other(e.to_string()))?;
    std::fs::write(&marker_path, "")
}

/// Consumes the update marker from home storage (`~/.planning-agent/update-installed`).
pub fn consume_update_marker() -> bool {
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

/// Builds the arguments for `cargo install` during update.
/// Separated from perform_update() for testability.
///
/// Returns (args, features_msg) where:
/// - args: The complete argument list for cargo install
/// - features_msg: Human-readable description of features being installed (empty if none)
pub fn build_update_args() -> (Vec<&'static str>, String) {
    let mut args = vec![
        "install",
        "--git",
        "https://github.com/metjm/planning-agent.git",
        "--force",
    ];

    let features_msg = if BUILD_FEATURES.is_empty() {
        String::new()
    } else {
        args.push("--features");
        args.push(BUILD_FEATURES);
        format!(" with features: {}", BUILD_FEATURES)
    };

    (args, features_msg)
}

pub fn perform_update() -> UpdateResult {
    if which::which("git").is_err() {
        return UpdateResult::GitNotFound;
    }

    if which::which("cargo").is_err() {
        return UpdateResult::CargoNotFound;
    }

    let (args, features_msg) = build_update_args();

    let output = Command::new("cargo").args(&args).output();

    match output {
        Ok(result) => {
            if result.status.success() {
                match which::which("planning") {
                    Ok(path) => UpdateResult::Success(path, features_msg),
                    Err(_) => {
                        if let Some(home) = dirs::home_dir() {
                            let fallback = home.join(".cargo/bin/planning");
                            if fallback.exists() {
                                return UpdateResult::Success(fallback, features_msg);
                            }
                        }
                        UpdateResult::BinaryNotFound
                    }
                }
            } else {
                let stderr = String::from_utf8_lossy(&result.stderr);
                let stdout = String::from_utf8_lossy(&result.stdout);
                let combined = format!("cargo install failed:\n{}\n{}", stdout, stderr);

                // Check if this is a feature-related error
                let is_feature_error = stderr.contains("unknown feature")
                    || stderr.contains("does not have the feature")
                    || stdout.contains("unknown feature")
                    || stdout.contains("does not have the feature");

                UpdateResult::InstallFailed(combined, is_feature_error)
            }
        }
        Err(e) => UpdateResult::InstallFailed(format!("Failed to run cargo: {}", e), false),
    }
}

#[cfg(test)]
#[path = "tests/update_tests.rs"]
mod tests;
