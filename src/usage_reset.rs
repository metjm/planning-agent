//! Shared types for usage windows with reset timestamps.
//!
//! This module provides `UsageWindow` and `ResetTimestamp` types used across
//! all provider usage tracking (Claude, Gemini, Codex) to support countdown
//! rendering in the UI.

// Some methods/types are defined for future use in UI rendering
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Represents the span/duration of a usage window.
///
/// Used to display duration-based labels (e.g., "5h", "24h", "7d") when the
/// window duration is known, falling back to slot labels (Session/Daily/Weekly)
/// when unknown.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum UsageWindowSpan {
    /// Window duration is not known
    #[default]
    Unknown,
    /// Window duration in minutes (e.g., 300 = 5h)
    Minutes(u16),
    /// Window duration in hours (e.g., 5, 24)
    Hours(u16),
    /// Window duration in days (e.g., 1, 7)
    Days(u16),
}

impl UsageWindowSpan {
    /// Returns the duration of this window span, or None if unknown.
    pub fn duration(&self) -> Option<Duration> {
        match self {
            UsageWindowSpan::Unknown => None,
            UsageWindowSpan::Minutes(m) => Some(Duration::from_secs(*m as u64 * 60)),
            UsageWindowSpan::Hours(h) => Some(Duration::from_secs(*h as u64 * 3600)),
            UsageWindowSpan::Days(d) => Some(Duration::from_secs(*d as u64 * 86400)),
        }
    }

    /// Returns a short label for this window span (e.g., "5h", "24h", "7d").
    /// Returns None if unknown, allowing callers to use fallback labels.
    pub fn label(&self) -> Option<String> {
        match self {
            UsageWindowSpan::Unknown => None,
            UsageWindowSpan::Minutes(m) => {
                // Convert to hours if evenly divisible
                if *m >= 60 && *m % 60 == 0 {
                    Some(format!("{}h", m / 60))
                } else {
                    Some(format!("{}m", m))
                }
            }
            UsageWindowSpan::Hours(h) => {
                // Convert to days if evenly divisible and >= 24h
                if *h >= 24 && *h % 24 == 0 {
                    Some(format!("{}d", h / 24))
                } else {
                    Some(format!("{}h", h))
                }
            }
            UsageWindowSpan::Days(d) => Some(format!("{}d", d)),
        }
    }
}

/// Status of usage pace compared to time elapsed in the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageTimeStatus {
    /// Usage is outpacing time elapsed (used more than expected)
    Ahead,
    /// Usage is roughly on track (within +/- 10 percentage points)
    OnTrack,
    /// Usage is lagging behind time elapsed (used less than expected)
    Behind,
    /// Cannot determine status (missing span or reset data)
    Unknown,
}

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
    /// The duration/span of this usage window
    #[serde(default)]
    pub window_span: UsageWindowSpan,
}

impl UsageWindow {
    /// Creates a new usage window with just the percent used.
    pub fn with_percent(percent: u8) -> Self {
        Self {
            used_percent: Some(percent),
            reset_at: None,
            window_span: UsageWindowSpan::Unknown,
        }
    }

    /// Creates a new usage window with percent and reset timestamp.
    #[allow(dead_code)]
    pub fn with_percent_and_reset(percent: u8, reset_at: ResetTimestamp) -> Self {
        Self {
            used_percent: Some(percent),
            reset_at: Some(reset_at),
            window_span: UsageWindowSpan::Unknown,
        }
    }

    /// Creates a new usage window with percent, reset timestamp, and window span.
    pub fn with_percent_reset_and_span(
        percent: u8,
        reset_at: ResetTimestamp,
        span: UsageWindowSpan,
    ) -> Self {
        Self {
            used_percent: Some(percent),
            reset_at: Some(reset_at),
            window_span: span,
        }
    }

