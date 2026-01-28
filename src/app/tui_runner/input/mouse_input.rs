use crate::tui::scroll_regions::{ScrollRegion, ScrollableRegions};
use crate::tui::{FocusedPanel, Session};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

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

/// Handle mouse click events for panel focus selection.
/// Returns true if the event was handled (panel was focused).
///
/// Click-to-focus respects the same visibility rules as Tab navigation:
/// - Todos: only focusable when visible (terminal width >= 80 AND todos exist)
/// - Summary: only focusable when summary_state != None
/// - ChatInput: not directly clickable (use Tab from Chat panel)
/// - Modal regions: ignored (modals capture input separately)
///
/// Only left-clicks are processed (standard UX for focus selection).
pub fn handle_mouse_click(
    mouse: MouseEvent,
    session: &mut Session,
    regions: &ScrollableRegions,
    todos_visible: bool,
    summary_visible: bool,
) -> bool {
    // Only handle left mouse button down events
    if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
        return false;
    }

    let Some(region) = regions.region_at(mouse.column, mouse.row) else {
        return false;
    };

    // Map ScrollRegion to FocusedPanel, respecting visibility constraints
    let new_focus = match region {
        ScrollRegion::OutputPanel => Some(FocusedPanel::Output),
        ScrollRegion::TodosPanel => {
            if todos_visible {
                Some(FocusedPanel::Todos)
            } else {
                None
            }
        }
        ScrollRegion::ChatContent => Some(FocusedPanel::Chat),
        ScrollRegion::SummaryPanel => {
            if summary_visible {
                Some(FocusedPanel::Summary)
            } else {
                None
            }
        }
        // ReviewHistory and modal regions are not in the tab cycle
        ScrollRegion::ReviewHistory
        | ScrollRegion::PlanModal
        | ScrollRegion::ReviewModal
        | ScrollRegion::ErrorOverlay
        | ScrollRegion::ApprovalSummary => None,
    };

    if let Some(panel) = new_focus {
        session.focused_panel = panel;
        true
    } else {
        false
    }
}
