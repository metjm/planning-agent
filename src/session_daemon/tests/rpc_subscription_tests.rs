use super::*;

#[test]
fn test_subscription_event_debug() {
    let record = SessionRecord::new(
        "test".to_string(),
        "Test".to_string(),
        std::path::PathBuf::from("/test"),
        std::path::PathBuf::from("/test/sessions/test"),
        "Planning".to_string(),
        1,
        "Planning".to_string(),
        12345,
    );

    let event = SubscriptionEvent::SessionChanged(Box::new(record));
    let debug_str = format!("{:?}", event);
    assert!(debug_str.contains("SessionChanged"));

    let event = SubscriptionEvent::DaemonRestarting;
    let debug_str = format!("{:?}", event);
    assert!(debug_str.contains("DaemonRestarting"));
}
