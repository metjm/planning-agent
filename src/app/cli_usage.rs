//! Account usage types and fetching for TUI display.
//!
//! This module provides usage data for display in the TUI stats panel.
//! It uses the account_usage module which fetches data via direct HTTP APIs.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;

use crate::account_usage::fetcher::fetch_all_usage;
use crate::account_usage::store::UsageStore;
use crate::account_usage::types::AccountUsageState;
use crate::usage_reset::UsageWindow;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub provider: String,
    pub display_name: String,
    /// Session/5h usage window with reset timestamp
    pub session: UsageWindow,
    /// Weekly/daily usage window with reset timestamp
    pub weekly: UsageWindow,
    pub plan_type: Option<String>,
    /// Timestamp when usage was fetched - skipped during serialization
    #[serde(skip)]
    pub fetched_at: Option<Instant>,
    pub status_message: Option<String>,
    pub supports_usage: bool,
}

impl ProviderUsage {
    /// Convert from the new AccountUsageState type.
    fn from_account_usage_state(state: &AccountUsageState) -> Self {
        let display_name = match state.provider.as_str() {
            "claude" => "Claude",
            "gemini" => "Gemini",
            "codex" => "Codex",
            _ => &state.provider,
        }
        .to_string();

        Self {
            provider: state.provider.clone(),
            display_name,
            session: state.session_window.clone(),
            weekly: state.weekly_window.clone(),
            plan_type: state.plan_type.clone(),
            fetched_at: Some(Instant::now()),
            status_message: state.error.clone(),
            supports_usage: true,
        }
    }

    pub fn has_error(&self) -> bool {
        self.status_message.is_some()
            && self.session.used_percent.is_none()
            && self.weekly.used_percent.is_none()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountUsage {
    pub providers: HashMap<String, ProviderUsage>,
}

impl AccountUsage {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    pub fn update(&mut self, usage: ProviderUsage) {
        self.providers.insert(usage.provider.clone(), usage);
    }
}

/// Fetch all provider usage via direct HTTP APIs.
/// This is a synchronous blocking function for use in spawn_blocking.
pub fn fetch_all_provider_usage_sync() -> AccountUsage {
    let mut store = UsageStore::new();
    fetch_all_usage(&mut store, None);

    let mut account_usage = AccountUsage::new();

    for record in store.get_all_accounts() {
        if let Some(state) = &record.current_usage {
            let provider_usage = ProviderUsage::from_account_usage_state(state);
            account_usage.update(provider_usage);
        }
    }

    account_usage
}

#[cfg(test)]
#[path = "tests/cli_usage_tests.rs"]
mod tests;
