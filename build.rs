use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_LINES: usize = 1000; // Temporarily increased for CQRS migration
const MAX_RS_FILES_PER_FOLDER: usize = 12;
const MAX_SUBFOLDERS_PER_FOLDER: usize = 12;

const CHECKED_EXTENSIONS: &[&str] = &["rs", "md", "yaml", "toml"];

const EXCLUDED_DIRS: &[&str] = &["target", ".git", "node_modules"];

const EXCLUDED_FILES: &[&str] = &["Cargo.lock", "docs/plans/file-line-limit.md", "build.rs"];

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads/main");
    println!("cargo:rerun-if-changed=.git/packed-refs");

    let sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=PLANNING_AGENT_GIT_SHA={}", sha);

    // Get commit timestamp (Unix epoch seconds) for version comparison
    let timestamp = Command::new("git")
        .args(["show", "-s", "--format=%ct", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout)
                    .ok()
                    .and_then(|s| s.trim().parse::<u64>().ok())
            } else {
                None
            }
        })
        .unwrap_or(0);

    println!(
        "cargo:rustc-env=PLANNING_AGENT_BUILD_TIMESTAMP={}",
        timestamp
    );

    // Capture enabled features for update consistency.
    // This uses CARGO_FEATURE_* env vars set by Cargo during build.
    // MAINTAINER NOTE: When adding new features to Cargo.toml, add them here too.
    let mut features = Vec::new();
    if std::env::var("CARGO_FEATURE_HOST_GUI").is_ok() {
        features.push("host-gui");
    }
    if std::env::var("CARGO_FEATURE_HOST_GUI_TRAY").is_ok() {
        features.push("host-gui-tray");
    }

    // Join with comma (empty string if no features)
    let features_str = features.join(",");
    println!(
        "cargo:rustc-env=PLANNING_AGENT_BUILD_FEATURES={}",
        features_str
    );

    enforce_formatting();
    enforce_line_limits();
    enforce_max_files_per_folder();
    enforce_max_subfolders_per_folder();
    enforce_tests_in_test_paths();
    enforce_no_dead_code_allows();
    enforce_no_ignored_tests();
    enforce_no_test_skips();
    enforce_no_nested_runtimes();
    enforce_serial_for_env_mutations();
    enforce_all_features_compile();
    enforce_no_unsafe_string_ops();
    enforce_tui_zero_guards();
    enforce_decision_functions_in_mod_only();
    enforce_select_for_channel_recv();
    enforce_no_send_unwrap();
    enforce_no_expect_in_workflow();
    enforce_phase_interrupt_consistency();
}

/// Ensures all feature combinations compile.
///
/// This runs `cargo check` for each feature to catch compilation errors
/// that would only appear when building with specific features.
/// Only runs when ENFORCE_ALL_FEATURES=1 to avoid slowing down normal builds.
fn enforce_all_features_compile() {
    // Only run when explicitly requested (e.g., in CI or pre-commit)
    if std::env::var("ENFORCE_ALL_FEATURES").is_err() {
        return;
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");

    // Features to check. host-gui-tray is excluded on Linux (depends on gtk3-rs).
    let features_to_check = if cfg!(target_os = "linux") {
        vec!["host-gui"]
    } else {
        vec!["host-gui", "host-gui-tray"]
    };

    for feature in features_to_check {
        eprintln!("Checking feature: {}", feature);

        let output = Command::new("cargo")
            .args(["check", "--features", feature])
            .current_dir(&manifest_dir)
            .output();

        match output {
            Ok(result) if !result.status.success() => {
                let stderr = String::from_utf8_lossy(&result.stderr);
                eprintln!("\n========================================");
                eprintln!("FEATURE '{}' FAILED TO COMPILE", feature);
                eprintln!("========================================");
                eprintln!("{}", stderr);
                eprintln!("========================================\n");
                panic!(
                    "Build failed: feature '{}' does not compile. Fix the errors above.",
                    feature
                );
            }
            Err(e) => {
                eprintln!("cargo:warning=Failed to check feature '{}': {}", feature, e);
            }
            Ok(_) => {
                eprintln!("Feature '{}' compiled successfully", feature);
            }
        }
    }
}

fn enforce_line_limits() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    let root = PathBuf::from(&manifest_dir);

    let files = collect_files_to_check(&root);

    for file in &files {
        println!("cargo:rerun-if-changed={}", file.display());
    }

    let mut violations = Vec::new();
    for file in &files {
        match count_lines(file) {
            Ok(line_count) if line_count > MAX_LINES => {
                let rel_path = file.strip_prefix(&root).unwrap_or(file);
                violations.push((rel_path.to_path_buf(), line_count));
            }
            Ok(_) => {}
            Err(e) => {
                let rel_path = file.strip_prefix(&root).unwrap_or(file);
                println!(
                    "cargo:warning=Could not read file {}: {}",
                    rel_path.display(),
                    e
                );
            }
        }
    }

    if !violations.is_empty() {
        eprintln!("\n========================================");
        eprintln!("FILE LINE LIMIT EXCEEDED (max {} lines)", MAX_LINES);
        eprintln!("========================================");
        for (path, lines) in &violations {
            eprintln!(
                "  {} - {} lines (exceeds by {})",
                path.display(),
                lines,
                lines - MAX_LINES
            );
        }
        eprintln!("========================================\n");
        eprintln!("Please split these files into smaller modules.");
        eprintln!("See docs/plans/file-line-limit.md for guidance.\n");
        panic!(
            "Build failed: {} file(s) exceed the {} line limit",
            violations.len(),
            MAX_LINES
        );
    }
}

/// Enforces that no folder contains more than MAX_RS_FILES_PER_FOLDER .rs files.
///
/// This forces a better module structure by requiring code to be split into
/// subfolders when a directory gets too large.
fn enforce_max_files_per_folder() {
    // Skip check if SKIP_FOLDER_CHECK is set (useful during migration)
    if std::env::var("SKIP_FOLDER_CHECK").is_ok() {
        return;
    }

    use std::collections::HashMap;

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    let root = PathBuf::from(&manifest_dir);

    // Collect all .rs files
    let rust_files: Vec<PathBuf> = collect_files_to_check(&root)
        .into_iter()
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("rs"))
        .collect();

    // Group by parent directory
    let mut files_per_folder: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    for file in rust_files {
        if let Some(parent) = file.parent() {
            files_per_folder
                .entry(parent.to_path_buf())
                .or_default()
                .push(file);
        }
    }

    // Find violations
    let mut violations: Vec<(PathBuf, usize)> = Vec::new();
    for (folder, files) in &files_per_folder {
        if files.len() > MAX_RS_FILES_PER_FOLDER {
            let rel_path = folder.strip_prefix(&root).unwrap_or(folder).to_path_buf();
            violations.push((rel_path, files.len()));
        }
    }

    if !violations.is_empty() {
        violations.sort_by(|a, b| b.1.cmp(&a.1)); // Sort by count descending

        eprintln!("\n========================================");
        eprintln!(
            "TOO MANY FILES PER FOLDER (max {} .rs files)",
            MAX_RS_FILES_PER_FOLDER
        );
        eprintln!("========================================");
        for (path, count) in &violations {
            eprintln!(
                "  {} - {} files (exceeds by {})",
                path.display(),
                count,
                count - MAX_RS_FILES_PER_FOLDER
            );
        }
        eprintln!("========================================\n");
        eprintln!("Please organize these folders into submodules.");
        eprintln!("For example:");
        eprintln!("  src/foo/bar.rs");
        eprintln!("  src/foo/baz.rs");
        eprintln!("  ... (many files)");
        eprintln!();
        eprintln!("Should become:");
        eprintln!("  src/foo/bar/mod.rs");
        eprintln!("  src/foo/bar/helpers.rs");
        eprintln!("  src/foo/baz/mod.rs");
        eprintln!("  ...");
        eprintln!("\n========================================\n");
        panic!(
            "Build failed: {} folder(s) exceed the {} file limit",
            violations.len(),
            MAX_RS_FILES_PER_FOLDER
        );
    }
}

