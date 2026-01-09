use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;

use crate::claude_usage::{self, ClaudeUsage};
use crate::codex_usage::{self, CodexUsage};
use crate::gemini_usage::{self, GeminiUsage};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub provider: String,
    pub display_name: String,
    pub session_used: Option<u8>,
    pub weekly_used: Option<u8>,
    pub plan_type: Option<String>,
    /// Timestamp when usage was fetched - skipped during serialization
    #[serde(skip)]
    pub fetched_at: Option<Instant>,
    pub status_message: Option<String>,
    pub supports_usage: bool,
}

impl ProviderUsage {

    #[allow(dead_code)]
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

    pub fn from_gemini_usage(usage: GeminiUsage) -> Self {

        let daily_used = usage.usage_remaining.map(|r| 100u8.saturating_sub(r));

        Self {
            provider: "gemini".to_string(),
            display_name: "Gemini".to_string(),
            session_used: None, 
            weekly_used: daily_used, 
            plan_type: None, // Gemini /stats doesn't provide plan info; constrained_model is a model name, not a plan
            fetched_at: usage.fetched_at,
            status_message: usage.error_message,
            supports_usage: true,
        }
    }

    pub fn from_codex_usage(usage: CodexUsage) -> Self {

        let session_used = usage.hourly_remaining.map(|r| 100u8.saturating_sub(r));
        let weekly_used = usage.weekly_remaining.map(|r| 100u8.saturating_sub(r));

        Self {
            provider: "codex".to_string(),
            display_name: "Codex".to_string(),
            session_used, 
            weekly_used,
            plan_type: usage.plan_type,
            fetched_at: usage.fetched_at,
            status_message: usage.error_message,
            supports_usage: true,
        }
    }

    pub fn has_error(&self) -> bool {
        self.status_message.is_some() && self.session_used.is_none() && self.weekly_used.is_none()
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

    #[allow(dead_code)]
    pub fn get(&self, provider: &str) -> Option<&ProviderUsage> {
        self.providers.get(provider)
    }

    #[allow(dead_code)]
    pub fn claude(&self) -> Option<&ProviderUsage> {
        self.providers.get("claude")
    }
}

/// Fetch usage from a single provider with a timeout wrapper.
/// Returns None if the fetch exceeds the given timeout.
fn fetch_with_timeout<T, F>(fetch_fn: F, timeout: std::time::Duration) -> Option<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = fetch_fn();
        let _ = tx.send(result);
    });

    rx.recv_timeout(timeout).ok()
}

