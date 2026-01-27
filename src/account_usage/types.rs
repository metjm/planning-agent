//! Data types for account usage tracking.

use crate::usage_reset::UsageWindow;
use serde::{Deserialize, Serialize};

/// Unique account identifier using provider and email.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountId(pub String);

impl AccountId {
    pub fn new(provider: &str, email: &str) -> Self {
        Self(format!(
            "{}:{}",
            provider.to_lowercase(),
            email.to_lowercase()
        ))
    }
}

impl std::fmt::Display for AccountId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Provider-specific credentials needed to fetch usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProviderCredentials {
    Claude {
        access_token: String,
        expires_at: Option<i64>,
    },
    Gemini {
        access_token: String,
        expires_at: Option<i64>,
    },
    Codex {
        access_token: String,
        account_id: String,
    },
}

/// Current usage state for a single provider account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountUsageState {
    pub account_id: AccountId,
    pub provider: String,
    pub email: String,
    pub plan_type: Option<String>,
    pub rate_limit_tier: Option<String>,
    pub session_window: UsageWindow,
    pub weekly_window: UsageWindow,
    pub fetched_at: String,
    pub error: Option<String>,
    pub token_valid: bool,
}

/// Historical usage snapshot for persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub timestamp: String,
    pub session_percent: Option<u8>,
    pub weekly_percent: Option<u8>,
}

/// Persistent account record stored in host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountRecord {
    pub account_id: AccountId,
    pub provider: String,
    pub email: String,
    pub plan_type: Option<String>,
    pub first_seen: String,
    pub last_successful_fetch: Option<String>,
    pub current_usage: Option<AccountUsageState>,
    /// Last successful usage state, preserved for extrapolation when current fetch fails.
    /// Only updated when a fetch succeeds (error.is_none()).
    #[serde(default)]
    pub last_successful_usage: Option<AccountUsageState>,
    pub history: Vec<UsageSnapshot>,
    pub credentials_available: bool,
    pub seen_in_containers: Vec<String>,
}

#[cfg(test)]
#[path = "tests/types_tests.rs"]
mod tests;
