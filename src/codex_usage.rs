use crate::planning_paths;
use serde::Deserialize;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn is_debug_enabled() -> bool {
    std::env::var("CODEX_USAGE_DEBUG")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

#[derive(Debug, Clone, Default)]
pub struct CodexUsage {
    pub hourly_remaining: Option<u8>,
    pub weekly_remaining: Option<u8>,
    pub plan_type: Option<String>,
    pub fetched_at: Option<Instant>,
    pub error_message: Option<String>,
}

impl CodexUsage {
    pub fn with_error(error: String) -> Self {
        Self {
            error_message: Some(error),
            fetched_at: Some(Instant::now()),
            ..Default::default()
        }
    }

    pub fn not_available() -> Self {
        Self {
            error_message: Some("CLI not found".to_string()),
            fetched_at: Some(Instant::now()),
            ..Default::default()
        }
    }
}

#[derive(Debug, Deserialize)]
struct SessionEntry {
    #[serde(rename = "type")]
    entry_type: String,
    payload: Option<SessionPayload>,
}

#[derive(Debug, Deserialize)]
struct SessionPayload {
    #[serde(rename = "type")]
    payload_type: Option<String>,
    rate_limits: Option<RateLimits>,
}

#[derive(Debug, Deserialize)]
struct RateLimits {
    primary: Option<LimitInfo>,
    secondary: Option<LimitInfo>,
}

#[derive(Debug, Deserialize)]
struct LimitInfo {
    used_percent: f64,
}

pub fn is_codex_available() -> bool {
    which::which("codex").is_ok()
}

fn get_codex_home() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("CODEX_HOME") {
        return Some(PathBuf::from(home));
    }
    dirs::home_dir().map(|h| h.join(".codex"))
}

fn find_session_files_sorted() -> Vec<PathBuf> {
    let Some(codex_home) = get_codex_home() else {
        return Vec::new();
    };
    let sessions_dir = codex_home.join("sessions");
    if !sessions_dir.exists() {
        return Vec::new();
    }
    let mut all_files: Vec<PathBuf> = Vec::new();
fn collect_jsonl_files(dir: &Path, files: &mut Vec<PathBuf>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    collect_jsonl_files(&path, files);
                } else if path.extension().is_some_and(|e| e == "jsonl") {
                    files.push(path);
                }
            }
        }
    }
    collect_jsonl_files(&sessions_dir, &mut all_files);
    all_files.sort_by(|a, b| {
        let a_time = fs::metadata(a).and_then(|m| m.modified()).ok();
        let b_time = fs::metadata(b).and_then(|m| m.modified()).ok();
        b_time.cmp(&a_time)
    });
    all_files
}

fn read_rate_limits_from_session(path: &Path) -> Option<(f64, f64)> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut primary_used: Option<f64> = None;
    let mut secondary_used: Option<f64> = None;
    for line in reader.lines().map_while(Result::ok) {
        if let Ok(entry) = serde_json::from_str::<SessionEntry>(&line) {
            if entry.entry_type == "event_msg" {
                if let Some(payload) = entry.payload {
                    if payload.payload_type.as_deref() == Some("token_count") {
                        if let Some(rate_limits) = payload.rate_limits {
                            if let Some(primary) = rate_limits.primary {
                                primary_used = Some(primary.used_percent);
                            }
                            if let Some(secondary) = rate_limits.secondary {
                                secondary_used = Some(secondary.used_percent);
                            }
                        }
                    }
                }
            }
        }
    }
    match (primary_used, secondary_used) {
        (Some(p), Some(s)) => Some((p, s)),
        _ => None,
    }
}

pub fn fetch_codex_usage_sync() -> CodexUsage {
    if !is_codex_available() {
        return CodexUsage::not_available();
    }
    let session_files = find_session_files_sorted();
    if session_files.is_empty() {
        return CodexUsage {
            error_message: Some("No session data".to_string()),
            fetched_at: Some(Instant::now()),
            ..Default::default()
        };
    }
    for session_file in session_files.iter().take(10) {
        if let Some((primary_used, secondary_used)) = read_rate_limits_from_session(session_file) {
            let hourly_remaining = (100.0 - primary_used).round().clamp(0.0, 100.0) as u8;
            let weekly_remaining = (100.0 - secondary_used).round().clamp(0.0, 100.0) as u8;
            if is_debug_enabled() {
                // Use home-based log path
                if let Ok(log_path) = planning_paths::codex_status_log_path() {
                    if let Ok(mut file) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(log_path)
                    {
                        use std::io::Write as _;
                        let _ = writeln!(file, "\n=== Session File Parse ===");
                        let _ = writeln!(file, "File: {:?}", session_file);
                        let _ = writeln!(file, "Primary used: {}%", primary_used);
                        let _ = writeln!(file, "Secondary used: {}%", secondary_used);
                        let _ = writeln!(
                            file,
                            "Hourly remaining: {}%, Weekly remaining: {}%",
                            hourly_remaining, weekly_remaining
                        );
                    }
                }
            }
            return CodexUsage {
                hourly_remaining: Some(hourly_remaining),
                weekly_remaining: Some(weekly_remaining),
                plan_type: None,
                fetched_at: Some(Instant::now()),
                error_message: None,
            };
        }
    }
    CodexUsage {
        error_message: Some("No rate limit data".to_string()),
        fetched_at: Some(Instant::now()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codex_usage_with_error_sets_fetched_at() {
        let usage = super::CodexUsage::with_error("Test error".to_string());
        assert!(usage.fetched_at.is_some());
        assert_eq!(usage.error_message, Some("Test error".to_string()));
    }

    #[test]
    fn test_codex_usage_not_available_sets_fetched_at() {
        let usage = CodexUsage::not_available();
        assert!(usage.fetched_at.is_some());
        assert_eq!(usage.error_message, Some("CLI not found".to_string()));
    }

    #[test]
    #[ignore]
    fn test_fetch_codex_usage_real() {
        if !is_codex_available() {
            eprintln!("Codex CLI not found, skipping");
            return;
        }
        eprintln!("Fetching real Codex usage...");
        let usage = fetch_codex_usage_sync();
        eprintln!("Result: {:?}", usage);
        assert!(usage.fetched_at.is_some());
        if usage.error_message.is_none() {
            eprintln!("5h remaining: {:?}%", usage.hourly_remaining);
            eprintln!("Weekly remaining: {:?}%", usage.weekly_remaining);
            eprintln!("Plan: {:?}", usage.plan_type);
        }
    }
}
