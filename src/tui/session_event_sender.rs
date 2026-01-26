//! Session-scoped event sender for workflow integration.
//!
//! This module provides `SessionEventSender` which wraps the event channel
//! to automatically inject session IDs into all events.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;

use crate::app::workflow_decisions::IterativePhase;
use crate::domain::view::WorkflowView;
use crate::state::State;
use crate::tui::session::{CliInstanceId, ReviewKind, TodoItem, ToolResultSummary};

use super::event::{Event, TokenUsage};

/// Sends events scoped to a specific session.
#[derive(Clone)]
pub struct SessionEventSender {
    session_id: usize,
    run_id: u64,
    inner: mpsc::UnboundedSender<Event>,
    /// Monotonic counter for generating unique CLI instance IDs per session.
    cli_instance_counter: Arc<AtomicU64>,
}

/// Some methods may not be used in all code paths but are part of the
/// public API for workflow integration.
impl SessionEventSender {
    pub fn new(session_id: usize, run_id: u64, sender: mpsc::UnboundedSender<Event>) -> Self {
        Self {
            session_id,
            run_id,
            inner: sender,
            cli_instance_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn send_output(&self, line: String) {
        let _ = self.inner.send(Event::SessionOutput {
            session_id: self.session_id,
            line,
        });
    }

    pub fn send_streaming(&self, line: String) {
        let _ = self.inner.send(Event::SessionStreaming {
            session_id: self.session_id,
            line,
        });
    }

    pub fn send_state_update(&self, state: State) {
        let _ = self.inner.send(Event::SessionStateUpdate {
            session_id: self.session_id,
            state,
        });
    }

    /// Send a CQRS view update (event-sourced replacement for send_state_update).
    pub fn send_view_update(&self, view: WorkflowView) {
        let _ = self.inner.send(Event::SessionViewUpdate {
            session_id: self.session_id,
            view,
        });
    }

    pub fn send_approval_request(&self, summary: String) {
        let _ = self.inner.send(Event::SessionApprovalRequest {
            session_id: self.session_id,
            summary,
        });
    }

    pub fn send_review_decision_request(&self, summary: String) {
        let _ = self.inner.send(Event::SessionReviewDecisionRequest {
            session_id: self.session_id,
            summary,
        });
    }

    pub fn send_token_usage(&self, usage: TokenUsage) {
        let _ = self.inner.send(Event::SessionTokenUsage {
            session_id: self.session_id,
            usage,
        });
    }

    pub fn send_tool_started(
        &self,
        phase: String,
        tool_id: Option<String>,
        display_name: String,
        input_preview: String,
        agent_name: String,
    ) {
        let _ = self.inner.send(Event::SessionToolStarted {
            session_id: self.session_id,
            tool_id,
            display_name,
            input_preview,
            agent_name,
            phase,
        });
    }

    pub fn send_tool_finished(&self, tool_id: Option<String>, agent_name: String) {
        let _ = self.inner.send(Event::SessionToolFinished {
            session_id: self.session_id,
            tool_id,
            agent_name,
        });
    }

    pub fn send_bytes_received(&self, bytes: usize) {
        let _ = self.inner.send(Event::SessionBytesReceived {
            session_id: self.session_id,
            bytes,
        });
    }

    pub fn send_phase_started(&self, phase: String) {
        let _ = self.inner.send(Event::SessionPhaseStarted {
            session_id: self.session_id,
            phase,
        });
    }

    pub fn send_turn_completed(&self) {
        let _ = self.inner.send(Event::SessionTurnCompleted {
            session_id: self.session_id,
        });
    }

    pub fn send_model_detected(&self, name: String) {
        let _ = self.inner.send(Event::SessionModelDetected {
            session_id: self.session_id,
            name,
        });
    }

    pub fn send_tool_result_received(
        &self,
        phase: String,
        tool_id: Option<String>,
        is_error: bool,
        summary: ToolResultSummary,
        agent_name: String,
    ) {
        let _ = self.inner.send(Event::SessionToolResultReceived {
            session_id: self.session_id,
            tool_id,
            is_error,
            agent_name,
            phase,
            summary,
        });
    }

    pub fn send_stop_reason(&self, reason: String) {
        let _ = self.inner.send(Event::SessionStopReason {
            session_id: self.session_id,
            reason,
        });
    }

    pub fn send_plan_generation_failed(&self, error: String) {
        let _ = self.inner.send(Event::SessionPlanGenerationFailed {
            session_id: self.session_id,
            error,
        });
    }

    pub fn send_all_reviewers_failed(&self, summary: String) {
        let _ = self.inner.send(Event::SessionAllReviewersFailed {
            session_id: self.session_id,
            summary,
        });
    }

    pub fn send_review_round_started(&self, kind: ReviewKind, round: u32) {
        let _ = self.inner.send(Event::SessionReviewRoundStarted {
            session_id: self.session_id,
            kind,
            round,
        });
    }

    pub fn send_reviewer_started(&self, kind: ReviewKind, round: u32, display_id: String) {
        let _ = self.inner.send(Event::SessionReviewerStarted {
            session_id: self.session_id,
            kind,
            round,
            display_id,
        });
    }

    pub fn send_reviewer_completed(
        &self,
        kind: ReviewKind,
        round: u32,
        display_id: String,
        approved: bool,
        summary: String,
        duration_ms: u64,
    ) {
        let _ = self.inner.send(Event::SessionReviewerCompleted {
            session_id: self.session_id,
            kind,
            round,
            display_id,
            approved,
            summary,
            duration_ms,
        });
    }

    pub fn send_reviewer_failed(
        &self,
        kind: ReviewKind,
        round: u32,
        display_id: String,
        error: String,
    ) {
        let _ = self.inner.send(Event::SessionReviewerFailed {
            session_id: self.session_id,
            kind,
            round,
            display_id,
            error,
        });
    }

    pub fn send_review_round_completed(&self, kind: ReviewKind, round: u32, approved: bool) {
        let _ = self.inner.send(Event::SessionReviewRoundCompleted {
            session_id: self.session_id,
            kind,
            round,
            approved,
        });
    }

    pub fn send_workflow_failure(&self, summary: String) {
        let _ = self.inner.send(Event::SessionWorkflowFailure {
            session_id: self.session_id,
            summary,
        });
    }

    /// Sends a max iterations reached event to trigger the decision modal.
    pub fn send_max_iterations_reached(&self, phase: IterativePhase, summary: String) {
        let _ = self.inner.send(Event::SessionMaxIterationsReached {
            session_id: self.session_id,
            phase,
            summary,
        });
    }

    pub fn send_user_override_approval(&self, summary: String) {
        let _ = self.inner.send(Event::SessionUserOverrideApproval {
            session_id: self.session_id,
            summary,
        });
    }

    pub fn send_agent_message(&self, agent_name: String, phase: String, message: String) {
        let _ = self.inner.send(Event::SessionAgentMessage {
            session_id: self.session_id,
            agent_name,
            phase,
            message,
        });
    }

    pub fn send_todos_update(&self, agent_name: String, todos: Vec<TodoItem>) {
        let _ = self.inner.send(Event::SessionTodosUpdate {
            session_id: self.session_id,
            agent_name,
            todos,
        });
    }

    pub fn send_run_tab_summary_generating(&self, phase: String) {
        let _ = self.inner.send(Event::SessionRunTabSummaryGenerating {
            session_id: self.session_id,
            phase,
            run_id: self.run_id,
        });
    }

    pub fn send_run_tab_summary_ready(&self, phase: String, summary: String) {
        let _ = self.inner.send(Event::SessionRunTabSummaryReady {
            session_id: self.session_id,
            phase,
            summary,
            run_id: self.run_id,
        });
    }

    pub fn send_run_tab_summary_error(&self, phase: String, error: String) {
        let _ = self.inner.send(Event::SessionRunTabSummaryError {
            session_id: self.session_id,
            phase,
            error,
            run_id: self.run_id,
        });
    }

    // CLI instance lifecycle event helpers

    /// Allocates a new unique CLI instance ID for this session.
    pub fn next_cli_instance_id(&self) -> CliInstanceId {
        CliInstanceId(self.cli_instance_counter.fetch_add(1, Ordering::Relaxed))
    }

    /// Sends a CLI instance started event.
    pub fn send_cli_instance_started(
        &self,
        id: CliInstanceId,
        agent_name: String,
        pid: Option<u32>,
        started_at: Instant,
    ) {
        let _ = self.inner.send(Event::SessionCliInstanceStarted {
            session_id: self.session_id,
            id,
            agent_name,
            pid,
            started_at,
        });
    }

    /// Sends a CLI instance activity event.
    pub fn send_cli_instance_activity(&self, id: CliInstanceId, activity_at: Instant) {
        let _ = self.inner.send(Event::SessionCliInstanceActivity {
            session_id: self.session_id,
            id,
            activity_at,
        });
    }

    /// Sends a CLI instance finished event.
    pub fn send_cli_instance_finished(&self, id: CliInstanceId) {
        let _ = self.inner.send(Event::SessionCliInstanceFinished {
            session_id: self.session_id,
            id,
        });
    }

    /// Sends an implementation success event to trigger the success modal.
    pub fn send_implementation_success(&self, iterations_used: u32) {
        let _ = self.inner.send(Event::SessionImplementationSuccess {
            session_id: self.session_id,
            iterations_used,
        });
    }

    /// Sends an implementation interaction finished event.
    pub fn send_implementation_interaction_finished(&self) {
        let _ = self
            .inner
            .send(Event::SessionImplementationInteractionFinished {
                session_id: self.session_id,
            });
    }
}

#[cfg(test)]
#[path = "tests/session_event_sender_tests.rs"]
mod tests;