/// Enforces that no folder contains more than MAX_SUBFOLDERS_PER_FOLDER subfolders.
///
/// This prevents excessive nesting and encourages consolidation of related modules.
fn enforce_max_subfolders_per_folder() {
    // Skip check if SKIP_FOLDER_CHECK is set (useful during migration)
    if std::env::var("SKIP_FOLDER_CHECK").is_ok() {
        return;
    }

    use std::collections::HashMap;

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    let root = PathBuf::from(&manifest_dir);
    let src_dir = root.join("src");

    // Count subfolders per folder (only in src/)
    let mut subfolders_per_folder: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();

    fn count_subfolders(dir: &Path, subfolders_map: &mut HashMap<PathBuf, Vec<PathBuf>>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                // Skip hidden folders, target, tests folders (tests are allowed to have many subfolders)
                if name.starts_with('.') || name == "target" || name.contains("test") {
                    continue;
                }

                subfolders_map
                    .entry(dir.to_path_buf())
                    .or_default()
                    .push(path.clone());

                // Recurse
                count_subfolders(&path, subfolders_map);
            }
        }
    }

    count_subfolders(&src_dir, &mut subfolders_per_folder);

    // Find violations
    let mut violations: Vec<(PathBuf, usize)> = Vec::new();
    for (folder, subfolders) in &subfolders_per_folder {
        if subfolders.len() > MAX_SUBFOLDERS_PER_FOLDER {
            let rel_path = folder.strip_prefix(&root).unwrap_or(folder).to_path_buf();
            violations.push((rel_path, subfolders.len()));
        }
    }

    if !violations.is_empty() {
        violations.sort_by(|a, b| b.1.cmp(&a.1)); // Sort by count descending

        eprintln!("\n========================================");
        eprintln!(
            "TOO MANY SUBFOLDERS PER FOLDER (max {} subfolders)",
            MAX_SUBFOLDERS_PER_FOLDER
        );
        eprintln!("========================================");
        for (path, count) in &violations {
            eprintln!(
                "  {} - {} subfolders (exceeds by {})",
                path.display(),
                count,
                count - MAX_SUBFOLDERS_PER_FOLDER
            );
        }
        eprintln!("========================================\n");
        eprintln!("Please consolidate related modules together.");
        eprintln!("For example, merge small related modules or use feature groupings.");
        eprintln!("\n========================================\n");
        panic!(
            "Build failed: {} folder(s) exceed the {} subfolder limit",
            violations.len(),
            MAX_SUBFOLDERS_PER_FOLDER
        );
    }
}

/// Enforces that all tests are in folders whose name contains "test" AND the file
/// itself contains "test" in its name.
///
/// This ensures a clear separation between production code and test code.
/// Examples:
///   - `src/foo/tests/bar_tests.rs` - OK (parent has "test", file has "test")
///   - `src/foo/tests/mod.rs` - OK (parent has "test", mod.rs is special-cased)
///   - `tests/integration_tests.rs` - OK
///   - `src/foo/tests/bar.rs` - NOT OK (file doesn't have "test")
///   - `src/foo/bar_tests.rs` - NOT OK (parent doesn't have "test")
fn enforce_tests_in_test_paths() {
    // Skip check if SKIP_TEST_FOLDER_CHECK is set (useful during migration)
    if std::env::var("SKIP_TEST_FOLDER_CHECK").is_ok() {
        return;
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    let root = PathBuf::from(&manifest_dir);

    // Collect all .rs files (excluding build.rs)
    let rust_files: Vec<PathBuf> = collect_files_to_check(&root)
        .into_iter()
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("rs")
                && p.file_name().and_then(|n| n.to_str()) != Some("build.rs")
        })
        .collect();

    let mut violations: Vec<(PathBuf, Vec<String>)> = Vec::new();

    for file in &rust_files {
        let rel_path = file.strip_prefix(&root).unwrap_or(file);

        // Check if parent folder name contains "test"
        let parent_has_test = rel_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|name| name.to_lowercase().contains("test"))
            .unwrap_or(false);

        // Check if filename contains "test" (or is mod.rs which is allowed)
        let filename = rel_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let file_has_test = filename.to_lowercase().contains("test") || filename == "mod.rs";

        if parent_has_test && file_has_test {
            // This file is in a test folder AND has test in its name
            continue;
        }

        // Check if file contains test functions
        if let Ok(content) = std::fs::read_to_string(file) {
            let mut test_fns: Vec<String> = Vec::new();

            let lines: Vec<&str> = content.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                let trimmed = line.trim();

                // Look for #[test] or #[tokio::test]
                if trimmed == "#[test]" || trimmed.starts_with("#[tokio::test") {
                    // Look ahead for function name
                    for lookahead_line in lines.iter().skip(i + 1).take(4) {
                        if lookahead_line.contains("fn ") {
                            if let Some(fn_pos) = lookahead_line.find("fn ") {
                                let after_fn = lookahead_line.get(fn_pos + 3..).unwrap_or("");
                                if let Some(paren) = after_fn.find('(') {
                                    let fn_name = after_fn.get(..paren).unwrap_or("").trim();
                                    test_fns.push(fn_name.to_string());
                                }
                            }
                            break;
                        }
                    }
                }
            }

            if !test_fns.is_empty() {
                violations.push((rel_path.to_path_buf(), test_fns));
            }
        }
    }

    if !violations.is_empty() {
        let total_tests: usize = violations.iter().map(|(_, fns)| fns.len()).sum();

        eprintln!("\n========================================");
        eprintln!("TESTS MUST BE IN TEST FOLDERS");
        eprintln!("========================================");
        eprintln!();
        for (path, fns) in &violations {
            eprintln!("  {}", path.display());
            for fn_name in fns {
                eprintln!("    - {}", fn_name);
            }
            eprintln!();
        }
        eprintln!("========================================");
        eprintln!();
        eprintln!("Tests must be in folders with \"test\" in the name AND");
        eprintln!("the file itself must have \"test\" in its name.");
        eprintln!();
        eprintln!("Valid locations:");
        eprintln!("  - src/foo/tests/bar_tests.rs  (folder + file both have 'test')");
        eprintln!("  - src/foo/tests/mod.rs        (mod.rs is allowed in test folders)");
        eprintln!("  - tests/integration_tests.rs  (both have 'test')");
        eprintln!();
        eprintln!("Invalid locations:");
        eprintln!("  - src/foo/tests/bar.rs        (file missing 'test')");
        eprintln!("  - src/foo/bar_tests.rs        (folder missing 'test')");
        eprintln!();
        eprintln!("Move tests to: src/foo/tests/bar_tests.rs");
        eprintln!();
        eprintln!("========================================\n");
        panic!(
            "Build failed: {} test(s) in {} file(s) are not in test folders",
            total_tests,
            violations.len()
        );
    }
}

