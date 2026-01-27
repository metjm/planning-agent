//! Helper functions for GUI rendering.

/// Format a relative timestamp string (e.g., "5m ago", "2h ago").
pub fn format_relative_time(timestamp: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .map(|dt| {
            let elapsed = chrono::Utc::now().signed_duration_since(dt.with_timezone(&chrono::Utc));
            if elapsed.num_seconds() < 60 {
                "just now".to_string()
            } else if elapsed.num_minutes() < 60 {
                format!("{}m ago", elapsed.num_minutes())
            } else if elapsed.num_hours() < 24 {
                format!("{}h ago", elapsed.num_hours())
            } else {
                format!("{}d ago", elapsed.num_days())
            }
        })
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Format a Unix timestamp into a human-readable date/time.
pub fn format_build_timestamp(timestamp: u64) -> String {
    use chrono::{TimeZone, Utc};
    if timestamp == 0 {
        return "unknown".to_string();
    }
    Utc.timestamp_opt(timestamp as i64, 0)
        .single()
        .map(|dt| dt.format("%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "invalid".to_string())
}

/// Format a duration as a human-readable string (e.g., "5m", "2h 30m").
pub fn format_duration(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        if mins > 0 {
            format!("{}h {}m", hours, mins)
        } else {
            format!("{}h", hours)
        }
    }
}

/// Format ping duration (e.g., "2s ago", "45s ago").
pub fn format_ping_duration(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else {
        format!("{}h ago", secs / 3600)
    }
}

/// Truncate a path for display, showing the end portion.
/// Uses character-based truncation to avoid panicking on UTF-8 boundaries.
pub fn truncate_path(path: &str, max_len: usize) -> String {
    let char_count = path.chars().count();
    if char_count <= max_len {
        path.to_string()
    } else {
        let skip_count = char_count.saturating_sub(max_len - 3);
        let suffix: String = path.chars().skip(skip_count).collect();
        format!("...{}", suffix)
    }
}

/// Format a reset timestamp as a countdown.
pub fn format_reset_countdown(epoch_seconds: i64) -> String {
    let reset = chrono::DateTime::from_timestamp(epoch_seconds, 0);
    match reset {
        Some(dt) => {
            let now = chrono::Utc::now();
            let diff = dt.signed_duration_since(now);
            if diff.num_seconds() <= 0 {
                "expired".to_string()
            } else if diff.num_minutes() < 60 {
                format!("{}m", diff.num_minutes())
            } else if diff.num_hours() < 24 {
                format!("{}h {}m", diff.num_hours(), diff.num_minutes() % 60)
            } else {
                format!("{}d {}h", diff.num_days(), diff.num_hours() % 24)
            }
        }
        None => "unknown".to_string(),
    }
}
