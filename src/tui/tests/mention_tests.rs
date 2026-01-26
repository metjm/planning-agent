use super::*;

#[test]
fn test_detect_mention_at_start() {
    let result = detect_mention_at_cursor("@test", 5);
    assert_eq!(result, Some((0, "test".to_string())));
}

#[test]
fn test_detect_mention_after_space() {
    let result = detect_mention_at_cursor("hello @wor", 10);
    assert_eq!(result, Some((6, "wor".to_string())));
}

#[test]
fn test_detect_mention_after_newline() {
    let result = detect_mention_at_cursor("line1\n@file", 11);
    assert_eq!(result, Some((6, "file".to_string())));
}

#[test]
fn test_detect_mention_after_punctuation() {
    let result = detect_mention_at_cursor("(@path", 6);
    assert_eq!(result, Some((1, "path".to_string())));
}

#[test]
fn test_no_mention_without_at() {
    let result = detect_mention_at_cursor("hello world", 11);
    assert_eq!(result, None);
}

#[test]
fn test_no_mention_with_space_in_query() {
    let result = detect_mention_at_cursor("@hello world", 12);
    assert_eq!(result, None);
}

#[test]
fn test_escaped_at_ignored() {
    let result = detect_mention_at_cursor("\\@test", 6);
    assert_eq!(result, None);
}

#[test]
fn test_at_in_middle_of_word() {
    let result = detect_mention_at_cursor("email@test", 10);
    assert_eq!(result, None);
}

#[test]
fn test_empty_query() {
    let result = detect_mention_at_cursor("@", 1);
    assert_eq!(result, Some((0, "".to_string())));
}

#[test]
fn test_cursor_at_start() {
    let result = detect_mention_at_cursor("@test", 0);
    assert_eq!(result, None);
}

#[test]
fn test_mention_state_select_prev() {
    use std::path::PathBuf;
    let mut state = MentionState {
        active: true,
        query: "test".to_string(),
        start_byte: 0,
        matches: vec![
            MentionMatch {
                display_path: "a".to_string(),
                absolute_path: PathBuf::from("a"),
                score: 10,
            },
            MentionMatch {
                display_path: "b".to_string(),
                absolute_path: PathBuf::from("b"),
                score: 9,
            },
            MentionMatch {
                display_path: "c".to_string(),
                absolute_path: PathBuf::from("c"),
                score: 8,
            },
        ],
        selected_idx: 0,
    };

    state.select_prev();
    assert_eq!(state.selected_idx, 2); // Wraps to end

    state.select_prev();
    assert_eq!(state.selected_idx, 1);
}

#[test]
fn test_mention_state_select_next() {
    use std::path::PathBuf;
    let mut state = MentionState {
        active: true,
        query: "test".to_string(),
        start_byte: 0,
        matches: vec![
            MentionMatch {
                display_path: "a".to_string(),
                absolute_path: PathBuf::from("a"),
                score: 10,
            },
            MentionMatch {
                display_path: "b".to_string(),
                absolute_path: PathBuf::from("b"),
                score: 9,
            },
        ],
        selected_idx: 0,
    };

    state.select_next();
    assert_eq!(state.selected_idx, 1);

    state.select_next();
    assert_eq!(state.selected_idx, 0); // Wraps to start
}

#[test]
fn test_mention_state_clear() {
    use std::path::PathBuf;
    let mut state = MentionState {
        active: true,
        query: "test".to_string(),
        start_byte: 5,
        matches: vec![MentionMatch {
            display_path: "a".to_string(),
            absolute_path: PathBuf::from("a"),
            score: 10,
        }],
        selected_idx: 0,
    };

    state.clear();
    assert!(!state.active);
    assert!(state.query.is_empty());
    assert_eq!(state.start_byte, 0);
    assert!(state.matches.is_empty());
    assert_eq!(state.selected_idx, 0);
}

#[test]
fn test_selected_match() {
    use std::path::PathBuf;
    let mut state = MentionState {
        active: true,
        query: "test".to_string(),
        start_byte: 0,
        matches: vec![
            MentionMatch {
                display_path: "a".to_string(),
                absolute_path: PathBuf::from("a"),
                score: 10,
            },
            MentionMatch {
                display_path: "b".to_string(),
                absolute_path: PathBuf::from("b"),
                score: 9,
            },
        ],
        selected_idx: 1,
    };

    assert_eq!(state.selected_match().unwrap().display_path, "b");

    state.active = false;
    assert!(state.selected_match().is_none());
}

#[test]
fn test_insert_text_with_absolute_path() {
    use std::path::PathBuf;
    // File with absolute path
    let file_match = MentionMatch {
        display_path: "src/main.rs".to_string(),
        absolute_path: PathBuf::from("/repo/src/main.rs"),
        score: 10,
    };
    assert_eq!(file_match.insert_text(), "/repo/src/main.rs");

    // Folder with absolute path (should preserve trailing slash)
    let folder_match = MentionMatch {
        display_path: "src/".to_string(),
        absolute_path: PathBuf::from("/repo/src"),
        score: 10,
    };
    assert_eq!(folder_match.insert_text(), "/repo/src/");
}

#[test]
fn test_insert_text_fallback_to_display_path() {
    use std::path::PathBuf;
    // When absolute_path is relative (fallback for tests without repo_root)
    let file_match = MentionMatch {
        display_path: "src/main.rs".to_string(),
        absolute_path: PathBuf::from("src/main.rs"),
        score: 10,
    };
    assert_eq!(file_match.insert_text(), "src/main.rs");
}
