//! Dropdown rendering for autocomplete features.
//!
//! Contains the rendering logic for @-mention and slash command dropdowns.

use crate::tui::mention::MentionState;
use crate::tui::slash::SlashState;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

/// Draw the @-mention dropdown below the given cursor position.
/// The dropdown shows matching files and allows selection.
pub fn draw_mention_dropdown(
    frame: &mut Frame,
    mention_state: &MentionState,
    cursor_x: u16,
    cursor_y: u16,
    max_width: u16,
) {
    if !mention_state.active || mention_state.matches.is_empty() {
        return;
    }

    let screen_area = frame.area();
    let matches = &mention_state.matches;
    let selected_idx = mention_state.selected_idx;

    // Dropdown dimensions
    let dropdown_height = (matches.len() as u16).min(10) + 2; // +2 for borders
    let max_path_width = matches
        .iter()
        .map(|m| m.display_path.width())
        .max()
        .unwrap_or(20) as u16;
    let dropdown_width = (max_path_width + 4).min(max_width).max(30); // +4 for padding and selection indicator

    // Position dropdown below cursor, but flip above if not enough space below
    let space_below = screen_area.height.saturating_sub(cursor_y + 1);
    let space_above = cursor_y;

    let (dropdown_y, fits_below) = if space_below >= dropdown_height {
        (cursor_y + 1, true)
    } else if space_above >= dropdown_height {
        (cursor_y.saturating_sub(dropdown_height), false)
    } else {
        // Not enough space either way, prefer below with truncation
        (cursor_y + 1, true)
    };

    let actual_height = if fits_below {
        dropdown_height.min(space_below)
    } else {
        dropdown_height.min(space_above)
    };

    // Adjust x position if dropdown would extend past screen edge
    let dropdown_x = if cursor_x + dropdown_width > screen_area.width {
        screen_area.width.saturating_sub(dropdown_width)
    } else {
        cursor_x
    };

    let dropdown_area = Rect::new(dropdown_x, dropdown_y, dropdown_width, actual_height);

    // Clear background
    frame.render_widget(Clear, dropdown_area);

    // Build dropdown content
    let visible_matches = (actual_height.saturating_sub(2)) as usize;
    let scroll_offset = if selected_idx >= visible_matches {
        selected_idx.saturating_sub(visible_matches - 1)
    } else {
        0
    };

    let items: Vec<Line> = matches
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_matches)
        .map(|(i, m)| {
            let is_selected = i == selected_idx;
            let prefix = if is_selected { "› " } else { "  " };
            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            Line::from(Span::styled(format!("{}{}", prefix, m.display_path), style))
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Files & Folders (↑/↓ to navigate, Tab/Enter to select) ");

    let dropdown = Paragraph::new(items).block(block);
    frame.render_widget(dropdown, dropdown_area);
}

/// Draw the slash command dropdown below the given cursor position.
/// The dropdown shows matching commands with their descriptions.
pub fn draw_slash_dropdown(
    frame: &mut Frame,
    slash_state: &SlashState,
    cursor_x: u16,
    cursor_y: u16,
    max_width: u16,
) {
    if !slash_state.active || slash_state.matches.is_empty() {
        return;
    }

    let screen_area = frame.area();
    let matches = &slash_state.matches;
    let selected_idx = slash_state.selected_idx;

    // Calculate widths for command and description columns
    let max_cmd_width = matches
        .iter()
        .map(|m| m.display.width())
        .max()
        .unwrap_or(10) as u16;
    let max_desc_width = matches
        .iter()
        .map(|m| m.description.width())
        .max()
        .unwrap_or(20) as u16;

    // Dropdown dimensions: "› /command   description"
    // +4 for "› " prefix and "  " separator, +4 for borders and padding
    let dropdown_height = (matches.len() as u16).min(10) + 2; // +2 for borders
    let ideal_width = max_cmd_width + max_desc_width + 8;
    let dropdown_width = ideal_width.min(max_width).max(40);

    // Position dropdown below cursor, but flip above if not enough space below
    let space_below = screen_area.height.saturating_sub(cursor_y + 1);
    let space_above = cursor_y;

    let (dropdown_y, fits_below) = if space_below >= dropdown_height {
        (cursor_y + 1, true)
    } else if space_above >= dropdown_height {
        (cursor_y.saturating_sub(dropdown_height), false)
    } else {
        // Not enough space either way, prefer below with truncation
        (cursor_y + 1, true)
    };

    let actual_height = if fits_below {
        dropdown_height.min(space_below)
    } else {
        dropdown_height.min(space_above)
    };

    // Adjust x position if dropdown would extend past screen edge
    let dropdown_x = if cursor_x + dropdown_width > screen_area.width {
        screen_area.width.saturating_sub(dropdown_width)
    } else {
        cursor_x
    };

    let dropdown_area = Rect::new(dropdown_x, dropdown_y, dropdown_width, actual_height);

    // Clear background
    frame.render_widget(Clear, dropdown_area);

    // Build dropdown content
    let visible_matches = (actual_height.saturating_sub(2)) as usize;
    let scroll_offset = if selected_idx >= visible_matches {
        selected_idx.saturating_sub(visible_matches - 1)
    } else {
        0
    };

    // Calculate available width for content (minus borders)
    let content_width = dropdown_width.saturating_sub(2) as usize;
    // Reserve space for "› " prefix
    let available_for_text = content_width.saturating_sub(2);

    let items: Vec<Line> = matches
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_matches)
        .map(|(i, m)| {
            let is_selected = i == selected_idx;
            let prefix = if is_selected { "› " } else { "  " };

            // Calculate how much space we have for description
            let cmd_len = m.display.len();
            let separator = "  ";
            let desc_space = available_for_text
                .saturating_sub(cmd_len)
                .saturating_sub(separator.len());

            // Truncate description if needed
            let desc: String = if m.description.len() > desc_space {
                format!("{}...", &m.description[..desc_space.saturating_sub(3)])
            } else {
                m.description.clone()
            };

            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let desc_style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(m.display.clone(), style),
                Span::styled(separator, style),
                Span::styled(desc, desc_style),
            ])
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(" Slash Commands (↑/↓ to navigate, Tab/Enter to select) ");

    let dropdown = Paragraph::new(items).block(block);
    frame.render_widget(dropdown, dropdown_area);
}
