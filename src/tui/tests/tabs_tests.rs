use super::*;

#[test]
fn test_new_creates_one_session() {
    let manager = TabManager::new();
    assert_eq!(manager.len(), 1);
    assert_eq!(manager.active_tab, 0);
}

#[test]
fn test_add_session() {
    let mut manager = TabManager::new();
    let initial_len = manager.len();

    manager.add_session();
    assert_eq!(manager.len(), initial_len + 1);
    assert_eq!(manager.active_tab, manager.len() - 1);
}

#[test]
fn test_next_tab_wraps() {
    let mut manager = TabManager::new();
    manager.add_session();
    manager.add_session();
    assert_eq!(manager.len(), 3);

    manager.active_tab = 2;
    manager.next_tab();
    assert_eq!(manager.active_tab, 0);
}

#[test]
fn test_prev_tab_wraps() {
    let mut manager = TabManager::new();
    manager.add_session();
    manager.add_session();
    assert_eq!(manager.len(), 3);

    manager.active_tab = 0;
    manager.prev_tab();
    assert_eq!(manager.active_tab, 2);
}

#[test]
fn test_switch_to_tab() {
    let mut manager = TabManager::new();
    manager.add_session();
    manager.add_session();

    manager.switch_to_tab(1);
    assert_eq!(manager.active_tab, 1);

    manager.switch_to_tab(100);
    assert_eq!(manager.active_tab, 1);
}

#[test]
fn test_switch_to_tab_out_of_bounds() {
    let mut manager = TabManager::new();
    manager.switch_to_tab(100);
    assert_eq!(manager.active_tab, 0);
}

#[test]
fn test_close_tab_adjusts_active() {
    let mut manager = TabManager::new();
    manager.add_session();
    manager.add_session();
    assert_eq!(manager.len(), 3);

    manager.active_tab = 2;
    manager.close_tab(1);
    assert_eq!(manager.len(), 2);
    assert_eq!(manager.active_tab, 1);
}

#[test]
fn test_cannot_close_last_tab() {
    let mut manager = TabManager::new();
    assert_eq!(manager.len(), 1);

    manager.close_tab(0);
    assert_eq!(manager.len(), 1);
}

#[test]
fn test_add_session_with_name() {
    let mut manager = TabManager::new();
    manager.add_session_with_name("test-feature".to_string());

    assert_eq!(manager.len(), 2);
    assert_eq!(manager.active().name, "test-feature");
}
