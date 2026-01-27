use super::*;
use crate::domain::types::{
    FeatureName, FeedbackPath, MaxIterations, Objective, PlanPath, TimestampUtc, WorkingDir,
};
use crate::domain::view::WorkflowView;
use crate::domain::WorkflowEvent;
use crate::planning_paths::{session_dir, set_home_for_test};
use std::path::{Path, PathBuf};
use tempfile::tempdir;
use uuid::Uuid;

/// Creates a session with a workflow_view that has a random workflow_id.
/// Returns (session, session_id_string) where session_id_string is the
/// UUID string that should be used for session_dir().
fn setup_session() -> (Session, String) {
    let mut session = Session::new(0);
    let workflow_id = Uuid::new_v4();
    let agg_id = workflow_id.to_string();
    let mut view = WorkflowView::default();

    view.apply_event(
        &agg_id,
        &WorkflowEvent::WorkflowCreated {
            feature_name: FeatureName::from("test-feature"),
            objective: Objective::from("Test objective"),
            working_dir: WorkingDir::from(PathBuf::from("/tmp/test").as_path()),
            max_iterations: MaxIterations(3),
            plan_path: PlanPath::from(PathBuf::from("/tmp/test/plan.md")),
            feedback_path: FeedbackPath::from(PathBuf::from("/tmp/test/feedback.md")),
            created_at: TimestampUtc::now(),
        },
        1,
    );

    session.workflow_view = Some(view);
    (session, agg_id)
}

#[test]
fn test_review_modal_loads_plan_and_implementation_reviews() {
    let temp = tempdir().expect("tempdir");
    let _guard = set_home_for_test(temp.path().to_path_buf());

    let (mut session, session_id) = setup_session();
    let dir = session_dir(&session_id).expect("session dir");
    fs::create_dir_all(&dir).expect("create dir");
    fs::write(dir.join("feedback_1.md"), "Plan review").expect("write plan");
    fs::write(
        dir.join("implementation_review_1.md"),
        "Implementation review",
    )
    .expect("write implementation");

    assert!(session.toggle_review_modal(Path::new(".")));

    let names: Vec<String> = session
        .review_modal_entries
        .iter()
        .map(|entry| entry.display_name.clone())
        .collect();
    assert!(names.contains(&"Plan Round 1".to_string()));
    assert!(names.contains(&"Implementation Review 1".to_string()));
}

#[test]
fn test_review_modal_orders_by_iteration() {
    let temp = tempdir().expect("tempdir");
    let _guard = set_home_for_test(temp.path().to_path_buf());

    let (mut session, session_id) = setup_session();
    let dir = session_dir(&session_id).expect("session dir");
    fs::create_dir_all(&dir).expect("create dir");
    fs::write(dir.join("feedback_1.md"), "Plan review").expect("write plan");
    fs::write(
        dir.join("implementation_review_2.md"),
        "Implementation review",
    )
    .expect("write implementation");

    assert!(session.toggle_review_modal(Path::new(".")));

    let first = session.review_modal_entries.first().expect("first entry");
    assert_eq!(first.display_name, "Implementation Review 2");
}

// Helper to create test entries with specified display names
fn create_test_entries(names: &[&str]) -> Vec<crate::tui::session::model::ReviewModalEntry> {
    names
        .iter()
        .enumerate()
        .map(|(i, name)| crate::tui::session::model::ReviewModalEntry {
            kind: crate::tui::session::model::ReviewKind::Plan,
            display_name: name.to_string(),
            content: String::new(),
            sort_key: (i as u64 + 1) * 1_000_000_000,
        })
        .collect()
}

#[test]
fn test_compute_tab_viewport_with_narrow_width() {
    // 5 entries with ~20 char names, ~23 width with brackets "[name] "
    let entries = create_test_entries(&[
        "Plan Round 1 - claude",
        "Plan Round 2 - gemini",
        "Plan Round 3 - codex",
        "Plan Round 4 - test",
        "Plan Round 5 - final",
    ]);

    // Width of 50 should fit ~2 tabs (each ~24 chars + overflow indicators)
    let (range, has_left, has_right) = super::compute_tab_viewport(&entries, 0, 50);

    assert_eq!(range.start, 0);
    assert!(range.end <= 3); // Should fit 1-2 tabs
    assert!(!has_left);
    assert!(has_right);
}

#[test]
fn test_compute_tab_viewport_scrolls_right() {
    let entries = create_test_entries(&[
        "Plan Round 1 - claude",
        "Plan Round 2 - gemini",
        "Plan Round 3 - codex",
        "Plan Round 4 - test",
        "Plan Round 5 - final",
    ]);

    // Start scrolled to tab 3
    let (range, has_left, has_right) = super::compute_tab_viewport(&entries, 3, 50);

    assert_eq!(range.start, 3);
    assert!(has_left);
    assert!(has_right || range.end == 5); // Either has right overflow or showing last tabs
}

#[test]
fn test_compute_tab_viewport_scrolls_left() {
    let entries = create_test_entries(&[
        "Plan Round 1 - claude",
        "Plan Round 2 - gemini",
        "Plan Round 3 - codex",
    ]);

    // At scroll 0, should show left indicator false
    let (_, has_left, _) = super::compute_tab_viewport(&entries, 0, 50);
    assert!(!has_left);

    // At scroll 1, should show left indicator true
    let (_, has_left, _) = super::compute_tab_viewport(&entries, 1, 50);
    assert!(has_left);
}