fn collect_files_to_check(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();

    if let Ok(output) = Command::new("git")
        .args(["ls-files"])
        .current_dir(root)
        .output()
    {
        if output.status.success() {
            if let Ok(stdout) = String::from_utf8(output.stdout) {
                for line in stdout.lines() {
                    let path = root.join(line);
                    // Skip files that don't exist (e.g., deleted but not yet committed)
                    if path.exists() && should_check_file(&path, root) {
                        files.push(path);
                    }
                }
                return files;
            }
        }
    }

    walk_directory(root, root, &mut files);
    files
}

fn walk_directory(dir: &Path, root: &Path, files: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if EXCLUDED_DIRS.contains(&name) {
                    continue;
                }
            }
            walk_directory(&path, root, files);
        } else if should_check_file(&path, root) {
            files.push(path);
        }
    }
}

fn should_check_file(path: &Path, root: &Path) -> bool {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e,
        None => return false,
    };

    if !CHECKED_EXTENSIONS.contains(&ext) {
        return false;
    }

    if let Ok(rel_path) = path.strip_prefix(root) {
        let rel_str = rel_path.to_string_lossy();
        for excluded in EXCLUDED_FILES {
            if rel_str == *excluded {
                return false;
            }
        }

        for component in rel_path.components() {
            if let Some(name) = component.as_os_str().to_str() {
                if EXCLUDED_DIRS.contains(&name) {
                    return false;
                }
            }
        }
    }

    true
}

fn count_lines(path: &Path) -> std::io::Result<usize> {
    let content = std::fs::read_to_string(path)?;
    Ok(count_non_empty_lines(&content))
}

fn count_non_empty_lines(content: &str) -> usize {
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count()
}

fn enforce_formatting() {
    // Skip formatting check if SKIP_FORMAT_CHECK is set (useful for CI or initial setup)
    if std::env::var("SKIP_FORMAT_CHECK").is_ok() {
        return;
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    let root = PathBuf::from(&manifest_dir);

    // Collect all .rs files
    let rust_files: Vec<PathBuf> = collect_files_to_check(&root)
        .into_iter()
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("rs"))
        .collect();

    if rust_files.is_empty() {
        return;
    }

    // Check if rustfmt is available
    let rustfmt_check = Command::new("rustfmt").arg("--version").output();

    if rustfmt_check.is_err() || !rustfmt_check.unwrap().status.success() {
        // rustfmt not available, skip check
        println!("cargo:warning=rustfmt not found, skipping format check");
        return;
    }

    // Run rustfmt --check on all rust files
    let mut cmd = Command::new("rustfmt");
    cmd.arg("--check").arg("--edition").arg("2021");

    for file in &rust_files {
        cmd.arg(file);
    }

    let output = match cmd.output() {
        Ok(o) => o,
        Err(e) => {
            println!("cargo:warning=Failed to run rustfmt: {}", e);
            return;
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Extract unformatted file names from rustfmt output
        let mut unformatted_files: Vec<String> = Vec::new();
        for line in stdout.lines().chain(stderr.lines()) {
            if line.starts_with("Diff in ") {
                if let Some(path) = line.strip_prefix("Diff in ") {
                    let path = path.trim_end_matches(':');
                    if let Ok(rel) = Path::new(path).strip_prefix(&root) {
                        unformatted_files.push(rel.to_string_lossy().to_string());
                    } else {
                        unformatted_files.push(path.to_string());
                    }
                }
            }
        }

        eprintln!("\n========================================");
        eprintln!("CODE FORMATTING CHECK FAILED");
        eprintln!("========================================");
        if !unformatted_files.is_empty() {
            eprintln!("The following files are not formatted:");
            for file in &unformatted_files {
                eprintln!("  - {}", file);
            }
        } else {
            eprintln!("Some files are not properly formatted.");
        }
        eprintln!("========================================");
        eprintln!("\nTo fix, run:\n");
        eprintln!("    cargo fmt");
        eprintln!("\n========================================\n");
        panic!("Build failed: code is not formatted. Run 'cargo fmt' to fix.");
    }
}

fn enforce_no_dead_code_allows() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    let root = PathBuf::from(&manifest_dir);

    // Collect all .rs files (excluding build.rs itself)
    let rust_files: Vec<PathBuf> = collect_files_to_check(&root)
        .into_iter()
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("rs")
                && p.file_name().and_then(|n| n.to_str()) != Some("build.rs")
        })
        .collect();

    let mut violations: Vec<(PathBuf, Vec<(usize, String)>)> = Vec::new();

    for file in &rust_files {
        if let Ok(content) = std::fs::read_to_string(file) {
            let mut file_violations = Vec::new();
            for (line_num, line) in content.lines().enumerate() {
                let trimmed = line.trim();
                // Check for #[allow(dead_code)] or #![allow(dead_code)]
                if (trimmed.starts_with("#[allow(") || trimmed.starts_with("#![allow("))
                    && trimmed.contains("dead_code")
                {
                    file_violations.push((line_num + 1, line.to_string()));
                }
            }
            if !file_violations.is_empty() {
                let rel_path = file.strip_prefix(&root).unwrap_or(file).to_path_buf();
                violations.push((rel_path, file_violations));
            }
        }
    }

    if !violations.is_empty() {
        let total_count: usize = violations.iter().map(|(_, v)| v.len()).sum();

        eprintln!("\n========================================");
        eprintln!("#[allow(dead_code)] IS NOT ALLOWED");
        eprintln!("========================================");
        eprintln!();
        for (path, lines) in &violations {
            for (line_num, line_content) in lines {
                eprintln!("  {}:{}", path.display(), line_num);
                eprintln!("    {}", line_content.trim());
                eprintln!();
            }
        }
        eprintln!("========================================");
        eprintln!();
        eprintln!("Do NOT use #[allow(dead_code)] to silence warnings.");
        eprintln!();
        eprintln!("Instead:");
        eprintln!("  - DELETE unused code entirely");
        eprintln!("  - If the code is for tests, use #[cfg(test)]");
        eprintln!("  - If the code is a public API, make it actually public");
        eprintln!();
        eprintln!("Keeping dead code around \"just in case\" creates");
        eprintln!("maintenance burden and hides real issues.");
        eprintln!();
        eprintln!("========================================\n");
        panic!(
            "Build failed: {} #[allow(dead_code)] occurrence(s) found. Remove the dead code.",
            total_count
        );
    }
}

/// Bans #[ignore] attribute on tests.
///
/// Ignored tests provide false confidence - they show up as "passing" in test counts
/// but don't actually verify anything. Tests should either run or be deleted.
fn enforce_no_ignored_tests() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    let root = PathBuf::from(&manifest_dir);

    let rust_files: Vec<PathBuf> = collect_files_to_check(&root)
        .into_iter()
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("rs")
                && p.file_name().and_then(|n| n.to_str()) != Some("build.rs")
        })
        .collect();

    let mut violations: Vec<(PathBuf, Vec<(usize, String)>)> = Vec::new();

    for file in &rust_files {
        if let Ok(content) = std::fs::read_to_string(file) {
            let mut file_violations = Vec::new();

            for (line_num, line) in content.lines().enumerate() {
                let trimmed = line.trim();
                if trimmed == "#[ignore]" || trimmed.starts_with("#[ignore(") {
                    file_violations.push((line_num + 1, line.to_string()));
                }
            }

            if !file_violations.is_empty() {
                let rel_path = file.strip_prefix(&root).unwrap_or(file).to_path_buf();
                violations.push((rel_path, file_violations));
            }
        }
    }

    if !violations.is_empty() {
        let total_count: usize = violations.iter().map(|(_, v)| v.len()).sum();

        eprintln!("\n========================================");
        eprintln!("#[ignore] TESTS ARE NOT ALLOWED");
        eprintln!("========================================");
        eprintln!();
        for (path, lines) in &violations {
            for (line_num, line_content) in lines {
                eprintln!("  {}:{}", path.display(), line_num);
                eprintln!("    {}", line_content.trim());
                eprintln!();
            }
        }
        eprintln!("========================================");
        eprintln!();
        eprintln!("Ignored tests provide false confidence - they appear");
        eprintln!("to pass but don't actually verify anything.");
        eprintln!();
        eprintln!("Instead:");
        eprintln!("  - Fix the test so it can run reliably");
        eprintln!("  - Delete the test if it can't be fixed");
        eprintln!("  - If it requires external tools, the unit tests");
        eprintln!("    for parsing logic are sufficient");
        eprintln!();
        eprintln!("========================================\n");
        panic!(
            "Build failed: {} #[ignore] test(s) found. Fix or delete them.",
            total_count
        );
    }
}

