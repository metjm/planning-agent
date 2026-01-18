
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use unicode_width::UnicodeWidthChar;

pub fn wrap_text_at_width(text: &str, width: usize) -> String {
    if width == 0 {
        return text.to_string();
    }

    let mut result = String::new();
    for line in text.split('\n') {
        if !result.is_empty() {
            result.push('\n');
        }
        let mut current_width = 0;
        for c in line.chars() {
            let char_width = c.width().unwrap_or(0);
            if current_width + char_width > width && current_width > 0 {
                result.push('\n');
                current_width = 0;
            }
            result.push(c);
            current_width += char_width;
        }
    }
    result
}

pub fn format_bytes(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

pub fn format_tokens(tokens: u64) -> String {
    if tokens > 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens > 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

pub fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else {
        format!("{}m{}s", secs / 60, secs % 60)
    }
}

pub fn parse_markdown_line(line: &str) -> Line<'static> {
    let trimmed = line.trim();

    if let Some(rest) = trimmed.strip_prefix("### ") {
        return Line::from(vec![Span::styled(
            rest.to_string(),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )]);
    }
    if let Some(rest) = trimmed.strip_prefix("## ") {
        return Line::from(vec![Span::styled(
            rest.to_string(),
            Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
        )]);
    }
    if let Some(rest) = trimmed.strip_prefix("# ") {
        return Line::from(vec![Span::styled(
            rest.to_string(),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )]);
    }

    if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
        let mut spans = vec![Span::styled("• ", Style::default().fg(Color::Yellow))];
        spans.extend(parse_inline_markdown(rest));
        return Line::from(spans);
    }

    if trimmed == "---" || trimmed == "***" || trimmed == "___" {
        return Line::from(Span::styled(
            "─".repeat(40),
            Style::default().fg(Color::DarkGray),
        ));
    }

    Line::from(parse_inline_markdown(trimmed))
}

pub fn parse_inline_markdown(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '*' && chars.peek() == Some(&'*') {

            chars.next(); 
            if !current.is_empty() {
                spans.push(Span::raw(std::mem::take(&mut current)));
            }

            let mut bold_text = String::new();
            while let Some(bc) = chars.next() {
                if bc == '*' && chars.peek() == Some(&'*') {
                    chars.next();
                    break;
                }
                bold_text.push(bc);
            }
            spans.push(Span::styled(
                bold_text,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        } else if c == '`' {
            if !current.is_empty() {
                spans.push(Span::raw(std::mem::take(&mut current)));
            }

            let mut code_text = String::new();
            for bc in chars.by_ref() {
                if bc == '`' {
                    break;
                }
                code_text.push(bc);
            }
            spans.push(Span::styled(
                code_text,
                Style::default().fg(Color::Green),
            ));
        } else {
            current.push(c);
        }
    }

    if !current.is_empty() {
        spans.push(Span::raw(current));
    }

    if spans.is_empty() {
        spans.push(Span::raw(""));
    }

    spans
}

/// Compute the wrapped line count for styled `Line` content.
///
/// Uses a block-less `Paragraph` with wrapping to get accurate line counts
/// that match ratatui's rendering. The block is intentionally omitted because
/// `Paragraph::line_count` includes block padding when a block is set.
pub fn compute_wrapped_line_count(lines: &[Line], width: u16) -> usize {
    if width == 0 {
        return 0;
    }
    let paragraph = Paragraph::new(lines.to_vec()).wrap(Wrap { trim: false });
    paragraph.line_count(width)
}

/// Compute the wrapped line count for plain text content.
///
/// Uses a block-less `Paragraph` with wrapping to get accurate line counts
/// that match ratatui's rendering.
pub fn compute_wrapped_line_count_text(text: &str, width: u16) -> usize {
    if width == 0 {
        return 0;
    }
    let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
    paragraph.line_count(width)
}

/// Compute the inner size of the approval popup given terminal dimensions.
///
/// This replicates the layout logic used in `draw_choice_popup` to ensure
/// input handlers compute the same dimensions as the renderer:
/// - Popup is 80% of terminal size
/// - Title area: 3 rows
/// - Summary area: remaining space minus footer (4 rows)
/// - Summary block has 1-row borders on top and bottom
///
/// Returns (inner_width, inner_height) of the summary area.
pub fn compute_popup_summary_inner_size(terminal_width: u16, terminal_height: u16) -> (u16, u16) {
    // Match draw_choice_popup layout: 80% of terminal size
    let popup_width = (terminal_width as f32 * 0.8) as u16;
    let popup_height = (terminal_height as f32 * 0.8) as u16;

    // Layout: [Title: 3] [Summary: Min(0)] [Instructions: 4]
    // So summary height = popup_height - 3 - 4 = popup_height - 7
    let summary_height = popup_height.saturating_sub(7);

    // Summary block has borders (1 row each for top/bottom)
    let inner_height = summary_height.saturating_sub(2);

    // Summary block has borders (1 col each for left/right)
    let inner_width = popup_width.saturating_sub(2);

    (inner_width, inner_height)
}

/// Compute the inner size of the summary panel given terminal dimensions.
///
/// This replicates the layout logic used in `draw_summary_panel`:
/// - Main layout: top bar (2), main content (min 0), footer (3)
/// - Main content split: 70%/30% horizontal
/// - Left side split: 40%/60% vertical for output/chat
/// - Chat area split: 50%/50% when summary is shown
/// - Summary block has 1-row borders on top and bottom
///
/// Returns (inner_width, inner_height) of the summary panel inner area.
pub fn compute_summary_panel_inner_size(terminal_width: u16, terminal_height: u16) -> (u16, u16) {
    // Main layout: top bar (2) + footer (3) = 5 rows overhead
    let main_content_height = terminal_height.saturating_sub(5);

    // Horizontal split: 70% left, 30% right - we're in the left 70%
    let left_width = (terminal_width as f32 * 0.70) as u16;

    // Vertical split: 40% output, 60% chat
    let chat_height = (main_content_height as f32 * 0.60) as u16;

    // Chat area has run tabs (1 row)
    let chat_content_height = chat_height.saturating_sub(1);

    // Chat content split: 50% chat, 50% summary
    let summary_width = left_width / 2;
    let summary_height = chat_content_height;

    // Summary block has borders (1 row each for top/bottom, 1 col each for left/right)
    let inner_height = summary_height.saturating_sub(2);
    let inner_width = summary_width.saturating_sub(2);

    (inner_width, inner_height)
}

/// Compute the inner dimensions of the plan modal for scroll calculations.
///
/// This replicates the layout logic used in `draw_plan_modal`:
/// - Modal is 80% of terminal size
/// - Layout: Title (3), Content (Min), Instructions (3)
/// - Content block has 1-row borders on top and bottom
///
/// Returns (inner_width, visible_height) of the content area.
pub fn compute_plan_modal_inner_size(terminal_width: u16, terminal_height: u16) -> (u16, u16) {
    let popup_width = (terminal_width as f32 * 0.8) as u16;
    let popup_height = (terminal_height as f32 * 0.8) as u16;

    // Layout: [Title: 3] [Content: Min(0)] [Instructions: 3]
    // So content height = popup_height - 3 - 3 = popup_height - 6
    let content_height = popup_height.saturating_sub(6);

    // Content block has borders (1 row each for top/bottom)
    let inner_height = content_height.saturating_sub(2);

    // Content block has borders (1 col each for left/right)
    let inner_width = popup_width.saturating_sub(2);

    (inner_width, inner_height)
}

