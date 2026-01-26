use super::*;

#[test]
fn test_workflow_browser_state_new() {
    let state = WorkflowBrowserState::new();
    assert!(!state.open);
    assert!(state.entries.is_empty());
    assert_eq!(state.selected_idx, 0);
    assert_eq!(state.scroll_offset, 0);
}

#[test]
fn test_workflow_browser_close() {
    let mut state = WorkflowBrowserState::new();
    state.open = true;
    state.entries.push(WorkflowEntry {
        name: "test".to_string(),
        source: "built-in".to_string(),
        is_selected: false,
        planning_agent: "claude".to_string(),
        reviewing_agents: "claude".to_string(),
        sequential_review: false,
        aggregation: "any-rejects".to_string(),
        implementing_agent: "codex".to_string(),
        implementation_reviewing_agent: "claude".to_string(),
    });

    state.close();
    assert!(!state.open);
    assert!(state.entries.is_empty());
}

#[test]
fn test_select_prev_wraps() {
    let mut state = WorkflowBrowserState::new();
    state.entries = vec![
        WorkflowEntry {
            name: "a".to_string(),
            source: "built-in".to_string(),
            is_selected: false,
            planning_agent: "claude".to_string(),
            reviewing_agents: "claude".to_string(),
            sequential_review: false,
            aggregation: "any-rejects".to_string(),
            implementing_agent: "codex".to_string(),
            implementation_reviewing_agent: "claude".to_string(),
        },
        WorkflowEntry {
            name: "b".to_string(),
            source: "built-in".to_string(),
            is_selected: false,
            planning_agent: "claude".to_string(),
            reviewing_agents: "claude".to_string(),
            sequential_review: false,
            aggregation: "any-rejects".to_string(),
            implementing_agent: "codex".to_string(),
            implementation_reviewing_agent: "claude".to_string(),
        },
        WorkflowEntry {
            name: "c".to_string(),
            source: "built-in".to_string(),
            is_selected: false,
            planning_agent: "claude".to_string(),
            reviewing_agents: "claude".to_string(),
            sequential_review: false,
            aggregation: "any-rejects".to_string(),
            implementing_agent: "codex".to_string(),
            implementation_reviewing_agent: "claude".to_string(),
        },
    ];
    state.selected_idx = 0;

    state.select_prev();
    assert_eq!(state.selected_idx, 2); // Should wrap to end
}

#[test]
fn test_select_next_wraps() {
    let mut state = WorkflowBrowserState::new();
    state.entries = vec![
        WorkflowEntry {
            name: "a".to_string(),
            source: "built-in".to_string(),
            is_selected: false,
            planning_agent: "claude".to_string(),
            reviewing_agents: "claude".to_string(),
            sequential_review: false,
            aggregation: "any-rejects".to_string(),
            implementing_agent: "codex".to_string(),
            implementation_reviewing_agent: "claude".to_string(),
        },
        WorkflowEntry {
            name: "b".to_string(),
            source: "built-in".to_string(),
            is_selected: false,
            planning_agent: "claude".to_string(),
            reviewing_agents: "claude".to_string(),
            sequential_review: false,
            aggregation: "any-rejects".to_string(),
            implementing_agent: "codex".to_string(),
            implementation_reviewing_agent: "claude".to_string(),
        },
        WorkflowEntry {
            name: "c".to_string(),
            source: "built-in".to_string(),
            is_selected: false,
            planning_agent: "claude".to_string(),
            reviewing_agents: "claude".to_string(),
            sequential_review: false,
            aggregation: "any-rejects".to_string(),
            implementing_agent: "codex".to_string(),
            implementation_reviewing_agent: "claude".to_string(),
        },
    ];
    state.selected_idx = 2;

    state.select_next();
    assert_eq!(state.selected_idx, 0); // Should wrap to start
}

