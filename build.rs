use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_LINES: usize = 750;

const CHECKED_EXTENSIONS: &[&str] = &["rs", "md", "yaml", "toml"];

const EXCLUDED_DIRS: &[&str] = &["target", ".git", "node_modules"];

const EXCLUDED_FILES: &[&str] = &["Cargo.lock", "docs/plans/file-line-limit.md"];

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

    enforce_formatting();
    enforce_line_limits();
    enforce_no_dead_code_allows();
    enforce_no_test_skips();
    enforce_no_nested_runtimes();
    enforce_serial_for_env_mutations();
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
                    if should_check_file(&path, root) {
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
                    for j in (i + 1)..lines.len().min(i + 5) {
                        if lines[j].contains("fn ") {
                            test_fn_start = i + 1;
                            if let Some(fn_pos) = lines[j].find("fn ") {
                                let after_fn = &lines[j][fn_pos + 3..];
                                if let Some(paren) = after_fn.find('(') {
                                    test_fn_name = after_fn[..paren].trim().to_string();
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
                    for j in (i + 1)..lines.len().min(i + 5) {
                        if lines[j].contains("fn ") {
                            test_fn_start = i + 1;
                            // Extract function name
                            if let Some(fn_pos) = lines[j].find("fn ") {
                                let after_fn = &lines[j][fn_pos + 3..];
                                if let Some(paren) = after_fn.find('(') {
                                    test_fn_name = after_fn[..paren].trim().to_string();
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
