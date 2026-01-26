use super::*;

#[test]
fn test_event_includes_session_id() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let sender = SessionEventSender::new(42, 0, tx);

    sender.send_output("test line".to_string());

    let event = rx.try_recv().unwrap();
    match event {
        Event::SessionOutput { session_id, line } => {
            assert_eq!(session_id, 42);
            assert_eq!(line, "test line");
        }
        _ => panic!("Expected SessionOutput event"),
    }
}

#[test]
fn test_multiple_senders() {
    let (tx, mut rx) = mpsc::unbounded_channel();

    let sender1 = SessionEventSender::new(1, 0, tx.clone());
    let sender2 = SessionEventSender::new(2, 0, tx);

    sender1.send_output("from session 1".to_string());
    sender2.send_output("from session 2".to_string());

    let event1 = rx.try_recv().unwrap();
    let event2 = rx.try_recv().unwrap();

    match event1 {
        Event::SessionOutput { session_id, .. } => assert_eq!(session_id, 1),
        _ => panic!("Expected SessionOutput event"),
    }

    match event2 {
        Event::SessionOutput { session_id, .. } => assert_eq!(session_id, 2),
        _ => panic!("Expected SessionOutput event"),
    }
}

#[test]
fn test_summary_events_include_run_id() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let sender = SessionEventSender::new(1, 42, tx);

    sender.send_run_tab_summary_generating("Planning".to_string());
    sender.send_run_tab_summary_ready("Planning".to_string(), "Summary content".to_string());
    sender.send_run_tab_summary_error("Planning".to_string(), "Error message".to_string());

    match rx.try_recv().unwrap() {
        Event::SessionRunTabSummaryGenerating {
            session_id,
            phase,
            run_id,
        } => {
            assert_eq!(session_id, 1);
            assert_eq!(phase, "Planning");
            assert_eq!(run_id, 42);
        }
        _ => panic!("Expected SessionRunTabSummaryGenerating event"),
    }

    match rx.try_recv().unwrap() {
        Event::SessionRunTabSummaryReady {
            session_id,
            phase,
            summary,
            run_id,
        } => {
            assert_eq!(session_id, 1);
            assert_eq!(phase, "Planning");
            assert_eq!(summary, "Summary content");
            assert_eq!(run_id, 42);
        }
        _ => panic!("Expected SessionRunTabSummaryReady event"),
    }

    match rx.try_recv().unwrap() {
        Event::SessionRunTabSummaryError {
            session_id,
            phase,
            error,
            run_id,
        } => {
            assert_eq!(session_id, 1);
            assert_eq!(phase, "Planning");
            assert_eq!(error, "Error message");
            assert_eq!(run_id, 42);
        }
        _ => panic!("Expected SessionRunTabSummaryError event"),
    }
}
