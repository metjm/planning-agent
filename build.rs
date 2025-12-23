use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_LINES: usize = 750;

const CHECKED_EXTENSIONS: &[&str] = &["rs", "md", "yaml", "toml"];

const EXCLUDED_DIRS: &[&str] = &["target", ".git", "node_modules"];

const EXCLUDED_FILES: &[&str] = &[
    "Cargo.lock",
    "docs/plans/file-line-limit.md",
];

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

    enforce_line_limits();
}

fn enforce_line_limits() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR must be set");
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
            eprintln!("  {} - {} lines (exceeds by {})", path.display(), lines, lines - MAX_LINES);
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

    let is_rust = path.extension().and_then(|e| e.to_str()) == Some("rs");

    if is_rust {
        let stripped = strip_rust_comments(&content);
        Ok(count_non_empty_lines(&stripped))
    } else {
        Ok(count_non_empty_lines(&content))
    }
}

fn count_non_empty_lines(content: &str) -> usize {
    content.lines().filter(|line| !line.trim().is_empty()).count()
}

fn strip_rust_comments(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let chars: Vec<char> = content.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == 'r' && i + 1 < len && (chars[i + 1] == '"' || chars[i + 1] == '#') {
            let start = i;
            i += 1;

            let mut hash_count = 0;
            while i < len && chars[i] == '#' {
                hash_count += 1;
                i += 1;
            }

            if i < len && chars[i] == '"' {
                result.push_str(&chars[start..=i].iter().collect::<String>());
                i += 1;

                while i < len {
                    if chars[i] == '"' {
                        let close_start = i;
                        i += 1;
                        let mut close_hashes = 0;
                        while i < len && chars[i] == '#' && close_hashes < hash_count {
                            close_hashes += 1;
                            i += 1;
                        }
                        result.push_str(&chars[close_start..i].iter().collect::<String>());
                        if close_hashes == hash_count {
                            break;
                        }
                    } else {
                        result.push(chars[i]);
                        i += 1;
                    }
                }
                continue;
            } else {
                result.push_str(&chars[start..i].iter().collect::<String>());
                continue;
            }
        }

        if chars[i] == '"' {
            result.push(chars[i]);
            i += 1;
            while i < len {
                if chars[i] == '\\' && i + 1 < len {
                    result.push(chars[i]);
                    result.push(chars[i + 1]);
                    i += 2;
                } else if chars[i] == '"' {
                    result.push(chars[i]);
                    i += 1;
                    break;
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            }
            continue;
        }

        if chars[i] == '\'' {
            result.push(chars[i]);
            i += 1;
            if i < len {
                if chars[i] == '\\' && i + 1 < len {
                    result.push(chars[i]);
                    result.push(chars[i + 1]);
                    i += 2;
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
                if i < len && chars[i] == '\'' {
                    result.push(chars[i]);
                    i += 1;
                }
            }
            continue;
        }

        if chars[i] == '/' && i + 1 < len && chars[i + 1] == '/' {
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            if i < len {
                result.push('\n');
                i += 1;
            }
            continue;
        }

        if chars[i] == '/' && i + 1 < len && chars[i + 1] == '*' {
            i += 2;
            let mut depth = 1;
            while i < len && depth > 0 {
                if chars[i] == '/' && i + 1 < len && chars[i + 1] == '*' {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && i + 1 < len && chars[i + 1] == '/' {
                    depth -= 1;
                    i += 2;
                } else {
                    if chars[i] == '\n' {
                        result.push('\n');
                    }
                    i += 1;
                }
            }
            continue;
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}
