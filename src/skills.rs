use std::fs;
use std::path::PathBuf;

const PLANNING_SKILL: &str = include_str!("../skills/planning/SKILL.md");
const PLAN_REVIEW_ADVERSARIAL_SKILL: &str =
    include_str!("../skills/plan-review-adversarial/SKILL.md");
const PLAN_REVIEW_OPERATIONAL_SKILL: &str =
    include_str!("../skills/plan-review-operational/SKILL.md");
const PLAN_REVIEW_CODEBASE_SKILL: &str = include_str!("../skills/plan-review-codebase/SKILL.md");
const METHODICAL_DEBUGGING_SKILL: &str = include_str!("../skills/methodical-debugging/SKILL.md");
const IMPLEMENTATION_SKILL: &str = include_str!("../skills/implementation/SKILL.md");
const IMPLEMENTATION_REVIEW_SKILL: &str = include_str!("../skills/implementation-review/SKILL.md");

pub fn install_skills_if_needed() -> anyhow::Result<()> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    let skills_dirs = [home.join(".claude/skills"), codex_home.join("skills")];

    let skills = [
        ("planning", PLANNING_SKILL),
        ("plan-review-adversarial", PLAN_REVIEW_ADVERSARIAL_SKILL),
        ("plan-review-operational", PLAN_REVIEW_OPERATIONAL_SKILL),
        ("plan-review-codebase", PLAN_REVIEW_CODEBASE_SKILL),
        ("methodical-debugging", METHODICAL_DEBUGGING_SKILL),
        ("implementation", IMPLEMENTATION_SKILL),
        ("implementation-review", IMPLEMENTATION_REVIEW_SKILL),
    ];

    for skills_dir in &skills_dirs {
        for (name, content) in skills.iter().copied() {
            let skill_dir = skills_dir.join(name);
            let skill_file = skill_dir.join("SKILL.md");

            let mut action = "Installing";
            let should_write = match fs::read_to_string(&skill_file) {
                Ok(existing) => {
                    if existing == content {
                        false
                    } else {
                        action = "Updating";
                        true
                    }
                }
                Err(_) => true,
            };

            if should_write {
                eprintln!("[planning-agent] {} skill: {}", action, name);
                fs::create_dir_all(&skill_dir)?;
                fs::write(&skill_file, content)?;
            }
        }

        // Clean up deprecated skill: plan-review (replaced by specialized variants)
        let old_skill_dir = skills_dir.join("plan-review");
        if old_skill_dir.exists() {
            eprintln!("[planning-agent] Removing deprecated skill: plan-review");
            fs::remove_dir_all(&old_skill_dir)?;
        }
    }

    Ok(())
}

#[cfg(test)]
#[path = "tests/skills_tests.rs"]
mod tests;