#[test]
fn test_compute_tab_viewport_all_tabs_fit() {
    // 2 entries with short names
    let entries = create_test_entries(&["Tab1", "Tab2"]);

    let (range, has_left, has_right) = super::compute_tab_viewport(&entries, 0, 200);

    assert_eq!(range, 0..2);
    assert!(!has_left);
    assert!(!has_right);
}

#[test]
fn test_compute_tab_viewport_single_tab() {
    let entries = create_test_entries(&["Single Tab"]);

    let (range, has_left, has_right) = super::compute_tab_viewport(&entries, 0, 50);

    assert_eq!(range, 0..1);
    assert!(!has_left);
    assert!(!has_right);
}

#[test]
fn test_compute_tab_viewport_zero_width() {
    let entries = create_test_entries(&["Tab1", "Tab2", "Tab3"]);

    let (range, has_left, has_right) = super::compute_tab_viewport(&entries, 0, 0);

    assert_eq!(range, 0..0);
    assert!(!has_left);
    assert!(!has_right);
}

#[test]
fn test_compute_tab_viewport_very_small_width() {
    // Very small width should still show at least one tab
    let entries = create_test_entries(&[
        "Plan Round 1 - claude",
        "Plan Round 2 - gemini",
        "Plan Round 3 - codex",
    ]);

    let (range, _, _) = super::compute_tab_viewport(&entries, 0, 10);

    // Should force at least one tab visible
    assert_eq!(range, 0..1);
}

#[test]
fn test_compute_tab_viewport_empty_entries() {
    let entries: Vec<crate::tui::session::model::ReviewModalEntry> = Vec::new();

    let (range, has_left, has_right) = super::compute_tab_viewport(&entries, 0, 50);

    assert_eq!(range, 0..0);
    assert!(!has_left);
    assert!(!has_right);
}

#[test]
fn test_ensure_tab_visible_scrolls_right() {
    let temp = tempdir().expect("tempdir");
    let _guard = set_home_for_test(temp.path().to_path_buf());

    let (mut session, _) = setup_session();
    session.review_modal_entries = create_test_entries(&[
        "Plan Round 1 - claude",
        "Plan Round 2 - gemini",
        "Plan Round 3 - codex",
        "Plan Round 4 - test",
        "Plan Round 5 - final",
    ]);
    session.review_modal_tab = 4; // Select last tab
    session.review_modal_tab_scroll = 0;

    session.ensure_review_tab_visible(50);

    // Should have scrolled to make tab 4 visible
    assert!(session.review_modal_tab_scroll > 0);
}

#[test]
fn test_ensure_tab_visible_scrolls_left() {
    let temp = tempdir().expect("tempdir");
    let _guard = set_home_for_test(temp.path().to_path_buf());

    let (mut session, _) = setup_session();
    session.review_modal_entries = create_test_entries(&[
        "Plan Round 1 - claude",
        "Plan Round 2 - gemini",
        "Plan Round 3 - codex",
        "Plan Round 4 - test",
        "Plan Round 5 - final",
    ]);
    session.review_modal_tab = 0; // Select first tab
    session.review_modal_tab_scroll = 3; // But scroll is at tab 3

    session.ensure_review_tab_visible(50);

    // Should scroll left to show tab 0
    assert_eq!(session.review_modal_tab_scroll, 0);
}

#[test]
fn test_review_modal_next_tab_maintains_visibility() {
    let temp = tempdir().expect("tempdir");
    let _guard = set_home_for_test(temp.path().to_path_buf());

    let (mut session, _) = setup_session();
    session.review_modal_entries = create_test_entries(&[
        "Plan Round 1 - claude",
        "Plan Round 2 - gemini",
        "Plan Round 3 - codex",
        "Plan Round 4 - test",
        "Plan Round 5 - final",
    ]);
    session.review_modal_tab = 0;
    session.review_modal_tab_scroll = 0;

    // Navigate through all tabs and verify the selected tab is always in visible range
    for _ in 0..5 {
        session.review_modal_next_tab(50);
        let (range, _, _) = super::compute_tab_viewport(
            &session.review_modal_entries,
            session.review_modal_tab_scroll,
            50,
        );
        assert!(
            range.contains(&session.review_modal_tab),
            "Tab {} should be in visible range {:?}",
            session.review_modal_tab,
            range
        );
    }
}

#[test]
fn test_review_modal_prev_tab_wraps_with_visibility() {
    let temp = tempdir().expect("tempdir");
    let _guard = set_home_for_test(temp.path().to_path_buf());

    let (mut session, _) = setup_session();
    session.review_modal_entries = create_test_entries(&[
        "Plan Round 1 - claude",
        "Plan Round 2 - gemini",
        "Plan Round 3 - codex",
        "Plan Round 4 - test",
        "Plan Round 5 - final",
    ]);
    session.review_modal_tab = 0;
    session.review_modal_tab_scroll = 0;

    // Wrap from first to last tab
    session.review_modal_prev_tab(50);

    assert_eq!(session.review_modal_tab, 4); // Should wrap to last
    let (range, _, _) = super::compute_tab_viewport(
        &session.review_modal_entries,
        session.review_modal_tab_scroll,
        50,
    );
    assert!(
        range.contains(&session.review_modal_tab),
        "Tab 4 should be visible after wrapping"
    );
}
