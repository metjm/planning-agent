//! Tests for usage extrapolation logic.

use super::*;
use crate::account_usage::types::{AccountId, AccountRecord, AccountUsageState};
use crate::usage_reset::{ResetTimestamp, UsageWindow, UsageWindowSpan};
use std::time::{SystemTime, UNIX_EPOCH};

/// Helper to get current Unix timestamp.
fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Creates a test UsageWindow with given percent and reset_at offset from now.
fn make_window(percent: u8, seconds_from_now: i64) -> UsageWindow {
    let reset_at = ResetTimestamp::from_epoch_seconds(now_epoch() + seconds_from_now);
    UsageWindow::with_percent_reset_and_span(percent, reset_at, UsageWindowSpan::Hours(5))
}

/// Creates a test AccountUsageState with given windows.
fn make_usage_state(
    session_pct: u8,
    session_reset_offset: i64,
    weekly_pct: u8,
    weekly_reset_offset: i64,
    error: Option<String>,
) -> AccountUsageState {
    let token_valid = error.is_none();
    AccountUsageState {
        account_id: AccountId::new("claude", "test@example.com"),
        provider: "claude".to_string(),
        email: "test@example.com".to_string(),
        plan_type: Some("pro".to_string()),
        rate_limit_tier: None,
        session_window: make_window(session_pct, session_reset_offset),
        weekly_window: make_window(weekly_pct, weekly_reset_offset),
        fetched_at: chrono::Utc::now().to_rfc3339(),
        error,
        token_valid,
    }
}

/// Creates a test AccountRecord with given usage states.
fn make_record(
    current: Option<AccountUsageState>,
    last_successful: Option<AccountUsageState>,
    last_fetch: Option<String>,
) -> AccountRecord {
    AccountRecord {
        account_id: AccountId::new("claude", "test@example.com"),
        provider: "claude".to_string(),
        email: "test@example.com".to_string(),
        plan_type: Some("pro".to_string()),
        first_seen: chrono::Utc::now().to_rfc3339(),
        last_successful_fetch: last_fetch,
        current_usage: current,
        last_successful_usage: last_successful,
        history: Vec::new(),
        credentials_available: true,
        seen_in_containers: Vec::new(),
    }
}

#[test]
fn extrapolate_usage_returns_zero_after_reset() {
    // Window reset 1 hour ago
    let window = make_window(50, -3600);
    let result = extrapolate_usage(&window);
    assert_eq!(
        result,
        Some(0),
        "Should return 0 after reset time has passed"
    );
}

#[test]
fn extrapolate_usage_preserves_value_before_reset() {
    // Window resets in 1 hour
    let window = make_window(50, 3600);
    let result = extrapolate_usage(&window);
    assert_eq!(result, Some(50), "Should preserve value before reset");
}

#[test]
fn extrapolate_usage_returns_none_without_percent() {
    let window = UsageWindow::default();
    let result = extrapolate_usage(&window);
    assert_eq!(result, None, "Should return None without percent");
}

#[test]
fn extrapolate_usage_returns_none_without_reset_at() {
    let window = UsageWindow::with_percent(50);
    let result = extrapolate_usage(&window);
    assert_eq!(result, None, "Should return None without reset_at");
}

#[test]
fn extrapolate_usage_boundary_reset_at_now() {
    // Reset at exactly now (within tolerance)
    let window = make_window(50, 0);
    let result = extrapolate_usage(&window);
    // At boundary, duration_from_now returns None (0 is not > 0)
    assert_eq!(result, Some(0), "Should return 0 at boundary");
}

#[test]
fn get_display_usage_returns_fresh_data_when_no_error() {
    let current = make_usage_state(50, 3600, 30, 86400, None);
    let record = make_record(Some(current), None, None);

    let result = get_display_usage(&record);
    assert!(result.is_some(), "Should return usage");

    let (usage, is_stale) = result.unwrap();
    assert!(!is_stale, "Should not be stale");
    assert_eq!(usage.session_window.used_percent, Some(50));
}

