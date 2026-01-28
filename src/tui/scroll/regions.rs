use ratatui::layout::{Position, Rect};

/// Identifies a scrollable region in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScrollRegion {
    OutputPanel,
    TodosPanel,
    ChatContent,
    SummaryPanel,
    ReviewHistory,
    PlanModal,
    ReviewModal,
    ErrorOverlay,
    ApprovalSummary,
}

/// Tracks the screen positions and scroll bounds of all scrollable regions.
/// Updated on each frame during rendering. This is the single source of truth
/// for max_scroll values - computed once during render, used by mouse/keyboard handlers.
#[derive(Debug, Default)]
pub struct ScrollableRegions {
    /// (region, bounds, max_scroll)
    regions: Vec<(ScrollRegion, Rect, usize)>,
}

impl ScrollableRegions {
    pub fn new() -> Self {
        Self {
            regions: Vec::new(),
        }
    }

    /// Clear all registered regions (call at start of each frame).
    pub fn clear(&mut self) {
        self.regions.clear();
    }

    /// Register a scrollable region with its screen bounds and max scroll value.
    /// The max_scroll should be computed by the render function using actual content.
    pub fn register(&mut self, region: ScrollRegion, bounds: Rect, max_scroll: usize) {
        self.regions.push((region, bounds, max_scroll));
    }

    /// Find which region contains the given screen coordinate.
    /// Returns the highest-priority region (last registered = highest priority).
    pub fn region_at(&self, column: u16, row: u16) -> Option<ScrollRegion> {
        let pos = Position { x: column, y: row };
        // Iterate in reverse so overlays (registered last) take priority
        for (region, bounds, _) in self.regions.iter().rev() {
            if bounds.contains(pos) {
                return Some(*region);
            }
        }
        None
    }

    /// Get the max_scroll value for a region.
    /// Returns 0 if region is not found.
    pub fn max_scroll_for(&self, target: ScrollRegion) -> usize {
        // Search in reverse for overlays priority
        for (region, _, max_scroll) in self.regions.iter().rev() {
            if *region == target {
                return *max_scroll;
            }
        }
        0
    }
}
