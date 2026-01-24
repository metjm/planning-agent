//! HTTP API clients for fetching usage from providers.

use super::types::{AccountId, AccountUsageState, ProviderCredentials};
use crate::usage_reset::{ResetTimestamp, UsageWindow, UsageWindowSpan};
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::time::Duration;

const API_TIMEOUT: Duration = Duration::from_secs(15);

/// Fetch result that includes token validity info even on failure.
pub struct FetchResult {
    pub usage: Option<AccountUsageState>,
    pub token_valid: bool,
    pub error: Option<String>,
}

pub fn fetch_claude_usage(access_token: &str) -> FetchResult {
    match fetch_claude_usage_inner(access_token) {
        Ok(usage) => FetchResult {
            usage: Some(usage),
            token_valid: true,
            error: None,
        },
        Err(e) => {
            // Check if error indicates invalid token (401/403)
            let error_str = e.to_string();
            let token_valid = !error_str.contains("401")
                && !error_str.contains("403")
                && !error_str.contains("unauthorized")
                && !error_str.contains("Unauthorized");
            FetchResult {
                usage: None,
                token_valid,
                error: Some(error_str),
            }
        }
    }
}

fn fetch_claude_usage_inner(access_token: &str) -> Result<AccountUsageState> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(API_TIMEOUT))
        .build()
        .into();

    // Fetch profile for email
    let profile_body: String = agent
        .get("https://api.anthropic.com/api/oauth/profile")
        .header("Authorization", &format!("Bearer {}", access_token))
        .call()
        .context("Failed to fetch Claude profile")?
        .body_mut()
        .read_to_string()
        .context("Failed to read profile response")?;

    let profile: serde_json::Value = serde_json::from_str(&profile_body)?;
    let email = profile["account"]["email"]
        .as_str()
        .context("Missing email in profile")?
        .to_string();
    let plan_type = profile["organization"]["organization_type"]
        .as_str()
        .map(String::from);
    let rate_limit_tier = profile["account"]["rate_limit_tier"]
        .as_str()
        .map(String::from);

    // Fetch usage
    let usage_body: String = agent
        .get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", &format!("Bearer {}", access_token))
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("Content-Type", "application/json")
        .call()
        .context("Failed to fetch Claude usage")?
        .body_mut()
        .read_to_string()
        .context("Failed to read usage response")?;

    let usage: serde_json::Value = serde_json::from_str(&usage_body)?;

    let session_window = parse_claude_window(&usage["five_hour"], UsageWindowSpan::Hours(5));
    let weekly_window = parse_claude_window(&usage["seven_day"], UsageWindowSpan::Days(7));

    Ok(AccountUsageState {
        account_id: AccountId::new(&email),
        provider: "claude".to_string(),
        email,
        plan_type,
        rate_limit_tier,
        session_window,
        weekly_window,
        fetched_at: chrono::Utc::now().to_rfc3339(),
        error: None,
        token_valid: true,
    })
}

fn parse_claude_window(value: &serde_json::Value, span: UsageWindowSpan) -> UsageWindow {
    if value.is_null() {
        return UsageWindow::default();
    }

    let used_percent = value["utilization"].as_u64().map(|u| u.min(100) as u8);
    let reset_at = value["resets_at"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| ResetTimestamp::from_epoch_seconds(dt.timestamp()));

    match (used_percent, reset_at) {
        (Some(pct), Some(ts)) => UsageWindow::with_percent_reset_and_span(pct, ts, span),
        (Some(pct), None) => UsageWindow::with_percent_and_span(pct, span),
        _ => UsageWindow::default(),
    }
}

/// Fetches Gemini usage via the cloudcode quota API.
pub fn fetch_gemini_usage(access_token: &str) -> FetchResult {
    match fetch_gemini_usage_inner(access_token) {
        Ok(usage) => FetchResult {
            usage: Some(usage),
            token_valid: true,
            error: None,
        },
        Err(e) => {
            let error_str = e.to_string();
            let token_valid = !error_str.contains("401")
                && !error_str.contains("403")
                && !error_str.contains("unauthorized")
                && !error_str.contains("Unauthorized");
            FetchResult {
                usage: None,
                token_valid,
                error: Some(error_str),
            }
        }
    }
}

