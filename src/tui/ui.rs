use crate::state::Phase;
use crate::tui::{
    ApprovalContext, ApprovalMode, FocusedPanel, InputMode, Session, SessionStatus, TabManager,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    Frame,
};

/// Wrap text at character boundaries using unicode display width.
/// This ensures cursor position calculation matches the rendered text.
fn wrap_text_at_width(text: &str, width: usize) -> String {
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

fn format_bytes(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Main draw function for multi-tab interface
pub fn draw(frame: &mut Frame, tab_manager: &TabManager) {
    // Main layout: tab bar, content, footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Tab bar (2 lines for tabs + border)
            Constraint::Min(0),    // Main content
            Constraint::Length(3), // Footer
        ])
        .split(frame.area());

    draw_tab_bar(frame, tab_manager, chunks[0]);

    let session = tab_manager.active();
    draw_main(frame, session, chunks[1]);
    draw_footer(frame, session, tab_manager, chunks[2]);

    // Draw overlays
    if session.approval_mode != ApprovalMode::None {
        draw_approval_overlay(frame, session);
    }
    if session.input_mode == InputMode::NamingTab {
        draw_tab_input_overlay(frame, session);
    }
    if session.error_state.is_some() {
        draw_error_overlay(frame, session);
    }
}

/// Draw the tab bar at the top of the screen
fn draw_tab_bar(frame: &mut Frame, tab_manager: &TabManager, area: Rect) {
    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::raw(" "));

    for (i, session) in tab_manager.sessions.iter().enumerate() {
        let is_active = i == tab_manager.active_tab;

        // Status indicator
        let status_icon = match session.status {
            SessionStatus::InputPending => "...",
            SessionStatus::Planning => "",
            SessionStatus::GeneratingSummary => "◐",
            SessionStatus::AwaitingApproval => "?",
            SessionStatus::Complete => "+",
            SessionStatus::Error => "!",
        };

        // Tab name (truncate if too long)
        let name = if session.name.is_empty() {
            "New Tab"
        } else {
            &session.name
        };
        let display_name: String = if name.len() > 15 {
            format!("{}...", &name[..12])
        } else {
            name.to_string()
        };

        // Format tab label
        let label = if status_icon.is_empty() {
            format!("[{}]", display_name)
        } else {
            format!("[{} {}]", display_name, status_icon)
        };

        let style = if is_active {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else if session.approval_mode != ApprovalMode::None {
            // Highlight tabs needing attention
            Style::default().fg(Color::Magenta)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
    }

    // Add new tab button hint
    spans.push(Span::styled("[Ctrl++]", Style::default().fg(Color::Green).dim()));

    // Build title with plan file path if available
    let active_session = tab_manager.active();
    let title = if let Some(ref state) = active_session.workflow_state {
        let plan_path = state.plan_file.display().to_string();
        format!(" Planning Agent - {} ", plan_path)
    } else {
        " Planning Agent ".to_string()
    };

    let tabs = Paragraph::new(Line::from(spans)).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(title)
            .title_alignment(Alignment::Center),
    );

    frame.render_widget(tabs, area);
}

fn draw_main(frame: &mut Frame, session: &Session, area: Rect) {
    // Split main area: left (70%) | stats (30%)
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(area);

    // Split left side: output (40%) | chat/streaming (60%)
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[0]);

    draw_output(frame, session, left_chunks[0]);
    draw_chat(frame, session, left_chunks[1]);  // Use chat panel (falls back to streaming if no tabs)
    draw_stats(frame, session, chunks[1]);
}

fn draw_output(frame: &mut Frame, session: &Session, area: Rect) {
    // Build title with scroll indicator and focus indicator
    let is_focused = session.focused_panel == FocusedPanel::Output;
    let title = if session.output_follow_mode {
        if is_focused {
            " Output [*] "
        } else {
            " Output "
        }
    } else if is_focused {
        " Output [SCROLLED *] "
    } else {
        " Output [SCROLLED] "
    };

    let border_color = if is_focused { Color::Yellow } else { Color::Blue };

    let output_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color));

    let inner_area = output_block.inner(area);
    let visible_height = inner_area.height as usize;

    // Calculate which lines to show based on follow mode
    let total_lines = session.output_lines.len();
    let max_scroll = total_lines.saturating_sub(visible_height);

    let start = if session.output_follow_mode {
        // Follow mode: show the last visible_height lines
        max_scroll
    } else {
        // Manual scroll: use user's position, clamped to valid range
        session.scroll_position.min(max_scroll)
    };

    let end = (start + visible_height).min(total_lines);

    let lines: Vec<Line> = if total_lines == 0 {
        vec![Line::from(Span::styled(
            "Waiting for output...",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        session.output_lines[start..end]
            .iter()
            .map(|line| {
                // Color different prefixes
                if line.starts_with("[planning]") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::Cyan)))
                } else if line.starts_with("[claude]") || line.starts_with("[planning-agent]") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::Green)))
                } else if line.contains("error") || line.contains("Error") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::Red)))
                } else {
                    Line::from(line.clone())
                }
            })
            .collect()
    };

    let paragraph = Paragraph::new(lines)
        .block(output_block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);

    // Scrollbar
    if total_lines > visible_height {
        let mut scrollbar_state = ScrollbarState::new(total_lines).position(start);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓")),
            area,
            &mut scrollbar_state,
        );
    }
}