#[test]
fn test_selected_entry() {
    let mut state = WorkflowBrowserState::new();
    assert!(state.selected_entry().is_none());

    state.entries.push(WorkflowEntry {
        name: "test".to_string(),
        source: "built-in".to_string(),
        is_selected: false,
        planning_agent: "claude".to_string(),
        reviewing_agents: "claude".to_string(),
        sequential_review: false,
        aggregation: "any-rejects".to_string(),
        implementing_agent: "codex".to_string(),
        implementation_reviewing_agent: "claude".to_string(),
    });
    state.selected_idx = 0;

    let entry = state.selected_entry().unwrap();
    assert_eq!(entry.name, "test");
}

#[test]
fn test_ensure_visible_scrolls_up() {
    let mut state = WorkflowBrowserState::new();
    // Add 15 entries
    for i in 0..15 {
        state.entries.push(WorkflowEntry {
            name: format!("workflow-{}", i),
            source: "built-in".to_string(),
            is_selected: false,
            planning_agent: "claude".to_string(),
            reviewing_agents: "claude".to_string(),
            sequential_review: false,
            aggregation: "any-rejects".to_string(),
            implementing_agent: "codex".to_string(),
            implementation_reviewing_agent: "claude".to_string(),
        });
    }

    // Start scrolled down
    state.scroll_offset = 10;
    state.selected_idx = 5;
    state.ensure_visible();

    // Should scroll up to show selected
    assert!(state.scroll_offset <= state.selected_idx);
}

#[test]
fn test_ensure_visible_scrolls_down() {
    let mut state = WorkflowBrowserState::new();
    // Add 15 entries
    for i in 0..15 {
        state.entries.push(WorkflowEntry {
            name: format!("workflow-{}", i),
            source: "built-in".to_string(),
            is_selected: false,
            planning_agent: "claude".to_string(),
            reviewing_agents: "claude".to_string(),
            sequential_review: false,
            aggregation: "any-rejects".to_string(),
            implementing_agent: "codex".to_string(),
            implementation_reviewing_agent: "claude".to_string(),
        });
    }

    // Start at top
    state.scroll_offset = 0;
    state.selected_idx = 12;
    state.ensure_visible();

    // Should scroll down to show selected (viewport size is 8)
    assert!(state.selected_idx < state.scroll_offset + 8);
}

#[test]
fn test_refresh_loads_builtin_workflows() {
    let mut state = WorkflowBrowserState::new();
    let temp_dir = std::env::temp_dir();

    state.refresh(&temp_dir);

    // Should have at least the built-in workflows
    assert!(state.entries.iter().any(|e| e.name == "default"));
    assert!(state.entries.iter().any(|e| e.name == "claude-only"));
    assert!(state.entries.iter().any(|e| e.name == "codex-only"));
}

#[test]
fn test_refresh_preselects_current_workflow() {
    let mut state = WorkflowBrowserState::new();
    let temp_dir = std::env::temp_dir();

    // Simulate a workflow being marked as selected
    state.refresh(&temp_dir);

    // The default selection is "claude-only", so it should be pre-selected
    if let Some(idx) = state.entries.iter().position(|e| e.is_selected) {
        assert_eq!(state.selected_idx, idx);
    }
}

#[test]
fn test_refresh_populates_implementation_agents_for_default() {
    let mut state = WorkflowBrowserState::new();
    let temp_dir = std::env::temp_dir();

    state.refresh(&temp_dir);

    // Find the default workflow
    let default = state.entries.iter().find(|e| e.name == "default").unwrap();
    // Default workflow should have codex for implementing and claude for review
    assert_eq!(default.implementing_agent, "codex");
    assert_eq!(default.implementation_reviewing_agent, "claude");
}

#[test]
fn test_refresh_populates_implementation_agents_for_codex_only() {
    let mut state = WorkflowBrowserState::new();
    let temp_dir = std::env::temp_dir();

    state.refresh(&temp_dir);

    // Find the codex-only workflow
    let codex_only = state
        .entries
        .iter()
        .find(|e| e.name == "codex-only")
        .unwrap();
    // Codex-only workflow should have codex for implementing and codex-reviewer for review
    assert_eq!(codex_only.implementing_agent, "codex");
    assert_eq!(codex_only.implementation_reviewing_agent, "codex-reviewer");
}
