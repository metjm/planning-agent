pub fn extract_bash_command(cmd: &str) -> String {
    let cmd = cmd.trim();

    let first_cmd = cmd
        .split("&&")
        .next()
        .unwrap_or(cmd)
        .split("||")
        .next()
        .unwrap_or(cmd)
        .split(';')
        .next()
        .unwrap_or(cmd)
        .split('|')
        .next()
        .unwrap_or(cmd)
        .trim();

    let tokens: Vec<&str> = first_cmd.split_whitespace().collect();

    for token in tokens {
        if token.contains('=') && !token.starts_with('-') {
            continue;
        }

        if token == "sudo" || token == "env" || token == "time" || token == "nice" {
            continue;
        }

        let cmd_name = token.rsplit('/').next().unwrap_or(token);
        return format!("Bash:{}", cmd_name);
    }

    "Bash".to_string()
}

#[cfg(test)]
#[path = "tests/util_tests.rs"]
mod tests;
