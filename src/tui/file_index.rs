//! File index for @-mention auto-complete functionality.
//! Built once at TUI startup from `git ls-files` output.
//! Includes both files and folders from the working directory.
use std::collections::HashSet;
use std::path::PathBuf;

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
    /// Repository root path for computing absolute paths.
    /// When `Some`, `find_matches` will produce absolute paths for insertion.
    /// When `None` (for tests or loading state), insertion falls back to relative display paths.
    pub repo_root: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct MentionMatch {
    /// Relative path for display in the dropdown (original git ls-files path)
    pub display_path: String,
    /// Absolute path for insertion (repo_root joined with relative path)
    pub absolute_path: PathBuf,
    pub score: i32,
}

impl MentionMatch {
    /// Returns the text to insert when this match is selected.
    /// Uses absolute path if available, otherwise falls back to display_path.
    /// Preserves trailing slash for folders.
    pub fn insert_text(&self) -> String {
        if self.absolute_path.is_absolute() {
            let mut path_str = self.absolute_path.to_string_lossy().to_string();
            // Preserve trailing slash for folders (display_path has trailing / for folders)
            if self.display_path.ends_with('/') && !path_str.ends_with('/') {
                path_str.push('/');
            }
            path_str
        } else {
            // Fallback for tests without repo_root
            self.display_path.clone()
        }
    }
}

impl FileIndex {
    pub fn new() -> Self {
        Self {
            status: FileIndexStatus::Loading,
            entries: Vec::new(),
            repo_root: None,
        }
    }

    /// Create a file index with entries but no repo root (for tests).
    /// When repo_root is None, insert_text() returns display_path (relative path).
    #[cfg(test)]
    pub fn with_entries(entries: Vec<FileEntry>) -> Self {
        Self {
            status: FileIndexStatus::Ready,
            entries,
            repo_root: None,
        }
    }

    /// Create a file index with entries and a repo root (for tests that need absolute paths).
    pub fn with_entries_and_root(entries: Vec<FileEntry>, repo_root: PathBuf) -> Self {
        Self {
            status: FileIndexStatus::Ready,
            entries,
            repo_root: Some(repo_root),
        }
    }

    pub fn with_error() -> Self {
        Self {
            status: FileIndexStatus::Error,
            entries: Vec::new(),
            repo_root: None,
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
                // Compute absolute path if repo_root is available
                let absolute_path = match &self.repo_root {
                    Some(root) => root.join(&entry.path),
                    None => PathBuf::from(&entry.path), // Fallback: relative path as PathBuf
                };
                matches.push(MentionMatch {
                    display_path: entry.path.clone(),
                    absolute_path,
                    score,
                });
            }
        }

        // Sort by score (higher is better), then by path length (shorter is better),
        // then lexicographically for stability
        matches.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.display_path.len().cmp(&b.display_path.len()))
                .then_with(|| a.display_path.cmp(&b.display_path))
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
        else if entry
            .path_lower
            .split('/')
            .any(|seg| seg.starts_with(query_lower))
        {
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
///
/// Note: `git ls-files` outputs paths relative to the **repository root**, not the working_dir.
/// We use `git rev-parse --show-toplevel` to get the repo root for computing correct absolute paths.
pub fn build_file_index(working_dir: &std::path::Path) -> FileIndex {
    use std::process::Command;

    // First, get the repository root using `git rev-parse --show-toplevel`.
    // This is necessary because `git ls-files` outputs repo-root-relative paths,
    // not working_dir-relative paths. Using working_dir as base would produce
    // incorrect paths when working_dir is a subdirectory.
    let repo_root_output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(working_dir)
        .output();

    let repo_root = match repo_root_output {
        Ok(output) if output.status.success() => {
            let raw_root = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let root_path = PathBuf::from(&raw_root);
            // Canonicalize to resolve symlinks; fall back to raw path if canonicalization fails
            std::fs::canonicalize(&root_path).unwrap_or(root_path)
        }
        _ => {
            // Not a git repo or git not available
            return FileIndex::with_error();
        }
    };

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
                let file_paths: Vec<&str> = stdout.split('\0').filter(|s| !s.is_empty()).collect();

                // Extract unique folder paths from file paths
                let mut folder_set: HashSet<String> = HashSet::new();
                for path in &file_paths {
                    // Extract all parent directories
                    let mut current = *path;
                    while let Some(pos) = current.rfind('/') {
                        let folder = current.get(..pos).unwrap_or("");
                        if !folder.is_empty() && folder_set.insert(folder.to_string()) {
                            // Continue to extract parent folders
                        }
                        current = folder;
                    }
                }

                // Build entries: folders first, then files
                let mut entries: Vec<FileEntry> =
                    Vec::with_capacity(folder_set.len() + file_paths.len());

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

                FileIndex::with_entries_and_root(entries, repo_root)
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
}
