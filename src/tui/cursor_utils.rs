//! Safe cursor utilities for string slicing at byte positions.
//!
//! Cursor positions in this codebase are stored as byte offsets, with the invariant
//! that they are always at valid UTF-8 character boundaries. This module provides
//! helper functions that make this invariant explicit and validate it in debug builds.

/// Get the substring from start to a cursor position (byte offset).
///
/// # Safety Invariant
/// `cursor` must be at a valid UTF-8 character boundary. This is validated in debug builds.
/// The cursor movement functions in `session/input.rs` maintain this invariant.
#[inline]
pub fn slice_up_to_cursor(s: &str, cursor: usize) -> &str {
    debug_assert!(
        cursor <= s.len() && s.is_char_boundary(cursor),
        "cursor {} is not at a char boundary in string of len {}",
        cursor,
        s.len()
    );
    // Use get() for safety in release builds - returns empty string if boundary check fails
    s.get(..cursor).unwrap_or("")
}

/// Get the substring from a cursor position (byte offset) to the end.
///
/// # Safety Invariant
/// `cursor` must be at a valid UTF-8 character boundary. This is validated in debug builds.
#[inline]
pub fn slice_from_cursor(s: &str, cursor: usize) -> &str {
    debug_assert!(
        cursor <= s.len() && s.is_char_boundary(cursor),
        "cursor {} is not at a char boundary in string of len {}",
        cursor,
        s.len()
    );
    s.get(cursor..).unwrap_or("")
}

/// Get a substring between two cursor positions (byte offsets).
///
/// # Safety Invariant
/// Both `start` and `end` must be at valid UTF-8 character boundaries.
#[inline]
pub fn slice_between_cursors(s: &str, start: usize, end: usize) -> &str {
    debug_assert!(
        start <= end && end <= s.len() && s.is_char_boundary(start) && s.is_char_boundary(end),
        "cursors {}..{} are not valid boundaries in string of len {}",
        start,
        end,
        s.len()
    );
    s.get(start..end).unwrap_or("")
}

#[cfg(test)]
#[path = "tests/cursor_utils_tests.rs"]
mod tests;
