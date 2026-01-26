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

/// Integration tests that run against real APIs with real credentials.
/// These tests require actual credential files to be present.
mod integration {
    use super::*;
    use crate::account_usage::credentials::{read_claude_credentials, read_codex_credentials};

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
