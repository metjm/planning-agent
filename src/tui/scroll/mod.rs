//! Scroll-related types for TUI panels.
//!
//! - `ScrollableRegions`: Tracks screen positions and scroll bounds of all scrollable regions
//! - `ScrollState`: Encapsulates scroll position and follow mode for a scrollable panel

mod regions;
mod state;

pub use regions::{ScrollRegion, ScrollableRegions};
pub use state::ScrollState;
