//! HTTP API clients for fetching usage from providers.

use super::types::{AccountId, AccountUsageState, ProviderCredentials};
use crate::usage_reset::{ResetTimestamp, UsageWindow, UsageWindowSpan};
use anyhow::{Context, Result};
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
    let rate_limit_tier = profile["organization"]["rate_limit_tier"]
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

    // utilization can be float (74.0) or int (74), use as_f64 which handles both
    let used_percent = value["utilization"]
        .as_f64()
        .map(|u| u.round().clamp(0.0, 100.0) as u8);
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

/// Fetches Codex usage via API call.
/// Makes a minimal API request to get usage from response headers.
pub fn fetch_codex_usage(access_token: &str, account_id: &str) -> FetchResult {
    match fetch_codex_usage_inner(access_token, account_id) {
        Ok(usage) => FetchResult {
            usage: Some(usage),
            token_valid: true,
            error: None,
        },
        Err(e) => {
            let error_str = format!("{:#}", e); // anyhow alternate format for error chain
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

fn fetch_codex_usage_inner(access_token: &str, account_id: &str) -> Result<AccountUsageState> {
    // Extract email and plan from JWT
    let email = super::credentials::extract_email_from_jwt(access_token)
        .context("Failed to extract email from Codex token")?;
    let plan_type = extract_codex_plan_from_jwt(access_token);

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(API_TIMEOUT))
        .build()
        .into();

    // Minimal API request using mini model to get usage headers
    // Note: stream=true is required, API returns 400 without it
    let request_body = serde_json::json!({
        "model": "gpt-5.1-codex-mini",
        "instructions": ".",
        "input": [{"role": "user", "content": [{"type": "input_text", "text": "."}]}],
        "store": false,
        "stream": true
    });
    let request_body_str =
        serde_json::to_string(&request_body).context("Failed to serialize request body")?;

    let response = agent
        .post("https://chatgpt.com/backend-api/codex/responses")
        .header("Authorization", &format!("Bearer {}", access_token))
        .header("chatgpt-account-id", account_id)
        .header("Content-Type", "application/json")
        .header("user-agent", "codex_exec/0.89.0 (Linux)")
        .header("originator", "codex_exec")
        .send(&request_body_str)
        .context("Failed to fetch Codex usage")?;

    // Parse usage from response headers
    let session_window = parse_codex_usage_headers(&response, "primary", UsageWindowSpan::Hours(5));
    let weekly_window = parse_codex_usage_headers(&response, "secondary", UsageWindowSpan::Days(7));

    // Get plan type from header if not in JWT
    let plan_type = plan_type.or_else(|| {
        response
            .headers()
            .get("x-codex-plan-type")
            .and_then(|v: &ureq::http::HeaderValue| v.to_str().ok())
            .map(String::from)
    });

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

/// Extracts plan type from Codex JWT token.
fn extract_codex_plan_from_jwt(token: &str) -> Option<String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    use base64::Engine;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&payload).ok()?;

    json["https://api.openai.com/auth"]["chatgpt_plan_type"]
        .as_str()
        .map(String::from)
}

/// Parses usage from Codex response headers.
fn parse_codex_usage_headers(
    response: &ureq::http::Response<ureq::Body>,
    window: &str,
    span: UsageWindowSpan,
) -> UsageWindow {
    let used_percent = response
        .headers()
        .get(format!("x-codex-{}-used-percent", window))
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<f64>().ok())
        .map(|p: f64| p.round().clamp(0.0, 100.0) as u8);

    let reset_at = response
        .headers()
        .get(format!("x-codex-{}-reset-at", window))
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok())
        .map(ResetTimestamp::from_epoch_seconds);

    match (used_percent, reset_at) {
        (Some(p), Some(t)) => UsageWindow::with_percent_reset_and_span(p, t, span),
        (Some(p), None) => UsageWindow::with_percent_and_span(p, span),
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
        (
            "codex",
            ProviderCredentials::Codex {
                access_token,
                account_id,
            },
        ) => fetch_codex_usage(access_token, account_id),
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
}

