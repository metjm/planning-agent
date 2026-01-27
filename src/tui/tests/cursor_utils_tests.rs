use super::*;

#[test]
fn slice_up_to_cursor_at_start() {
    let s = "hello world";
    assert_eq!(slice_up_to_cursor(s, 0), "");
}

#[test]
fn slice_up_to_cursor_at_end() {
    let s = "hello world";
    assert_eq!(slice_up_to_cursor(s, s.len()), "hello world");
}

#[test]
fn slice_up_to_cursor_mid_ascii() {
    let s = "hello world";
    assert_eq!(slice_up_to_cursor(s, 5), "hello");
}

#[test]
fn slice_up_to_cursor_with_multibyte() {
    let s = "hello 🎉 world";
    // '🎉' is 4 bytes, starts at byte 6
    assert_eq!(slice_up_to_cursor(s, 6), "hello ");
    assert_eq!(slice_up_to_cursor(s, 10), "hello 🎉"); // after emoji
}

#[test]
fn slice_from_cursor_at_start() {
    let s = "hello world";
    assert_eq!(slice_from_cursor(s, 0), "hello world");
}

#[test]
fn slice_from_cursor_at_end() {
    let s = "hello world";
    assert_eq!(slice_from_cursor(s, s.len()), "");
}

#[test]
fn slice_from_cursor_mid_ascii() {
    let s = "hello world";
    assert_eq!(slice_from_cursor(s, 6), "world");
}

#[test]
fn slice_between_cursors_basic() {
    let s = "hello world";
    assert_eq!(slice_between_cursors(s, 0, 5), "hello");
    assert_eq!(slice_between_cursors(s, 6, 11), "world");
}

#[test]
fn slice_between_cursors_empty() {
    let s = "hello world";
    assert_eq!(slice_between_cursors(s, 5, 5), "");
}
