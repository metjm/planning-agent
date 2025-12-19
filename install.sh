#!/bin/bash
set -e

echo "Building planning-agent..."
cargo build --release

echo "Installing to ~/.cargo/bin/planning..."
cargo install --path . --force

echo "Done! Run 'planning --help' to get started."
