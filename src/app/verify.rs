use crate::config::WorkflowConfig;
use crate::phases::{
    parse_verification_verdict, run_fixing_phase, run_verification_phase, VerificationVerdictResult,
};
use crate::session_logger::SessionLogger;
use crate::tui::SessionEventSender;
use crate::verification_state::{normalize_plan_path, VerificationPhase, VerificationState};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Result of the verification workflow.
#[derive(Debug, Clone)]
pub enum VerificationResult {
    /// All verification checks passed
    Approved,
    /// Verification failed even after max iterations
    Failed { iterations_used: u32 },
}

/// Runs the verification workflow loop.
///
/// This runs the verify/fix cycle until either:
/// - Verification passes (returns VerificationResult::Approved)
/// - Max iterations reached (returns VerificationResult::Failed)
/// - An error occurs (returns Err)
pub async fn run_verification_workflow(
    plan_path: &Path,
    working_dir: &Path,
    config: &WorkflowConfig,
    session_sender: SessionEventSender,
    session_logger: Arc<SessionLogger>,
) -> Result<VerificationResult> {
    // Normalize plan path (accept both folder and file paths)
    let plan_folder = normalize_plan_path(plan_path);

    // Validate plan exists
    let plan_file = plan_folder.join("plan.md");
    if !plan_file.exists() {
        anyhow::bail!(
            "Plan file not found: {}. Expected a plan.md file in the plan folder.",
            plan_file.display()
        );
    }

    // Check verification is enabled
    if !config.verification.enabled {
        anyhow::bail!(
            "Verification is not enabled in the workflow config. \
             Add 'verification: enabled: true' to your workflow.yaml to enable."
        );
    }

    // Load or create verification state
    let mut verification_state = match VerificationState::load(&plan_folder)? {
        Some(mut state) => {
            session_sender.send_output(format!(
                "[verification] Resuming verification from iteration {} (phase: {:?})",
                state.iteration, state.phase
            ));
            // Reset phase to Verifying if we're resuming from a completed or failed state
            if state.phase == VerificationPhase::Complete {
                state.phase = VerificationPhase::Verifying;
                state.iteration = 1;
            }
            state
        }
        None => {
            session_sender.send_output(format!(
                "[verification] Starting new verification for plan: {}",
                plan_folder.display()
            ));
            VerificationState::new(
                plan_folder.clone(),
                working_dir.to_path_buf(),
                config.verification.max_iterations,
                None,
            )
        }
    };

    // Save initial state
    verification_state.save()?;

    // Run verification loop
    while verification_state.should_continue() {
        match verification_state.phase {
            VerificationPhase::Verifying => {
                session_sender.send_output(format!(
                    "[verification] === Verification Round {}/{} ===",
                    verification_state.iteration, verification_state.max_iterations
                ));
                session_sender.send_verification_started(verification_state.iteration);

                // Run verification phase
                let report = run_verification_phase(&mut verification_state, config, session_sender.clone(), session_logger.clone())
                    .await
                    .context("Verification phase failed")?;

                // Parse verdict and transition
                let verdict = parse_verification_verdict(&report);
                let verdict_str = match &verdict {
                    VerificationVerdictResult::Approved => "APPROVED",
                    VerificationVerdictResult::NeedsRevision => "NEEDS_REVISION",
                    VerificationVerdictResult::ParseFailure { .. } => "PARSE_FAILURE",
                };
                session_sender.send_verification_completed(verdict_str.to_string(), report.clone());

                match verdict {
                    VerificationVerdictResult::Approved => {
                        verification_state.transition(VerificationPhase::Complete)?;
                        verification_state.save()?;
                        session_sender.send_output(
                            "[verification] Implementation verified successfully!".to_string(),
                        );
                        session_sender.send_verification_result(true, verification_state.iteration);
                        return Ok(VerificationResult::Approved);
                    }
                    VerificationVerdictResult::NeedsRevision
                    | VerificationVerdictResult::ParseFailure { .. } => {
                        if verification_state.iteration >= verification_state.max_iterations {
                            session_sender.send_output(format!(
                                "[verification] Max iterations ({}) reached without approval",
                                verification_state.max_iterations
                            ));
                            verification_state.phase = VerificationPhase::Complete;
                            verification_state.save()?;
                            session_sender.send_verification_result(false, verification_state.iteration);
                            return Ok(VerificationResult::Failed {
                                iterations_used: verification_state.iteration,
                            });
                        }

                        session_sender.send_output(
                            "[verification] Issues found, transitioning to fixing phase...".to_string(),
                        );
                        verification_state.transition(VerificationPhase::Fixing)?;
                        verification_state.save()?;
                    }
                }
            }
            VerificationPhase::Fixing => {
                session_sender.send_output(format!(
                    "[fixing] === Fix Round {}/{} ===",
                    verification_state.iteration, verification_state.max_iterations
                ));
                session_sender.send_fixing_started(verification_state.iteration);

                // Load the latest verification report
                let report_path = verification_state.verification_report_path();
                let report = std::fs::read_to_string(&report_path)
                    .with_context(|| format!("Failed to read verification report: {}", report_path.display()))?;

                // Run fixing phase
                run_fixing_phase(&mut verification_state, config, &report, session_sender.clone(), session_logger.clone())
                    .await
                    .context("Fixing phase failed")?;

                session_sender.send_fixing_completed();

                // Transition back to verification
                session_sender.send_output(
                    "[fixing] Fix complete, transitioning back to verification...".to_string(),
                );
                verification_state.transition(VerificationPhase::Verifying)?;
                verification_state.save()?;
            }
            VerificationPhase::Complete => {
                // Should not reach here in the loop, but handle gracefully
                break;
            }
        }
    }

    // If we exited the loop without returning, max iterations was exceeded
    Ok(VerificationResult::Failed {
        iterations_used: verification_state.iteration,
    })
}

