//! Approval-related methods for Session.

use super::{ApprovalContext, ApprovalMode, FeedbackTarget, Session, SessionStatus};

impl Session {
    pub fn start_approval(&mut self, summary: String) {
        self.plan_summary = summary;
        self.plan_summary_scroll = 0;
        self.approval_mode = ApprovalMode::AwaitingChoice;
        self.approval_context = ApprovalContext::PlanApproval;
        self.user_feedback.clear();
        self.cursor_position = 0;
        self.status = SessionStatus::AwaitingApproval;
    }

    pub fn start_review_decision(&mut self, summary: String) {
        self.plan_summary = summary;
        self.plan_summary_scroll = 0;
        self.approval_mode = ApprovalMode::AwaitingChoice;
        self.approval_context = ApprovalContext::ReviewDecision;
        self.user_feedback.clear();
        self.cursor_position = 0;
        self.status = SessionStatus::AwaitingApproval;
    }

    pub fn start_max_iterations_prompt(&mut self, summary: String) {
        self.plan_summary = summary;
        self.plan_summary_scroll = 0;
        self.approval_mode = ApprovalMode::AwaitingChoice;
        self.approval_context = ApprovalContext::MaxIterationsReached;
        self.user_feedback.clear();
        self.cursor_position = 0;
        self.status = SessionStatus::AwaitingApproval;
    }

    pub fn start_plan_generation_failed(&mut self, error: String) {
        self.plan_summary = error;
        self.plan_summary_scroll = 0;
        self.approval_mode = ApprovalMode::AwaitingChoice;
        self.approval_context = ApprovalContext::PlanGenerationFailed;
        self.user_feedback.clear();
        self.cursor_position = 0;
        self.status = SessionStatus::AwaitingApproval;
    }

    pub fn start_user_override_approval(&mut self, summary: String) {
        self.plan_summary = summary;
        self.plan_summary_scroll = 0;
        self.approval_mode = ApprovalMode::AwaitingChoice;
        self.approval_context = ApprovalContext::UserOverrideApproval;
        self.user_feedback.clear();
        self.cursor_position = 0;
        self.status = SessionStatus::AwaitingApproval;
    }

    pub fn start_all_reviewers_failed(&mut self, summary: String) {
        self.plan_summary = summary;
        self.plan_summary_scroll = 0;
        self.approval_mode = ApprovalMode::AwaitingChoice;
        self.approval_context = ApprovalContext::AllReviewersFailed;
        self.user_feedback.clear();
        self.cursor_position = 0;
        self.status = SessionStatus::AwaitingApproval;
    }

    pub fn start_workflow_failure(&mut self, summary: String) {
        self.plan_summary = summary;
        self.plan_summary_scroll = 0;
        self.approval_mode = ApprovalMode::AwaitingChoice;
        self.approval_context = ApprovalContext::WorkflowFailure;
        self.user_feedback.clear();
        self.cursor_position = 0;
        self.status = SessionStatus::AwaitingApproval;
    }

    pub fn scroll_summary_up(&mut self) {
        self.plan_summary_scroll = self.plan_summary_scroll.saturating_sub(1);
    }

    pub fn scroll_summary_down(&mut self, max_scroll: usize) {
        if self.plan_summary_scroll < max_scroll {
            self.plan_summary_scroll += 1;
        }
    }

    pub fn start_feedback_input(&mut self) {
        self.start_feedback_input_for(FeedbackTarget::ApprovalDecline);
    }

    pub fn start_feedback_input_for(&mut self, target: FeedbackTarget) {
        self.approval_mode = ApprovalMode::EnteringFeedback;
        self.feedback_target = target;
        self.user_feedback.clear();
        self.cursor_position = 0;
        self.feedback_scroll = 0;
    }
}