fn fetch_gemini_usage_inner(access_token: &str) -> Result<AccountUsageState> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(API_TIMEOUT))
        .build()
        .into();

    // Fetch user email from Google UserInfo endpoint
    let userinfo_body: String = agent
        .get("https://www.googleapis.com/oauth2/v2/userinfo")
        .header("Authorization", &format!("Bearer {}", access_token))
        .call()
        .context("Failed to fetch Gemini user info")?
        .body_mut()
        .read_to_string()
        .context("Failed to read userinfo response")?;

    let userinfo: serde_json::Value = serde_json::from_str(&userinfo_body)?;
    let email = userinfo["email"]
        .as_str()
        .context("Missing email in userinfo")?
        .to_string();

    // Fetch quota from cloudcode API
    let request_body = serde_json::json!({});
    let request_body_str =
        serde_json::to_string(&request_body).context("Failed to serialize request body")?;

    let quota_body: String = agent
        .post("https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota")
        .header("Authorization", &format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .send(&request_body_str)
        .context("Failed to fetch Gemini quota")?
        .body_mut()
        .read_to_string()
        .context("Failed to read quota response")?;

    let quota: serde_json::Value = serde_json::from_str(&quota_body)?;

    // Parse quota buckets - find lowest remaining fraction (most restrictive)
    let buckets = quota["buckets"].as_array();
    let (used_percent, reset_at) = parse_gemini_buckets(buckets);

    // Gemini uses daily quota, so map to session window (no weekly separate)
    let session_window = match (used_percent, reset_at) {
        (Some(pct), Some(ts)) => {
            UsageWindow::with_percent_reset_and_span(pct, ts, UsageWindowSpan::Hours(24))
        }
        (Some(pct), None) => UsageWindow::with_percent_and_span(pct, UsageWindowSpan::Hours(24)),
        _ => UsageWindow::default(),
    };

    Ok(AccountUsageState {
        account_id: AccountId::new(&email),
        provider: "gemini".to_string(),
        email,
        plan_type: None, // Gemini doesn't expose plan type in quota API
        rate_limit_tier: None,
        session_window,
        weekly_window: UsageWindow::default(), // Gemini only has daily quota
        fetched_at: chrono::Utc::now().to_rfc3339(),
        error: None,
        token_valid: true,
    })
}

/// Parses Gemini quota buckets to find the most restrictive usage.
fn parse_gemini_buckets(
    buckets: Option<&Vec<serde_json::Value>>,
) -> (Option<u8>, Option<ResetTimestamp>) {
    let buckets = match buckets {
        Some(b) if !b.is_empty() => b,
        _ => return (None, None),
    };

    // Find bucket with lowest remainingFraction (most used)
    let mut lowest_remaining: Option<f64> = None;
    let mut reset_time: Option<ResetTimestamp> = None;

    for bucket in buckets {
        // Only consider REQUESTS type for now
        let token_type = bucket["tokenType"].as_str().unwrap_or("");
        if token_type != "REQUESTS" {
            continue;
        }

        if let Some(remaining) = bucket["remainingFraction"].as_f64() {
            let is_lower = lowest_remaining.is_none_or(|prev| remaining < prev);
            if is_lower {
                lowest_remaining = Some(remaining);

                // Parse reset time for this bucket
                if let Some(reset_str) = bucket["resetTime"].as_str() {
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(reset_str) {
                        reset_time = Some(ResetTimestamp::from_epoch_seconds(dt.timestamp()));
                    }
                }
            }
        }
    }

    // Convert remaining fraction to used percent (0.75 remaining = 25% used)
    let used_percent = lowest_remaining.map(|r| ((1.0 - r) * 100.0).round() as u8);

    (used_percent.map(|p| p.min(100)), reset_time)
}

