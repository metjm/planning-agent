//! Usage fetcher that coordinates credential reading and API calls.

use super::api_client::fetch_usage_for_provider;
use super::credentials::read_all_credentials;
use super::store::UsageStore;
use super::types::{AccountId, AccountUsageState, ProviderCredentials};
use crate::usage_reset::UsageWindow;

/// Fetches usage for all available credentials and updates the store.
pub fn fetch_all_usage(store: &mut UsageStore, container_id: Option<&str>) {
    let credentials = read_all_credentials();

    for (provider, creds) in credentials {
        let result = fetch_usage_for_provider(&provider, &creds);
        if let Some(usage) = result.usage {
            store.update_account(usage, container_id);
        } else if let Some(error) = result.error {
            // Create an error record
            let email = extract_email_from_creds(&creds).unwrap_or_else(|| "unknown".to_string());
            let usage = AccountUsageState {
                account_id: AccountId::new(&email),
                provider: provider.clone(),
                email,
                plan_type: None,
                rate_limit_tier: None,
                session_window: UsageWindow::default(),
                weekly_window: UsageWindow::default(),
                fetched_at: chrono::Utc::now().to_rfc3339(),
                error: Some(error),
                token_valid: result.token_valid,
            };
            store.update_account(usage, container_id);
        }
    }
}

/// Extracts email from credentials (used for error reporting).
fn extract_email_from_creds(creds: &ProviderCredentials) -> Option<String> {
    match creds {
        ProviderCredentials::Codex { access_token, .. } => {
            super::credentials::extract_email_from_jwt(access_token)
        }
        // For Claude and Gemini, we need to make an API call to get email
        // so we don't have it until we successfully fetch
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planning_paths::set_home_for_test;
    use serial_test::serial;
    use tempfile::TempDir;

    #[test]
    #[serial]
    fn test_fetch_all_usage_no_credentials() {
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

        let mut store = UsageStore::new();
        fetch_all_usage(&mut store, None);

        // Restore env
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        std::env::remove_var("GEMINI_DIR");
        std::env::remove_var("CODEX_HOME");

        // No credentials available, so no accounts
        assert!(store.get_all_accounts().is_empty());
    }
}