    /// Creates a new usage window with percent and window span (no reset timestamp).
    pub fn with_percent_and_span(percent: u8, span: UsageWindowSpan) -> Self {
        Self {
            used_percent: Some(percent),
            reset_at: None,
            window_span: span,
        }
    }

    /// Returns the time remaining until reset, or None if no reset time is set
    /// or if the reset time has already passed.
    pub fn time_until_reset(&self) -> Option<Duration> {
        self.reset_at.and_then(|ts| ts.duration_from_now())
    }

    /// Computes the usage time status based on usage percent vs time elapsed.
    ///
    /// Uses a +/- 10 percentage point threshold for "on track" status:
    /// - Ahead: usage percent > time elapsed percent + 10
    /// - Behind: usage percent < time elapsed percent - 10
    /// - OnTrack: within +/- 10 percentage points
    /// - Unknown: missing span, reset, or usage data
    pub fn time_status(&self) -> UsageTimeStatus {
        let Some(used_pct) = self.used_percent else {
            return UsageTimeStatus::Unknown;
        };

        let Some(window_duration) = self.window_span.duration() else {
            return UsageTimeStatus::Unknown;
        };

        let Some(remaining) = self.time_until_reset() else {
            return UsageTimeStatus::Unknown;
        };

        // Calculate time elapsed percent
        let total_secs = window_duration.as_secs_f64();
        let remaining_secs = remaining.as_secs_f64();
        let elapsed_secs = (total_secs - remaining_secs).max(0.0);
        let time_elapsed_pct = ((elapsed_secs / total_secs) * 100.0).clamp(0.0, 100.0);

        let used = used_pct as f64;
        let threshold = 10.0;

        if used > time_elapsed_pct + threshold {
            UsageTimeStatus::Ahead
        } else if used < time_elapsed_pct - threshold {
            UsageTimeStatus::Behind
        } else {
            UsageTimeStatus::OnTrack
        }
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

/// Formats a duration as "Xd Yh Zm" countdown string with consistent padding.
///
/// - Days are shown only if >= 1 day
/// - Hours are shown if >= 1 hour OR if days are shown (padded with leading zero)
/// - Minutes are always shown when hours are shown (padded with leading zero)
/// - Seconds are not shown for cleaner display
/// - Returns "0m" if duration is zero or negative
///
/// Examples: "4h 05m", "2d 3h", "1d 0h 30m", "45m"
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

    if days > 0 {
        // Show days and hours, with minutes if non-zero
        if minutes > 0 {
            format!("{}d {}h {:02}m", days, hours, minutes)
        } else if hours > 0 {
            format!("{}d {}h", days, hours)
        } else {
            format!("{}d", days)
        }
    } else if hours > 0 {
        // Show hours and minutes with zero-padding for minutes
        format!("{}h {:02}m", hours, minutes)
    } else {
        // Minutes only
        format!("{}m", minutes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_usage_window_default() {
        let window = UsageWindow::default();
        assert_eq!(window.used_percent, None);
        assert_eq!(window.reset_at, None);
        assert_eq!(window.window_span, UsageWindowSpan::Unknown);
    }

    #[test]
    fn test_usage_window_with_percent() {
        let window = UsageWindow::with_percent(42);
        assert_eq!(window.used_percent, Some(42));
        assert_eq!(window.reset_at, None);
        assert_eq!(window.window_span, UsageWindowSpan::Unknown);
    }

    #[test]
    fn test_usage_window_with_percent_and_reset() {
        let ts = ResetTimestamp::from_epoch_seconds(1700000000);
        let window = UsageWindow::with_percent_and_reset(50, ts);
        assert_eq!(window.used_percent, Some(50));
        assert_eq!(window.reset_at, Some(ts));
        assert_eq!(window.window_span, UsageWindowSpan::Unknown);
    }

    #[test]
    fn test_usage_window_with_percent_reset_and_span() {
        let ts = ResetTimestamp::from_epoch_seconds(1700000000);
        let window =
            UsageWindow::with_percent_reset_and_span(50, ts, UsageWindowSpan::Hours(5));
        assert_eq!(window.used_percent, Some(50));
        assert_eq!(window.reset_at, Some(ts));
        assert_eq!(window.window_span, UsageWindowSpan::Hours(5));
    }

    #[test]
    fn test_usage_window_with_percent_and_span() {
        let window = UsageWindow::with_percent_and_span(50, UsageWindowSpan::Days(7));
        assert_eq!(window.used_percent, Some(50));
        assert_eq!(window.reset_at, None);
        assert_eq!(window.window_span, UsageWindowSpan::Days(7));
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

    // UsageWindowSpan tests

    #[test]
    fn test_window_span_unknown() {
        let span = UsageWindowSpan::Unknown;
        assert_eq!(span.duration(), None);
        assert_eq!(span.label(), None);
    }

    #[test]
    fn test_window_span_minutes() {
        // 300 minutes = 5 hours
        let span = UsageWindowSpan::Minutes(300);
        assert_eq!(span.duration(), Some(Duration::from_secs(300 * 60)));
        assert_eq!(span.label(), Some("5h".to_string()));

        // 45 minutes (not evenly divisible by 60)
        let span = UsageWindowSpan::Minutes(45);
        assert_eq!(span.duration(), Some(Duration::from_secs(45 * 60)));
        assert_eq!(span.label(), Some("45m".to_string()));
    }

    #[test]
    fn test_window_span_hours() {
        let span = UsageWindowSpan::Hours(5);
        assert_eq!(span.duration(), Some(Duration::from_secs(5 * 3600)));
        assert_eq!(span.label(), Some("5h".to_string()));

        // 24 hours = 1 day
        let span = UsageWindowSpan::Hours(24);
        assert_eq!(span.duration(), Some(Duration::from_secs(24 * 3600)));
        assert_eq!(span.label(), Some("1d".to_string()));

        // 168 hours = 7 days
        let span = UsageWindowSpan::Hours(168);
        assert_eq!(span.duration(), Some(Duration::from_secs(168 * 3600)));
        assert_eq!(span.label(), Some("7d".to_string()));
    }

    #[test]
    fn test_window_span_days() {
        let span = UsageWindowSpan::Days(1);
        assert_eq!(span.duration(), Some(Duration::from_secs(86400)));
        assert_eq!(span.label(), Some("1d".to_string()));

        let span = UsageWindowSpan::Days(7);
        assert_eq!(span.duration(), Some(Duration::from_secs(7 * 86400)));
        assert_eq!(span.label(), Some("7d".to_string()));
    }

    // UsageTimeStatus tests

    #[test]
    fn test_time_status_unknown_no_percent() {
        let window = UsageWindow::default();
        assert_eq!(window.time_status(), UsageTimeStatus::Unknown);
    }

    #[test]
    fn test_time_status_unknown_no_span() {
        let ts = ResetTimestamp::from_duration_from_now(Duration::from_secs(3600));
        let window = UsageWindow::with_percent_and_reset(50, ts);
        assert_eq!(window.time_status(), UsageTimeStatus::Unknown);
    }

    #[test]
    fn test_time_status_ahead() {
        // 5h window, 2.5h remaining (50% elapsed), but 70% used -> ahead
        let ts = ResetTimestamp::from_duration_from_now(Duration::from_secs(2 * 3600 + 1800));
        let window =
            UsageWindow::with_percent_reset_and_span(70, ts, UsageWindowSpan::Hours(5));
        assert_eq!(window.time_status(), UsageTimeStatus::Ahead);
    }

    #[test]
    fn test_time_status_behind() {
        // 5h window, 2.5h remaining (50% elapsed), but only 30% used -> behind
        let ts = ResetTimestamp::from_duration_from_now(Duration::from_secs(2 * 3600 + 1800));
        let window =
            UsageWindow::with_percent_reset_and_span(30, ts, UsageWindowSpan::Hours(5));
        assert_eq!(window.time_status(), UsageTimeStatus::Behind);
    }

    #[test]
    fn test_time_status_on_track() {
        // 5h window, 2.5h remaining (50% elapsed), 50% used -> on track
        let ts = ResetTimestamp::from_duration_from_now(Duration::from_secs(2 * 3600 + 1800));
        let window =
            UsageWindow::with_percent_reset_and_span(50, ts, UsageWindowSpan::Hours(5));
        assert_eq!(window.time_status(), UsageTimeStatus::OnTrack);

        // Also on track at threshold boundaries (+/- 10%)
        let window =
            UsageWindow::with_percent_reset_and_span(59, ts, UsageWindowSpan::Hours(5));
        assert_eq!(window.time_status(), UsageTimeStatus::OnTrack);

        let window =
            UsageWindow::with_percent_reset_and_span(41, ts, UsageWindowSpan::Hours(5));
        assert_eq!(window.time_status(), UsageTimeStatus::OnTrack);
    }

    // format_countdown tests

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
        assert_eq!(format_countdown(Some(Duration::from_secs(45 * 60))), "45m");
    }

    #[test]
    fn test_format_countdown_hours_and_minutes() {
        // Hours with zero-padded minutes
        assert_eq!(format_countdown(Some(Duration::from_secs(3600))), "1h 00m");
        assert_eq!(format_countdown(Some(Duration::from_secs(3660))), "1h 01m");
        assert_eq!(format_countdown(Some(Duration::from_secs(4 * 3600 + 5 * 60))), "4h 05m");
        assert_eq!(
            format_countdown(Some(Duration::from_secs(23 * 3600 + 18 * 60))),
            "23h 18m"
        );
    }

    #[test]
    fn test_format_countdown_days_hours_minutes() {
        // Days with hours only when minutes are 0
        assert_eq!(format_countdown(Some(Duration::from_secs(86400))), "1d");
        assert_eq!(format_countdown(Some(Duration::from_secs(86400 + 3600))), "1d 1h");
        // Days with hours and zero-padded minutes
        assert_eq!(
            format_countdown(Some(Duration::from_secs(86400 + 3600 + 60))),
            "1d 1h 01m"
        );
        assert_eq!(
            format_countdown(Some(Duration::from_secs(2 * 86400 + 5 * 3600 + 30 * 60))),
            "2d 5h 30m"
        );
        // Days with no hours but with minutes
        assert_eq!(
            format_countdown(Some(Duration::from_secs(86400 + 30 * 60))),
            "1d 0h 30m"
        );
    }

    #[test]
    fn test_usage_window_serialization_roundtrip() {
        let ts = ResetTimestamp::from_epoch_seconds(1700000000);
        let window =
            UsageWindow::with_percent_reset_and_span(75, ts, UsageWindowSpan::Hours(5));

        let json = serde_json::to_string(&window).unwrap();
        let parsed: UsageWindow = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed, window);
    }

    #[test]
    fn test_usage_window_deserialization_default_span() {
        // Test that old JSON without window_span deserializes with Unknown
        let json = r#"{"used_percent":50,"reset_at":{"epoch_seconds":1700000000}}"#;
        let parsed: UsageWindow = serde_json::from_str(json).unwrap();

        assert_eq!(parsed.used_percent, Some(50));
        assert_eq!(parsed.window_span, UsageWindowSpan::Unknown);
    }

    #[test]
    fn test_window_span_serialization() {
        let span = UsageWindowSpan::Hours(5);
        let json = serde_json::to_string(&span).unwrap();
        let parsed: UsageWindowSpan = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, span);

        let span = UsageWindowSpan::Unknown;
        let json = serde_json::to_string(&span).unwrap();
        let parsed: UsageWindowSpan = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, span);
    }
}
