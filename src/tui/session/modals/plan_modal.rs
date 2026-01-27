//! Plan modal methods for Session.

use super::super::{ImplementationSuccessModal, Session};

impl Session {
    /// Toggle the plan modal open/closed.
    /// When opening, reads the plan file from disk and populates plan_modal_content.
    /// Returns true if the modal was opened, false if it was closed or no plan file exists.
    pub fn toggle_plan_modal(&mut self, working_dir: &std::path::Path) -> bool {
        if self.plan_modal_open {
            // Close the modal
            self.plan_modal_open = false;
            self.plan_modal_content.clear();
            false
        } else {
            // Try to open the modal
            let plan_path_opt = self
                .workflow_view
                .as_ref()
                .and_then(|v| v.plan_path())
                .map(|p| {
                    let path = p.as_path();
                    if path.is_absolute() {
                        path.to_path_buf()
                    } else {
                        working_dir.join(path)
                    }
                });

            if let Some(plan_path) = plan_path_opt {
                match std::fs::read_to_string(&plan_path) {
                    Ok(content) => {
                        self.plan_modal_content = content;
                        self.plan_modal_open = true;
                        self.plan_modal_scroll = 0;
                        true
                    }
                    Err(e) => {
                        self.plan_modal_content = format!(
                            "Unable to read plan file:\n{}\n\nError: {}",
                            plan_path.display(),
                            e
                        );
                        self.plan_modal_open = true;
                        self.plan_modal_scroll = 0;
                        true
                    }
                }
            } else {
                // No workflow view or plan path, cannot open modal
                false
            }
        }
    }

    /// Close the plan modal if it's open.
    pub fn close_plan_modal(&mut self) {
        self.plan_modal_open = false;
        self.plan_modal_content.clear();
    }

    /// Scroll the plan modal up by one line.
    pub fn plan_modal_scroll_up(&mut self) {
        self.plan_modal_scroll = self.plan_modal_scroll.saturating_sub(1);
    }

    /// Scroll the plan modal down by one line, respecting max_scroll.
    pub fn plan_modal_scroll_down(&mut self, max_scroll: usize) {
        if self.plan_modal_scroll < max_scroll {
            self.plan_modal_scroll += 1;
        }
    }

    /// Scroll the plan modal to the top.
    pub fn plan_modal_scroll_to_top(&mut self) {
        self.plan_modal_scroll = 0;
    }

    /// Scroll the plan modal to the bottom.
    pub fn plan_modal_scroll_to_bottom(&mut self, max_scroll: usize) {
        self.plan_modal_scroll = max_scroll;
    }

    /// Scroll the plan modal by a page (visible height).
    pub fn plan_modal_page_down(&mut self, visible_height: usize, max_scroll: usize) {
        self.plan_modal_scroll = (self.plan_modal_scroll + visible_height).min(max_scroll);
    }

    /// Scroll the plan modal up by a page (visible height).
    pub fn plan_modal_page_up(&mut self, visible_height: usize) {
        self.plan_modal_scroll = self.plan_modal_scroll.saturating_sub(visible_height);
    }

    /// Open the implementation success modal with the given iteration count.
    /// Closes any conflicting modals (plan modal) if open.
    pub fn open_implementation_success(&mut self, iterations_used: u32) {
        // Close plan modal if open to avoid modal conflicts
        if self.plan_modal_open {
            self.close_plan_modal();
        }
        self.implementation_success_modal = Some(ImplementationSuccessModal { iterations_used });
    }

    /// Close the implementation success modal.
    pub fn close_implementation_success(&mut self) {
        self.implementation_success_modal = None;
    }
}
