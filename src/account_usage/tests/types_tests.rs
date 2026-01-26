use super::*;

#[test]
fn test_account_id_display() {
    let id = AccountId("user@example.com".to_string());
    assert_eq!(format!("{}", id), "user@example.com");
}

#[test]
fn test_account_id_new_lowercases() {
    let id = AccountId::new("Claude", "User@Example.COM");
    assert_eq!(id.0, "claude:user@example.com");
}