/// Fetches Codex usage by parsing session files.
/// This is the primary approach - doesn't consume API quota.
pub fn fetch_codex_usage_from_sessions() -> FetchResult {
    match fetch_codex_usage_from_sessions_inner() {
        Ok(usage) => FetchResult {
            usage: Some(usage),
            token_valid: true,
            error: None,
        },
        Err(e) => FetchResult {
            usage: None,
            token_valid: true, // Session parse failure doesn't indicate token issue
            error: Some(e.to_string()),
        },
    }
}

fn fetch_codex_usage_from_sessions_inner() -> Result<AccountUsageState> {
    let creds =
        super::credentials::read_codex_credentials()?.context("No Codex credentials found")?;

    let ProviderCredentials::Codex {
        access_token,
        account_id: _,
    } = creds
    else {
        anyhow::bail!("Invalid credential type");
    };

    // Extract email from JWT access_token
    let email = super::credentials::extract_email_from_jwt(&access_token)
        .context("Failed to extract email from Codex token")?;

    // Parse session files for usage data
    let sessions_dir = codex_sessions_dir()?;
    let (session_window, weekly_window, plan_type) = parse_codex_session_files(&sessions_dir)?;

    Ok(AccountUsageState {
        account_id: AccountId::new(&email),
        provider: "codex".to_string(),
        email,
        plan_type,
        rate_limit_tier: None,
        session_window,
        weekly_window,
        fetched_at: chrono::Utc::now().to_rfc3339(),
        error: None,
        token_valid: true,
    })
}

fn codex_sessions_dir() -> Result<PathBuf> {
    let config_dir = std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .ok()
        .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
        .context("Cannot determine Codex config directory")?;
    Ok(config_dir.join("sessions"))
}

