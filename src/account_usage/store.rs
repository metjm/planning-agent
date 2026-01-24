//! Persistent storage for account usage data.

use super::types::{AccountId, AccountRecord, AccountUsageState, UsageSnapshot};
#[cfg(any(feature = "host-gui", test))]
use crate::planning_paths;
#[cfg(any(feature = "host-gui", test))]
use anyhow::{Context, Result};
use std::collections::HashMap;

#[cfg(any(feature = "host-gui", test))]
const STORE_FILENAME: &str = "usage_store.json";
const MAX_HISTORY_ENTRIES: usize = 100;

/// Persistent storage for account usage data.
pub struct UsageStore {
    accounts: HashMap<AccountId, AccountRecord>,
    dirty: bool,
}

impl UsageStore {
    /// Creates a new empty store.
    pub fn new() -> Self {
        Self {
            accounts: HashMap::new(),
            dirty: false,
        }
    }

    /// Loads the usage store from disk.
    #[cfg(any(feature = "host-gui", test))]
    pub fn load() -> Result<Self> {
        let path = planning_paths::planning_agent_home_dir()?.join(STORE_FILENAME);

        let accounts = if path.exists() {
            let content = std::fs::read_to_string(&path).context("Failed to read usage store")?;
            let records: Vec<AccountRecord> =
                serde_json::from_str(&content).context("Failed to parse usage store")?;
            records
                .into_iter()
                .map(|r| (r.account_id.clone(), r))
                .collect()
        } else {
            HashMap::new()
        };

        Ok(Self {
            accounts,
            dirty: false,
        })
    }

    /// Saves the usage store to disk if dirty.
    #[cfg(any(feature = "host-gui", test))]
    pub fn save(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }

        let path = planning_paths::planning_agent_home_dir()?.join(STORE_FILENAME);
        let records: Vec<&AccountRecord> = self.accounts.values().collect();
        let content =
            serde_json::to_string_pretty(&records).context("Failed to serialize usage store")?;
        std::fs::write(&path, content).context("Failed to write usage store")?;

        self.dirty = false;
        Ok(())
    }

    /// Updates an account with new usage data.
    pub fn update_account(&mut self, usage: AccountUsageState, container_id: Option<&str>) {
        let now = chrono::Utc::now().to_rfc3339();

        // Remove any existing entries for the same provider with different account_id.
        // This handles the case where an error record was created with "unknown" email,
        // and we now have a successful fetch with the real email.
        let provider = usage.provider.clone();
        let new_account_id = usage.account_id.clone();
        self.accounts
            .retain(|id, record| record.provider != provider || *id == new_account_id);

        let record = self
            .accounts
            .entry(usage.account_id.clone())
            .or_insert_with(|| AccountRecord {
                account_id: usage.account_id.clone(),
                provider: usage.provider.clone(),
                email: usage.email.clone(),
                plan_type: usage.plan_type.clone(),
                first_seen: now.clone(),
                last_successful_fetch: None,
                current_usage: None,
                history: Vec::new(),
                credentials_available: true,
                seen_in_containers: Vec::new(),
            });

        // Add history entry only on successful fetch
        if usage.error.is_none() {
            record.history.push(UsageSnapshot {
                timestamp: now.clone(),
                session_percent: usage.session_window.used_percent,
                weekly_percent: usage.weekly_window.used_percent,
            });

            // Trim history
            if record.history.len() > MAX_HISTORY_ENTRIES {
                record.history.remove(0);
            }

            record.last_successful_fetch = Some(now);
        }

        // Track container
        if let Some(cid) = container_id {
            if !record.seen_in_containers.contains(&cid.to_string()) {
                record.seen_in_containers.push(cid.to_string());
            }
        }

        record.current_usage = Some(usage);
        record.credentials_available = true;
        self.dirty = true;
    }

    /// Gets all account records.
    pub fn get_all_accounts(&self) -> Vec<&AccountRecord> {
        self.accounts.values().collect()
    }
}

impl Default for UsageStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planning_paths::set_home_for_test;
    use crate::usage_reset::UsageWindow;
    use tempfile::TempDir;

    fn make_usage_state(email: &str, provider: &str) -> AccountUsageState {
        AccountUsageState {
            account_id: AccountId::new(email),
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
}