fn draw_streaming(frame: &mut Frame, session: &Session, area: Rect) {
    // Build title with scroll indicator and focus indicator
    let is_focused = session.focused_panel == FocusedPanel::Chat;
    let title = if session.streaming_follow_mode {
        if is_focused {
            " Agent Output [*] "
        } else {
            " Agent Output "
        }
    } else if is_focused {
        " Agent Output [SCROLLED *] "
    } else {
        " Agent Output [SCROLLED] "
    };

    let border_color = if is_focused { Color::Yellow } else { Color::Green };

    let streaming_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color));

    let inner_area = streaming_block.inner(area);
    let visible_height = inner_area.height as usize;
    let inner_width = inner_area.width;

    let lines: Vec<Line> = if session.streaming_lines.is_empty() {
        vec![Line::from(Span::styled(
            "Waiting for Claude output...",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        session
            .streaming_lines
            .iter()
            .map(|line| {
                // Color based on content type
                if line.starts_with("[Tool:") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::Yellow)))
                } else if line.starts_with("[Result]") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::Cyan)))
                } else if line.starts_with("[stderr]") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::Magenta)))
                } else if line.contains("error") || line.contains("Error") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::Red)))
                } else {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::White)))
                }
            })
            .collect()
    };

    // Calculate wrapped line count WITHOUT block (line_count adds block padding which we don't want)
    let paragraph_for_count = Paragraph::new(lines.clone()).wrap(Wrap { trim: false });
    let wrapped_line_count = paragraph_for_count.line_count(inner_width);

    // Calculate scroll offset based on follow mode
    let max_scroll = wrapped_line_count.saturating_sub(visible_height);
    let scroll_offset = if session.streaming_follow_mode {
        // Follow mode: show latest content at bottom
        max_scroll as u16
    } else {
        // Manual scroll: use user's position, clamped to valid range
        (session.streaming_scroll_position.min(max_scroll)) as u16
    };

    let paragraph = Paragraph::new(lines)
        .block(streaming_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_offset, 0));
    frame.render_widget(paragraph, area);

    // Scrollbar
    if wrapped_line_count > visible_height {
        let mut scrollbar_state =
            ScrollbarState::new(wrapped_line_count).position(scroll_offset as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓")),
            area,
            &mut scrollbar_state,
        );
    }
}

/// Draw the chat panel with run tabs (replaces streaming panel when tabs exist)
fn draw_chat(frame: &mut Frame, session: &Session, area: Rect) {
    let is_focused = session.focused_panel == FocusedPanel::Chat;
    let border_color = if is_focused { Color::Yellow } else { Color::Green };

    // If no run tabs, fall back to legacy streaming view
    if session.run_tabs.is_empty() {
        draw_streaming(frame, session, area);
        return;
    }

    // Split area: tab bar (1 line) | content
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    // Draw run tabs
    draw_run_tabs(frame, session, chunks[0]);

    // Draw active tab content
    let active_tab = session.run_tabs.get(session.active_run_tab);

    let title = if let Some(tab) = active_tab {
        if session.chat_follow_mode {
            if is_focused {
                format!(" {} [*] ", tab.phase)
            } else {
                format!(" {} ", tab.phase)
            }
        } else if is_focused {
            format!(" {} [SCROLLED *] ", tab.phase)
        } else {
            format!(" {} [SCROLLED] ", tab.phase)
        }
    } else {
        " Chat ".to_string()
    };

    let chat_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color));

    let inner_area = chat_block.inner(chunks[1]);
    let visible_height = inner_area.height as usize;
    let inner_width = inner_area.width;

    let lines: Vec<Line> = if let Some(tab) = active_tab {
        if tab.messages.is_empty() {
            vec![Line::from(Span::styled(
                "Waiting for agent output...",
                Style::default().fg(Color::DarkGray),
            ))]
        } else {
            tab.messages
                .iter()
                .flat_map(|msg| {
                    // Create agent badge + message
                    let agent_color = match msg.agent_name.as_str() {
                        "claude" => Color::Cyan,
                        "codex" => Color::Magenta,
                        "gemini" => Color::Blue,
                        _ => Color::Yellow,
                    };
                    let badge = Span::styled(
                        format!("[{}] ", msg.agent_name),
                        Style::default().fg(agent_color).add_modifier(Modifier::BOLD),
                    );
                    let content = Span::styled(
                        msg.message.clone(),
                        Style::default().fg(Color::White),
                    );
                    vec![Line::from(vec![badge, content])]
                })
                .collect()
        }
    } else {
        vec![Line::from(Span::styled(
            "No active tab",
            Style::default().fg(Color::DarkGray),
        ))]
    };

    // Calculate wrapped line count
    let paragraph_for_count = Paragraph::new(lines.clone()).wrap(Wrap { trim: false });
    let wrapped_line_count = paragraph_for_count.line_count(inner_width);

    // Calculate scroll offset based on follow mode
    let max_scroll = wrapped_line_count.saturating_sub(visible_height);
    let scroll_offset = if session.chat_follow_mode {
        max_scroll as u16
    } else {
        let tab_scroll = active_tab.map(|t| t.scroll_position).unwrap_or(0);
        (tab_scroll.min(max_scroll)) as u16
    };

    let paragraph = Paragraph::new(lines)
        .block(chat_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_offset, 0));
    frame.render_widget(paragraph, chunks[1]);

    // Scrollbar
    if wrapped_line_count > visible_height {
        let mut scrollbar_state =
            ScrollbarState::new(wrapped_line_count).position(scroll_offset as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓")),
            chunks[1],
            &mut scrollbar_state,
        );
    }
}

