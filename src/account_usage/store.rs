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

        // Remove placeholder email records for the same provider when we get a real email.
        // This cleans up temporary error records but preserves other accounts for the provider.
        if !usage.email.is_empty() && usage.email != "unknown" {
            let provider = usage.provider.clone();
            self.accounts.retain(|_id, record| {
                record.provider != provider
                    || (record.email != "unknown" && !record.email.is_empty())
            });
        }

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
#[path = "tests/store_tests.rs"]
mod tests;
