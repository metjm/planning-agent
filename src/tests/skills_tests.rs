use super::*;

const NO_TIMELINE_DIRECTIVE: &str =
    "DO NOT include timelines, schedules, dates, durations, or time estimates";
const EXAMPLE_PHRASES: [&str; 3] = ["in two weeks", "Sprint 1", "Q1 delivery"];

#[test]
fn planning_skill_contains_no_timeline_directive() {
    assert!(
        PLANNING_SKILL.contains(NO_TIMELINE_DIRECTIVE),
        "Planning skill must contain the no-timeline directive"
    );
    for phrase in EXAMPLE_PHRASES {
        assert!(
            PLANNING_SKILL.contains(phrase),
            "Planning skill must contain example phrase: {}",
            phrase
        );
    }
}

#[test]
fn plan_review_adversarial_skill_contains_no_timeline_directive() {
    assert!(
        PLAN_REVIEW_ADVERSARIAL_SKILL.contains(NO_TIMELINE_DIRECTIVE),
        "Plan-review-adversarial skill must contain the no-timeline directive"
    );
    for phrase in EXAMPLE_PHRASES {
        assert!(
            PLAN_REVIEW_ADVERSARIAL_SKILL.contains(phrase),
            "Plan-review-adversarial skill must contain example phrase: {}",
            phrase
        );
    }
}

#[test]
fn plan_review_operational_skill_contains_no_timeline_directive() {
    assert!(
        PLAN_REVIEW_OPERATIONAL_SKILL.contains(NO_TIMELINE_DIRECTIVE),
        "Plan-review-operational skill must contain the no-timeline directive"
    );
    for phrase in EXAMPLE_PHRASES {
        assert!(
            PLAN_REVIEW_OPERATIONAL_SKILL.contains(phrase),
            "Plan-review-operational skill must contain example phrase: {}",
            phrase
        );
    }
}

#[test]
fn plan_review_codebase_skill_contains_no_timeline_directive() {
    assert!(
        PLAN_REVIEW_CODEBASE_SKILL.contains(NO_TIMELINE_DIRECTIVE),
        "Plan-review-codebase skill must contain the no-timeline directive"
    );
    for phrase in EXAMPLE_PHRASES {
        assert!(
            PLAN_REVIEW_CODEBASE_SKILL.contains(phrase),
            "Plan-review-codebase skill must contain example phrase: {}",
            phrase
        );
    }
}

#[test]
fn methodical_debugging_skill_contains_no_timeline_directive() {
    assert!(
        METHODICAL_DEBUGGING_SKILL.contains(NO_TIMELINE_DIRECTIVE),
        "Methodical-debugging skill must contain the no-timeline directive"
    );
    for phrase in EXAMPLE_PHRASES {
        assert!(
            METHODICAL_DEBUGGING_SKILL.contains(phrase),
            "Methodical-debugging skill must contain example phrase: {}",
            phrase
        );
    }
}

#[test]
fn all_plan_review_skills_contain_verification_requirements() {
    // All plan review skills must include critical verification requirements
    let verification_markers = [
        "Library and API Verification",
        "Precision Requirements",
        "Code Quality",
    ];

    for marker in verification_markers {
        assert!(
            PLAN_REVIEW_ADVERSARIAL_SKILL.contains(marker),
            "Plan-review-adversarial skill must contain '{}' section",
            marker
        );
        assert!(
            PLAN_REVIEW_OPERATIONAL_SKILL.contains(marker),
            "Plan-review-operational skill must contain '{}' section",
            marker
        );
        assert!(
            PLAN_REVIEW_CODEBASE_SKILL.contains(marker),
            "Plan-review-codebase skill must contain '{}' section",
            marker
        );
    }
}

#[test]
fn all_plan_review_skills_have_distinct_descriptions() {
    // Each skill must have a unique description in its frontmatter
    fn extract_description(content: &str) -> Option<&str> {
        for line in content.lines() {
            if line.starts_with("description:") {
                return Some(line.trim_start_matches("description:").trim());
            }
        }
        None
    }

    let desc_adversarial = extract_description(PLAN_REVIEW_ADVERSARIAL_SKILL);
    let desc_operational = extract_description(PLAN_REVIEW_OPERATIONAL_SKILL);
    let desc_codebase = extract_description(PLAN_REVIEW_CODEBASE_SKILL);

    assert!(
        desc_adversarial.is_some(),
        "Adversarial skill must have description"
    );
    assert!(
        desc_operational.is_some(),
        "Operational skill must have description"
    );
    assert!(
        desc_codebase.is_some(),
        "Codebase skill must have description"
    );

    assert_ne!(
        desc_adversarial, desc_operational,
        "Adversarial and Operational skills must have different descriptions"
    );
    assert_ne!(
        desc_adversarial, desc_codebase,
        "Adversarial and Codebase skills must have different descriptions"
    );
    assert_ne!(
        desc_operational, desc_codebase,
        "Operational and Codebase skills must have different descriptions"
    );
}

#[test]
fn all_plan_review_skills_use_plan_feedback_tags() {
    // All skills must use <plan-feedback> tags for output format compatibility
    assert!(
        PLAN_REVIEW_ADVERSARIAL_SKILL.contains("<plan-feedback>"),
        "Adversarial skill must use <plan-feedback> tags"
    );
    assert!(
        PLAN_REVIEW_OPERATIONAL_SKILL.contains("<plan-feedback>"),
        "Operational skill must use <plan-feedback> tags"
    );
    assert!(
        PLAN_REVIEW_CODEBASE_SKILL.contains("<plan-feedback>"),
        "Codebase skill must use <plan-feedback> tags"
    );
}
