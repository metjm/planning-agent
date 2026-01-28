use serde::{Deserialize, Serialize};

/// Encapsulates scroll position and follow mode for a scrollable panel.
///
/// Follow mode means the view auto-scrolls to the bottom when content is added.
/// When the user manually scrolls UP, follow mode is disabled.
/// When the user scrolls DOWN, follow mode is preserved (allowing catching up).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrollState {
    /// Current scroll position (line offset from top)
    #[serde(default)]
    pub position: usize,
    /// When true, render at max_scroll regardless of stored position
    #[serde(default = "default_follow")]
    pub follow: bool,
}

fn default_follow() -> bool {
    true
}

impl Default for ScrollState {
    fn default() -> Self {
        Self::new()
    }
}

impl ScrollState {
    /// Create a new ScrollState with follow mode enabled.
    pub fn new() -> Self {
        Self {
            position: 0,
            follow: true,
        }
    }

    /// Scroll up by one line. Disables follow mode.
    pub fn scroll_up(&mut self) {
        self.follow = false;
        self.position = self.position.saturating_sub(1);
    }

    /// Scroll down by one line, clamped to max_scroll.
    ///
    /// NOTE: This preserves follow mode (unlike scroll_up which disables it).
    /// This allows users to scroll down to "catch up" without losing auto-scroll.
    pub fn scroll_down(&mut self, max_scroll: usize) {
        if self.position < max_scroll {
            self.position = self.position.saturating_add(1);
        }
    }

    /// Scroll to top. Disables follow mode.
    pub fn scroll_to_top(&mut self) {
        self.follow = false;
        self.position = 0;
    }

    /// Scroll to bottom. Enables follow mode and syncs position to max_scroll.
    ///
    /// This method REQUIRES max_scroll to ensure position is correctly synced.
    /// This prevents position jumps when follow mode is later disabled.
    pub fn scroll_to_bottom(&mut self, max_scroll: usize) {
        self.follow = true;
        self.position = max_scroll;
    }

    /// Get the effective scroll position for rendering.
    ///
    /// When follow mode is active, returns max_scroll (always show bottom).
    /// Otherwise returns the stored position clamped to max_scroll.
    ///
    /// ## Example Calculations
    ///
    /// ```text
    /// Given: position=50, follow=false, max_scroll=100
    /// effective_position(100) = min(50, 100) = 50
    ///
    /// Given: position=50, follow=true, max_scroll=100
    /// effective_position(100) = 100 (follow mode overrides stored position)
    ///
    /// Given: position=150, follow=false, max_scroll=100
    /// effective_position(100) = min(150, 100) = 100 (clamped to max)
    /// ```
    pub fn effective_position(&self, max_scroll: usize) -> usize {
        if self.follow {
            max_scroll
        } else {
            self.position.min(max_scroll)
        }
    }
}

#[cfg(test)]
#[path = "tests/state_tests.rs"]
mod tests;
