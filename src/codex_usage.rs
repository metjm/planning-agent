use crate::planning_paths;
use crate::usage_reset::{ResetTimestamp, UsageWindow, UsageWindowSpan};
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
    /// Session (5h) usage window with reset timestamp
    pub session: UsageWindow,
    /// Weekly usage window with reset timestamp
    pub weekly: UsageWindow,
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
    /// Window duration in minutes (300 = 5h, 10080 = weekly)
    window_minutes: Option<u64>,
    /// Unix timestamp when this window resets
    resets_at: Option<i64>,
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

/// Parsed rate limit data from a Codex session
struct ParsedRateLimits {
    session_used: f64,
    session_resets_at: Option<i64>,
    session_window_minutes: Option<u64>,
    weekly_used: f64,
    weekly_resets_at: Option<i64>,
    weekly_window_minutes: Option<u64>,
}

fn read_rate_limits_from_session(path: &Path) -> Option<ParsedRateLimits> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut primary_used: Option<f64> = None;
    let mut primary_resets_at: Option<i64> = None;
    let mut primary_window_minutes: Option<u64> = None;
    let mut secondary_used: Option<f64> = None;
    let mut secondary_resets_at: Option<i64> = None;
    let mut secondary_window_minutes: Option<u64> = None;

    for line in reader.lines().map_while(Result::ok) {
        if let Ok(entry) = serde_json::from_str::<SessionEntry>(&line) {
            if entry.entry_type == "event_msg" {
                if let Some(payload) = entry.payload {
                    if payload.payload_type.as_deref() == Some("token_count") {
                        if let Some(rate_limits) = payload.rate_limits {
                            if let Some(primary) = rate_limits.primary {
                                primary_used = Some(primary.used_percent);
                                primary_window_minutes = primary.window_minutes;
                                // Only capture resets_at if window is 5h (300 minutes)
                                if primary.window_minutes == Some(300) {
                                    primary_resets_at = primary.resets_at;
                                }
                            }
                            if let Some(secondary) = rate_limits.secondary {
                                secondary_used = Some(secondary.used_percent);
                                secondary_window_minutes = secondary.window_minutes;
                                // Only capture resets_at if window is weekly (10080 minutes)
                                if secondary.window_minutes == Some(10080) {
                                    secondary_resets_at = secondary.resets_at;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    match (primary_used, secondary_used) {
        (Some(p), Some(s)) => Some(ParsedRateLimits {
            session_used: p,
            session_resets_at: primary_resets_at,
            session_window_minutes: primary_window_minutes,
            weekly_used: s,
            weekly_resets_at: secondary_resets_at,
            weekly_window_minutes: secondary_window_minutes,
        }),
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
        if let Some(limits) = read_rate_limits_from_session(session_file) {
            let session_used_pct = limits.session_used.round().clamp(0.0, 100.0) as u8;
            let weekly_used_pct = limits.weekly_used.round().clamp(0.0, 100.0) as u8;

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
                        let _ = writeln!(file, "Session used: {}%", limits.session_used);
                        let _ = writeln!(file, "Session resets_at: {:?}", limits.session_resets_at);
                        let _ = writeln!(file, "Weekly used: {}%", limits.weekly_used);
                        let _ = writeln!(file, "Weekly resets_at: {:?}", limits.weekly_resets_at);
                    }
                }
            }

            // Convert window_minutes to UsageWindowSpan
            let session_span = match limits.session_window_minutes {
                Some(300) => UsageWindowSpan::Minutes(300), // 5h
                Some(mins) => UsageWindowSpan::Minutes(mins as u16),
                None => UsageWindowSpan::Unknown,
            };
            let weekly_span = match limits.weekly_window_minutes {
                Some(10080) => UsageWindowSpan::Days(7), // 7d
                Some(mins) => UsageWindowSpan::Minutes(mins as u16),
                None => UsageWindowSpan::Unknown,
            };

            // Build usage windows with reset timestamps and spans
            let session = match limits.session_resets_at {
                Some(ts) => UsageWindow::with_percent_reset_and_span(
                    session_used_pct,
                    ResetTimestamp::from_epoch_seconds(ts),
                    session_span,
                ),
                None => UsageWindow::with_percent_and_span(session_used_pct, session_span),
            };
            let weekly = match limits.weekly_resets_at {
                Some(ts) => UsageWindow::with_percent_reset_and_span(
                    weekly_used_pct,
                    ResetTimestamp::from_epoch_seconds(ts),
                    weekly_span,
                ),
                None => UsageWindow::with_percent_and_span(weekly_used_pct, weekly_span),
            };

            return CodexUsage {
                session,
                weekly,
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
        let usage = CodexUsage::with_error("Test error".to_string());
        assert!(usage.fetched_at.is_some());
        assert_eq!(usage.error_message, Some("Test error".to_string()));
    }

    #[test]
    fn test_codex_usage_not_available_sets_fetched_at() {
        let usage = CodexUsage::not_available();
        assert!(usage.fetched_at.is_some());
        assert_eq!(usage.error_message, Some("CLI not found".to_string()));
    }
}
