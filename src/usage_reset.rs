//! Shared types for usage windows with reset timestamps.
//!
//! This module provides `UsageWindow` and `ResetTimestamp` types used across
//! all provider usage tracking (Claude, Gemini, Codex) to support countdown
//! rendering in the UI.

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
    #[cfg(test)]
    pub fn with_percent(percent: u8) -> Self {
        Self {
            used_percent: Some(percent),
            reset_at: None,
            window_span: UsageWindowSpan::Unknown,
        }
    }

    /// Creates a new usage window with percent and reset timestamp.
    #[cfg(test)]
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
    pub fn from_epoch_seconds(seconds: i64) -> Self {
        Self {
            epoch_seconds: seconds,
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
#[path = "tests/usage_reset_tests.rs"]
mod tests;
