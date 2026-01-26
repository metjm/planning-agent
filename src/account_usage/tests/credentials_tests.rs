use super::*;

#[test]
fn test_extract_email_from_jwt_invalid() {
    assert_eq!(extract_email_from_jwt("not.a.jwt"), None);
    assert_eq!(extract_email_from_jwt("invalid"), None);
}

#[test]
fn test_extract_email_from_jwt_valid() {
    // Create a simple JWT-like structure with email in payload
    // Header: {"alg":"none"}
    // Payload: {"email":"test@example.com"}
    use base64::Engine;
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
    let payload =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"email":"test@example.com"}"#);
    let token = format!("{}.{}.sig", header, payload);

    assert_eq!(
        extract_email_from_jwt(&token),
        Some("test@example.com".to_string())
    );
}

/// Integration tests that run against real credentials on the machine.
mod integration {
    use super::*;

    #[test]
    fn test_real_credential_file_paths() {
        let paths = credential_file_paths();
        // Should return 3 paths (claude, gemini, codex)
        assert_eq!(paths.len(), 3, "Should have 3 credential file paths");

        // Verify paths look reasonable
        for path in &paths {
            let path_str = path.to_string_lossy();
            assert!(
                path_str.contains(".claude")
                    || path_str.contains(".gemini")
                    || path_str.contains(".codex"),
                "Path should be for a known provider: {}",
                path_str
            );
        }
    }

    #[test]
    fn test_real_read_all_credentials() {
        // This test reads actual credential files if they exist
        let creds = read_all_credentials();

        // Print what we found for debugging
        eprintln!("Found {} credential sets", creds.len());
        for (provider, _cred) in &creds {
            eprintln!("  - {}", provider);
        }

        // If we're on a machine with credentials, verify we got them
        let paths = credential_file_paths();
        let existing_files: Vec<_> = paths.iter().filter(|p| p.exists()).collect();

        if !existing_files.is_empty() {
            assert!(
                !creds.is_empty(),
                "Should find credentials when credential files exist"
            );
        }
    }

    #[test]
    fn test_real_read_all_credential_info() {
        // This test reads actual credentials and converts to CredentialInfo
        let infos = read_all_credential_info();

        eprintln!("Found {} credential infos", infos.len());
        for info in &infos {
            eprintln!("  Provider: {}", info.provider);
            eprintln!(
                "    Email: {}",
                if info.email.is_empty() {
                    "(empty - fetched by host)"
                } else {
                    &info.email
                }
            );
            eprintln!("    Token valid: {}", info.token_valid);
            eprintln!("    Expires at: {:?}", info.expires_at);
        }

        // Check if any of the credential files exist
        let paths = credential_file_paths();
        let existing_files: Vec<_> = paths.iter().filter(|p| p.exists()).collect();

        if !existing_files.is_empty() {
            assert!(
                !infos.is_empty(),
                "Should find credential infos when credential files exist"
            );

            // Verify each info has required fields
            for info in &infos {
                assert!(!info.provider.is_empty(), "Provider should not be empty");
                // Email may be empty for Claude (fetched via API)
                // but Codex should have email from JWT
                if info.provider == "codex" {
                    assert!(!info.email.is_empty(), "Codex should have email from JWT");
                }
            }
        }
    }
}
