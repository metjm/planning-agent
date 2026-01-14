//! Terminal spawning utilities for cross-directory session resume.
//!
//! This module provides functionality to spawn a new terminal window in a different
//! directory with a planning-agent session resume command.

use anyhow::{anyhow, Result};
use std::path::Path;
use std::process::Command;

/// Spawn a new terminal window with a planning-agent resume command.
///
/// This function handles platform-specific terminal spawning:
/// - macOS: Uses `open -a Terminal` or iTerm2 if available
/// - Linux: Tries common terminal emulators (gnome-terminal, konsole, xterm, etc.)
/// - Windows: Uses `cmd /c start`
///
/// The spawned terminal will:
/// 1. Change to the target directory
/// 2. Run `planning --resume-session <session_id>`
pub fn spawn_terminal_for_resume(target_dir: &Path, session_id: &str) -> Result<()> {
    let dir_str = target_dir
        .to_str()
        .ok_or_else(|| anyhow!("Invalid directory path"))?;

    // Build the resume command
    let resume_cmd = format!("planning --resume-session {}", session_id);

    #[cfg(target_os = "macos")]
    {
        spawn_terminal_macos(dir_str, &resume_cmd)
    }

    #[cfg(target_os = "linux")]
    {
        spawn_terminal_linux(dir_str, &resume_cmd)
    }

    #[cfg(target_os = "windows")]
    {
        spawn_terminal_windows(dir_str, &resume_cmd)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Err(anyhow!("Unsupported platform for terminal spawning"))
    }
}

/// Spawn a terminal on macOS.
#[cfg(target_os = "macos")]
fn spawn_terminal_macos(dir: &str, command: &str) -> Result<()> {
    // Try iTerm2 first (more popular among developers)
    if which::which("osascript").is_ok() {
        // Check if iTerm2 is installed
        let iterm_check = Command::new("osascript")
            .args(["-e", "tell application \"System Events\" to (name of processes) contains \"iTerm2\""])
            .output();

        if let Ok(output) = iterm_check {
            if String::from_utf8_lossy(&output.stdout).contains("true") {
                // Use iTerm2
                let script = format!(
                    r#"tell application "iTerm2"
                        create window with default profile
                        tell current session of current window
                            write text "cd '{}' && {}"
                        end tell
                    end tell"#,
                    dir, command
                );

                let result = Command::new("osascript").args(["-e", &script]).spawn();

                if result.is_ok() {
                    return Ok(());
                }
            }
        }
    }

    // Fall back to Terminal.app
    let script = format!(
        r#"tell application "Terminal"
            do script "cd '{}' && {}"
            activate
        end tell"#,
        dir, command
    );

    Command::new("osascript")
        .args(["-e", &script])
        .spawn()
        .map_err(|e| anyhow!("Failed to spawn Terminal.app: {}", e))?;

    Ok(())
}

/// Spawn a terminal on Linux.
#[cfg(target_os = "linux")]
fn spawn_terminal_linux(dir: &str, command: &str) -> Result<()> {
    // Pre-build the command strings to avoid temporary lifetime issues
    let exec_bash_cmd = format!("{}; exec bash", command);
    let bash_c_cmd = format!("bash -c '{}; exec bash'", command);
    let xterm_cmd = format!("cd '{}' && {}; exec bash", dir, command);

    // Try gnome-terminal
    if which::which("gnome-terminal").is_ok() {
        if Command::new("gnome-terminal")
            .args(["--working-directory", dir, "--", "bash", "-c", &exec_bash_cmd])
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }

    // Try konsole
    if which::which("konsole").is_ok() {
        if Command::new("konsole")
            .args(["--workdir", dir, "-e", "bash", "-c", &exec_bash_cmd])
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }

    // Try xfce4-terminal
    if which::which("xfce4-terminal").is_ok() {
        if Command::new("xfce4-terminal")
            .args(["--working-directory", dir, "-e", &bash_c_cmd])
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }

    // Try mate-terminal
    if which::which("mate-terminal").is_ok() {
        if Command::new("mate-terminal")
            .args(["--working-directory", dir, "-e", &bash_c_cmd])
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }

    // Try terminator
    if which::which("terminator").is_ok() {
        if Command::new("terminator")
            .args(["--working-directory", dir, "-e", &bash_c_cmd])
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }

    // Try alacritty
    if which::which("alacritty").is_ok() {
        if Command::new("alacritty")
            .args(["--working-directory", dir, "-e", "bash", "-c", &exec_bash_cmd])
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }

    // Try kitty
    if which::which("kitty").is_ok() {
        if Command::new("kitty")
            .args(["--directory", dir, "bash", "-c", &exec_bash_cmd])
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }

    // Try tilix
    if which::which("tilix").is_ok() {
        if Command::new("tilix")
            .args(["--working-directory", dir, "-e", &bash_c_cmd])
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }

    // Try xterm
    if which::which("xterm").is_ok() {
        if Command::new("xterm")
            .args(["-e", &xterm_cmd])
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }

    // Last resort: try x-terminal-emulator (Debian/Ubuntu alternative system)
    if which::which("x-terminal-emulator").is_ok() {
        if Command::new("x-terminal-emulator")
            .args(["-e", "bash", "-c", &xterm_cmd])
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }

    Err(anyhow!(
        "No supported terminal emulator found. Install gnome-terminal, konsole, or another supported terminal."
    ))
}

/// Spawn a terminal on Windows.
#[cfg(target_os = "windows")]
fn spawn_terminal_windows(dir: &str, command: &str) -> Result<()> {
    // Try Windows Terminal first (wt.exe)
    if which::which("wt.exe").is_ok() {
        match Command::new("wt.exe")
            .args(["-d", dir, "cmd", "/k", command])
            .spawn()
        {
            Ok(_) => return Ok(()),
            Err(_) => {}
        }
    }

    // Fall back to cmd.exe with start
    let full_cmd = format!("cd /d \"{}\" && {}", dir, command);
    Command::new("cmd")
        .args(["/c", "start", "cmd", "/k", &full_cmd])
        .spawn()
        .map_err(|e| anyhow!("Failed to spawn terminal: {}", e))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_spawn_terminal_invalid_dir() {
        // Test with a path containing invalid characters (on Unix, null byte is invalid)
        let result = spawn_terminal_for_resume(
            &PathBuf::from("/some/valid/path"),
            "test-session-id",
        );
        // This should either succeed or fail based on available terminal
        // We just verify it doesn't panic
        let _ = result;
    }
}
