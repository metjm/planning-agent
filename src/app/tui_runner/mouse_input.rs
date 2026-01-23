use crate::tui::scroll_regions::{ScrollRegion, ScrollableRegions};
use crate::tui::Session;
use crossterm::event::{MouseEvent, MouseEventKind};

use super::input::{
    compute_chat_content_max_scroll, compute_error_overlay_max_scroll,
    compute_output_panel_max_scroll, compute_plan_modal_max_scroll,
    compute_review_history_max_scroll, compute_review_modal_max_scroll,
    compute_run_tab_summary_max_scroll, compute_todo_panel_max_scroll,
};

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
            let max_scroll = calculate_max_scroll(region, session);
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

/// Calculate max scroll for a given region.
fn calculate_max_scroll(region: ScrollRegion, session: &Session) -> usize {
    let (term_width, term_height) = crossterm::terminal::size().unwrap_or((80, 24));

    match region {
        ScrollRegion::OutputPanel => compute_output_panel_max_scroll(session),
        ScrollRegion::TodosPanel => compute_todo_panel_max_scroll(session),
        ScrollRegion::ChatContent => compute_chat_content_max_scroll(session),
        ScrollRegion::SummaryPanel => session
            .run_tabs
            .get(session.active_run_tab)
            .map(|tab| compute_run_tab_summary_max_scroll(&tab.summary_text))
            .unwrap_or(0),
        ScrollRegion::ReviewHistory => {
            compute_review_history_max_scroll(session, term_width, term_height)
        }
        ScrollRegion::PlanModal => compute_plan_modal_max_scroll(&session.plan_modal_content),
        ScrollRegion::ReviewModal => {
            compute_review_modal_max_scroll(session.current_review_content())
        }
        ScrollRegion::ErrorOverlay => session
            .error_state
            .as_ref()
            .map(|e| compute_error_overlay_max_scroll(e))
            .unwrap_or(0),
        ScrollRegion::ApprovalSummary => {
            // Approval summary uses same layout as plan summary
            0 // Computed dynamically in approval_input.rs
        }
    }
}
