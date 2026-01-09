//! File index for @-mention auto-complete functionality.
//! Built once at TUI startup from `git ls-files` output.
//! Includes both files and folders from the working directory.
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct FileEntry {
    /// The relative file path (folders have trailing `/`)
    pub path: String,
    /// Lowercase version of the full path for case-insensitive matching
    pub path_lower: String,
    /// Lowercase version of just the filename/folder name for boosted matching
    pub file_name_lower: String,
}

impl FileEntry {
    pub fn new(path: String) -> Self {
        Self::new_with_type(path)
    }

    pub fn new_folder(path: String) -> Self {
        // Ensure folder paths end with /
        let path = if path.ends_with('/') {
            path
        } else {
            format!("{}/", path)
        };
        Self::new_with_type(path)
    }

    fn new_with_type(path: String) -> Self {
        let path_lower = path.to_lowercase();
        // For folders, strip trailing / before extracting name
        let path_for_name = path.trim_end_matches('/');
        let file_name_lower = path_for_name
            .rsplit('/')
            .next()
            .unwrap_or(path_for_name)
            .to_lowercase();
        Self {
            path,
            path_lower,
            file_name_lower,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub enum FileIndexStatus {
    /// Index is still being built
    #[default]
    Loading,
    /// Index is ready for use
    Ready,
    /// Index build failed (e.g., not a git repo, git not available)
    Error,
}

#[derive(Debug, Clone, Default)]
pub struct FileIndex {
    pub status: FileIndexStatus,
    pub entries: Vec<FileEntry>,
}

#[derive(Debug, Clone)]
pub struct MentionMatch {
    pub path: String,
    pub score: i32,
}

impl FileIndex {
    pub fn new() -> Self {
        Self {
            status: FileIndexStatus::Loading,
            entries: Vec::new(),
        }
    }

    pub fn with_entries(entries: Vec<FileEntry>) -> Self {
        Self {
            status: FileIndexStatus::Ready,
            entries,
        }
    }

    pub fn with_error() -> Self {
        Self {
            status: FileIndexStatus::Error,
            entries: Vec::new(),
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self.status, FileIndexStatus::Ready)
    }

    /// Find matches for the given query string.
    /// Returns up to `limit` matches sorted by relevance score.
    pub fn find_matches(&self, query: &str, limit: usize) -> Vec<MentionMatch> {
        if query.is_empty() || !self.is_ready() {
            return Vec::new();
        }

        let query_lower = query.to_lowercase();
        let mut matches: Vec<MentionMatch> = Vec::new();

        for entry in &self.entries {
            if let Some(score) = self.compute_score(entry, &query_lower) {
                matches.push(MentionMatch {
                    path: entry.path.clone(),
                    score,
                });
            }
        }

        // Sort by score (higher is better), then by path length (shorter is better),
        // then lexicographically for stability
        matches.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.path.len().cmp(&b.path.len()))
                .then_with(|| a.path.cmp(&b.path))
        });

        matches.truncate(limit);
        matches
    }

    /// Compute a relevance score for an entry given a query.
    /// Returns None if the entry doesn't match the query at all.
    fn compute_score(&self, entry: &FileEntry, query_lower: &str) -> Option<i32> {
        let mut score = 0i32;
        let mut matched = false;

        // Check exact filename match (highest boost)
        if entry.file_name_lower == query_lower {
            score += 100;
            matched = true;
        }
        // Check filename prefix match
        else if entry.file_name_lower.starts_with(query_lower) {
            score += 50;
            matched = true;
        }
        // Check filename substring match
        else if entry.file_name_lower.contains(query_lower) {
            score += 30;
            matched = true;
        }
        // Check path segment prefix match (e.g., "src/" matches "src/main.rs")
        else if entry.path_lower.split('/').any(|seg| seg.starts_with(query_lower)) {
            score += 20;
            matched = true;
        }
        // Check full path substring match
        else if entry.path_lower.contains(query_lower) {
            score += 10;
            matched = true;
        }

        if matched {
            // Boost for shorter paths (more specific)
            let path_len_penalty = (entry.path.len() as i32).min(50);
            score -= path_len_penalty / 5;

            Some(score)
        } else {
            None
        }
    }
}

/// Build a file index by running `git ls-files` in the given directory.
/// Includes both files and folders (extracted from file paths).
/// This function is meant to be called via `tokio::task::spawn_blocking`.
pub fn build_file_index(working_dir: &std::path::Path) -> FileIndex {
    use std::process::Command;

    // Try to run git ls-files
    let output = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(working_dir)
        .output();

    match output {
        Ok(output) => {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);

                // Collect all file entries
                let file_paths: Vec<&str> = stdout
                    .split('\0')
                    .filter(|s| !s.is_empty())
                    .collect();

                // Extract unique folder paths from file paths
                let mut folder_set: HashSet<String> = HashSet::new();
                for path in &file_paths {
                    // Extract all parent directories
                    let mut current = *path;
                    while let Some(pos) = current.rfind('/') {
                        let folder = &current[..pos];
                        if !folder.is_empty() && folder_set.insert(folder.to_string()) {
                            // Continue to extract parent folders
                        }
                        current = folder;
                    }
                }

                // Build entries: folders first, then files
                let mut entries: Vec<FileEntry> = Vec::with_capacity(
                    folder_set.len() + file_paths.len()
                );

                // Add folders (sorted for consistent ordering)
                let mut folders: Vec<String> = folder_set.into_iter().collect();
                folders.sort();
                for folder in folders {
                    entries.push(FileEntry::new_folder(folder));
                }

                // Add files
                for path in file_paths {
                    entries.push(FileEntry::new(path.to_string()));
                }

                FileIndex::with_entries(entries)
            } else {
                FileIndex::with_error()
            }
        }
        Err(_) => FileIndex::with_error(),
    }
}

#[cfg(test)]
mod tests {
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
        assert!(matches.iter().any(|m| m.path == "src/"));
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
        assert!(matches.iter().any(|m| m.path.ends_with("components/")));
    }

    #[test]
    fn test_empty_query_returns_no_matches() {
        let index = FileIndex::with_entries(vec![
            FileEntry::new("src/main.rs".to_string()),
        ]);
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
        assert!(matches[0].path.ends_with("main.rs"));
        assert!(!matches[0].path.contains("test"));
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
        let index = FileIndex::with_entries(vec![
            FileEntry::new("src/MainFile.rs".to_string()),
        ]);
        let matches = index.find_matches("mainfile", 10);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path, "src/MainFile.rs");
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
        assert_eq!(matches[0].path, "short/file.rs");
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
}
