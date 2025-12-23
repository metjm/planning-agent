
use super::util::{format_bytes, format_duration, format_tokens};
use crate::state::Phase;
use crate::tui::{Session, SessionStatus};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

pub fn draw_stats(frame: &mut Frame, session: &Session, area: Rect) {
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

    let cost = session.display_cost();

    let mut stats_text = vec![
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

    stats_text.extend(build_account_usage(session));

    stats_text.push(Line::from(""));
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

    if session.status == SessionStatus::GeneratingSummary {
        stats_text.push(Line::from(""));
        stats_text.push(Line::from(vec![Span::styled(
            "── Summary ──",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )]));

        let spinner_chars = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let spinner_char = spinner_chars[(session.spinner_frame as usize) % spinner_chars.len()];

        stats_text.push(Line::from(vec![
            Span::styled(format!(" {} ", spinner_char), Style::default().fg(Color::Yellow)),
            Span::styled("Generating...", Style::default().fg(Color::Cyan)),
        ]));
    }

    stats_text.extend(build_model_info(session));

    stats_text.push(Line::from(""));

    stats_text.extend(build_cache_stats(session));

    stats_text.extend(build_stream_stats(session));

    stats_text.extend(build_tool_stats(session));

    stats_text.extend(build_timing_stats(session));

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

fn build_account_usage(session: &Session) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let has_any_usage = !session.account_usage.providers.is_empty();

    if has_any_usage {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "── Account ──",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )]));

        let mut providers: Vec<_> = session.account_usage.providers.values().collect();
        providers.sort_by(|a, b| {
            if a.provider == "claude" { std::cmp::Ordering::Less }
            else if b.provider == "claude" { std::cmp::Ordering::Greater }
            else { a.provider.cmp(&b.provider) }
        });

        for provider in providers {
            if provider.fetched_at.is_none() {
                continue;
            }

            let header_style = if provider.supports_usage {
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            if provider.has_error() || !provider.supports_usage {
                let reason = provider.status_message.as_deref().unwrap_or("N/A").to_string();
                lines.push(Line::from(vec![
                    Span::styled(format!(" {}: ", provider.display_name), Style::default().fg(Color::White)),
                    Span::styled("N/A", Style::default().fg(Color::DarkGray)),
                    Span::styled(format!(" ({})", reason), Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(format!(" {}", provider.display_name), header_style),
                ]));

                if let Some(ref plan) = provider.plan_type {
                    lines.push(Line::from(vec![
                        Span::styled("  Plan: ", Style::default().fg(Color::White)),
                        Span::styled(plan.clone(), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    ]));
                }

                if let Some(session_pct) = provider.session_used {
                    let color = if session_pct >= 90 { Color::Red }
                               else if session_pct >= 70 { Color::Yellow }
                               else { Color::Green };
                    let label = if provider.provider == "codex" { "  5h: " } else { "  Session: " };
                    lines.push(Line::from(vec![
                        Span::styled(label, Style::default().fg(Color::White)),
                        Span::styled(format!("{}% used", session_pct), Style::default().fg(color)),
                    ]));
                }

                if let Some(weekly_pct) = provider.weekly_used {
                    let color = if weekly_pct >= 90 { Color::Red }
                               else if weekly_pct >= 70 { Color::Yellow }
                               else { Color::Green };
                    let label = if provider.provider == "gemini" { "  Daily: " } else { "  Weekly: " };
                    lines.push(Line::from(vec![
                        Span::styled(label, Style::default().fg(Color::White)),
                        Span::styled(format!("{}% used", weekly_pct), Style::default().fg(color)),
                    ]));
                }
            }
        }
    }

    lines
}

fn build_model_info(session: &Session) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if let Some(ref model) = session.model_name {
        lines.push(Line::from(vec![
            Span::styled(" Model: ", Style::default().fg(Color::DarkGray)),
            Span::styled(model.clone(), Style::default().fg(Color::Cyan)),
        ]));
    }

    if let Some(ref reason) = session.last_stop_reason {
        let (icon, color) = match reason.as_str() {
            "end_turn" => ("●", Color::Green),
            "tool_use" => ("⚙", Color::Yellow),
            "max_tokens" => ("!", Color::Red),
            _ => ("?", Color::Gray),
        };
        lines.push(Line::from(vec![
            Span::styled(" Stop: ", Style::default().fg(Color::DarkGray)),
            Span::styled(icon, Style::default().fg(color)),
            Span::styled(format!(" {}", reason), Style::default().fg(color)),
        ]));
    }

    lines
}

