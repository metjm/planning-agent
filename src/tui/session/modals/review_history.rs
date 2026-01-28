//! Review history methods for Session.
//!
//! This module contains methods for tracking review rounds and their statuses
//! in the TUI session.

use super::super::{ReviewKind, ReviewRound, ReviewerEntry, ReviewerStatus, Session};

impl Session {
    /// Start a new review round
    pub fn start_review_round(&mut self, kind: ReviewKind, round: u32) {
        // Remove any existing round with same number (in case of retry)
        self.review_history
            .retain(|r| !(r.kind == kind && r.round == round));
        self.review_history.push(ReviewRound::new(kind, round));
        self.review_history_scroll.follow = true;
    }

    /// Mark a reviewer as started in the current round
    pub fn reviewer_started(&mut self, kind: ReviewKind, round: u32, display_id: String) {
        if let Some(review_round) = self
            .review_history
            .iter_mut()
            .find(|r| r.kind == kind && r.round == round)
        {
            // Remove existing entry for this reviewer (in case of retry)
            review_round
                .reviewers
                .retain(|r| r.display_id != display_id);
            review_round.reviewers.push(ReviewerEntry {
                display_id,
                status: ReviewerStatus::Running,
            });
            self.review_history_scroll.follow = true;
        }
    }

    /// Mark a reviewer as completed in the current round
    pub fn reviewer_completed(
        &mut self,
        kind: ReviewKind,
        round: u32,
        display_id: String,
        approved: bool,
        summary: String,
        duration_ms: u64,
    ) {
        if let Some(review_round) = self
            .review_history
            .iter_mut()
            .find(|r| r.kind == kind && r.round == round)
        {
            if let Some(entry) = review_round
                .reviewers
                .iter_mut()
                .find(|r| r.display_id == display_id)
            {
                entry.status = ReviewerStatus::Completed {
                    approved,
                    summary,
                    duration_ms,
                };
                self.review_history_scroll.follow = true;
            }
        }
    }

    /// Mark a reviewer as failed in the current round
    pub fn reviewer_failed(
        &mut self,
        kind: ReviewKind,
        round: u32,
        display_id: String,
        error: String,
    ) {
        if let Some(review_round) = self
            .review_history
            .iter_mut()
            .find(|r| r.kind == kind && r.round == round)
        {
            if let Some(entry) = review_round
                .reviewers
                .iter_mut()
                .find(|r| r.display_id == display_id)
            {
                entry.status = ReviewerStatus::Failed { error };
                self.review_history_scroll.follow = true;
            }
        }
    }

    /// Set aggregate verdict for a round
    pub fn set_round_verdict(&mut self, kind: ReviewKind, round: u32, approved: bool) {
        if let Some(review_round) = self
            .review_history
            .iter_mut()
            .find(|r| r.kind == kind && r.round == round)
        {
            review_round.aggregate_verdict = Some(approved);
        }
    }

    /// Advance review history spinner (called from tick handler)
    pub fn advance_review_history_spinner(&mut self) {
        if self.has_running_reviewer() {
            self.review_history_spinner_frame = self.review_history_spinner_frame.wrapping_add(1);
        }
    }

    /// Check if any reviewer is currently running
    pub fn has_running_reviewer(&self) -> bool {
        self.review_history.iter().any(|round| {
            round
                .reviewers
                .iter()
                .any(|r| matches!(r.status, ReviewerStatus::Running))
        })
    }

    /// Scroll review history up
    pub fn review_history_scroll_up(&mut self) {
        self.review_history_scroll.scroll_up();
    }

    /// Scroll review history down
    pub fn review_history_scroll_down(&mut self, max_scroll: usize) {
        self.review_history_scroll.scroll_down(max_scroll);
    }

    /// Scroll review history to bottom
    pub fn review_history_scroll_to_bottom(&mut self, max_scroll: usize) {
        self.review_history_scroll.scroll_to_bottom(max_scroll);
    }
}

#[cfg(test)]
#[path = "../tests/review_history_tests.rs"]
mod tests;
