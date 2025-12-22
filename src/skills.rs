use std::fs;
use std::path::PathBuf;

// Embed skill files at compile time
const PLANNING_SKILL: &str = include_str!("../skills/planning/SKILL.md");
const PLAN_REVIEW_SKILL: &str = include_str!("../skills/plan-review/SKILL.md");

/// Install bundled skills to ~/.claude/skills and ~/.codex/skills (or $CODEX_HOME/skills) if needed.
pub fn install_skills_if_needed() -> anyhow::Result<()> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    let skills_dirs = [home.join(".claude/skills"), codex_home.join("skills")];

    let skills = [
        ("planning", PLANNING_SKILL),
        ("plan-review", PLAN_REVIEW_SKILL),
    ];

    for skills_dir in skills_dirs {
        for (name, content) in skills.iter().copied() {
            let skill_dir = skills_dir.join(name);
            let skill_file = skill_dir.join("SKILL.md");

            if !skill_file.exists() {
                eprintln!("[planning-agent] Installing skill: {}", name);
                fs::create_dir_all(&skill_dir)?;
                fs::write(&skill_file, content)?;
            }
        }
    }

    Ok(())
}