/// Runs verification workflow in headless mode (no TUI).
pub async fn run_headless_verification(
    plan_path: PathBuf,
    working_dir: PathBuf,
    config_path: Option<PathBuf>,
) -> Result<()> {
    // Load config
    let config = if let Some(path) = config_path {
        WorkflowConfig::load(&path)?
    } else {
        WorkflowConfig::default_config()
    };

    println!("╔════════════════════════════════════════════════════════════════╗");
    println!("║              Planning Agent - Verification Mode                 ║");
    println!("╚════════════════════════════════════════════════════════════════╝");
    println!();
    println!("Plan: {}", plan_path.display());
    println!("Working Dir: {}", working_dir.display());
    println!("Max Iterations: {}", config.verification.max_iterations);
    println!();

    // Create session logger for headless verification
    let session_id = format!("verify-{}", uuid::Uuid::new_v4());
    let session_logger = SessionLogger::new(&session_id)
        .context("Failed to create session logger for verification")?;
    let session_logger = Arc::new(session_logger);

    // Create a simple event sender that prints to stdout
    let (tx, mut rx) = mpsc::unbounded_channel();
    let session_sender = SessionEventSender::new(0, 0, tx);

    // Spawn a task to print events
    let print_handle = tokio::spawn(async move {
        use crate::tui::Event;
        while let Some(event) = rx.recv().await {
            match event {
                Event::SessionOutput { line, .. } => {
                    println!("{}", line);
                }
                Event::SessionVerificationStarted { iteration, .. } => {
                    println!("\n┌─────────────────────────────────────────────────────────────────┐");
                    println!("│ Verification Round {}                                             │", iteration);
                    println!("└─────────────────────────────────────────────────────────────────┘");
                }
                Event::SessionVerificationCompleted { verdict, .. } => {
                    println!("\n→ Verdict: {}", verdict);
                }
                Event::SessionFixingStarted { iteration, .. } => {
                    println!("\n┌─────────────────────────────────────────────────────────────────┐");
                    println!("│ Fix Round {}                                                      │", iteration);
                    println!("└─────────────────────────────────────────────────────────────────┘");
                }
                Event::SessionFixingCompleted { .. } => {
                    println!("\n→ Fix round complete");
                }
                Event::SessionVerificationResult { approved, iterations_used, .. } => {
                    println!();
                    if approved {
                        println!("╔════════════════════════════════════════════════════════════════╗");
                        println!("║                    ✓ VERIFICATION PASSED                        ║");
                        println!("╚════════════════════════════════════════════════════════════════╝");
                    } else {
                        println!("╔════════════════════════════════════════════════════════════════╗");
                        println!("║                    ✗ VERIFICATION FAILED                        ║");
                        println!("╚════════════════════════════════════════════════════════════════╝");
                    }
                    println!("Iterations used: {}", iterations_used);
                }
                _ => {}
            }
        }
    });

    // Run verification
    let result = run_verification_workflow(&plan_path, &working_dir, &config, session_sender, session_logger).await;

    // Wait for print task to finish
    drop(print_handle);

    match result {
        Ok(VerificationResult::Approved) => {
            Ok(())
        }
        Ok(VerificationResult::Failed { iterations_used }) => {
            eprintln!("\nVerification failed after {} iterations.", iterations_used);
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("\nError during verification: {}", e);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_plan_path_folder() {
        let path = PathBuf::from("/tmp/plan-folder");
        let normalized = normalize_plan_path(&path);
        assert_eq!(normalized, path);
    }

    #[test]
    fn test_normalize_plan_path_file() {
        let path = PathBuf::from("/tmp/plan-folder/plan.md");
        let normalized = normalize_plan_path(&path);
        assert_eq!(normalized, PathBuf::from("/tmp/plan-folder"));
    }
}
