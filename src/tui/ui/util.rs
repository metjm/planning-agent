
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
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

#[allow(dead_code)]
pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
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

    if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
        let rest = &trimmed[2..];
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
