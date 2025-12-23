use anyhow::{Context, Result};
use std::process::Command;
use std::time::Duration;

pub const BUILD_SHA: &str = env!("PLANNING_AGENT_GIT_SHA");

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
        .map(|d| format_commit_date(d))
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

#[derive(Debug, Clone)]
pub enum UpdateResult {
    Success(std::path::PathBuf),
    GitNotFound,
    CargoNotFound,
    InstallFailed(String),
    BinaryNotFound,
}

const UPDATE_MARKER_FILE: &str = "update-installed";

pub fn write_update_marker(working_dir: &std::path::Path) -> std::io::Result<()> {
    let marker_path = working_dir.join(".planning-agent").join(UPDATE_MARKER_FILE);

    if let Some(parent) = marker_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&marker_path, "")
}

pub fn consume_update_marker(working_dir: &std::path::Path) -> bool {
    let marker_path = working_dir.join(".planning-agent").join(UPDATE_MARKER_FILE);
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
    use std::fs;

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

        let temp_dir = std::env::temp_dir().join(format!(
            "planning-agent-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        assert!(!consume_update_marker(&temp_dir));

        write_update_marker(&temp_dir).unwrap();

        let marker_path = temp_dir.join(".planning-agent").join(UPDATE_MARKER_FILE);
        assert!(marker_path.exists());

        assert!(consume_update_marker(&temp_dir));
        assert!(!marker_path.exists());

        assert!(!consume_update_marker(&temp_dir));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