/// Bans tests that silently skip instead of failing.
///
/// Tests that return early without doing work hide failures and give false confidence.
/// If a test can't run, it should FAIL, not silently pass.
fn enforce_no_test_skips() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    let root = PathBuf::from(&manifest_dir);

    let rust_files: Vec<PathBuf> = collect_files_to_check(&root)
        .into_iter()
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("rs")
                && p.file_name().and_then(|n| n.to_str()) != Some("build.rs")
        })
        .collect();

    // Patterns that indicate a test is silently skipping.
    // These are checked only within test function bodies.
    let skip_patterns = [
        "Skipping test",
        "skipping test",
        "Test skipped",
        "test skipped",
        "daemon not available",
        "not connected, skipping",
    ];

    let mut violations: Vec<(PathBuf, Vec<(usize, String)>)> = Vec::new();

    for file in &rust_files {
        if let Ok(content) = std::fs::read_to_string(file) {
            let mut file_violations = Vec::new();
            let lines: Vec<&str> = content.lines().collect();

            // Track test function context
            let mut in_test_fn = false;
            let mut test_fn_start = 0;
            let mut test_fn_name = String::new();
            let mut brace_depth = 0;

            for (i, line) in lines.iter().enumerate() {
                let trimmed = line.trim();

                // Check for #[test] or #[tokio::test]
                if trimmed == "#[test]" || trimmed.starts_with("#[tokio::test") {
                    // Look ahead for function name
                    for (offset, lookahead_line) in lines.iter().skip(i + 1).take(4).enumerate() {
                        if lookahead_line.contains("fn ") {
                            test_fn_start = i + 1 + offset;
                            if let Some(fn_pos) = lookahead_line.find("fn ") {
                                let after_fn = lookahead_line.get(fn_pos + 3..).unwrap_or("");
                                if let Some(paren) = after_fn.find('(') {
                                    test_fn_name =
                                        after_fn.get(..paren).unwrap_or("").trim().to_string();
                                }
                            }
                            in_test_fn = true;
                            brace_depth = 0;
                            break;
                        }
                    }
                }

                // Track brace depth
                if in_test_fn {
                    for c in line.chars() {
                        if c == '{' {
                            brace_depth += 1;
                        } else if c == '}' {
                            brace_depth -= 1;
                            if brace_depth == 0 {
                                in_test_fn = false;
                            }
                        }
                    }

                    // Check for skip patterns (anywhere in test)
                    for pattern in &skip_patterns {
                        if line.contains(pattern) {
                            file_violations.push((
                                test_fn_start,
                                format!(
                                    "test `{}` contains skip pattern: {}",
                                    test_fn_name, pattern
                                ),
                            ));
                            in_test_fn = false; // Only report once per test
                            break;
                        }
                    }

                    // Check for bare "return;" which indicates early exit (silent skip)
                    // This catches patterns like: if some_condition { return; }
                    if in_test_fn && trimmed == "return;" && brace_depth > 1 {
                        // brace_depth > 1 means we're inside a nested block (like an if)
                        // This is the telltale sign of a conditional early return
                        file_violations.push((
                            test_fn_start,
                            format!(
                                "test `{}` has conditional early return (silent skip)",
                                test_fn_name
                            ),
                        ));
                        in_test_fn = false; // Only report once per test
                    }
                }
            }

            if !file_violations.is_empty() {
                let rel_path = file.strip_prefix(&root).unwrap_or(file).to_path_buf();
                violations.push((rel_path, file_violations));
            }
        }
    }

    if !violations.is_empty() {
        let total_count: usize = violations.iter().map(|(_, v)| v.len()).sum();

        eprintln!("\n========================================");
        eprintln!("SILENT TEST SKIPS ARE NOT ALLOWED");
        eprintln!("========================================");
        eprintln!();
        for (path, lines) in &violations {
            for (line_num, line_content) in lines {
                eprintln!("  {}:{}", path.display(), line_num);
                eprintln!("    {}", line_content.trim());
                eprintln!();
            }
        }
        eprintln!("========================================");
        eprintln!();
        eprintln!("Tests must FAIL if they cannot run, not silently pass.");
        eprintln!();
        eprintln!("Instead of skipping:");
        eprintln!("  - Spin up real test infrastructure (use TestServer)");
        eprintln!("  - Use assert!() to verify preconditions");
        eprintln!("  - If truly optional, use #[ignore] with a reason");
        eprintln!();
        eprintln!("========================================\n");
        panic!(
            "Build failed: {} silent test skip(s) found. Make tests fail instead of skip.",
            total_count
        );
    }
}

/// Bans spawning threads that create their own tokio runtime.
///
/// This pattern causes subtle bugs: tarpc clients created in a nested runtime
/// get dropped when that runtime is dropped, causing "connection already shutdown" errors.
fn enforce_no_nested_runtimes() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    let root = PathBuf::from(&manifest_dir);

    let rust_files: Vec<PathBuf> = collect_files_to_check(&root)
        .into_iter()
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("rs")
                && p.file_name().and_then(|n| n.to_str()) != Some("build.rs")
        })
        .collect();

    let mut violations: Vec<(PathBuf, Vec<(usize, String)>)> = Vec::new();

    for file in &rust_files {
        if let Ok(content) = std::fs::read_to_string(file) {
            let lines: Vec<&str> = content.lines().collect();
            let mut file_violations = Vec::new();

            // Look for std::thread::spawn or thread::spawn
            for (i, line) in lines.iter().enumerate() {
                let trimmed = line.trim();
                if (trimmed.contains("std::thread::spawn") || trimmed.contains("thread::spawn("))
                    && !trimmed.starts_with("//")
                {
                    // Check surrounding context (next 20 lines) for runtime creation
                    let end = (i + 20).min(lines.len());
                    let context = lines[i..end].join("\n");

                    if context.contains("Runtime::new()")
                        || context.contains("runtime::Builder")
                        || context.contains("tokio::runtime::Builder")
                    {
                        file_violations.push((i + 1, line.to_string()));
                    }
                }
            }

            if !file_violations.is_empty() {
                let rel_path = file.strip_prefix(&root).unwrap_or(file).to_path_buf();
                violations.push((rel_path, file_violations));
            }
        }
    }

    if !violations.is_empty() {
        let total_count: usize = violations.iter().map(|(_, v)| v.len()).sum();

        eprintln!("\n========================================");
        eprintln!("NESTED TOKIO RUNTIMES ARE NOT ALLOWED");
        eprintln!("========================================");
        eprintln!();
        for (path, lines) in &violations {
            for (line_num, line_content) in lines {
                eprintln!("  {}:{}", path.display(), line_num);
                eprintln!("    {}", line_content.trim());
                eprintln!();
            }
        }
        eprintln!("========================================");
        eprintln!();
        eprintln!("Spawning a thread that creates its own tokio runtime");
        eprintln!("causes async clients (tarpc, etc) to break when that");
        eprintln!("thread exits and the runtime is dropped.");
        eprintln!();
        eprintln!("Instead:");
        eprintln!("  - Make the function async and call it from the main runtime");
        eprintln!("  - Use tokio::spawn() for concurrent async work");
        eprintln!("  - Pass async clients from the parent runtime");
        eprintln!();
        eprintln!("========================================\n");
        panic!(
            "Build failed: {} nested runtime(s) found. Use async functions instead.",
            total_count
        );
    }
}

