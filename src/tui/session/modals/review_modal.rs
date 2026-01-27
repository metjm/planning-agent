//! Review modal methods for Session.
//!
//! Provides functionality to toggle, navigate, and scroll the review feedback modal.

use super::super::model::{ReviewKind, ReviewModalEntry};
use super::super::Session;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;

impl Session {
    /// Toggle the review modal open/closed.
    /// When opening, scans session directory for feedback files and loads them.
    /// Returns true if modal was opened, false if closed or no reviews exist.
    pub fn toggle_review_modal(&mut self, _working_dir: &Path) -> bool {
        if self.review_modal_open {
            self.close_review_modal();
            false
        } else {
            self.open_review_modal()
        }
    }

    fn open_review_modal(&mut self) -> bool {
        let Some(ref view) = self.workflow_view else {
            return false;
        };
        let Some(workflow_id) = view.workflow_id() else {
            return false;
        };

        // Get session directory
        let session_id = workflow_id.0.to_string();
        let session_dir = match crate::planning_paths::session_dir(&session_id) {
            Ok(dir) => dir,
            Err(_) => return false,
        };

        // Scan for feedback files
        let mut entries = Vec::new();
        if let Ok(read_dir) = fs::read_dir(&session_dir) {
            for entry in read_dir.flatten() {
                let path = entry.path();
                let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

                if let Some(entry) = Self::parse_review_entry(&path, filename) {
                    entries.push(entry);
                }
            }
        }

        if entries.is_empty() {
            return false;
        }

        // Sort by sort_key descending (most recent first, then by agent name)
        entries.sort_by(|a, b| {
            b.sort_key
                .cmp(&a.sort_key)
                .then_with(|| b.kind.sort_rank().cmp(&a.kind.sort_rank()))
        });

        self.review_modal_entries = entries;
        self.review_modal_tab = 0; // Select most recent
        self.review_modal_scroll = 0;
        self.review_modal_open = true;
        true
    }

    /// Compute a deterministic ordinal for agent name using hash.
    /// Returns 0 for None (single-agent reviews), hash-based value for named agents.
    fn agent_ordinal(agent_name: Option<&str>) -> u64 {
        match agent_name {
            None => 0,
            Some(name) => {
                let mut hasher = DefaultHasher::new();
                name.hash(&mut hasher);
                (hasher.finish() % 999_999) + 1 // 1 to 999_999
            }
        }
    }

    fn parse_review_entry(path: &Path, filename: &str) -> Option<ReviewModalEntry> {
        let (kind, iteration, agent_name) = if let Some(stem) = filename
            .strip_prefix("feedback_")
            .and_then(|s| s.strip_suffix(".md"))
        {
            let (iteration, agent) = if let Some((iter, agent)) = stem.split_once('_') {
                (iter.parse::<u32>().ok()?, Some(agent))
            } else {
                (stem.parse::<u32>().ok()?, None)
            };
            (ReviewKind::Plan, iteration, agent)
        } else if let Some(stem) = filename
            .strip_prefix("implementation_review_")
            .and_then(|s| s.strip_suffix(".md"))
        {
            let (iteration, agent) = if let Some((iter, agent)) = stem.split_once('_') {
                (iter.parse::<u32>().ok()?, Some(agent))
            } else {
                (stem.parse::<u32>().ok()?, None)
            };
            (ReviewKind::Implementation, iteration, agent)
        } else {
            return None;
        };

        let content =
            fs::read_to_string(path).unwrap_or_else(|e| format!("Error reading file: {}", e));

        let display_name = match kind {
            ReviewKind::Plan => match agent_name {
                Some(agent) => format!("Plan Round {} - {}", iteration, agent),
                None => format!("Plan Round {}", iteration),
            },
            ReviewKind::Implementation => match agent_name {
                Some(agent) => format!("Implementation Review {} - {}", iteration, agent),
                None => format!("Implementation Review {}", iteration),
            },
        };

        // Sort key: iteration * 1_000_000_000 + (kind_rank * 1_000_000) + (1_000_000 - agent_ordinal)
        // This gives higher sort_key to higher iterations, then kind rank, then agent ordinal
        let ordinal = Self::agent_ordinal(agent_name);
        let sort_key = (iteration as u64) * 1_000_000_000
            + (kind.sort_rank() * 1_000_000)
            + (1_000_000 - ordinal);

        Some(ReviewModalEntry {
            kind,
            display_name,
            content,
            sort_key,
        })
    }

    pub fn close_review_modal(&mut self) {
        self.review_modal_open = false;
        self.review_modal_entries.clear();
        self.review_modal_scroll = 0;
        self.review_modal_tab = 0;
    }

    pub fn review_modal_next_tab(&mut self) {
        if !self.review_modal_entries.is_empty() {
            self.review_modal_tab = (self.review_modal_tab + 1) % self.review_modal_entries.len();
            self.review_modal_scroll = 0; // Reset scroll when switching tabs
        }
    }

    pub fn review_modal_prev_tab(&mut self) {
        if !self.review_modal_entries.is_empty() {
            self.review_modal_tab = if self.review_modal_tab == 0 {
                self.review_modal_entries.len() - 1
            } else {
                self.review_modal_tab - 1
            };
            self.review_modal_scroll = 0;
        }
    }

    pub fn review_modal_scroll_up(&mut self) {
        self.review_modal_scroll = self.review_modal_scroll.saturating_sub(1);
    }

    pub fn review_modal_scroll_down(&mut self, max_scroll: usize) {
        if self.review_modal_scroll < max_scroll {
            self.review_modal_scroll += 1;
        }
    }

    pub fn review_modal_scroll_to_top(&mut self) {
        self.review_modal_scroll = 0;
    }

    pub fn review_modal_scroll_to_bottom(&mut self, max_scroll: usize) {
        self.review_modal_scroll = max_scroll;
    }

    pub fn review_modal_page_down(&mut self, visible_height: usize, max_scroll: usize) {
        self.review_modal_scroll = (self.review_modal_scroll + visible_height).min(max_scroll);
    }

    pub fn review_modal_page_up(&mut self, visible_height: usize) {
        self.review_modal_scroll = self.review_modal_scroll.saturating_sub(visible_height);
    }

    /// Get the currently selected review content.
    pub fn current_review_content(&self) -> &str {
        self.review_modal_entries
            .get(self.review_modal_tab)
            .map(|e| e.content.as_str())
            .unwrap_or("")
    }
}

#[cfg(test)]
#[path = "../tests/review_modal_tests.rs"]
mod tests;
