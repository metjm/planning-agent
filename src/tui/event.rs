use anyhow::Result;
use crossterm::event::{Event as CrosstermEvent, KeyEvent, KeyEventKind};
use futures::StreamExt;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::state::State;

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
}

/// User's response to approval request
#[derive(Debug, Clone)]
pub enum UserApprovalResponse {
    Accept,
    Decline(String), // Contains user's feedback for changes
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
                                if key.kind == KeyEventKind::Press {
                                    if event_tx.send(Event::Key(key)).is_err() {
                                        break;
                                    }
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