/// Requires #[serial] for tests that mutate environment variables.
///
/// Environment variables are global state. Tests that modify them without
/// #[serial] cause flaky failures when running in parallel.
fn enforce_serial_for_env_mutations() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    let root = PathBuf::from(&manifest_dir);

    let rust_files: Vec<PathBuf> = collect_files_to_check(&root)
        .into_iter()
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("rs")
                && p.file_name().and_then(|n| n.to_str()) != Some("build.rs")
        })
        .collect();

    let mut violations: Vec<(PathBuf, Vec<(usize, String)>)> = Vec::new();

    for file in &rust_files {
        if let Ok(content) = std::fs::read_to_string(file) {
            let lines: Vec<&str> = content.lines().collect();
            let mut file_violations = Vec::new();

            // Find test functions that use set_var or remove_var
            let mut in_test_fn = false;
            let mut test_fn_start = 0;
            let mut test_fn_name = String::new();
            let mut has_serial = false;
            let mut brace_depth = 0;

            for (i, line) in lines.iter().enumerate() {
                let trimmed = line.trim();

                // Check for #[serial] or #[serial_test::serial] attribute
                if trimmed == "#[serial]" || trimmed == "#[serial_test::serial]" {
                    has_serial = true;
                }

                // Check for #[test] or #[tokio::test]
                if trimmed == "#[test]" || trimmed.starts_with("#[tokio::test") {
                    // Look ahead for function name
                    for (offset, lookahead_line) in lines.iter().skip(i + 1).take(4).enumerate() {
                        if lookahead_line.contains("fn ") {
                            test_fn_start = i + 1 + offset;
                            // Extract function name
                            if let Some(fn_pos) = lookahead_line.find("fn ") {
                                let after_fn = lookahead_line.get(fn_pos + 3..).unwrap_or("");
                                if let Some(paren) = after_fn.find('(') {
                                    test_fn_name =
                                        after_fn.get(..paren).unwrap_or("").trim().to_string();
                                }
                            }
                            in_test_fn = true;
                            brace_depth = 0;
                            break;
                        }
                    }
                }

                // Track brace depth to know when test function ends
                if in_test_fn {
                    for c in line.chars() {
                        if c == '{' {
                            brace_depth += 1;
                        } else if c == '}' {
                            brace_depth -= 1;
                            if brace_depth == 0 {
                                in_test_fn = false;
                                has_serial = false;
                            }
                        }
                    }

                    // Check for env mutations
                    if !has_serial
                        && (trimmed.contains("std::env::set_var")
                            || trimmed.contains("std::env::remove_var")
                            || (trimmed.contains("env::set_var") && !trimmed.starts_with("//"))
                            || (trimmed.contains("env::remove_var") && !trimmed.starts_with("//")))
                    {
                        file_violations.push((
                            test_fn_start,
                            format!("test `{}` mutates env without #[serial]", test_fn_name),
                        ));
                        // Only report once per test
                        in_test_fn = false;
                        has_serial = false;
                    }
                }
            }

            if !file_violations.is_empty() {
                let rel_path = file.strip_prefix(&root).unwrap_or(file).to_path_buf();
                violations.push((rel_path, file_violations));
            }
        }
    }

    if !violations.is_empty() {
        let total_count: usize = violations.iter().map(|(_, v)| v.len()).sum();

        eprintln!("\n========================================");
        eprintln!("ENV MUTATIONS REQUIRE #[serial]");
        eprintln!("========================================");
        eprintln!();
        for (path, issues) in &violations {
            for (line_num, msg) in issues {
                eprintln!("  {}:{}", path.display(), line_num);
                eprintln!("    {}", msg);
                eprintln!();
            }
        }
        eprintln!("========================================");
        eprintln!();
        eprintln!("Tests that call std::env::set_var or std::env::remove_var");
        eprintln!("modify global state and cause flaky failures in parallel.");
        eprintln!();
        eprintln!("Add #[serial] attribute from serial_test crate:");
        eprintln!();
        eprintln!("    use serial_test::serial;");
        eprintln!();
        eprintln!("    #[test]");
        eprintln!("    #[serial]");
        eprintln!("    fn test_with_env_var() {{ ... }}");
        eprintln!();
        eprintln!("========================================\n");
        panic!(
            "Build failed: {} test(s) mutate env vars without #[serial].",
            total_count
        );
    }
}

