use super::*;

/// Test helper: create ResetTimestamp from duration from now.
fn reset_timestamp_from_duration(duration: Duration) -> ResetTimestamp {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    ResetTimestamp::from_epoch_seconds((now + duration).as_secs() as i64)
}

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
    let window = UsageWindow::with_percent_reset_and_span(50, ts, UsageWindowSpan::Hours(5));
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
    let ts = reset_timestamp_from_duration(Duration::from_secs(3600));
    let window = UsageWindow::with_percent_and_reset(50, ts);
    assert_eq!(window.time_status(), UsageTimeStatus::Unknown);
}

#[test]
fn test_time_status_ahead() {
    // 5h window, 2.5h remaining (50% elapsed), but 70% used -> ahead
    let ts = reset_timestamp_from_duration(Duration::from_secs(2 * 3600 + 1800));
    let window = UsageWindow::with_percent_reset_and_span(70, ts, UsageWindowSpan::Hours(5));
    assert_eq!(window.time_status(), UsageTimeStatus::Ahead);
}

#[test]
fn test_time_status_behind() {
    // 5h window, 2.5h remaining (50% elapsed), but only 30% used -> behind
    let ts = reset_timestamp_from_duration(Duration::from_secs(2 * 3600 + 1800));
    let window = UsageWindow::with_percent_reset_and_span(30, ts, UsageWindowSpan::Hours(5));
    assert_eq!(window.time_status(), UsageTimeStatus::Behind);
}

#[test]
fn test_time_status_on_track() {
    // 5h window, 2.5h remaining (50% elapsed), 50% used -> on track
    let ts = reset_timestamp_from_duration(Duration::from_secs(2 * 3600 + 1800));
    let window = UsageWindow::with_percent_reset_and_span(50, ts, UsageWindowSpan::Hours(5));
    assert_eq!(window.time_status(), UsageTimeStatus::OnTrack);

    // Also on track at threshold boundaries (+/- 10%)
    let window = UsageWindow::with_percent_reset_and_span(59, ts, UsageWindowSpan::Hours(5));
    assert_eq!(window.time_status(), UsageTimeStatus::OnTrack);

    let window = UsageWindow::with_percent_reset_and_span(41, ts, UsageWindowSpan::Hours(5));
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
    assert_eq!(
        format_countdown(Some(Duration::from_secs(4 * 3600 + 5 * 60))),
        "4h 05m"
    );
    assert_eq!(
        format_countdown(Some(Duration::from_secs(23 * 3600 + 18 * 60))),
        "23h 18m"
    );
}

#[test]
fn test_format_countdown_days_hours_minutes() {
    // Days with hours only when minutes are 0
    assert_eq!(format_countdown(Some(Duration::from_secs(86400))), "1d");
    assert_eq!(
        format_countdown(Some(Duration::from_secs(86400 + 3600))),
        "1d 1h"
    );
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
    let window = UsageWindow::with_percent_reset_and_span(75, ts, UsageWindowSpan::Hours(5));

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
