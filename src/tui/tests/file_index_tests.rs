use super::*;

#[test]
fn test_file_entry_creation() {
    let entry = FileEntry::new("src/main.rs".to_string());
    assert_eq!(entry.path, "src/main.rs");
    assert_eq!(entry.path_lower, "src/main.rs");
    assert_eq!(entry.file_name_lower, "main.rs");
    assert!(!entry.path.ends_with('/'));
}

#[test]
fn test_file_entry_with_mixed_case() {
    let entry = FileEntry::new("Src/MainFile.RS".to_string());
    assert_eq!(entry.path, "Src/MainFile.RS");
    assert_eq!(entry.path_lower, "src/mainfile.rs");
    assert_eq!(entry.file_name_lower, "mainfile.rs");
    assert!(!entry.path.ends_with('/'));
}

#[test]
fn test_folder_entry_creation() {
    let entry = FileEntry::new_folder("src/components".to_string());
    assert_eq!(entry.path, "src/components/");
    assert_eq!(entry.path_lower, "src/components/");
    assert_eq!(entry.file_name_lower, "components");
    assert!(entry.path.ends_with('/'));
}

#[test]
fn test_folder_entry_already_has_slash() {
    let entry = FileEntry::new_folder("src/components/".to_string());
    assert_eq!(entry.path, "src/components/");
    assert!(entry.path.ends_with('/'));
}

#[test]
fn test_folder_matching() {
    let index = FileIndex::with_entries(vec![
        FileEntry::new_folder("src".to_string()),
        FileEntry::new_folder("src/components".to_string()),
        FileEntry::new("src/main.rs".to_string()),
    ]);
    let matches = index.find_matches("src", 10);
    assert_eq!(matches.len(), 3);
    // "src/" should match with high score
    assert!(matches.iter().any(|m| m.display_path == "src/"));
}

#[test]
fn test_folder_name_matching() {
    let index = FileIndex::with_entries(vec![
        FileEntry::new_folder("src/components".to_string()),
        FileEntry::new_folder("lib/components".to_string()),
        FileEntry::new("src/components.rs".to_string()),
    ]);
    let matches = index.find_matches("components", 10);
    assert_eq!(matches.len(), 3);
    // All should match
    assert!(matches
        .iter()
        .any(|m| m.display_path.ends_with("components/")));
}

#[test]
fn test_empty_query_returns_no_matches() {
    let index = FileIndex::with_entries(vec![FileEntry::new("src/main.rs".to_string())]);
    let matches = index.find_matches("", 10);
    assert!(matches.is_empty());
}

#[test]
fn test_exact_filename_match_scores_highest() {
    let index = FileIndex::with_entries(vec![
        FileEntry::new("src/main.rs".to_string()),
        FileEntry::new("src/main_test.rs".to_string()),
        FileEntry::new("other/main.rs".to_string()),
    ]);
    let matches = index.find_matches("main.rs", 10);
    assert!(!matches.is_empty());
    // Both exact matches should be first
    assert!(matches[0].display_path.ends_with("main.rs"));
    assert!(!matches[0].display_path.contains("test"));
}

#[test]
fn test_prefix_match() {
    let index = FileIndex::with_entries(vec![
        FileEntry::new("src/config.rs".to_string()),
        FileEntry::new("src/configure.rs".to_string()),
        FileEntry::new("src/reconfig.rs".to_string()),
    ]);
    let matches = index.find_matches("config", 10);
    assert_eq!(matches.len(), 3);
    // Prefix matches should score higher than substring
    assert!(matches[0].score > matches[2].score);
}

#[test]
fn test_case_insensitive_matching() {
    let index = FileIndex::with_entries(vec![FileEntry::new("src/MainFile.rs".to_string())]);
    let matches = index.find_matches("mainfile", 10);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].display_path, "src/MainFile.rs");
}

#[test]
fn test_limit_results() {
    let index = FileIndex::with_entries(vec![
        FileEntry::new("a/test1.rs".to_string()),
        FileEntry::new("b/test2.rs".to_string()),
        FileEntry::new("c/test3.rs".to_string()),
        FileEntry::new("d/test4.rs".to_string()),
        FileEntry::new("e/test5.rs".to_string()),
    ]);
    let matches = index.find_matches("test", 3);
    assert_eq!(matches.len(), 3);
}

#[test]
fn test_shorter_paths_preferred() {
    let index = FileIndex::with_entries(vec![
        FileEntry::new("very/deep/nested/path/file.rs".to_string()),
        FileEntry::new("short/file.rs".to_string()),
    ]);
    let matches = index.find_matches("file.rs", 10);
    assert_eq!(matches.len(), 2);
    // Shorter path should come first (same base score, but shorter path bonus)
    assert_eq!(matches[0].display_path, "short/file.rs");
}

#[test]
fn test_loading_index_returns_no_matches() {
    let index = FileIndex::new();
    let matches = index.find_matches("test", 10);
    assert!(matches.is_empty());
}

#[test]
fn test_error_index_returns_no_matches() {
    let index = FileIndex::with_error();
    let matches = index.find_matches("test", 10);
    assert!(matches.is_empty());
}