/// Bans unsafe string operations that don't account for UTF-8 character boundaries.
///
/// Even with clippy::string_slice denied, code can still do unsafe operations like:
/// - `s.get(..n)` or `s.get(start..end)` - byte ranges may split UTF-8 chars
/// - Using `.len()` to determine truncation point instead of `.chars().count()`
///
/// This check catches patterns that look like they're treating strings as byte arrays.
///
/// For cursor-based operations (where byte positions are maintained at UTF-8 boundaries),
/// use the helpers in `tui/cursor_utils.rs`:
/// - `slice_up_to_cursor(s, cursor)` - equivalent to s.get(..cursor)
/// - `slice_from_cursor(s, cursor)` - equivalent to s.get(cursor..)
/// - `slice_between_cursors(s, start, end)` - equivalent to s.get(start..end)
fn enforce_no_unsafe_string_ops() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    let root = PathBuf::from(&manifest_dir);

    // Only check TUI-related files where string truncation is common
    let tui_files: Vec<PathBuf> = collect_files_to_check(&root)
        .into_iter()
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("rs")
                && p.file_name().and_then(|n| n.to_str()) != Some("build.rs")
                && p.to_string_lossy().contains("tui")
                // Skip the cursor_utils.rs file itself - it defines the safe wrappers
                && p.file_name().and_then(|n| n.to_str()) != Some("cursor_utils.rs")
        })
        .collect();

    let mut violations: Vec<(PathBuf, Vec<(usize, String)>)> = Vec::new();

    for file in &tui_files {
        if let Ok(content) = std::fs::read_to_string(file) {
            let mut file_violations = Vec::new();

            for (line_num, line) in content.lines().enumerate() {
                let trimmed = line.trim();

                // Skip comments
                if trimmed.starts_with("//") {
                    continue;
                }

                // Check for .get(..n) pattern on strings (byte-based slicing)
                // This catches: name.get(..12), s.get(0..n), etc.
                if trimmed.contains(".get(..") || trimmed.contains(".get(0..") {
                    // Allow if it's clearly on bytes or arrays
                    if !trimmed.contains("bytes()")
                        && !trimmed.contains("as_bytes()")
                        && !trimmed.contains("[u8]")
                    {
                        file_violations.push((
                            line_num + 1,
                            format!(
                                "unsafe byte-range on string: `{}`",
                                trimmed.chars().take(60).collect::<String>()
                            ),
                        ));
                    }
                }

                // Check for truncation using .len() comparison followed by byte slicing
                // This is a common anti-pattern: if s.len() > 15 { &s[..12] }
                if trimmed.contains(".len() >") && trimmed.contains("..") {
                    // Only flag if not dealing with bytes
                    if !trimmed.contains("bytes") {
                        file_violations.push((
                            line_num + 1,
                            format!(
                                "truncation using .len() may break UTF-8: `{}`",
                                trimmed.chars().take(60).collect::<String>()
                            ),
                        ));
                    }
                }
            }

            if !file_violations.is_empty() {
                let rel_path = file.strip_prefix(&root).unwrap_or(file).to_path_buf();
                violations.push((rel_path, file_violations));
            }
        }
    }

    if !violations.is_empty() {
        let total_count: usize = violations.iter().map(|(_, v)| v.len()).sum();

        eprintln!("\n========================================");
        eprintln!("UNSAFE STRING OPERATIONS DETECTED");
        eprintln!("========================================");
        eprintln!();
        for (path, issues) in &violations {
            for (line_num, msg) in issues {
                eprintln!("  {}:{}", path.display(), line_num);
                eprintln!("    {}", msg);
                eprintln!();
            }
        }
        eprintln!("========================================");
        eprintln!();
        eprintln!("String byte operations can break on multi-byte UTF-8 characters.");
        eprintln!("Characters like emoji or non-ASCII take multiple bytes.");
        eprintln!();
        eprintln!("FOR DISPLAY TRUNCATION use character-based operations:");
        eprintln!();
        eprintln!("    // WRONG");
        eprintln!("    if name.len() > 15 {{ name.get(..12) }}");
        eprintln!();
        eprintln!("    // CORRECT");
        eprintln!("    if name.chars().count() > 15 {{");
        eprintln!("        name.chars().take(12).collect::<String>()");
        eprintln!("    }}");
        eprintln!();
        eprintln!("FOR CURSOR-BASED SLICING use tui/cursor_utils.rs helpers:");
        eprintln!();
        eprintln!("    use crate::tui::cursor_utils::{{slice_up_to_cursor, slice_from_cursor}};");
        eprintln!();
        eprintln!("    // Instead of: text.get(..cursor)");
        eprintln!("    let before = slice_up_to_cursor(text, cursor);");
        eprintln!();
        eprintln!("    // Instead of: text.get(cursor..)");
        eprintln!("    let after = slice_from_cursor(text, cursor);");
        eprintln!();
        eprintln!("    // Instead of: text.get(start..end)");
        eprintln!("    let middle = slice_between_cursors(text, start, end);");
        eprintln!();
        eprintln!("========================================\n");
        panic!(
            "Build failed: {} unsafe string operation(s) in TUI code.",
            total_count
        );
    }
}

/// Ensures TUI code has zero-guards before division by width/height.
///
/// Division by zero can occur when terminal dimensions are very small or zero.
/// TUI code must check that width/height > 0 before dividing.
fn enforce_tui_zero_guards() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    let root = PathBuf::from(&manifest_dir);

    // Only check TUI-related files
    let tui_files: Vec<PathBuf> = collect_files_to_check(&root)
        .into_iter()
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("rs")
                && p.file_name().and_then(|n| n.to_str()) != Some("build.rs")
                && p.to_string_lossy().contains("tui")
        })
        .collect();

    let mut violations: Vec<(PathBuf, Vec<(usize, String)>)> = Vec::new();

    for file in &tui_files {
        if let Ok(content) = std::fs::read_to_string(file) {
            let lines: Vec<&str> = content.lines().collect();
            let mut file_violations = Vec::new();

            for (line_num, line) in lines.iter().enumerate() {
                let trimmed = line.trim();

                // Skip comments
                if trimmed.starts_with("//") {
                    continue;
                }

                // Check for division by width or height without nearby zero check
                // Patterns: / width, / height, / area.width, / inner.width, etc.
                let div_patterns = [
                    "/ width",
                    "/ height",
                    "/width",
                    "/height",
                    "/ area.width",
                    "/ area.height",
                    "/ inner.width",
                    "/ inner.height",
                    "/ chunk_width",
                    "/ chunk_height",
                    "/ col_width",
                    "/ row_height",
                ];

                for pattern in &div_patterns {
                    if trimmed.contains(pattern) {
                        // Check if there's a zero guard in the surrounding context (5 lines before)
                        let start = line_num.saturating_sub(5);
                        let context: String = lines[start..=line_num].join("\n");

                        let has_zero_guard = context.contains("== 0")
                            || context.contains("== 0 {")
                            || context.contains("< 1")
                            || context.contains(".is_empty()")
                            || context.contains("width == 0")
                            || context.contains("height == 0")
                            || context.contains("width < 1")
                            || context.contains("height < 1")
                            || context.contains("max(1")
                            || context.contains(".max(1)");

                        if !has_zero_guard {
                            file_violations.push((
                                line_num + 1,
                                format!(
                                    "division by dimension without zero guard: `{}`",
                                    trimmed.chars().take(50).collect::<String>()
                                ),
                            ));
                        }
                        break;
                    }
                }
            }

            if !file_violations.is_empty() {
                let rel_path = file.strip_prefix(&root).unwrap_or(file).to_path_buf();
                violations.push((rel_path, file_violations));
            }
        }
    }

    if !violations.is_empty() {
        let total_count: usize = violations.iter().map(|(_, v)| v.len()).sum();

        eprintln!("\n========================================");
        eprintln!("TUI DIVISION WITHOUT ZERO GUARD");
        eprintln!("========================================");
        eprintln!();
        for (path, issues) in &violations {
            for (line_num, msg) in issues {
                eprintln!("  {}:{}", path.display(), line_num);
                eprintln!("    {}", msg);
                eprintln!();
            }
        }
        eprintln!("========================================");
        eprintln!();
        eprintln!("Division by width/height can panic when terminal is very small.");
        eprintln!("Always guard against zero before division.");
        eprintln!();
        eprintln!("WRONG:");
        eprintln!("    let cols = total / width;");
        eprintln!();
        eprintln!("CORRECT:");
        eprintln!("    if width == 0 {{ return; }}");
        eprintln!("    let cols = total / width;");
        eprintln!();
        eprintln!("Or use max:");
        eprintln!("    let cols = total / width.max(1);");
        eprintln!();
        eprintln!("========================================\n");
        panic!(
            "Build failed: {} unguarded division(s) in TUI code. Add zero checks.",
            total_count
        );
    }
}

