use super::*;
use crate::domain::types::WorkflowId;
use crate::domain::view::WorkflowView;
use crate::planning_paths::{session_dir, set_home_for_test};
use std::path::Path;
use tempfile::tempdir;

/// Creates a session with a workflow_view that has a random workflow_id.
/// Returns (session, session_id_string) where session_id_string is the
/// UUID string that should be used for session_dir().
fn setup_session() -> (Session, String) {
    let mut session = Session::new(0);
    let workflow_id = uuid::Uuid::new_v4();
    let mut view = WorkflowView::default();
    view.workflow_id = Some(WorkflowId(workflow_id));
    session.workflow_view = Some(view);
    (session, workflow_id.to_string())
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