fn build_cache_stats(session: &Session) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if session.total_cache_read_tokens > 0 || session.total_cache_creation_tokens > 0 {
        lines.push(Line::from(vec![
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
        lines.push(Line::from(""));
    }

    lines
}

fn build_stream_stats(session: &Session) -> Vec<Line<'static>> {
    vec![
        Line::from(vec![Span::styled(
            " Stream",
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![
            Span::styled(" Recv: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format_bytes(session.bytes_received),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Rate: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}/s", format_bytes(session.bytes_per_second as usize)),
                Style::default().fg(if session.bytes_per_second > 100.0 {
                    Color::Green
                } else {
                    Color::Yellow
                }),
            ),
        ]),
        Line::from(""),
    ]
}

fn build_tool_stats(session: &Session) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(vec![Span::styled(
            " Tools",
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![
            Span::styled(" Calls: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", session.tool_call_count),
                Style::default().fg(Color::White),
            ),
        ]),
    ];

    if !session.active_tools.is_empty() {
        for (name, start_time) in session.active_tools.iter().take(10) {
            let elapsed = start_time.elapsed().as_secs();
            lines.push(Line::from(vec![
                Span::styled(" ▶ ", Style::default().fg(Color::Yellow)),
                Span::styled(
                    format!("{} ({}s)", name, elapsed),
                    Style::default().fg(Color::Yellow),
                ),
            ]));
        }
        if session.active_tools.len() > 10 {
            lines.push(Line::from(Span::styled(
                format!("   +{} more", session.active_tools.len() - 10),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    if session.tool_error_count > 0 {
        lines.push(Line::from(vec![
            Span::styled(" Errors: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", session.tool_error_count),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    if let Some(avg_ms) = session.average_tool_duration_ms() {
        let duration_display = if avg_ms > 1000 {
            format!("{:.1}s", avg_ms as f64 / 1000.0)
        } else {
            format!("{}ms", avg_ms)
        };
        lines.push(Line::from(vec![
            Span::styled(" Avg Tool: ", Style::default().fg(Color::DarkGray)),
            Span::styled(duration_display, Style::default().fg(Color::White)),
        ]));
    }

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
        lines.push(Line::from(vec![
            Span::styled(" Success: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{:.0}%", success_rate), Style::default().fg(color)),
        ]));
    }

    lines.push(Line::from(""));
    lines
}

fn build_timing_stats(session: &Session) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if !session.phase_times.is_empty() || session.current_phase_start.is_some() {
        lines.push(Line::from(vec![Span::styled(
            " Timing",
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        for (phase, duration) in &session.phase_times {
            lines.push(Line::from(vec![
                Span::styled(format!(" {}: ", phase), Style::default().fg(Color::DarkGray)),
                Span::styled(format_duration(*duration), Style::default().fg(Color::White)),
            ]));
        }
        if let Some((phase, start)) = &session.current_phase_start {
            lines.push(Line::from(vec![
                Span::styled(format!(" {}: ", phase), Style::default().fg(Color::Yellow)),
                Span::styled(
                    format_duration(start.elapsed()),
                    Style::default().fg(Color::Yellow),
                ),
            ]));
        }
        lines.push(Line::from(""));
    }

    lines
}
