//! Usage extrapolation logic for displaying cached account data.
//!
//! When an API fetch fails but we have cached successful data, this module
//! provides extrapolation logic to show meaningful usage information.

use crate::account_usage::types::{AccountRecord, AccountUsageState};
use crate::usage_reset::UsageWindow;

use super::helpers::format_relative_time;

/// Extrapolates current usage based on cached data and elapsed time.
///
/// Returns the extrapolated percentage, accounting for window resets.
/// If the reset time has passed, returns 0 (usage has reset).
pub fn extrapolate_usage(window: &UsageWindow) -> Option<u8> {
    let percent = window.used_percent?;
    let reset_at = window.reset_at?;

    // Check if the window has reset since the data was fetched
    // duration_from_now() returns None if the timestamp is in the past
    if reset_at.duration_from_now().is_none() {
        // Reset time has passed - usage has reset to 0
        return Some(0);
    }

    // Reset hasn't happened yet - return cached value
    // (Conservative: we don't try to extrapolate growth, only resets)
    Some(percent)
}

/// Gets the usage state to display, preferring last_successful_usage when current has error.
///
/// Returns `(usage_state, is_stale)` where `is_stale` indicates we're using fallback data.
pub fn get_display_usage(record: &AccountRecord) -> Option<(&AccountUsageState, bool)> {
    match &record.current_usage {
        Some(usage) if usage.error.is_none() => {
            // Fresh successful data
            Some((usage, false))
        }
        Some(_) => {
            // Current fetch failed - try to use last successful state
            record.last_successful_usage.as_ref().map(|u| (u, true))
        }
        None => {
            // No current usage - try last successful
            record.last_successful_usage.as_ref().map(|u| (u, true))
        }
    }
}

/// Formats staleness reason for display.
///
/// Reuses the existing `format_relative_time()` helper from `helpers.rs`.
pub fn format_stale_reason(
    last_successful_fetch: Option<&str>,
    token_valid: bool,
) -> Option<String> {
    let last_fetch = last_successful_fetch?;
    let ago = format_relative_time(last_fetch);

    if !token_valid {
        Some(format!("Credentials expired - data {}", ago))
    } else {
        Some(format!("Fetch failed - data {}", ago))
    }
}

#[cfg(test)]
#[path = "tests/usage_extrapolation_tests.rs"]
mod tests;
