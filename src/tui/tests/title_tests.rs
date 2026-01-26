use super::*;

#[test]
fn test_title_manager_creation() {
    let manager = TerminalTitleManager::new();
    let _ = manager.is_supported;
}
