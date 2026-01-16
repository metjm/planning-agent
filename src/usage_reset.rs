//! Shared types for usage windows with reset timestamps.
//!
//! This module provides `UsageWindow` and `ResetTimestamp` types used across
//! all provider usage tracking (Claude, Gemini, Codex) to support countdown
//! rendering in the UI.

use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A usage window with percent used and an optional reset timestamp.
///
/// This type pairs the usage percentage with when the window resets,
/// enabling countdown display in the UI.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UsageWindow {
    /// Percentage used (0-100), or None if unknown
    pub used_percent: Option<u8>,
    /// When this window resets, as an absolute timestamp
    pub reset_at: Option<ResetTimestamp>,
}

impl UsageWindow {
    /// Creates a new usage window with just the percent used.
    pub fn with_percent(percent: u8) -> Self {
        Self {
            used_percent: Some(percent),
            reset_at: None,
        }
    }

    /// Creates a new usage window with percent and reset timestamp.
    #[allow(dead_code)]
    pub fn with_percent_and_reset(percent: u8, reset_at: ResetTimestamp) -> Self {
        Self {
            used_percent: Some(percent),
            reset_at: Some(reset_at),
        }
    }

    /// Returns the time remaining until reset, or None if no reset time is set
    /// or if the reset time has already passed.
    pub fn time_until_reset(&self) -> Option<Duration> {
        self.reset_at.and_then(|ts| ts.duration_from_now())
    }
}

/// An absolute reset timestamp stored as Unix epoch seconds.
///
/// Using epoch seconds ensures consistent serialization and timezone-independent
/// storage, while allowing easy countdown computation at render time.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResetTimestamp {
    /// Unix timestamp in seconds
    pub epoch_seconds: i64,
}

impl ResetTimestamp {
    /// Creates a new reset timestamp from Unix epoch seconds.
    #[allow(dead_code)]
    pub fn from_epoch_seconds(seconds: i64) -> Self {
        Self {
            epoch_seconds: seconds,
        }
    }

    /// Creates a reset timestamp from the current time plus a duration.
    #[allow(dead_code)]
    pub fn from_duration_from_now(duration: Duration) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Self {
            epoch_seconds: (now + duration).as_secs() as i64,
        }
    }

    /// Returns the duration from now until this timestamp, or None if already past.
    pub fn duration_from_now(&self) -> Option<Duration> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let diff = self.epoch_seconds - now;
        if diff > 0 {
            Some(Duration::from_secs(diff as u64))
        } else {
            None
        }
    }
}

/// Formats a duration as "Xd Yh Zm" countdown string.
///
/// - Days are shown only if >= 1 day
/// - Hours are shown only if >= 1 hour
/// - Minutes are always shown (at least "0m")
/// - Seconds are not shown for cleaner display
/// - Returns "0m" if duration is zero or negative
pub fn format_countdown(duration: Option<Duration>) -> String {
    let Some(d) = duration else {
        return "0m".to_string();
    };

    let total_secs = d.as_secs();
    if total_secs == 0 {
        return "0m".to_string();
    }

    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    let minutes = (total_secs % 3600) / 60;

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{}d", days));
    }
    if hours > 0 {
        parts.push(format!("{}h", hours));
    }
    if minutes > 0 || parts.is_empty() {
        parts.push(format!("{}m", minutes));
    }

    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_usage_window_default() {
        let window = UsageWindow::default();
        assert_eq!(window.used_percent, None);
        assert_eq!(window.reset_at, None);
    }

    #[test]
    fn test_usage_window_with_percent() {
        let window = UsageWindow::with_percent(42);
        assert_eq!(window.used_percent, Some(42));
        assert_eq!(window.reset_at, None);
    }

    #[test]
    fn test_usage_window_with_percent_and_reset() {
        let ts = ResetTimestamp::from_epoch_seconds(1700000000);
        let window = UsageWindow::with_percent_and_reset(50, ts);
        assert_eq!(window.used_percent, Some(50));
        assert_eq!(window.reset_at, Some(ts));
    }

    #[test]
    fn test_reset_timestamp_from_epoch() {
        let ts = ResetTimestamp::from_epoch_seconds(1700000000);
        assert_eq!(ts.epoch_seconds, 1700000000);
    }

    #[test]
    fn test_reset_timestamp_duration_from_now_past() {
        // A timestamp in the past
        let ts = ResetTimestamp::from_epoch_seconds(0);
        assert_eq!(ts.duration_from_now(), None);
    }

    #[test]
    fn test_reset_timestamp_duration_from_now_future() {
        // A timestamp far in the future
        let ts = ResetTimestamp::from_epoch_seconds(i64::MAX / 2);
        assert!(ts.duration_from_now().is_some());
    }

    #[test]
    fn test_format_countdown_none() {
        assert_eq!(format_countdown(None), "0m");
    }

    #[test]
    fn test_format_countdown_zero() {
        assert_eq!(format_countdown(Some(Duration::from_secs(0))), "0m");
    }

    #[test]
    fn test_format_countdown_minutes_only() {
        assert_eq!(format_countdown(Some(Duration::from_secs(300))), "5m");
        assert_eq!(format_countdown(Some(Duration::from_secs(59))), "0m");
        assert_eq!(format_countdown(Some(Duration::from_secs(60))), "1m");
    }

    #[test]
    fn test_format_countdown_hours_and_minutes() {
        // When minutes are 0 and hours are present, only hours shown
        assert_eq!(format_countdown(Some(Duration::from_secs(3600))), "1h");
        assert_eq!(format_countdown(Some(Duration::from_secs(3660))), "1h 1m");
        assert_eq!(
            format_countdown(Some(Duration::from_secs(23 * 3600 + 18 * 60))),
            "23h 18m"
        );
    }

    #[test]
    fn test_format_countdown_days_hours_minutes() {
        // When minutes are 0 and higher units present, only higher units shown
        assert_eq!(format_countdown(Some(Duration::from_secs(86400))), "1d");
        assert_eq!(
            format_countdown(Some(Duration::from_secs(86400 + 3600 + 60))),
            "1d 1h 1m"
        );
        assert_eq!(
            format_countdown(Some(Duration::from_secs(2 * 86400 + 5 * 3600 + 30 * 60))),
            "2d 5h 30m"
        );
    }

    #[test]
    fn test_usage_window_serialization_roundtrip() {
        let ts = ResetTimestamp::from_epoch_seconds(1700000000);
        let window = UsageWindow::with_percent_and_reset(75, ts);

        let json = serde_json::to_string(&window).unwrap();
        let parsed: UsageWindow = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed, window);
    }
}
