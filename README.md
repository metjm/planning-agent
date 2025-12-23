# Planning Agent

<img width="982" height="770" alt="image" src="https://github.com/user-attachments/assets/6549dbf1-6aca-428c-88c0-c52a6ed19340" />

A TUI application for creating implementation plans.

## What is Planning Agent?

Planning Agent is a Rust-based terminal user interface (TUI) application that automates the creation of high-quality implementation plans. It orchestrates an iterative plan-review-revise cycle using Claude Code.

**Key Features:**

- **Automated Planning**: Creates comprehensive implementation plans based on your objective
- **AI-Powered Review**: Plans are automatically reviewed for correctness, completeness, and technical accuracy
- **Iterative Refinement**: Plans that need improvement are automatically revised and re-reviewed (up to 3 iterations by default)
- **User Approval Gate**: Final checkpoint where you can accept, decline with feedback, or hand off directly to Claude Code for implementation

## Workflow

The Planning Agent follows an iterative workflow: First, it creates an implementation plan. Then, an AI reviewer evaluates the plan and either approves it or requests revisions. If revisions are needed, the plan is updated and re-reviewed (up to 3 iterations by default). Once approved, the user can accept the final plan, decline with feedback to restart, or press `[i]` to hand off directly to Claude Code for implementation.

```mermaid
flowchart TD
    Start([User provides objective]) --> Planning

    subgraph "Planning Agent Workflow (up to 3 iterations)"
        Planning[Planning Phase<br/>Creates plan.md] --> Reviewing
        Reviewing[Reviewing Phase<br/>Creates feedback.md] --> Decision{Feedback Status}
        Decision -->|APPROVED| Complete[Complete]
        Decision -->|NEEDS REVISION| CheckIter{Iteration < Max?}
        CheckIter -->|Yes| Revising[Revising Phase<br/>Updates plan.md<br/>Iteration +1]
        CheckIter -->|No| MaxReached[Max Iterations<br/>Manual review needed]
        Revising --> Reviewing
    end

    Complete --> UserApproval{User Approval}
    UserApproval -->|Accept| Done([Workflow Complete])
    UserApproval -->|Decline with feedback| Restart([Restart workflow])

    MaxReached --> ManualDone([Workflow ends])
```

> **Note:** When the user approval dialog appears, pressing `[i]` hands off directly to Claude Code for implementation. This terminates the Planning Agent process and is not shown in the diagram as it's not a workflow state transition.

⚠️⚠️⚠️

This uses --dangerously-skip-permissions by default, so I do **NOT** recommend using it without a container.

⚠️⚠️⚠️

## Installation

### Quick Install

```bash
cargo install --git https://github.com/metjm/planning-agent.git --force
```

### From Source

Clone and build locally:

```bash
git clone https://github.com/metjm/planning-agent.git
cd planning-agent
./install.sh
```

### Troubleshooting

If `planning` command is not found after installation:

```bash
source "$HOME/.cargo/env"
```

Or add to your shell profile (~/.bashrc, ~/.zshrc):

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

## Usage

```bash
planning --help
```

## Requirements

- Rust toolchain (rustc + cargo)
- Git (for cloning)
