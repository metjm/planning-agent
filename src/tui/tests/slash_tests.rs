use super::*;

#[test]
fn test_detect_slash_at_start() {
    let result = detect_slash_at_cursor("/u", 2);
    assert!(matches!(
        result,
        Some(SlashContext::Command { query, .. }) if query == "/u"
    ));
}

#[test]
fn test_detect_slash_with_leading_whitespace() {
    let result = detect_slash_at_cursor("  /upd", 6);
    assert!(matches!(
        result,
        Some(SlashContext::Command { query, start_byte, .. }) if query == "/upd" && start_byte == 2
    ));
}

#[test]
fn test_detect_slash_empty_after_slash() {
    let result = detect_slash_at_cursor("/", 1);
    assert!(matches!(
        result,
        Some(SlashContext::Command { query, .. }) if query == "/"
    ));
}

#[test]
fn test_detect_dynamic_arg_config() {
    let result = detect_slash_at_cursor("/config d", 9);
    assert!(matches!(
        result,
        Some(SlashContext::DynamicArg { command, arg_query, .. }) if command == "/config" && arg_query == "d"
    ));
}

#[test]
fn test_detect_dynamic_arg_config_empty() {
    let result = detect_slash_at_cursor("/config ", 8);
    assert!(matches!(
        result,
        Some(SlashContext::DynamicArg { command, arg_query, .. }) if command == "/config" && arg_query.is_empty()
    ));
}

#[test]
fn test_detect_dynamic_arg_workflow() {
    let result = detect_slash_at_cursor("/workflow cl", 12);
    assert!(matches!(
        result,
        Some(SlashContext::DynamicArg { command, arg_query, .. }) if command == "/workflow" && arg_query == "cl"
    ));
}

#[test]
fn test_detect_dynamic_arg_workflow_empty() {
    let result = detect_slash_at_cursor("/workflow ", 10);
    assert!(matches!(
        result,
        Some(SlashContext::DynamicArg { command, arg_query, .. }) if command == "/workflow" && arg_query.is_empty()
    ));
}

#[test]
fn test_detect_no_slash() {
    let result = detect_slash_at_cursor("hello", 5);
    assert!(result.is_none());
}

#[test]
fn test_detect_non_leading_slash() {
    let result = detect_slash_at_cursor("hello /update", 13);
    assert!(result.is_none());
}

#[test]
fn test_find_matches_update() {
    let context = SlashContext::Command {
        start_byte: 0,
        end_byte: 2,
        query: "/u".to_string(),
    };
    let matches = find_slash_matches(&context, 10);
    assert!(!matches.is_empty());
    assert!(matches.iter().any(|m| m.display == "/update"));
}

#[test]
fn test_find_matches_config() {
    let context = SlashContext::Command {
        start_byte: 0,
        end_byte: 4,
        query: "/con".to_string(),
    };
    let matches = find_slash_matches(&context, 10);
    assert!(!matches.is_empty());
    assert!(matches.iter().any(|m| m.display == "/config-dangerous"));
}

#[test]
fn test_find_matches_dynamic_arg_config() {
    let context = SlashContext::DynamicArg {
        command: "/config".to_string(),
        command_start: 0,
        end_byte: 9,
        arg_query: "d".to_string(),
    };
    let matches = find_slash_matches(&context, 10);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].display, "/config dangerous");
}

#[test]
fn test_exact_match_scores_highest() {
    let context = SlashContext::Command {
        start_byte: 0,
        end_byte: 7,
        query: "/update".to_string(),
    };
    let matches = find_slash_matches(&context, 10);
    assert!(!matches.is_empty());
    assert_eq!(matches[0].display, "/update");
    assert_eq!(matches[0].score, 100);
}

#[test]
fn test_slash_state_navigation() {
    let mut state = SlashState {
        active: true,
        start_byte: 0,
        end_byte: 2,
        matches: vec![
            SlashMatch {
                display: "/update".to_string(),
                insert: "/update".to_string(),
                description: "desc".to_string(),
                score: 100,
            },
            SlashMatch {
                display: "/config-dangerous".to_string(),
                insert: "/config-dangerous".to_string(),
                description: "desc".to_string(),
                score: 50,
            },
        ],
        selected_idx: 0,
    };

    state.select_next();
    assert_eq!(state.selected_idx, 1);

    state.select_next();
    assert_eq!(state.selected_idx, 0); // Wraps

    state.select_prev();
    assert_eq!(state.selected_idx, 1); // Wraps backwards
}

#[test]
fn test_update_slash_state() {
    let mut state = SlashState::new();
    update_slash_state(&mut state, "/up", 3);
    assert!(state.active);
    assert!(!state.matches.is_empty());
    assert_eq!(state.start_byte, 0);
    assert_eq!(state.end_byte, 3);

    // Clear when not a slash command
    update_slash_state(&mut state, "hello", 5);
    assert!(!state.active);
    assert!(state.matches.is_empty());
}

#[test]
fn test_slash_state_clear() {
    let mut state = SlashState {
        active: true,
        start_byte: 0,
        end_byte: 5,
        matches: vec![SlashMatch {
            display: "test".to_string(),
            insert: "test".to_string(),
            description: "desc".to_string(),
            score: 100,
        }],
        selected_idx: 0,
    };

    state.clear();
    assert!(!state.active);
    assert_eq!(state.start_byte, 0);
    assert_eq!(state.end_byte, 0);
    assert!(state.matches.is_empty());
    assert_eq!(state.selected_idx, 0);
}

#[test]
fn test_selected_match() {
    let mut state = SlashState {
        active: true,
        start_byte: 0,
        end_byte: 2,
        matches: vec![
            SlashMatch {
                display: "/update".to_string(),
                insert: "/update".to_string(),
                description: "desc".to_string(),
                score: 100,
            },
            SlashMatch {
                display: "/config-dangerous".to_string(),
                insert: "/config-dangerous".to_string(),
                description: "desc".to_string(),
                score: 50,
            },
        ],
        selected_idx: 1,
    };

    assert_eq!(state.selected_match().unwrap().display, "/config-dangerous");

    state.active = false;
    assert!(state.selected_match().is_none());
}
