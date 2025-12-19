#!/bin/bash
set -e

# ============================================================================
# One-Liner Install Command
# ============================================================================
#
# Install from GitHub (SSH):
#   cargo install --git ssh://git@github.com/metjm/planning-agent.git --force
#
# If 'planning' command not found after install, run:
#   source "$HOME/.cargo/env"
# ============================================================================

echo "Building planning-agent..."
cargo build --release

echo "Installing to ~/.cargo/bin/planning..."
cargo install --path . --force

echo "Done! Run 'planning --help' to get started."
