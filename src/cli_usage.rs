//! CLI Usage tracking for all supported providers (Claude, Gemini, Codex)
//!
//! This module provides a unified interface for fetching account usage information
//! from various AI CLI tools. Currently:
//! - Claude: Supports /usage command with session and weekly limits
//! - Gemini: No usage command available (shows N/A)
//! - Codex: No usage command available (shows N/A)

use std::collections::HashMap;
use std::time::Instant;

use crate::claude_usage::{self, ClaudeUsage};

/// A provider's usage information (generalized for multi-provider support)
#[derive(Debug, Clone, Default)]
pub struct ProviderUsage {
    /// Provider name (e.g., "claude", "gemini", "codex")
    pub provider: String,
    /// Display name (e.g., "Claude", "Gemini", "Codex")
    pub display_name: String,
    /// Session/daily usage as percentage used (0-100)
    pub session_used: Option<u8>,
    /// Weekly usage as percentage used (0-100)
    pub weekly_used: Option<u8>,
    /// Plan type if available
    pub plan_type: Option<String>,
    /// When this data was fetched
    pub fetched_at: Option<Instant>,
    /// Error or status message (e.g., "Not available", "CLI not found")
    pub status_message: Option<String>,
    /// Whether this provider supports usage queries
    pub supports_usage: bool,
}

impl ProviderUsage {
    /// Create a "not available" status for providers that don't support usage queries
    pub fn not_available(provider: &str, display_name: &str) -> Self {
        Self {
            provider: provider.to_string(),
            display_name: display_name.to_string(),
            session_used: None,
            weekly_used: None,
            plan_type: None,
            fetched_at: Some(Instant::now()),
            status_message: Some("No usage command".to_string()),
            supports_usage: false,
        }
    }

    /// Create from a ClaudeUsage result
    pub fn from_claude_usage(usage: ClaudeUsage) -> Self {
        Self {
            provider: "claude".to_string(),
            display_name: "Claude".to_string(),
            session_used: usage.session_used,
            weekly_used: usage.weekly_used,
            plan_type: usage.plan_type,
            fetched_at: usage.fetched_at,
            status_message: usage.error_message,
            supports_usage: true,
        }
    }

    /// Check if this provider has an error status
    pub fn has_error(&self) -> bool {
        self.status_message.is_some() && self.session_used.is_none() && self.weekly_used.is_none()
    }
}

/// Container for all provider usages
#[derive(Debug, Clone, Default)]
pub struct AccountUsage {
    /// Map of provider name to usage data
    pub providers: HashMap<String, ProviderUsage>,
}

impl AccountUsage {
    /// Create a new AccountUsage with empty provider map
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    /// Update usage for a specific provider
    pub fn update(&mut self, usage: ProviderUsage) {
        self.providers.insert(usage.provider.clone(), usage);
    }

    /// Get usage for a specific provider
    #[allow(dead_code)]
    pub fn get(&self, provider: &str) -> Option<&ProviderUsage> {
        self.providers.get(provider)
    }

    /// Get Claude usage specifically (for backwards compatibility)
    #[allow(dead_code)]
    pub fn claude(&self) -> Option<&ProviderUsage> {
        self.providers.get("claude")
    }
}

/// Fetch usage for all supported providers
/// Returns a map of provider name to ProviderUsage
pub fn fetch_all_provider_usage_sync() -> AccountUsage {
    let mut account_usage = AccountUsage::new();

    // Fetch Claude usage (the only provider with usage support)
    let claude_usage = claude_usage::fetch_claude_usage_sync();
    account_usage.update(ProviderUsage::from_claude_usage(claude_usage));

    // Gemini: No usage command available
    if which::which("gemini").is_ok() {
        account_usage.update(ProviderUsage::not_available("gemini", "Gemini"));
    }

    // Codex: No usage command available
    if which::which("codex").is_ok() {
        account_usage.update(ProviderUsage::not_available("codex", "Codex"));
    }

    account_usage
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_usage_not_available() {
        let usage = ProviderUsage::not_available("gemini", "Gemini");
        assert_eq!(usage.provider, "gemini");
        assert_eq!(usage.display_name, "Gemini");
        assert!(!usage.supports_usage);
        assert!(usage.status_message.is_some());
        assert!(usage.fetched_at.is_some());
    }

    #[test]
    fn test_provider_usage_from_claude() {
        let claude_usage = ClaudeUsage {
            session_used: Some(5),
            weekly_used: Some(41),
            plan_type: Some("Max".to_string()),
            fetched_at: Some(Instant::now()),
            error_message: None,
        };
        let provider = ProviderUsage::from_claude_usage(claude_usage);
        assert_eq!(provider.provider, "claude");
        assert_eq!(provider.display_name, "Claude");
        assert_eq!(provider.session_used, Some(5));
        assert_eq!(provider.weekly_used, Some(41));
        assert_eq!(provider.plan_type, Some("Max".to_string()));
        assert!(provider.supports_usage);
    }

    #[test]
    fn test_account_usage_update() {
        let mut account = AccountUsage::new();
        account.update(ProviderUsage::not_available("gemini", "Gemini"));
        account.update(ProviderUsage::not_available("codex", "Codex"));

        assert!(account.get("gemini").is_some());
        assert!(account.get("codex").is_some());
        assert!(account.get("claude").is_none());
    }

    #[test]
    fn test_provider_has_error() {
        let error_usage = ProviderUsage {
            provider: "claude".to_string(),
            display_name: "Claude".to_string(),
            session_used: None,
            weekly_used: None,
            plan_type: None,
            fetched_at: Some(Instant::now()),
            status_message: Some("CLI not found".to_string()),
            supports_usage: true,
        };
        assert!(error_usage.has_error());

        let ok_usage = ProviderUsage {
            provider: "claude".to_string(),
            display_name: "Claude".to_string(),
            session_used: Some(10),
            weekly_used: Some(20),
            plan_type: None,
            fetched_at: Some(Instant::now()),
            status_message: None,
            supports_usage: true,
        };
        assert!(!ok_usage.has_error());
    }
}
