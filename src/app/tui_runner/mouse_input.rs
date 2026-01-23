use crate::tui::scroll_regions::{ScrollRegion, ScrollableRegions};
use crate::tui::Session;
use crossterm::event::{MouseEvent, MouseEventKind};

/// Handle mouse scroll events.
/// Returns true if the event was handled.
pub fn handle_mouse_scroll(
    mouse: MouseEvent,
    session: &mut Session,
    regions: &ScrollableRegions,
) -> bool {
    let Some(region) = regions.region_at(mouse.column, mouse.row) else {
        return false;
    };

    let scroll_amount = 3; // Lines per scroll notch

    match mouse.kind {
        MouseEventKind::ScrollUp => {
            for _ in 0..scroll_amount {
                scroll_region_up(region, session);
            }
            true
        }
        MouseEventKind::ScrollDown => {
            // max_scroll is computed during rendering and cached in regions
            let max_scroll = regions.max_scroll_for(region);
            for _ in 0..scroll_amount {
                scroll_region_down(region, session, max_scroll);
            }
            true
        }
        _ => false,
    }
}

fn scroll_region_up(region: ScrollRegion, session: &mut Session) {
    match region {
        ScrollRegion::OutputPanel => {
            session.output_follow_mode = false;
            session.scroll_up();
        }
        ScrollRegion::TodosPanel => session.todo_scroll_up(),
        ScrollRegion::ChatContent => session.chat_scroll_up(),
        ScrollRegion::SummaryPanel => session.summary_scroll_up(),
        ScrollRegion::ReviewHistory => session.review_history_scroll_up(),
        ScrollRegion::PlanModal => session.plan_modal_scroll_up(),
        ScrollRegion::ReviewModal => session.review_modal_scroll_up(),
        ScrollRegion::ErrorOverlay => session.error_scroll_up(),
        ScrollRegion::ApprovalSummary => session.scroll_summary_up(),
    }
}

fn scroll_region_down(region: ScrollRegion, session: &mut Session, max_scroll: usize) {
    match region {
        ScrollRegion::OutputPanel => session.scroll_down(max_scroll),
        ScrollRegion::TodosPanel => session.todo_scroll_down(max_scroll),
        ScrollRegion::ChatContent => session.chat_scroll_down(max_scroll),
        ScrollRegion::SummaryPanel => session.summary_scroll_down(max_scroll),
        ScrollRegion::ReviewHistory => session.review_history_scroll_down(max_scroll),
        ScrollRegion::PlanModal => session.plan_modal_scroll_down(max_scroll),
        ScrollRegion::ReviewModal => session.review_modal_scroll_down(max_scroll),
        ScrollRegion::ErrorOverlay => session.error_scroll_down(max_scroll),
        ScrollRegion::ApprovalSummary => session.scroll_summary_down(max_scroll),
    }
}
