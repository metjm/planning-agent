use super::*;
use crate::planning_paths::{session_dir, set_home_for_test};
use crate::state::State;
use std::path::Path;
use tempfile::tempdir;

fn setup_session(session_id: &str) -> Session {
    let mut session = Session::new(0);
    let mut state = State::new("test-feature", "objective", 1).expect("state");
    state.workflow_session_id = session_id.to_string();
    session.workflow_state = Some(state);
    session
}

#[test]
fn test_review_modal_loads_plan_and_implementation_reviews() {
    let temp = tempdir().expect("tempdir");
    let _guard = set_home_for_test(temp.path().to_path_buf());

    let session_id = "session-1";
    let dir = session_dir(session_id).expect("session dir");
    fs::create_dir_all(&dir).expect("create dir");
    fs::write(dir.join("feedback_1.md"), "Plan review").expect("write plan");
    fs::write(
        dir.join("implementation_review_1.md"),
        "Implementation review",
    )
    .expect("write implementation");

    let mut session = setup_session(session_id);

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

    let session_id = "session-2";
    let dir = session_dir(session_id).expect("session dir");
    fs::create_dir_all(&dir).expect("create dir");
    fs::write(dir.join("feedback_1.md"), "Plan review").expect("write plan");
    fs::write(
        dir.join("implementation_review_2.md"),
        "Implementation review",
    )
    .expect("write implementation");

    let mut session = setup_session(session_id);

    assert!(session.toggle_review_modal(Path::new(".")));

    let first = session.review_modal_entries.first().expect("first entry");
    assert_eq!(first.display_name, "Implementation Review 2");
}
