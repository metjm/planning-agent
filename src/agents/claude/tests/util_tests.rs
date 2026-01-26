use super::*;

#[test]
fn test_extract_bash_command() {
    assert_eq!(extract_bash_command("ls -la"), "Bash:ls");
    assert_eq!(extract_bash_command("cd /tmp && ls"), "Bash:cd");
    assert_eq!(extract_bash_command("FOO=bar npm install"), "Bash:npm");
    assert_eq!(extract_bash_command("sudo apt install"), "Bash:apt");
    assert_eq!(
        extract_bash_command("/usr/bin/python script.py"),
        "Bash:python"
    );
}
