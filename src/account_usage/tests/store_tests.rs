use super::*;
use crate::planning_paths::set_home_for_test;
use crate::usage_reset::UsageWindow;
use tempfile::TempDir;

fn make_usage_state(email: &str, provider: &str) -> AccountUsageState {
    AccountUsageState {
        account_id: AccountId::new(provider, email),
        provider: provider.to_string(),
        email: email.to_string(),
        plan_type: None,
        rate_limit_tier: None,
        session_window: UsageWindow::with_percent_and_span(
            50,
            crate::usage_reset::UsageWindowSpan::Hours(5),
        ),
        weekly_window: UsageWindow::default(),
        fetched_at: chrono::Utc::now().to_rfc3339(),
        error: None,
        token_valid: true,
    }
}

#[test]
fn test_store_new_empty() {
    let store = UsageStore::new();
    assert!(store.get_all_accounts().is_empty());
}

#[test]
fn test_store_load_empty_dir() {
    let temp_dir = TempDir::new().unwrap();
    let _guard = set_home_for_test(temp_dir.path().to_path_buf());

    let store = UsageStore::load().unwrap();
    assert!(store.get_all_accounts().is_empty());
}

#[test]
fn test_store_update_and_save() {
    let temp_dir = TempDir::new().unwrap();
    let _guard = set_home_for_test(temp_dir.path().to_path_buf());

    let mut store = UsageStore::new();
    let usage = make_usage_state("test@example.com", "claude");
    store.update_account(usage, Some("container1"));

    assert_eq!(store.get_all_accounts().len(), 1);
    assert!(store.dirty);

    store.save().unwrap();
    assert!(!store.dirty);

    // Reload and verify
    let store2 = UsageStore::load().unwrap();
    assert_eq!(store2.get_all_accounts().len(), 1);
    let record = store2.get_all_accounts()[0];
    assert_eq!(record.email, "test@example.com");
    assert!(record
        .seen_in_containers
        .contains(&"container1".to_string()));
}

#[test]
fn test_store_history_trimming() {
    let temp_dir = TempDir::new().unwrap();
    let _guard = set_home_for_test(temp_dir.path().to_path_buf());

    let mut store = UsageStore::new();

    // Add more than MAX_HISTORY_ENTRIES updates
    for i in 0..105 {
        let mut usage = make_usage_state("test@example.com", "claude");
        usage.session_window = UsageWindow::with_percent_and_span(
            (i % 100) as u8,
            crate::usage_reset::UsageWindowSpan::Hours(5),
        );
        store.update_account(usage, None);
    }

    let record = store.get_all_accounts()[0];
    assert_eq!(record.history.len(), MAX_HISTORY_ENTRIES);
}
