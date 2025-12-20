# Planning Agent

A TUI application for creating and managing implementation plans.

⚠️⚠️⚠️

This uses --dangerously-skip-permissions by default, so I do **NOT** recommend using it without a container.

⚠️⚠️⚠️

## Installation

### Quick Install

```bash
cargo install --git ssh://git@github.com/metjm/planning-agent.git --force
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

## License

See LICENSE file for details.