#[test]
fn get_display_usage_returns_fallback_when_current_has_error() {
    let current = make_usage_state(0, 3600, 0, 86400, Some("auth error".to_string()));
    let last_successful = make_usage_state(50, 3600, 30, 86400, None);
    let record = make_record(
        Some(current),
        Some(last_successful),
        Some(chrono::Utc::now().to_rfc3339()),
    );

    let result = get_display_usage(&record);
    assert!(result.is_some(), "Should return usage");

    let (usage, is_stale) = result.unwrap();
    assert!(is_stale, "Should be stale");
    assert_eq!(usage.session_window.used_percent, Some(50));
}

#[test]
fn get_display_usage_returns_none_when_no_fallback_exists() {
    let current = make_usage_state(0, 3600, 0, 86400, Some("auth error".to_string()));
    let record = make_record(Some(current), None, None);

    let result = get_display_usage(&record);
    assert!(result.is_none(), "Should return None without fallback");
}

#[test]
fn get_display_usage_returns_fallback_when_no_current() {
    let last_successful = make_usage_state(50, 3600, 30, 86400, None);
    let record = make_record(
        None,
        Some(last_successful),
        Some(chrono::Utc::now().to_rfc3339()),
    );

    let result = get_display_usage(&record);
    assert!(result.is_some(), "Should return usage");

    let (usage, is_stale) = result.unwrap();
    assert!(is_stale, "Should be stale");
    assert_eq!(usage.session_window.used_percent, Some(50));
}

#[test]
fn format_stale_reason_with_expired_credentials() {
    let two_hours_ago = chrono::Utc::now()
        .checked_sub_signed(chrono::Duration::hours(2))
        .unwrap()
        .to_rfc3339();

    let result = format_stale_reason(Some(&two_hours_ago), false);
    assert!(result.is_some());
    let reason = result.unwrap();
    assert!(
        reason.contains("Credentials expired"),
        "Should mention credentials: {}",
        reason
    );
    assert!(
        reason.contains("ago"),
        "Should contain relative time: {}",
        reason
    );
}

#[test]
fn format_stale_reason_with_fetch_failure() {
    let thirty_minutes_ago = chrono::Utc::now()
        .checked_sub_signed(chrono::Duration::minutes(30))
        .unwrap()
        .to_rfc3339();

    let result = format_stale_reason(Some(&thirty_minutes_ago), true);
    assert!(result.is_some());
    let reason = result.unwrap();
    assert!(
        reason.contains("Fetch failed"),
        "Should mention fetch failure: {}",
        reason
    );
    assert!(
        reason.contains("ago"),
        "Should contain relative time: {}",
        reason
    );
}

#[test]
fn format_stale_reason_returns_none_without_timestamp() {
    let result = format_stale_reason(None, false);
    assert!(result.is_none(), "Should return None without timestamp");
}

#[test]
fn end_to_end_extrapolation_with_stale_data_after_reset() {
    // Create record with last_successful_usage having reset_at in the past
    let last_successful = make_usage_state(75, -3600, 40, -7200, None); // Both resets in past
    let current = make_usage_state(0, 0, 0, 0, Some("auth error".to_string()));
    let record = make_record(
        Some(current),
        Some(last_successful),
        Some(chrono::Utc::now().to_rfc3339()),
    );

    let (usage, is_stale) = get_display_usage(&record).unwrap();
    assert!(is_stale);

    // Extrapolate should return 0 because reset has passed
    let session_pct = extrapolate_usage(&usage.session_window);
    let weekly_pct = extrapolate_usage(&usage.weekly_window);

    assert_eq!(session_pct, Some(0), "Session should be 0 after reset");
    assert_eq!(weekly_pct, Some(0), "Weekly should be 0 after reset");
}
