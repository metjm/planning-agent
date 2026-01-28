use super::*;

#[test]
fn test_scroll_state_new_defaults_to_follow_mode() {
    let state = ScrollState::new();
    assert!(state.follow);
    assert_eq!(state.position, 0);
}

#[test]
fn test_scroll_state_default_matches_new() {
    let state = ScrollState::default();
    assert!(state.follow);
    assert_eq!(state.position, 0);
}

#[test]
fn test_scroll_up_disables_follow_mode() {
    let mut state = ScrollState::new();
    state.position = 10;

    state.scroll_up();

    assert!(!state.follow);
    assert_eq!(state.position, 9);
}

#[test]
fn test_scroll_up_saturates_at_zero() {
    let mut state = ScrollState::new();
    state.position = 0;

    state.scroll_up();

    assert_eq!(state.position, 0);
}

#[test]
fn test_scroll_down_preserves_follow_mode() {
    let mut state = ScrollState::new();
    state.position = 5;
    state.follow = true;

    state.scroll_down(100);

    assert!(state.follow); // Follow mode preserved
    assert_eq!(state.position, 6);
}

#[test]
fn test_scroll_down_clamps_to_max_scroll() {
    let mut state = ScrollState::new();
    state.position = 99;

    state.scroll_down(100);

    assert_eq!(state.position, 100);

    // Should not go past max_scroll
    state.scroll_down(100);
    assert_eq!(state.position, 100);
}

#[test]
fn test_scroll_to_top_disables_follow_mode() {
    let mut state = ScrollState::new();
    state.position = 50;
    state.follow = true;

    state.scroll_to_top();

    assert!(!state.follow);
    assert_eq!(state.position, 0);
}

#[test]
fn test_scroll_to_bottom_syncs_position_and_enables_follow() {
    let mut state = ScrollState::new();
    state.position = 10;
    state.follow = false;

    state.scroll_to_bottom(100);

    assert!(state.follow);
    assert_eq!(state.position, 100);
}

#[test]
fn test_effective_position_returns_max_when_follow() {
    let state = ScrollState {
        position: 50,
        follow: true,
    };

    assert_eq!(state.effective_position(100), 100);
}

#[test]
fn test_effective_position_returns_position_when_not_follow() {
    let state = ScrollState {
        position: 50,
        follow: false,
    };

    assert_eq!(state.effective_position(100), 50);
}

#[test]
fn test_effective_position_clamps_to_max() {
    let state = ScrollState {
        position: 150, // position beyond max
        follow: false,
    };

    assert_eq!(state.effective_position(100), 100);
}

#[test]
fn test_serialization_roundtrip() {
    let state = ScrollState {
        position: 42,
        follow: false,
    };

    let json = serde_json::to_string(&state).unwrap();
    let restored: ScrollState = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.position, 42);
    assert!(!restored.follow);
}

#[test]
fn test_deserialization_with_defaults() {
    // Test that missing fields get defaults (for backward compatibility)
    let json = "{}";
    let state: ScrollState = serde_json::from_str(json).unwrap();

    assert_eq!(state.position, 0);
    assert!(state.follow); // Default is true (follow mode enabled)
}
