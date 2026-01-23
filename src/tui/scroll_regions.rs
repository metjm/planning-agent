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

/// Tracks the screen positions of all scrollable regions.
/// Updated on each frame during rendering.
#[derive(Debug, Default)]
pub struct ScrollableRegions {
    regions: Vec<(ScrollRegion, Rect)>,
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

    /// Register a scrollable region with its screen bounds.
    pub fn register(&mut self, region: ScrollRegion, bounds: Rect) {
        self.regions.push((region, bounds));
    }

    /// Find which region contains the given screen coordinate.
    /// Returns the highest-priority region (last registered = highest priority).
    pub fn region_at(&self, column: u16, row: u16) -> Option<ScrollRegion> {
        let pos = Position { x: column, y: row };
        // Iterate in reverse so overlays (registered last) take priority
        for (region, bounds) in self.regions.iter().rev() {
            if bounds.contains(pos) {
                return Some(*region);
            }
        }
        None
    }
}