/// Draw the run tabs row (Planning, Reviewing #1, etc.)
fn draw_run_tabs(frame: &mut Frame, session: &Session, area: Rect) {
    let mut spans: Vec<Span> = Vec::new();

    for (i, tab) in session.run_tabs.iter().enumerate() {
        let is_active = i == session.active_run_tab;

        // Shorten phase name for display
        let display_name: String = if tab.phase.len() > 12 {
            format!("{}...", &tab.phase[..9])
        } else {
            tab.phase.clone()
        };

        let style = if is_active {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        spans.push(Span::styled(format!(" [{}] ", display_name), style));
    }

    // Add navigation hint
    if session.run_tabs.len() > 1 {
        spans.push(Span::styled(
            " ←/→ ",
            Style::default().fg(Color::DarkGray).dim(),
        ));
    }

    let tabs = Paragraph::new(Line::from(spans));
    frame.render_widget(tabs, area);
}

fn format_tokens(tokens: u64) -> String {
    if tokens < 1000 {
        format!("{}", tokens)
    } else if tokens < 1_000_000 {
        format!("{:.1}K", tokens as f64 / 1000.0)
    } else {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    }
}

fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else {
        format!("{}m {:02}s", secs / 60, secs % 60)
    }
}

fn draw_stats(frame: &mut Frame, session: &Session, area: Rect) {
    let elapsed = session.elapsed();
    let minutes = elapsed.as_secs() / 60;
    let seconds = elapsed.as_secs() % 60;

    let (iter, max_iter) = session.iteration();

    let phase_color = match session.workflow_state.as_ref().map(|s| &s.phase) {
        Some(Phase::Planning) => Color::Yellow,
        Some(Phase::Reviewing) => Color::Blue,
        Some(Phase::Revising) => Color::Magenta,
        Some(Phase::Complete) => Color::Green,
        None => Color::Gray,
    };

    // Get cost (API-provided only)
    let cost = session.display_cost();

    let mut stats_text = vec![
        // Usage section at the top for prominence
        Line::from(vec![Span::styled(
            "── Usage ──",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![
            Span::styled(" Cost: ", Style::default().fg(Color::White)),
            Span::styled(
                format!("${:.4}", cost),
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Tokens: ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{}↓", format_tokens(session.total_input_tokens)),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(" ", Style::default()),
            Span::styled(
                format!("{}↑", format_tokens(session.total_output_tokens)),
                Style::default().fg(Color::Green),
            ),
        ]),
    ];

    // Account usage section - show all providers
    let has_any_usage = !session.account_usage.providers.is_empty();

    if has_any_usage {
        stats_text.push(Line::from(""));
        stats_text.push(Line::from(vec![Span::styled(
            "── Account ──",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )]));

        // Sort providers for consistent display order: claude first, then alphabetically
        let mut providers: Vec<_> = session.account_usage.providers.values().collect();
        providers.sort_by(|a, b| {
            if a.provider == "claude" { std::cmp::Ordering::Less }
            else if b.provider == "claude" { std::cmp::Ordering::Greater }
            else { a.provider.cmp(&b.provider) }
        });

        for provider in providers {
            // Skip providers that don't have usage AND don't have errors
            // (i.e., haven't been fetched yet)
            if provider.fetched_at.is_none() {
                continue;
            }

            // Show provider header with display name
            let header_style = if provider.supports_usage {
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            // Check if this provider has an error/N/A status
            if provider.has_error() || !provider.supports_usage {
                // Show compact "Provider: N/A (reason)" format
                let reason = provider.status_message.as_deref().unwrap_or("N/A");
                stats_text.push(Line::from(vec![
                    Span::styled(format!(" {}: ", provider.display_name), Style::default().fg(Color::White)),
                    Span::styled("N/A", Style::default().fg(Color::DarkGray)),
                    Span::styled(format!(" ({})", reason), Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)),
                ]));
            } else {
                // Show provider name as header
                stats_text.push(Line::from(vec![
                    Span::styled(format!(" {}", provider.display_name), header_style),
                ]));

                // Show plan type if available
                if let Some(ref plan) = provider.plan_type {
                    stats_text.push(Line::from(vec![
                        Span::styled("  Plan: ", Style::default().fg(Color::White)),
                        Span::styled(plan.clone(), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    ]));
                }

                // Show session usage (percentage used, color inverted - higher is worse)
                if let Some(session_pct) = provider.session_used {
                    let color = if session_pct >= 90 { Color::Red }
                               else if session_pct >= 70 { Color::Yellow }
                               else { Color::Green };
                    stats_text.push(Line::from(vec![
                        Span::styled("  Session: ", Style::default().fg(Color::White)),
                        Span::styled(format!("{}% used", session_pct), Style::default().fg(color)),
                    ]));
                }

                // Show weekly/daily usage (percentage used, color inverted - higher is worse)
                if let Some(weekly_pct) = provider.weekly_used {
                    let color = if weekly_pct >= 90 { Color::Red }
                               else if weekly_pct >= 70 { Color::Yellow }
                               else { Color::Green };
                    // Gemini uses daily limits, others use weekly
                    let label = if provider.provider == "gemini" { "  Daily: " } else { "  Weekly: " };
                    stats_text.push(Line::from(vec![
                        Span::styled(label, Style::default().fg(Color::White)),
                        Span::styled(format!("{}% used", weekly_pct), Style::default().fg(color)),
                    ]));
                }
            }
        }
    }

    stats_text.push(Line::from(""));
    // Status section
    stats_text.push(Line::from(vec![Span::styled(
        " Status",
        Style::default().add_modifier(Modifier::BOLD),
    )]));
    stats_text.push(Line::from(vec![
        Span::raw(" Phase: "),
        Span::styled(session.phase_name(), Style::default().fg(phase_color).bold()),
    ]));
    stats_text.push(Line::from(format!(" Iter: {}/{}", iter, max_iter)));
    stats_text.push(Line::from(vec![
        Span::styled(" Turn: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}", session.turn_count),
            Style::default().fg(Color::White),
        ),
    ]));
    stats_text.push(Line::from(format!(" Time: {}m {:02}s", minutes, seconds)));

    // Show generating summary spinner
    if session.status == SessionStatus::GeneratingSummary {
        stats_text.push(Line::from(""));
        stats_text.push(Line::from(vec![Span::styled(
            "── Summary ──",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )]));

        // Get current spinner character using frame counter
        let spinner_chars = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let spinner_char = spinner_chars[(session.spinner_frame as usize) % spinner_chars.len()];

        stats_text.push(Line::from(vec![
            Span::styled(format!(" {} ", spinner_char), Style::default().fg(Color::Yellow)),
            Span::styled("Generating...", Style::default().fg(Color::Cyan)),
        ]));
    }

    // Model name (if detected)
    if let Some(ref model) = session.model_name {
        stats_text.push(Line::from(vec![
            Span::styled(" Model: ", Style::default().fg(Color::DarkGray)),
            Span::styled(model.clone(), Style::default().fg(Color::Cyan)),
        ]));
    }

    // Stop reason (if available)
    if let Some(ref reason) = session.last_stop_reason {
        let (icon, color) = match reason.as_str() {
            "end_turn" => ("●", Color::Green),
            "tool_use" => ("⚙", Color::Yellow),
            "max_tokens" => ("!", Color::Red),
            _ => ("?", Color::Gray),
        };
        stats_text.push(Line::from(vec![
            Span::styled(" Stop: ", Style::default().fg(Color::DarkGray)),
            Span::styled(icon, Style::default().fg(color)),
            Span::styled(format!(" {}", reason), Style::default().fg(color)),
        ]));
    }

    stats_text.push(Line::from(""));

    // Cache stats (only show when there's cache usage)
    if session.total_cache_read_tokens > 0 || session.total_cache_creation_tokens > 0 {
        stats_text.push(Line::from(vec![
            Span::styled(" Cache: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(
                    "{}r/{}w",
                    format_tokens(session.total_cache_read_tokens),
                    format_tokens(session.total_cache_creation_tokens)
                ),
                Style::default().fg(Color::Blue),
            ),
        ]));
        stats_text.push(Line::from(""));
    }

    // Streaming stats
    stats_text.push(Line::from(vec![Span::styled(
        " Stream",
        Style::default().add_modifier(Modifier::BOLD),
    )]));
    stats_text.push(Line::from(vec![
        Span::styled(" Recv: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format_bytes(session.bytes_received),
            Style::default().fg(Color::White),
        ),
    ]));
    stats_text.push(Line::from(vec![
        Span::styled(" Rate: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}/s", format_bytes(session.bytes_per_second as usize)),
            Style::default().fg(if session.bytes_per_second > 100.0 {
                Color::Green
            } else {
                Color::Yellow
            }),
        ),
    ]));
    stats_text.push(Line::from(""));

    // Tool stats
    stats_text.push(Line::from(vec![Span::styled(
        " Tools",
        Style::default().add_modifier(Modifier::BOLD),
    )]));
    stats_text.push(Line::from(vec![
        Span::styled(" Calls: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}", session.tool_call_count),
            Style::default().fg(Color::White),
        ),
    ]));

    // Active tools (compact)
    if !session.active_tools.is_empty() {
        for (name, start_time) in session.active_tools.iter().take(10) {
            let elapsed = start_time.elapsed().as_secs();
            stats_text.push(Line::from(vec![
                Span::styled(" ▶ ", Style::default().fg(Color::Yellow)),
                Span::styled(
                    format!("{} ({}s)", name, elapsed),
                    Style::default().fg(Color::Yellow),
                ),
            ]));
        }
        if session.active_tools.len() > 10 {
            stats_text.push(Line::from(Span::styled(
                format!("   +{} more", session.active_tools.len() - 10),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    // Tool errors (only show if > 0)
    if session.tool_error_count > 0 {
        stats_text.push(Line::from(vec![
            Span::styled(" Errors: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", session.tool_error_count),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    // Average tool duration (computed locally)
    if let Some(avg_ms) = session.average_tool_duration_ms() {
        let duration_display = if avg_ms > 1000 {
            format!("{:.1}s", avg_ms as f64 / 1000.0)
        } else {
            format!("{}ms", avg_ms)
        };
        stats_text.push(Line::from(vec![
            Span::styled(" Avg Tool: ", Style::default().fg(Color::DarkGray)),
            Span::styled(duration_display, Style::default().fg(Color::White)),
        ]));
    }

    // Tool success rate
    if session.tool_call_count > 0 {
        let success_count = session.tool_call_count.saturating_sub(session.tool_error_count);
        let success_rate = (success_count as f64 / session.tool_call_count as f64) * 100.0;
        let color = if success_rate >= 95.0 {
            Color::Green
        } else if success_rate >= 80.0 {
            Color::Yellow
        } else {
            Color::Red
        };
        stats_text.push(Line::from(vec![
            Span::styled(" Success: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{:.0}%", success_rate), Style::default().fg(color)),
        ]));
    }

    stats_text.push(Line::from(""));

    // Phase timing (if any recorded)
    if !session.phase_times.is_empty() || session.current_phase_start.is_some() {
        stats_text.push(Line::from(vec![Span::styled(
            " Timing",
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        for (phase, duration) in &session.phase_times {
            stats_text.push(Line::from(vec![
                Span::styled(format!(" {}: ", phase), Style::default().fg(Color::DarkGray)),
                Span::styled(format_duration(*duration), Style::default().fg(Color::White)),
            ]));
        }
        // Show current phase timing
        if let Some((phase, start)) = &session.current_phase_start {
            stats_text.push(Line::from(vec![
                Span::styled(format!(" {}: ", phase), Style::default().fg(Color::Yellow)),
                Span::styled(
                    format_duration(start.elapsed()),
                    Style::default().fg(Color::Yellow),
                ),
            ]));
        }
        stats_text.push(Line::from(""));
    }

    // Keys (compact)
    stats_text.push(Line::from(vec![
        Span::styled(" Keys: ", Style::default().fg(Color::DarkGray)),
        Span::styled("Tab", Style::default().fg(Color::White)),
        Span::styled(" focus ", Style::default().fg(Color::DarkGray)),
        Span::styled("j/k", Style::default().fg(Color::White)),
        Span::styled(" scroll", Style::default().fg(Color::DarkGray)),
    ]));
    stats_text.push(Line::from(vec![
        Span::styled("       ", Style::default().fg(Color::DarkGray)),
        Span::styled("G", Style::default().fg(Color::White)),
        Span::styled(" bottom ", Style::default().fg(Color::DarkGray)),
        Span::styled("q", Style::default().fg(Color::White)),
        Span::styled(" quit", Style::default().fg(Color::DarkGray)),
    ]));

    let stats = Paragraph::new(stats_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Stats ")
            .border_style(Style::default().fg(Color::Magenta)),
    );

    frame.render_widget(stats, area);
}

fn draw_footer(frame: &mut Frame, session: &Session, tab_manager: &TabManager, area: Rect) {
    let phase = session.workflow_state.as_ref().map(|s| &s.phase);

    let phases = [
        ("Planning", Phase::Planning),
        ("Reviewing", Phase::Reviewing),
        ("Revising", Phase::Revising),
        ("Complete", Phase::Complete),
    ];

    let mut spans: Vec<Span> = Vec::new();

    // Tab indicator
    spans.push(Span::styled(
        format!(" Tab {}/{} ", tab_manager.active_tab + 1, tab_manager.len()),
        Style::default().fg(Color::Cyan),
    ));
    spans.push(Span::styled("│ ", Style::default().fg(Color::DarkGray)));

    for (i, (name, p)) in phases.iter().enumerate() {
        let is_current = phase == Some(p);
        let is_complete = matches!(
            (phase, p),
            (Some(Phase::Complete), _)
                | (Some(Phase::Revising), Phase::Planning)
                | (Some(Phase::Reviewing), Phase::Planning)
        );

        let style = if is_current {
            Style::default().fg(Color::Yellow).bold()
        } else if is_complete {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        spans.push(Span::styled(*name, style));

        if i < phases.len() - 1 {
            spans.push(Span::styled(" → ", Style::default().fg(Color::DarkGray)));
        }
    }

    // Add keybinding hints
    spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));

    if session.approval_mode != ApprovalMode::None {
        spans.push(Span::styled(
            "[↑/↓] Scroll  [Enter] Select  [Esc] Cancel",
            Style::default().fg(Color::DarkGray),
        ));
    } else {
        spans.push(Span::styled(
            "Tabs: [Ctrl+PgUp/Dn] Switch  [Ctrl+W] Close",
            Style::default().fg(Color::DarkGray),
        ));
    }

    let footer = Paragraph::new(Line::from(spans)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    frame.render_widget(footer, area);
}

fn draw_approval_overlay(frame: &mut Frame, session: &Session) {
    let area = frame.area();

    // Create a centered popup
    let popup_width = (area.width as f32 * 0.8) as u16;
    let popup_height = (area.height as f32 * 0.8) as u16;
    let popup_x = (area.width - popup_width) / 2;
    let popup_y = (area.height - popup_height) / 2;

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the background
    frame.render_widget(Clear, popup_area);

    match session.approval_mode {
        ApprovalMode::AwaitingChoice => {
            draw_choice_popup(frame, session, popup_area);
        }
        ApprovalMode::EnteringFeedback => {
            draw_feedback_popup(frame, session, popup_area);
        }
        ApprovalMode::None => {}
    }
}

/// Parse a markdown line into styled spans
fn parse_markdown_line(line: &str) -> Line<'static> {
    let trimmed = line.trim();

    // Headers
    if let Some(header) = trimmed.strip_prefix("## ") {
        return Line::from(vec![Span::styled(
            header.to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]);
    }
    if let Some(header) = trimmed.strip_prefix("# ") {
        return Line::from(vec![Span::styled(
            header.to_string(),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )]);
    }

    // Bullet points
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
        let content = &trimmed[2..];
        let mut spans = vec![Span::styled("  • ", Style::default().fg(Color::Yellow))];
        spans.extend(parse_inline_markdown(content));
        return Line::from(spans);
    }

    // Numbered lists
    if let Some(rest) = trimmed.strip_prefix(|c: char| c.is_ascii_digit()) {
        if let Some(content) = rest.strip_prefix(". ") {
            let num = &trimmed[..trimmed.len() - rest.len()];
            let mut spans = vec![Span::styled(
                format!("  {}. ", num),
                Style::default().fg(Color::Yellow),
            )];
            spans.extend(parse_inline_markdown(content));
            return Line::from(spans);
        }
    }

    // Regular line with inline formatting
    Line::from(parse_inline_markdown(trimmed))
}

/// Parse inline markdown (**bold**) into spans
fn parse_inline_markdown(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut remaining = text;

    while let Some(start) = remaining.find("**") {
        // Add text before **
        if start > 0 {
            spans.push(Span::raw(remaining[..start].to_string()));
        }
        remaining = &remaining[start + 2..];

        // Find closing **
        if let Some(end) = remaining.find("**") {
            spans.push(Span::styled(
                remaining[..end].to_string(),
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .fg(Color::White),
            ));
            remaining = &remaining[end + 2..];
        } else {
            // No closing **, add as-is
            spans.push(Span::raw("**".to_string()));
        }
    }

    // Add remaining text
    if !remaining.is_empty() {
        spans.push(Span::raw(remaining.to_string()));
    }

    if spans.is_empty() {
        spans.push(Span::raw(text.to_string()));
    }

    spans
}

fn draw_choice_popup(frame: &mut Frame, session: &Session, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Min(0),    // Summary content
            Constraint::Length(4), // Instructions
        ])
        .split(area);

    let (title_text, title_color, border_color, block_title, summary_title) =
        match session.approval_context {
            ApprovalContext::PlanApproval => (
                " ✓ Plan Approved by AI ",
                Color::Green,
                Color::Green,
                " Review Plan ",
                " Plan Summary (j/k to scroll) ",
            ),
            ApprovalContext::ReviewDecision => (
                " ! Reviewer Errors Detected ",
                Color::Yellow,
                Color::Yellow,
                " Review Decision ",
                " Review Failure Details (j/k to scroll) ",
            ),
            ApprovalContext::PlanGenerationFailed => (
                " ✗ Plan Generation Failed ",
                Color::Red,
                Color::Red,
                " Recovery Options ",
                " Error Details (j/k to scroll) ",
            ),
            ApprovalContext::MaxIterationsReached => (
                " ⚠ Max Review Iterations Reached ",
                Color::Yellow,
                Color::Yellow,
                " Workflow Decision ",
                " Status Summary (j/k to scroll) ",
            ),
            ApprovalContext::UserOverrideApproval => (
                " ⚠ Proceeding Without AI Approval ",
                Color::Magenta,
                Color::Magenta,
                " Final Confirmation ",
                " Override Summary (j/k to scroll) ",
            ),
        };

    // Title block
    let title = Paragraph::new(Line::from(vec![Span::styled(
        title_text,
        Style::default().fg(title_color).bold(),
    )]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(block_title),
    );
    frame.render_widget(title, chunks[0]);

    // Summary content with markdown parsing
    let summary_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(summary_title);

    let inner_area = summary_block.inner(chunks[1]);
    let visible_height = inner_area.height as usize;

    let summary_lines: Vec<Line> = session.plan_summary.lines().map(parse_markdown_line).collect();

    let total_lines = summary_lines.len();
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll_pos = session.plan_summary_scroll.min(max_scroll);

    let summary = Paragraph::new(summary_lines)
        .block(summary_block)
        .scroll((scroll_pos as u16, 0));
    frame.render_widget(summary, chunks[1]);

    // Scrollbar if needed
    if total_lines > visible_height {
        let mut scrollbar_state = ScrollbarState::new(total_lines).position(scroll_pos);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓")),
            chunks[1],
            &mut scrollbar_state,
        );
    }

    // Instructions
    let instructions = match session.approval_context {
        ApprovalContext::PlanApproval => Paragraph::new(vec![Line::from(vec![
            Span::styled("  [a] ", Style::default().fg(Color::Green).bold()),
            Span::raw("Accept  "),
            Span::styled("  [i] ", Style::default().fg(Color::Magenta).bold()),
            Span::raw("Implement  "),
            Span::styled("  [d] ", Style::default().fg(Color::Yellow).bold()),
            Span::raw("Decline  "),
            Span::styled("  [j/k] ", Style::default().fg(Color::Cyan).bold()),
            Span::raw("Scroll"),
        ])]),
        ApprovalContext::ReviewDecision => Paragraph::new(vec![Line::from(vec![
            Span::styled("  [c] ", Style::default().fg(Color::Green).bold()),
            Span::raw("Continue  "),
            Span::styled("  [r] ", Style::default().fg(Color::Yellow).bold()),
            Span::raw("Retry Failed  "),
            Span::styled("  [j/k] ", Style::default().fg(Color::Cyan).bold()),
            Span::raw("Scroll"),
        ])]),
        ApprovalContext::PlanGenerationFailed => Paragraph::new(vec![Line::from(vec![
            Span::styled("  [r] ", Style::default().fg(Color::Yellow).bold()),
            Span::raw("Retry  "),
            Span::styled("  [a] ", Style::default().fg(Color::Red).bold()),
            Span::raw("Abort  "),
            Span::styled("  [j/k] ", Style::default().fg(Color::Cyan).bold()),
            Span::raw("Scroll"),
        ])]),
        ApprovalContext::MaxIterationsReached => Paragraph::new(vec![Line::from(vec![
            Span::styled("  [p] ", Style::default().fg(Color::Green).bold()),
            Span::raw("Proceed  "),
            Span::styled("  [c] ", Style::default().fg(Color::Yellow).bold()),
            Span::raw("Continue Review  "),
            Span::styled("  [d] ", Style::default().fg(Color::Cyan).bold()),
            Span::raw("Decline/Restart  "),
            Span::styled("  [a] ", Style::default().fg(Color::Red).bold()),
            Span::raw("Abort"),
        ])]),
        ApprovalContext::UserOverrideApproval => Paragraph::new(vec![Line::from(vec![
            Span::styled("  [a] ", Style::default().fg(Color::Green).bold()),
            Span::raw("Accept  "),
            Span::styled("  [d] ", Style::default().fg(Color::Yellow).bold()),
            Span::raw("Decline  "),
            Span::styled("  [j/k] ", Style::default().fg(Color::Cyan).bold()),
            Span::raw("Scroll"),
        ])]),
    }
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(instructions, chunks[2]);
}

fn draw_feedback_popup(frame: &mut Frame, session: &Session, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Min(0),    // Input area
            Constraint::Length(3), // Instructions
        ])
        .split(area);

    // Title
    let title = Paragraph::new(Line::from(vec![Span::styled(
        " Enter your feedback ",
        Style::default().fg(Color::Yellow).bold(),
    )]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(" Request Changes "),
    );
    frame.render_widget(title, chunks[0]);

    // Input area with cursor
    let has_content = !session.user_feedback.is_empty() || session.has_feedback_pastes();
    let input_text = if has_content {
        session.get_display_text_feedback()
    } else {
        "Type your changes here...".to_string()
    };

    let input_style = if has_content {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Your Feedback ");

    let inner = input_block.inner(chunks[1]);
    let input_width = inner.width as usize;

    // Use character-based wrapping to match cursor calculation
    let wrapped_input = wrap_text_at_width(&input_text, input_width);
    let input = Paragraph::new(wrapped_input)
        .style(input_style)
        .block(input_block);
    frame.render_widget(input, chunks[1]);

    // Calculate cursor position using unicode-aware method
    if has_content {
        let (cursor_row, cursor_col) = session.get_feedback_cursor_position(input_width);
        let cursor_x = inner.x + cursor_col as u16;
        let cursor_y = inner.y + cursor_row as u16;
        if cursor_y < inner.y + inner.height {
            frame.set_cursor_position((cursor_x.min(inner.x + inner.width - 1), cursor_y));
        }
    } else {
        // Placeholder text - cursor at start
        frame.set_cursor_position((inner.x, inner.y));
    }

    // Instructions
    let instructions = Paragraph::new(Line::from(vec![
        Span::styled("  [Enter] ", Style::default().fg(Color::Green).bold()),
        Span::raw("Submit  "),
        Span::styled("  [Esc] ", Style::default().fg(Color::Red).bold()),
        Span::raw("Cancel"),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(instructions, chunks[2]);
}

/// Draw the tab input overlay for entering a new tab's objective
fn draw_tab_input_overlay(frame: &mut Frame, session: &Session) {
    let area = frame.area();

    // Create a centered popup - increased height for multiline input
    let popup_width = (area.width as f32 * 0.6).min(80.0) as u16;
    let popup_height = 15;
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the background
    frame.render_widget(Clear, popup_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Min(5),    // Input - more space for multiline
            Constraint::Length(2), // Instructions
        ])
        .split(popup_area);

    // Title
    let title = Paragraph::new(Line::from(vec![Span::styled(
        "Enter planning objective:",
        Style::default().fg(Color::Cyan).bold(),
    )]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" New Tab "),
    );
    frame.render_widget(title, chunks[0]);

    // Input with cursor - now supports multiline with wrapping
    let has_content = !session.tab_input.is_empty() || session.has_tab_input_pastes();
    let input_text = if has_content {
        session.get_display_text_tab()
    } else {
        "What do you want to plan?".to_string()
    };

    let input_style = if has_content {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = input_block.inner(chunks[1]);
    let input_width = inner.width as usize;
    let input_height = inner.height as usize;

    // Calculate scroll position based on cursor position
    let (cursor_line, cursor_col) = session.get_tab_input_cursor_position();

    // Calculate visual cursor position considering line wrapping
    let mut visual_row = 0;
    let mut visual_col = cursor_col;

    for (i, line) in session.tab_input.split('\n').enumerate() {
        if i < cursor_line {
            // Add wrapped rows for previous lines using unicode display width
            let line_rows = if line.is_empty() {
                1
            } else {
                line.width().div_ceil(input_width)
            };
            visual_row += line_rows;
        } else if i == cursor_line {
            // Calculate visual position within current line due to wrapping
            // cursor_col is already display width from get_tab_input_cursor_position
            visual_row += cursor_col / input_width;
            visual_col = cursor_col % input_width;
            break;
        }
    }

    // Adjust scroll to keep cursor visible
    let scroll = if visual_row >= session.tab_input_scroll + input_height {
        visual_row.saturating_sub(input_height - 1)
    } else if visual_row < session.tab_input_scroll {
        visual_row
    } else {
        session.tab_input_scroll
    };

    // Pre-wrap text at character boundaries to match our cursor calculation
    // This avoids mismatch between word-based wrapping and character-based cursor positioning
    let wrapped_text = wrap_text_at_width(&input_text, input_width);
    let input = Paragraph::new(wrapped_text)
        .style(input_style)
        .block(input_block)
        .scroll((scroll as u16, 0));
    frame.render_widget(input, chunks[1]);

    // Show cursor - now with proper 2D positioning
    if has_content {
        let cursor_x = inner.x + visual_col as u16;
        let cursor_y = inner.y + (visual_row - scroll) as u16;
        if cursor_y < inner.y + inner.height {
            frame.set_cursor_position((cursor_x.min(inner.x + inner.width - 1), cursor_y));
        }
    }

    // Instructions - updated with new keybindings
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[Enter]", Style::default().fg(Color::Green)),
        Span::raw(" Start  "),
        Span::styled("[Shift+Enter]", Style::default().fg(Color::Blue)),
        Span::raw(" Newline  "),
        Span::styled("[Esc]", Style::default().fg(Color::Red)),
        Span::raw(" Cancel  "),
        Span::styled("[Ctrl+C/q]", Style::default().fg(Color::Red)),
        Span::raw(" Quit"),
    ]))
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, chunks[2]);
}

/// Draw an error overlay for the session
fn draw_error_overlay(frame: &mut Frame, session: &Session) {
    if let Some(ref error) = session.error_state {
        let area = frame.area();

        let popup_width = (area.width as f32 * 0.5).min(60.0) as u16;
        let popup_height = 8;
        let popup_x = (area.width.saturating_sub(popup_width)) / 2;
        let popup_y = (area.height.saturating_sub(popup_height)) / 2;

        let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

        frame.render_widget(Clear, popup_area);

        let error_text = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(error.clone(), Style::default().fg(Color::Red))),
            Line::from(""),
            Line::from(vec![
                Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
                Span::raw(" Close  "),
                Span::styled("[Ctrl+W]", Style::default().fg(Color::Red)),
                Span::raw(" Close Tab"),
            ]),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red))
                .title(" Error "),
        )
        .wrap(Wrap { trim: false });

        frame.render_widget(error_text, popup_area);
    }
}

/// Helper function to create a centered rect (used for overlays)
#[allow(dead_code)]
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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
