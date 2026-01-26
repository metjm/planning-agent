use super::*;

#[test]
fn test_review_history_separates_kinds() {
    let mut session = Session::new(0);

    session.start_review_round(ReviewKind::Plan, 1);
    session.start_review_round(ReviewKind::Implementation, 1);

    session.reviewer_started(ReviewKind::Plan, 1, "plan-reviewer".to_string());
    session.reviewer_completed(
        ReviewKind::Plan,
        1,
        "plan-reviewer".to_string(),
        true,
        "Approved".to_string(),
        1200,
    );

    session.reviewer_started(ReviewKind::Implementation, 1, "impl-reviewer".to_string());
    session.reviewer_completed(
        ReviewKind::Implementation,
        1,
        "impl-reviewer".to_string(),
        false,
        "Needs revision".to_string(),
        2400,
    );

    assert_eq!(session.review_history.len(), 2);

    let plan_round = session
        .review_history
        .iter()
        .find(|round| round.kind == ReviewKind::Plan && round.round == 1)
        .expect("plan round should exist");
    let impl_round = session
        .review_history
        .iter()
        .find(|round| round.kind == ReviewKind::Implementation && round.round == 1)
        .expect("implementation round should exist");

    assert_eq!(plan_round.reviewers.len(), 1);
    assert_eq!(impl_round.reviewers.len(), 1);
}
