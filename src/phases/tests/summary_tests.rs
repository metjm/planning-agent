use super::*;

#[test]
fn test_build_plan_summary_input_utf8_boundary() {
    // Create content that exceeds 8000 bytes with multi-byte chars near the boundary
    // '─' is 3 bytes (U+2500: 0xE2 0x94 0x80)
    let padding = "a".repeat(7998);
    let content = format!("{}───", padding); // 7998 + 9 = 8007 bytes

    // This should not panic
    let result = build_plan_summary_input(&content, "test");
    assert!(result.contains("Content truncated"));
}

#[test]
fn test_build_plan_summary_input_just_over_boundary() {
    // Content just over the boundary (8001 bytes)
    let content = "a".repeat(8001);
    let result = build_plan_summary_input(&content, "test");
    assert!(result.contains("Content truncated"));
}

#[test]
fn test_build_plan_summary_input_under_limit() {
    let content = "short content";
    let result = build_plan_summary_input(content, "test");
    assert!(!result.contains("Content truncated"));
    assert!(result.contains("short content"));
}
