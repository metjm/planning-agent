use super::*;

#[test]
fn test_planning_theme_has_blue_tones() {
    let theme = Theme::planning();
    // Verify planning uses blue tones
    assert_eq!(theme.accent, Color::Rgb(100, 180, 255)); // Sky blue
    assert_eq!(theme.border, Color::Rgb(60, 100, 160)); // Steel blue
}

#[test]
fn test_implementation_theme_has_orange_tones() {
    let theme = Theme::implementation();
    // Verify implementation uses orange/warm tones
    assert_eq!(theme.accent, Color::Rgb(255, 165, 0)); // Orange
    assert_eq!(theme.border, Color::Rgb(180, 80, 40)); // Burnt orange
}

#[test]
fn test_complete_theme_has_green_tones() {
    let theme = Theme::complete();
    // Verify complete uses green tones
    assert_eq!(theme.accent, Color::Rgb(100, 220, 100)); // Bright green
    assert_eq!(theme.border, Color::Rgb(40, 120, 60)); // Forest green
}

#[test]
fn test_semantic_colors_consistent_across_themes() {
    let planning = Theme::planning();
    let implementation = Theme::implementation();
    let complete = Theme::complete();

    // Success should always be green
    assert_eq!(planning.success, Color::Green);
    assert_eq!(implementation.success, Color::Green);
    assert_eq!(complete.success, Color::Green);

    // Error should always be red
    assert_eq!(planning.error, Color::Red);
    assert_eq!(implementation.error, Color::Red);
    assert_eq!(complete.error, Color::Red);
}

#[test]
fn test_themes_are_visually_distinct() {
    let planning = Theme::planning();
    let implementation = Theme::implementation();
    let complete = Theme::complete();

    // Accent colors should all be different
    assert_ne!(planning.accent, implementation.accent);
    assert_ne!(planning.accent, complete.accent);
    assert_ne!(implementation.accent, complete.accent);

    // Border colors should all be different
    assert_ne!(planning.border, implementation.border);
    assert_ne!(planning.border, complete.border);
    assert_ne!(implementation.border, complete.border);
}