/// Integration tests that run against real APIs with real credentials.
/// These tests require actual credential files to be present.
#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::account_usage::credentials::{
        read_claude_credentials, read_codex_credentials, read_gemini_credentials,
    };

    #[test]
    fn test_real_claude_api() {
        let creds = read_claude_credentials()
            .expect("Failed to read Claude credentials")
            .expect("Claude credentials file not found");

        eprintln!("Testing Claude API with real credentials...");
        let result = fetch_usage_for_provider("claude", &creds);

        eprintln!("Result:");
        eprintln!("  token_valid: {}", result.token_valid);
        if let Some(err) = &result.error {
            eprintln!("  error: {}", err);
        }
        if let Some(usage) = &result.usage {
            eprintln!("  email: {}", usage.email);
            eprintln!("  plan_type: {:?}", usage.plan_type);
            eprintln!("  rate_limit_tier: {:?}", usage.rate_limit_tier);
            eprintln!("  session: {:?}%", usage.session_window.used_percent);
            eprintln!("  weekly: {:?}%", usage.weekly_window.used_percent);
        }

        assert!(result.token_valid, "Token should be valid");
        assert!(result.usage.is_some(), "Should have usage data");

        let usage = result.usage.unwrap();
        assert!(!usage.email.is_empty(), "Should have email");
        assert!(
            usage.session_window.used_percent.is_some(),
            "Should have session usage"
        );
    }

    #[test]
    fn test_real_gemini_api() {
        let creds = read_gemini_credentials()
            .expect("Failed to read Gemini credentials")
            .expect("Gemini credentials file not found");

        eprintln!("Testing Gemini API with real credentials...");
        let result = fetch_usage_for_provider("gemini", &creds);

        eprintln!("Result:");
        eprintln!("  token_valid: {}", result.token_valid);
        if let Some(err) = &result.error {
            eprintln!("  error: {}", err);
        }
        if let Some(usage) = &result.usage {
            eprintln!("  email: {}", usage.email);
            eprintln!("  session: {:?}%", usage.session_window.used_percent);
        }

        assert!(result.token_valid, "Token should be valid");
        assert!(result.usage.is_some(), "Should have usage data");

        let usage = result.usage.unwrap();
        assert!(!usage.email.is_empty(), "Should have email");
    }

    #[test]
    fn test_real_codex_api() {
        let creds = read_codex_credentials()
            .expect("Failed to read Codex credentials")
            .expect("Codex credentials file not found");

        eprintln!("Testing Codex API fetch...");
        let result = fetch_usage_for_provider("codex", &creds);

        eprintln!("Result:");
        eprintln!("  token_valid: {}", result.token_valid);
        if let Some(err) = &result.error {
            eprintln!("  error: {}", err);
        }
        if let Some(usage) = &result.usage {
            eprintln!("  email: {}", usage.email);
            eprintln!("  session: {:?}%", usage.session_window.used_percent);
            eprintln!("  weekly: {:?}%", usage.weekly_window.used_percent);
        }

        // Codex may not have usage data if no recent sessions
        // Just verify we got a result without panicking
        assert!(result.token_valid, "Token should be valid");
    }

    #[test]
    fn test_real_fetch_all() {
        use crate::account_usage::credentials::read_all_credentials;

        let all_creds = read_all_credentials();
        assert!(
            !all_creds.is_empty(),
            "No credentials found - need at least one provider"
        );

        eprintln!("Testing all providers with real credentials...");
        for (provider, creds) in &all_creds {
            eprintln!("\n=== {} ===", provider);
            let result = fetch_usage_for_provider(provider, creds);

            eprintln!("  token_valid: {}", result.token_valid);
            if let Some(err) = &result.error {
                eprintln!("  error: {}", err);
            }
            if let Some(usage) = &result.usage {
                eprintln!("  email: {}", usage.email);
                eprintln!("  session: {:?}%", usage.session_window.used_percent);
            }

            assert!(result.token_valid, "{} token should be valid", provider);
        }
    }
}