/// Ensures user decision functions are only called from the main workflow loop.
///
/// Functions like `await_max_iterations_decision` should only be called from
/// `src/app/workflow/mod.rs`. If phase handlers (planning.rs, reviewing.rs, etc.)
/// call these directly, it creates duplicate code paths for handling the same decision.
///
/// The correct pattern is:
/// 1. Phase handler dispatches a command (e.g., PlanningMaxIterationsReached)
/// 2. Phase handler returns
/// 3. Main workflow loop observes the phase change and handles the decision
fn enforce_decision_functions_in_mod_only() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    let root = PathBuf::from(&manifest_dir);
    let workflow_dir = root.join("src/app/workflow");

    // Decision functions that should only be called from mod.rs.
    // await_max_iterations_decision is the key one - it handles the AwaitingPlanningDecision
    // phase which must have a single handler to avoid duplicate modal displays.
    // Other decision functions (wait_for_review_decision, etc.) are currently allowed
    // in phase handlers as they don't have corresponding phase handlers in mod.rs.
    let decision_functions = [
        "await_max_iterations_decision",
        // Note: wait_for_review_decision, wait_for_all_reviewers_failed_decision,
        // and wait_for_workflow_failure_decision are allowed in phase handlers
        // because they handle decisions within the phase, not phase transitions.
    ];

    // Collect workflow .rs files (excluding mod.rs)
    let workflow_files: Vec<PathBuf> = collect_files_to_check(&root)
        .into_iter()
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("rs")
                && p.starts_with(&workflow_dir)
                && p.file_name().and_then(|n| n.to_str()) != Some("mod.rs")
        })
        .collect();

    let mut violations: Vec<(PathBuf, Vec<(usize, String)>)> = Vec::new();

    for file in &workflow_files {
        if let Ok(content) = std::fs::read_to_string(file) {
            let mut file_violations = Vec::new();

            for (line_num, line) in content.lines().enumerate() {
                let trimmed = line.trim();

                // Skip comments
                if trimmed.starts_with("//") {
                    continue;
                }

                // Check for decision function calls
                for func in &decision_functions {
                    if trimmed.contains(func) {
                        file_violations.push((
                            line_num + 1,
                            format!("`{}` should only be called from mod.rs", func),
                        ));
                        break;
                    }
                }
            }

            if !file_violations.is_empty() {
                let rel_path = file.strip_prefix(&root).unwrap_or(file).to_path_buf();
                violations.push((rel_path, file_violations));
            }
        }
    }

    if !violations.is_empty() {
        let total_count: usize = violations.iter().map(|(_, v)| v.len()).sum();

        eprintln!("\n========================================");
        eprintln!("DECISION FUNCTIONS CALLED OUTSIDE mod.rs");
        eprintln!("========================================");
        eprintln!();
        for (path, issues) in &violations {
            for (line_num, msg) in issues {
                eprintln!("  {}:{}", path.display(), line_num);
                eprintln!("    {}", msg);
                eprintln!();
            }
        }
        eprintln!("========================================");
        eprintln!();
        eprintln!("User-facing decision functions must only be called from");
        eprintln!("src/app/workflow/mod.rs to prevent duplicate handling.");
        eprintln!();
        eprintln!("The correct pattern is:");
        eprintln!("  1. Phase handler dispatches a state-change command");
        eprintln!("  2. Phase handler returns Ok(None)");
        eprintln!("  3. Main loop (mod.rs) handles the new phase");
        eprintln!();
        eprintln!("Example (in reviewing.rs):");
        eprintln!("  // WRONG - handling decision inline");
        eprintln!("  context.dispatch_command(PlanningMaxIterationsReached).await;");
        eprintln!("  let decision = await_max_iterations_decision(...).await;");
        eprintln!();
        eprintln!("  // CORRECT - dispatch and return");
        eprintln!("  context.dispatch_command(PlanningMaxIterationsReached).await;");
        eprintln!("  return Ok(None); // mod.rs handles AwaitingPlanningDecision");
        eprintln!();
        eprintln!("========================================\n");
        panic!(
            "Build failed: {} decision function call(s) outside mod.rs. Move to main loop.",
            total_count
        );
    }
}

/// Ensures user-facing channel .recv().await calls in workflow code use tokio::select!.
///
/// Single-channel awaits on approval_rx/control_rx can cause freeze bugs when the
/// user quits, because the control channel (for Stop commands) isn't being checked.
/// Internal channels like event_rx are exempt as they have different semantics.
fn enforce_select_for_channel_recv() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    let root = PathBuf::from(&manifest_dir);
    let workflow_dir = root.join("src/app/workflow");

    // User-facing channels that must use select! for proper quit handling
    let user_facing_channels = ["approval_rx", "control_rx"];

    // Only check workflow files where channel receives are critical
    let workflow_files: Vec<PathBuf> = collect_files_to_check(&root)
        .into_iter()
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("rs") && p.starts_with(&workflow_dir)
        })
        .collect();

    let mut violations: Vec<(PathBuf, Vec<(usize, String)>)> = Vec::new();

    for file in &workflow_files {
        if let Ok(content) = std::fs::read_to_string(file) {
            let lines: Vec<&str> = content.lines().collect();
            let mut file_violations = Vec::new();

            for (line_num, line) in lines.iter().enumerate() {
                let trimmed = line.trim();

                // Skip comments
                if trimmed.starts_with("//") {
                    continue;
                }

                // Check for user-facing channel .recv().await pattern
                let is_user_channel = user_facing_channels
                    .iter()
                    .any(|ch| trimmed.contains(&format!("{}.recv()", ch)));

                if is_user_channel && trimmed.contains(".recv().await") {
                    // Check if we're inside a select! block (look backwards for select!)
                    let start = line_num.saturating_sub(10);
                    let context: String = lines[start..=line_num].join("\n");

                    if !context.contains("select!") && !context.contains("tokio::select!") {
                        file_violations.push((
                            line_num + 1,
                            format!(
                                "user channel .recv().await without select!: `{}`",
                                trimmed.chars().take(60).collect::<String>()
                            ),
                        ));
                    }
                }
            }

            if !file_violations.is_empty() {
                let rel_path = file.strip_prefix(&root).unwrap_or(file).to_path_buf();
                violations.push((rel_path, file_violations));
            }
        }
    }

    if !violations.is_empty() {
        let total_count: usize = violations.iter().map(|(_, v)| v.len()).sum();

        eprintln!("\n========================================");
        eprintln!("CHANNEL RECV WITHOUT SELECT! IN WORKFLOW");
        eprintln!("========================================");
        eprintln!();
        for (path, issues) in &violations {
            for (line_num, msg) in issues {
                eprintln!("  {}:{}", path.display(), line_num);
                eprintln!("    {}", msg);
                eprintln!();
            }
        }
        eprintln!("========================================");
        eprintln!();
        eprintln!("Channel .recv().await in workflow code must use tokio::select!");
        eprintln!("to check both approval_rx AND control_rx. Single-channel awaits");
        eprintln!("cause freeze bugs when the user quits.");
        eprintln!();
        eprintln!("WRONG:");
        eprintln!("    let response = approval_rx.recv().await;");
        eprintln!();
        eprintln!("CORRECT:");
        eprintln!("    tokio::select! {{");
        eprintln!("        Some(cmd) = control_rx.recv() => {{ /* handle Stop */ }}");
        eprintln!("        response = approval_rx.recv() => {{ /* handle response */ }}");
        eprintln!("    }}");
        eprintln!();
        eprintln!("========================================\n");
        panic!(
            "Build failed: {} channel recv(s) without select! in workflow code.",
            total_count
        );
    }
}

