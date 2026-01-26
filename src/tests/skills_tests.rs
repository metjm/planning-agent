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
fn plan_review_skill_contains_no_timeline_directive() {
    assert!(
        PLAN_REVIEW_SKILL.contains(NO_TIMELINE_DIRECTIVE)
            || PLAN_REVIEW_SKILL
                .contains("timelines, schedules, dates, durations, or time estimates"),
        "Plan-review skill must contain the no-timeline directive"
    );
    for phrase in EXAMPLE_PHRASES {
        assert!(
            PLAN_REVIEW_SKILL.contains(phrase),
            "Plan-review skill must contain example phrase: {}",
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
