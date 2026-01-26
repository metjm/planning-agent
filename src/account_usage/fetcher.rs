//! Usage fetcher that coordinates credential reading and API calls.

use super::api_client::fetch_usage_for_provider;
use super::credentials::read_all_credentials;
use super::store::UsageStore;
use super::types::{AccountId, AccountUsageState, ProviderCredentials};
use crate::usage_reset::UsageWindow;

/// Fetches usage for all available credentials and updates the store.
/// Reads credentials from local files.
pub fn fetch_all_usage(store: &mut UsageStore, container_id: Option<&str>) {
    let credentials = read_all_credentials();
    fetch_usage_with_credentials(store, credentials, container_id);
}

/// Fetches usage using provided credentials (from daemon RPC or elsewhere).
pub fn fetch_usage_with_credentials(
    store: &mut UsageStore,
    credentials: Vec<(String, ProviderCredentials)>,
    container_id: Option<&str>,
) {
    for (provider, creds) in credentials {
        let result = fetch_usage_for_provider(&provider, &creds);
        if let Some(usage) = result.usage {
            store.update_account(usage, container_id);
        } else if let Some(error) = result.error {
            // Create an error record
            let email = extract_email_from_creds(&creds).unwrap_or_else(|| "unknown".to_string());
            let usage = AccountUsageState {
                account_id: AccountId::new(&provider, &email),
                provider: provider.clone(),
                email,
                plan_type: None,
                rate_limit_tier: None,
                session_window: UsageWindow::default(),
                weekly_window: UsageWindow::default(),
                fetched_at: chrono::Utc::now().to_rfc3339(),
                error: Some(error.clone()),
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
#[path = "tests/fetcher_tests.rs"]
mod tests;
