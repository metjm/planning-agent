use anyhow::Result;
use crossterm::event::{Event as CrosstermEvent, KeyEvent, KeyEventKind};
use futures::StreamExt;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::state::State;

/// Token usage statistics from Claude API
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
}

#[derive(Debug, Clone)]
pub enum Event {
    Key(KeyEvent),
    Tick,
    Output(String),
    Streaming(String),
    ToolStarted(String),
    ToolFinished(String),
    StateUpdate(State),
    /// Request user approval with a plan summary
    RequestUserApproval(String),
    /// Stats update: bytes received from Claude
    BytesReceived(usize),
    /// Token usage update from a Claude message
    TokenUsage(TokenUsage),
    /// Phase timing: phase started
    PhaseStarted(String),
    /// A conversation turn completed (assistant responded after user input)
    TurnCompleted,
    /// Claude model name detected
    ModelDetected(String),
    /// Tool result received with error status
    ToolResultReceived { tool_id: String, is_error: bool },
    /// Claude's stop reason (end_turn, tool_use, max_tokens)
    StopReason(String),
}

/// User's response to approval request
#[derive(Debug, Clone)]
pub enum UserApprovalResponse {
    Accept,
    AcceptAndImplement(PathBuf), // Accept and launch Claude to implement
    Decline(String),             // Contains user's feedback for changes
}

pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<Event>,
    _tx: mpsc::UnboundedSender<Event>,
}

impl EventHandler {
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let event_tx = tx.clone();

        // Spawn keyboard/tick event handler
        tokio::spawn(async move {
            let mut event_stream = crossterm::event::EventStream::new();
            let mut tick_interval = tokio::time::interval(tick_rate);

            loop {
                tokio::select! {
                    maybe_event = event_stream.next() => {
                        match maybe_event {
                            Some(Ok(CrosstermEvent::Key(key))) => {
                                if key.kind == KeyEventKind::Press
                                    && event_tx.send(Event::Key(key)).is_err()
                                {
                                    break;
                                }
                            }
                            Some(Err(_)) | None => break,
                            _ => {}
                        }
                    }
                    _ = tick_interval.tick() => {
                        if event_tx.send(Event::Tick).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Self { rx, _tx: tx }
    }

    pub fn sender(&self) -> mpsc::UnboundedSender<Event> {
        self._tx.clone()
    }

    pub async fn next(&mut self) -> Result<Event> {
        self.rx
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("Event channel closed"))
    }
}
