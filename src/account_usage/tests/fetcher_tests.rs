use super::*;
use crate::planning_paths::set_home_for_test;
use serial_test::serial;
use tempfile::TempDir;

#[test]
#[serial]
fn test_fetch_all_usage_no_credentials() {
    let temp_dir = TempDir::new().unwrap();
    let _guard = set_home_for_test(temp_dir.path().to_path_buf());

    // Set env vars to point to empty temp dirs so no real credentials are found
    let empty_claude = temp_dir.path().join("claude");
    let empty_gemini = temp_dir.path().join("gemini");
    let empty_codex = temp_dir.path().join("codex");
    std::fs::create_dir_all(&empty_claude).unwrap();
    std::fs::create_dir_all(&empty_gemini).unwrap();
    std::fs::create_dir_all(&empty_codex).unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", &empty_claude);
    std::env::set_var("GEMINI_DIR", &empty_gemini);
    std::env::set_var("CODEX_HOME", &empty_codex);

    let mut store = UsageStore::new();
    fetch_all_usage(&mut store, None);

    // Restore env
    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var("GEMINI_DIR");
    std::env::remove_var("CODEX_HOME");

    // No credentials available, so no accounts
    assert!(store.get_all_accounts().is_empty());
}
