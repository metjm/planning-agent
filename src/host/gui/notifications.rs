//! OS-native notification support for host-gui.
//!
//! Sends desktop notifications when:
//! - Sessions require human interaction (approval, decision)
//! - Sessions reach terminal failure states (failed, cancelled, error)

use super::session_table::{
    session_has_failed, session_needs_interaction, session_was_cancelled, DisplaySessionRow,
};
use std::collections::HashSet;
use std::sync::Once;

/// Ensure macOS notification application is set (once per process).
static MACOS_APP_INIT: Once = Once::new();

/// Initialize notification system for the current platform.
/// On macOS, this sets the application name so notifications appear correctly.
fn ensure_notifications_initialized() {
    MACOS_APP_INIT.call_once(|| {
        #[cfg(target_os = "macos")]
        {
            // On macOS, we need to set the application that "owns" the notifications.
            // Using Terminal as the sender since we're a CLI tool without a bundle.
            // This prevents the "where is use_default?" dialog.
            if let Err(e) = notify_rust::set_application("com.apple.Terminal") {
                eprintln!(
                    "[host] Warning: Failed to set notification application: {}",
                    e
                );
            }
        }
    });
}

/// Reason a notification was sent for a session.
/// Used for deduplication - a session can transition between states
/// and should be re-notified when entering a new notification-worthy state.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationReason {
    /// Session requires human input (from session_needs_interaction)
    NeedsInteraction,
    /// Session reached a terminal failure state (failed, cancelled, or error)
    Failed,
}

/// Check for sessions requiring attention and send notifications.
///
/// Updates the `notified_sessions` set to track which sessions have been notified.
/// Returns the updated set so the caller can clean up stale entries.
///
/// Notifies when:
/// - A session needs human interaction (approval, decision)
/// - A session has reached a terminal failure state (failed, cancelled, error)
///
/// Note: Liveness filtering is handled internally by `session_needs_interaction()`
/// and `session_has_failed()` - both return false for stopped sessions.
pub fn check_and_notify(
    sessions: &[DisplaySessionRow],
    notified_sessions: &mut HashSet<(String, NotificationReason)>,
) {
    let mut current_states: HashSet<(String, NotificationReason)> = HashSet::new();

    for session in sessions {
        // Check for failure state (includes liveness check internally)
        let is_failed = session_has_failed(
            &session.status,
            session.implementation_phase.as_deref(),
            session.liveness,
        );

        if is_failed {
            let key = (session.session_id.clone(), NotificationReason::Failed);
            current_states.insert(key.clone());

            if !notified_sessions.contains(&key) {
                notified_sessions.insert(key);
                let is_cancelled = session_was_cancelled(
                    session.implementation_phase.as_deref(),
                    session.liveness,
                );
                send_notification(NotificationReason::Failed, session, is_cancelled);
            }
        }

        // Check for needs-interaction state (includes liveness check internally)
        let needs_interaction = session_needs_interaction(
            &session.phase,
            session.implementation_phase.as_deref(),
            session.liveness,
        );

        if needs_interaction {
            let key = (
                session.session_id.clone(),
                NotificationReason::NeedsInteraction,
            );
            current_states.insert(key.clone());

            if !notified_sessions.contains(&key) {
                notified_sessions.insert(key.clone());
                send_notification(NotificationReason::NeedsInteraction, session, false);
            }
        }
    }

    // Clean up notifications for sessions no longer in notifiable states
    notified_sessions.retain(|key| current_states.contains(key));
}

/// Send a notification for a session based on the notification reason.
/// `is_cancelled` is used to adjust urgency for cancellation (user-initiated) vs failure (unexpected).
fn send_notification(reason: NotificationReason, session: &DisplaySessionRow, is_cancelled: bool) {
    ensure_notifications_initialized();

    let summary = get_notification_summary(reason, session);

    let (body, timeout, _is_critical) = match reason {
        NotificationReason::NeedsInteraction => {
            let body = format!(
                "{} on {} needs your attention",
                session.feature_name, session.container_name
            );
            (body, notify_rust::Timeout::Milliseconds(5000), false)
        }
        NotificationReason::Failed => {
            let body = format!(
                "{} on {} has {}",
                session.feature_name,
                session.container_name,
                if is_cancelled {
                    "been cancelled"
                } else {
                    "failed"
                }
            );
            let timeout = if is_cancelled {
                notify_rust::Timeout::Milliseconds(5000)
            } else {
                notify_rust::Timeout::Never
            };
            (body, timeout, !is_cancelled)
        }
    };

    let mut notification = notify_rust::Notification::new();
    notification.summary(&summary).body(&body).timeout(timeout);

    // Urgency is only available on Linux (freedesktop notification spec)
    #[cfg(target_os = "linux")]
    {
        let urgency = if _is_critical {
            notify_rust::Urgency::Critical
        } else {
            notify_rust::Urgency::Normal
        };
        notification.urgency(urgency);
    }

    if let Err(e) = notification.show() {
        eprintln!("[host] Warning: Could not send notification: {}", e);
    }
}

/// Get appropriate summary text for a notification.
fn get_notification_summary(reason: NotificationReason, session: &DisplaySessionRow) -> String {
    let phase_lower = session.phase.to_lowercase();
    let impl_phase_lower = session
        .implementation_phase
        .as_ref()
        .map(|s| s.to_lowercase());

    match reason {
        NotificationReason::NeedsInteraction => {
            if let Some(ref ip) = impl_phase_lower {
                if ip == "awaitingdecision" || ip == "awaiting_decision" {
                    return "Planning Agent - Implementation Decision".to_string();
                }
            }
            match phase_lower.as_str() {
                "complete" => "Planning Agent - Plan Ready for Approval".to_string(),
                "awaitingplanningdecision" => {
                    "Planning Agent - Planning Decision Needed".to_string()
                }
                _ => "Planning Agent - Action Required".to_string(),
            }
        }
        NotificationReason::Failed => {
            if let Some(ref ip) = impl_phase_lower {
                match ip.as_str() {
                    "failed" => return "Planning Agent - Implementation Failed".to_string(),
                    "cancelled" => return "Planning Agent - Implementation Cancelled".to_string(),
                    _ => {}
                }
            }
            // Default for planning workflow failures
            "Planning Agent - Session Failed".to_string()
        }
    }
}