/// Fetch all provider usage with independent timeouts per provider.
/// If one provider times out, the others still update.
pub fn fetch_all_provider_usage_sync() -> AccountUsage {
    let mut account_usage = AccountUsage::new();

    // Per-provider timeout (30 seconds each to allow for slow CLI startup)
    let provider_timeout = std::time::Duration::from_secs(30);

    // Fetch Claude usage (always attempted, independent timeout)
    if claude_usage::is_claude_available() {
        match fetch_with_timeout(claude_usage::fetch_claude_usage_sync, provider_timeout) {
            Some(usage) => {
                account_usage.update(ProviderUsage::from_claude_usage(usage));
            }
            None => {
                // Claude fetch timed out, add error status but don't block others
                account_usage.update(ProviderUsage::from_claude_usage(
                    ClaudeUsage::with_error("Fetch timed out".to_string()),
                ));
            }
        }
    } else {
        account_usage.update(ProviderUsage::from_claude_usage(
            ClaudeUsage::claude_not_available(),
        ));
    }

    // Fetch Gemini usage (independent timeout)
    if gemini_usage::is_gemini_available() {
        match fetch_with_timeout(gemini_usage::fetch_gemini_usage_sync, provider_timeout) {
            Some(usage) => {
                account_usage.update(ProviderUsage::from_gemini_usage(usage));
            }
            None => {
                account_usage.update(ProviderUsage::from_gemini_usage(
                    GeminiUsage::with_error("Fetch timed out".to_string()),
                ));
            }
        }
    }

    // Fetch Codex usage (independent timeout)
    if codex_usage::is_codex_available() {
        match fetch_with_timeout(codex_usage::fetch_codex_usage_sync, provider_timeout) {
            Some(usage) => {
                account_usage.update(ProviderUsage::from_codex_usage(usage));
            }
            None => {
                account_usage.update(ProviderUsage::from_codex_usage(
                    CodexUsage::with_error("Fetch timed out".to_string()),
                ));
            }
        }
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
    fn test_provider_usage_from_gemini() {
        let gemini_usage = GeminiUsage {
            usage_remaining: Some(75),
            constrained_model: Some("gemini-2.5-flash".to_string()),
            fetched_at: Some(Instant::now()),
            error_message: None,
        };
        let provider = ProviderUsage::from_gemini_usage(gemini_usage);
        assert_eq!(provider.provider, "gemini");
        assert_eq!(provider.display_name, "Gemini");
        assert_eq!(provider.session_used, None); // Gemini doesn't have session usage
        assert_eq!(provider.weekly_used, Some(25)); // 100 - 75 = 25% used
        // Plan type should be None - constrained_model is not a plan
        assert_eq!(provider.plan_type, None, "Gemini plan_type should be None (model name is not a plan)");
        assert!(provider.supports_usage);
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

    #[test]
    #[ignore]
    fn test_fetch_all_providers_real() {
        eprintln!("Fetching usage from all providers...\n");
        let account = fetch_all_provider_usage_sync();

        eprintln!("Found {} providers\n", account.providers.len());

        if let Some(claude) = account.get("claude") {
            eprintln!("Claude:");
            eprintln!("  supports_usage: {}", claude.supports_usage);
            eprintln!("  session_used: {:?}", claude.session_used);
            eprintln!("  weekly_used: {:?}", claude.weekly_used);
            eprintln!("  plan_type: {:?}", claude.plan_type);
            eprintln!("  status_message: {:?}", claude.status_message);

            assert!(claude.supports_usage, "Claude should support usage");
            if claude.status_message.is_none() {
                assert!(claude.session_used.is_some() || claude.weekly_used.is_some(),
                    "Claude should have usage data");
            }
        } else {
            eprintln!("Claude: not found (CLI not installed?)");
        }

        if let Some(gemini) = account.get("gemini") {
            eprintln!("\nGemini:");
            eprintln!("  supports_usage: {}", gemini.supports_usage);
            eprintln!("  daily_used (as weekly): {:?}", gemini.weekly_used);
            eprintln!("  plan_type: {:?}", gemini.plan_type);
            eprintln!("  status_message: {:?}", gemini.status_message);

            assert!(gemini.supports_usage, "Gemini should support usage via /stats");
            // plan_type should be None - Gemini /stats doesn't provide plan info
            assert_eq!(gemini.plan_type, None, "Gemini plan_type should be None");
            if gemini.status_message.is_none() {
                assert!(gemini.weekly_used.is_some(), "Gemini should have daily usage data");
                eprintln!("  Daily used: {}%", gemini.weekly_used.unwrap());
            }
        } else {
            eprintln!("\nGemini: not found (CLI not installed)");
        }

        if let Some(codex) = account.get("codex") {
            eprintln!("\nCodex:");
            eprintln!("  supports_usage: {}", codex.supports_usage);
            eprintln!("  session_used (5h): {:?}", codex.session_used);
            eprintln!("  weekly_used: {:?}", codex.weekly_used);
            eprintln!("  plan_type: {:?}", codex.plan_type);
            eprintln!("  status_message: {:?}", codex.status_message);

            assert!(codex.supports_usage, "Codex should support usage via /status");
            if codex.status_message.is_none() {

                if let Some(session) = codex.session_used {
                    eprintln!("  5h used: {}%", session);
                }
                if let Some(weekly) = codex.weekly_used {
                    eprintln!("  Weekly used: {}%", weekly);
                }
                if codex.session_used.is_none() && codex.weekly_used.is_none() {
                    eprintln!("  (usage data not available yet - normal for fresh sessions)");
                }
            }
        } else {
            eprintln!("\nCodex: not found (CLI not installed)");
        }

        eprintln!("\nAll provider checks passed!");
    }
}