/// Ensures .send().unwrap() is not used on channels.
///
/// Channel sends can fail when the receiver is dropped (e.g., during shutdown).
/// Using unwrap() causes a panic instead of graceful handling.
fn enforce_no_send_unwrap() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    let root = PathBuf::from(&manifest_dir);

    // Check all Rust files except tests
    let rust_files: Vec<PathBuf> = collect_files_to_check(&root)
        .into_iter()
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("rs")
                && p.file_name().and_then(|n| n.to_str()) != Some("build.rs")
                && !p.to_string_lossy().contains("/tests/")
                && !p.to_string_lossy().contains("_tests.rs")
        })
        .collect();

    let mut violations: Vec<(PathBuf, Vec<(usize, String)>)> = Vec::new();

    for file in &rust_files {
        if let Ok(content) = std::fs::read_to_string(file) {
            let mut file_violations = Vec::new();

            for (line_num, line) in content.lines().enumerate() {
                let trimmed = line.trim();

                // Skip comments
                if trimmed.starts_with("//") {
                    continue;
                }

                // Check for .send(...).unwrap() or .send(...).await.unwrap() patterns
                if (trimmed.contains(".send(") && trimmed.contains(").unwrap()"))
                    || (trimmed.contains(".send(") && trimmed.contains(").await.unwrap()"))
                {
                    file_violations.push((
                        line_num + 1,
                        format!(
                            "channel send with unwrap: `{}`",
                            trimmed.chars().take(60).collect::<String>()
                        ),
                    ));
                }
            }

            if !file_violations.is_empty() {
                let rel_path = file.strip_prefix(&root).unwrap_or(file).to_path_buf();
                violations.push((rel_path, file_violations));
            }
        }
    }

    if !violations.is_empty() {
        let total_count: usize = violations.iter().map(|(_, v)| v.len()).sum();

        eprintln!("\n========================================");
        eprintln!("CHANNEL SEND WITH UNWRAP");
        eprintln!("========================================");
        eprintln!();
        for (path, issues) in &violations {
            for (line_num, msg) in issues {
                eprintln!("  {}:{}", path.display(), line_num);
                eprintln!("    {}", msg);
                eprintln!();
            }
        }
        eprintln!("========================================");
        eprintln!();
        eprintln!("Channel .send().unwrap() can panic when the receiver is dropped");
        eprintln!("(e.g., during shutdown). Use explicit error handling instead.");
        eprintln!();
        eprintln!("WRONG:");
        eprintln!("    tx.send(message).unwrap();");
        eprintln!("    tx.send(message).await.unwrap();");
        eprintln!();
        eprintln!("CORRECT:");
        eprintln!("    let _ = tx.send(message);  // Receiver may be dropped");
        eprintln!("    if tx.send(message).await.is_err() {{ /* handle */ }}");
        eprintln!();
        eprintln!("========================================\n");
        panic!(
            "Build failed: {} channel send(s) with unwrap(). Handle errors gracefully.",
            total_count
        );
    }
}

/// Ensures .expect() is not used in workflow phase handlers.
///
/// Phase handlers should use Result types for error handling, not panics.
/// Expect panics in async code cause ungraceful shutdowns.
fn enforce_no_expect_in_workflow() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    let root = PathBuf::from(&manifest_dir);
    let workflow_dir = root.join("src/app/workflow");

    // Check workflow phase handler files (not mod.rs which is the orchestrator, not tests)
    let phase_files: Vec<PathBuf> = collect_files_to_check(&root)
        .into_iter()
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("rs")
                && p.starts_with(&workflow_dir)
                && p.file_name().and_then(|n| n.to_str()) != Some("mod.rs")
                && !p.to_string_lossy().contains("/tests/")
                && !p.to_string_lossy().contains("_tests.rs")
        })
        .collect();

    let mut violations: Vec<(PathBuf, Vec<(usize, String)>)> = Vec::new();

    for file in &phase_files {
        if let Ok(content) = std::fs::read_to_string(file) {
            let mut file_violations = Vec::new();

            for (line_num, line) in content.lines().enumerate() {
                let trimmed = line.trim();

                // Skip comments
                if trimmed.starts_with("//") {
                    continue;
                }

                // Check for .expect( pattern
                if trimmed.contains(".expect(") {
                    file_violations.push((
                        line_num + 1,
                        format!(
                            "expect() in phase handler: `{}`",
                            trimmed.chars().take(60).collect::<String>()
                        ),
                    ));
                }
            }

            if !file_violations.is_empty() {
                let rel_path = file.strip_prefix(&root).unwrap_or(file).to_path_buf();
                violations.push((rel_path, file_violations));
            }
        }
    }

    if !violations.is_empty() {
        let total_count: usize = violations.iter().map(|(_, v)| v.len()).sum();

        eprintln!("\n========================================");
        eprintln!("EXPECT() IN WORKFLOW PHASE HANDLERS");
        eprintln!("========================================");
        eprintln!();
        for (path, issues) in &violations {
            for (line_num, msg) in issues {
                eprintln!("  {}:{}", path.display(), line_num);
                eprintln!("    {}", msg);
                eprintln!();
            }
        }
        eprintln!("========================================");
        eprintln!();
        eprintln!("Phase handlers should use Result types, not expect() panics.");
        eprintln!("Panics in async code cause ungraceful shutdowns.");
        eprintln!();
        eprintln!("WRONG:");
        eprintln!("    let path = view.plan_path().expect(\"must be set\");");
        eprintln!();
        eprintln!("CORRECT:");
        eprintln!(
            "    let path = view.plan_path().ok_or_else(|| anyhow!(\"plan_path not set\"))?;"
        );
        eprintln!("    // Or use if-let with early return");
        eprintln!("    let Some(path) = view.plan_path() else {{");
        eprintln!("        return Err(anyhow!(\"plan_path not set\"));");
        eprintln!("    }};");
        eprintln!();
        eprintln!("========================================\n");
        panic!(
            "Build failed: {} expect() call(s) in workflow phase handlers. Use Result types.",
            total_count
        );
    }
}

/// Ensures phase interrupt handlers return NeedsRestart, not CancellationError.
///
/// This prevents the bug where revising.rs returned Err(CancellationError) instead
/// of Ok(Some(NeedsRestart)), which caused "hard fail" on user interrupt.
fn enforce_phase_interrupt_consistency() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    let root = PathBuf::from(&manifest_dir);
    let workflow_dir = root.join("src/app/workflow");

    // Phase handler files that should return NeedsRestart for interrupts
    let phase_files = ["planning.rs", "reviewing.rs", "revising.rs"];

    let mut violations: Vec<(PathBuf, Vec<(usize, String)>)> = Vec::new();

    for filename in &phase_files {
        let file = workflow_dir.join(filename);
        if let Ok(content) = std::fs::read_to_string(&file) {
            let mut file_violations = Vec::new();

            for (line_num, line) in content.lines().enumerate() {
                let trimmed = line.trim();

                // Skip comments
                if trimmed.starts_with("//") {
                    continue;
                }

                // Check for CancellationError being returned in interrupt handling
                // Pattern: return Err(CancellationError
                if trimmed.contains("return Err(CancellationError")
                    && !trimmed.contains("downcast_ref")
                {
                    file_violations.push((
                        line_num + 1,
                        format!(
                            "Phase interrupt should return NeedsRestart, not CancellationError: {}",
                            trimmed
                        ),
                    ));
                }
            }

            if !file_violations.is_empty() {
                violations.push((file, file_violations));
            }
        }
    }

    if !violations.is_empty() {
        let mut error_msg =
            String::from("\n\n=== Phase Interrupt Handler Consistency Violation ===\n\n");
        error_msg
            .push_str("Phase handlers should return Ok(Some(NeedsRestart)) for user interrupts,\n");
        error_msg.push_str(
            "not Err(CancellationError). CancellationError causes feedback to be lost.\n\n",
        );

        for (file, file_violations) in violations {
            error_msg.push_str(&format!("File: {}\n", file.display()));
            for (line_num, msg) in file_violations {
                error_msg.push_str(&format!("  Line {}: {}\n", line_num, msg));
            }
            error_msg.push('\n');
        }

        panic!("{}", error_msg);
    }
}