/// Parses Codex session JSONL files for the most recent usage data.
fn parse_codex_session_files(
    sessions_dir: &PathBuf,
) -> Result<(UsageWindow, UsageWindow, Option<String>)> {
    if !sessions_dir.exists() {
        return Ok((UsageWindow::default(), UsageWindow::default(), None));
    }

    // Find all .jsonl files
    let mut session_files: Vec<_> = std::fs::read_dir(sessions_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();

    // Sort by modification time (most recent first)
    session_files.sort_by(|a, b| {
        let a_time = std::fs::metadata(a)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let b_time = std::fs::metadata(b)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        b_time.cmp(&a_time)
    });

    // Try to find rate limit data in the most recent files
    for path in session_files.iter().take(10) {
        if let Ok(result) = parse_single_session_file(path) {
            return Ok(result);
        }
    }

    Ok((UsageWindow::default(), UsageWindow::default(), None))
}

fn parse_single_session_file(path: &PathBuf) -> Result<(UsageWindow, UsageWindow, Option<String>)> {
    let content = std::fs::read_to_string(path)?;

    for line in content.lines().rev() {
        // Each line is a JSON object
        let entry: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Look for event_msg with rate_limits
        if entry["type"].as_str() != Some("event_msg") {
            continue;
        }

        let payload = &entry["payload"];
        if payload["payload_type"].as_str() != Some("token_count") {
            continue;
        }

        let rate_limits = &payload["rate_limits"];
        if rate_limits.is_null() {
            continue;
        }

        // Parse primary (5h) window
        let primary = &rate_limits["primary"];
        let session_window = parse_codex_rate_limit(primary, 300, UsageWindowSpan::Hours(5));

        // Parse secondary (7d) window
        let secondary = &rate_limits["secondary"];
        let weekly_window = parse_codex_rate_limit(secondary, 10080, UsageWindowSpan::Days(7));

        return Ok((session_window, weekly_window, None));
    }

    anyhow::bail!("No rate limit data found in session file")
}

fn parse_codex_rate_limit(
    limit: &serde_json::Value,
    expected_window_minutes: u32,
    span: UsageWindowSpan,
) -> UsageWindow {
    if limit.is_null() {
        return UsageWindow::default();
    }

    let used_percent = limit["used_percent"]
        .as_f64()
        .map(|p| (p * 100.0).round() as u8);

    let window_minutes = limit["window_minutes"].as_u64().unwrap_or(0) as u32;

    // Only capture reset_at if window matches expected
    let reset_at = if window_minutes == expected_window_minutes {
        limit["resets_at"]
            .as_i64()
            .map(ResetTimestamp::from_epoch_seconds)
    } else {
        None
    };

    match (used_percent, reset_at) {
        (Some(p), Some(t)) => UsageWindow::with_percent_reset_and_span(p.min(100), t, span),
        (Some(p), None) => UsageWindow::with_percent_and_span(p.min(100), span),
        _ => UsageWindow::default(),
    }
}

/// Fetches usage for a provider using its credentials.
pub fn fetch_usage_for_provider(provider: &str, creds: &ProviderCredentials) -> FetchResult {
    match (provider, creds) {
        ("claude", ProviderCredentials::Claude { access_token, .. }) => {
            fetch_claude_usage(access_token)
        }
        ("gemini", ProviderCredentials::Gemini { access_token, .. }) => {
            fetch_gemini_usage(access_token)
        }
        ("codex", ProviderCredentials::Codex { .. }) => fetch_codex_usage_from_sessions(),
        _ => FetchResult {
            usage: None,
            token_valid: false,
            error: Some(format!("Unknown provider: {}", provider)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_gemini_buckets_empty() {
        let (pct, ts) = parse_gemini_buckets(None);
        assert_eq!(pct, None);
        assert_eq!(ts, None);
    }

    #[test]
    fn test_parse_gemini_buckets_full_quota() {
        let buckets = vec![serde_json::json!({
            "tokenType": "REQUESTS",
            "modelId": "gemini-2.5-pro",
            "remainingFraction": 1.0,
            "resetTime": "2026-01-25T14:35:08Z"
        })];

        let (pct, ts) = parse_gemini_buckets(Some(&buckets));
        assert_eq!(pct, Some(0)); // 1.0 remaining = 0% used
        assert!(ts.is_some());
    }

    #[test]
    fn test_parse_gemini_buckets_partial_usage() {
        let buckets = vec![
            serde_json::json!({
                "tokenType": "REQUESTS",
                "modelId": "gemini-2.5-flash",
                "remainingFraction": 0.75,
                "resetTime": "2026-01-25T14:35:08Z"
            }),
            serde_json::json!({
                "tokenType": "REQUESTS",
                "modelId": "gemini-2.5-pro",
                "remainingFraction": 0.50,
                "resetTime": "2026-01-25T14:35:08Z"
            }),
        ];

        let (pct, ts) = parse_gemini_buckets(Some(&buckets));
        // Should take the lowest remaining (0.50 = 50% used)
        assert_eq!(pct, Some(50));
        assert!(ts.is_some());
    }

    #[test]
    fn test_parse_codex_rate_limit_null() {
        let limit = serde_json::Value::Null;
        let window = parse_codex_rate_limit(&limit, 300, UsageWindowSpan::Hours(5));
        assert_eq!(window.used_percent, None);
    }

    #[test]
    fn test_parse_codex_rate_limit_valid() {
        let limit = serde_json::json!({
            "used_percent": 0.25,
            "window_minutes": 300,
            "resets_at": 1769283296
        });
        let window = parse_codex_rate_limit(&limit, 300, UsageWindowSpan::Hours(5));
        assert_eq!(window.used_percent, Some(25));
        assert!(window.reset_at.is_some());
    }

    #[test]
    fn test_parse_codex_rate_limit_wrong_window() {
        let limit = serde_json::json!({
            "used_percent": 0.50,
            "window_minutes": 600,  // Wrong window
            "resets_at": 1769283296
        });
        let window = parse_codex_rate_limit(&limit, 300, UsageWindowSpan::Hours(5));
        // Should have percent but no reset_at due to window mismatch
        assert_eq!(window.used_percent, Some(50));
        assert!(window.reset_at.is_none());
    }
}
