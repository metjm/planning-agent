use super::*;
use tempfile::TempDir;

#[test]
fn test_compute_filesystem_fingerprint_empty_dir() {
    let dir = TempDir::new().unwrap();
    let fp1 = compute_filesystem_fingerprint(dir.path()).unwrap();

    // Same empty dir should have same fingerprint
    let fp2 = compute_filesystem_fingerprint(dir.path()).unwrap();
    assert_eq!(fp1, fp2);
}

#[test]
fn test_compute_filesystem_fingerprint_changes_with_content() {
    let dir = TempDir::new().unwrap();

    let fp1 = compute_filesystem_fingerprint(dir.path()).unwrap();

    // Add a file
    let file_path = dir.path().join("test.txt");
    fs::write(&file_path, "hello").unwrap();

    let fp2 = compute_filesystem_fingerprint(dir.path()).unwrap();
    assert_ne!(fp1, fp2, "Fingerprint should change when file is added");

    // Modify the file
    fs::write(&file_path, "hello world").unwrap();

    let fp3 = compute_filesystem_fingerprint(dir.path()).unwrap();
    assert_ne!(fp2, fp3, "Fingerprint should change when file is modified");
}

#[test]
fn test_compute_filesystem_fingerprint_excludes_target() {
    let dir = TempDir::new().unwrap();

    // Add a regular file
    fs::write(dir.path().join("src.txt"), "source").unwrap();
    let fp1 = compute_filesystem_fingerprint(dir.path()).unwrap();

    // Add files in excluded directories
    let target_dir = dir.path().join("target");
    fs::create_dir_all(&target_dir).unwrap();
    fs::write(target_dir.join("build.txt"), "build output").unwrap();

    let node_modules = dir.path().join("node_modules");
    fs::create_dir_all(&node_modules).unwrap();
    fs::write(node_modules.join("package.json"), "{}").unwrap();

    let fp2 = compute_filesystem_fingerprint(dir.path()).unwrap();
    assert_eq!(
        fp1, fp2,
        "Fingerprint should not change for excluded directories"
    );
}

#[test]
fn test_is_git_repo() {
    let dir = TempDir::new().unwrap();
    assert!(!is_git_repo(dir.path()));

    // Create .git directory
    fs::create_dir_all(dir.path().join(".git")).unwrap();
    assert!(is_git_repo(dir.path()));
}

#[test]
fn test_collect_files_excludes_properly() {
    let dir = TempDir::new().unwrap();

    // Create files in various locations
    fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();

    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("lib.rs"), "// lib").unwrap();

    let target = dir.path().join("target");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("debug.txt"), "debug").unwrap();

    let mut entries = BTreeSet::new();
    collect_files(dir.path(), dir.path(), &mut entries).unwrap();

    assert!(entries.contains("main.rs"));
    assert!(entries.contains("src/lib.rs") || entries.contains("src\\lib.rs"));
    // target should be excluded
    assert!(!entries.iter().any(|e| e.contains("target")));
}
