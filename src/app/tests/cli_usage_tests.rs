use super::*;
use crate::planning_paths::set_home_for_test;
use crate::usage_reset::ResetTimestamp;
use serial_test::serial;
use tempfile::TempDir;

#[test]
fn test_provider_usage_has_error() {
    let error_usage = ProviderUsage {
        provider: "claude".to_string(),
        display_name: "Claude".to_string(),
        session: UsageWindow::default(),
        weekly: UsageWindow::default(),
        plan_type: None,
        fetched_at: Some(Instant::now()),
        status_message: Some("CLI not found".to_string()),
        supports_usage: true,
    };
    assert!(error_usage.has_error());

    let ok_usage = ProviderUsage {
        provider: "claude".to_string(),
        display_name: "Claude".to_string(),
        session: UsageWindow::with_percent(10),
        weekly: UsageWindow::with_percent(20),
        plan_type: None,
        fetched_at: Some(Instant::now()),
        status_message: None,
        supports_usage: true,
    };
    assert!(!ok_usage.has_error());
}

#[test]
fn test_provider_usage_from_account_state() {
    let ts = ResetTimestamp::from_epoch_seconds(1700000000);
    let state = AccountUsageState {
        account_id: crate::account_usage::types::AccountId::new("test", "test@example.com"),
        provider: "claude".to_string(),
        email: "test@example.com".to_string(),
        plan_type: Some("Max".to_string()),
        rate_limit_tier: None,
        session_window: UsageWindow::with_percent_and_reset(5, ts),
        weekly_window: UsageWindow::with_percent_and_reset(41, ts),
        fetched_at: chrono::Utc::now().to_rfc3339(),
        error: None,
        token_valid: true,
    };

    let provider = ProviderUsage::from_account_usage_state(&state);
    assert_eq!(provider.provider, "claude");
    assert_eq!(provider.display_name, "Claude");
    assert_eq!(provider.session.used_percent, Some(5));
    assert_eq!(provider.weekly.used_percent, Some(41));
    assert_eq!(provider.plan_type, Some("Max".to_string()));
    assert!(provider.supports_usage);
}

#[test]
#[serial]
fn test_fetch_all_provider_usage_sync_no_credentials() {
    let temp_dir = TempDir::new().unwrap();
    let _guard = set_home_for_test(temp_dir.path().to_path_buf());

    // Set env vars to point to empty temp dirs so no real credentials are found
    let empty_claude = temp_dir.path().join("claude");
    let empty_gemini = temp_dir.path().join("gemini");
    let empty_codex = temp_dir.path().join("codex");
    std::fs::create_dir_all(&empty_claude).unwrap();
    std::fs::create_dir_all(&empty_gemini).unwrap();
    std::fs::create_dir_all(&empty_codex).unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", &empty_claude);
    std::env::set_var("GEMINI_DIR", &empty_gemini);
    std::env::set_var("CODEX_HOME", &empty_codex);

    let usage = fetch_all_provider_usage_sync();

    // Restore env
    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var("GEMINI_DIR");
    std::env::remove_var("CODEX_HOME");

    // No credentials available, so no providers
    assert!(usage.providers.is_empty());
}

#[test]
fn test_account_usage_update() {
    let mut usage = AccountUsage::new();
    let provider = ProviderUsage {
        provider: "claude".to_string(),
        display_name: "Claude".to_string(),
        session: UsageWindow::with_percent(10),
        weekly: UsageWindow::with_percent(20),
        plan_type: None,
        fetched_at: Some(Instant::now()),
        status_message: None,
        supports_usage: true,
    };
    usage.update(provider.clone());
    assert_eq!(usage.providers.len(), 1);
    assert!(usage.providers.contains_key("claude"));
}
