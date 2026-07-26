#!/bin/bash
# GGTerm pre-commit hook: auto-format staged Rust files.
#
# Runs `cargo fmt` on the entire workspace, then re-stages any files
# that were modified. This ensures every commit passes the CI fmt check.
#
# Skip with: git commit --no-verify

set -euo pipefail

echo "▶ cargo fmt..."
cargo fmt --all

# Re-stage files that fmt may have changed.
# Only restage .rs files that are already in the index.
CHANGED=$(git diff --name-only --staged -- '*.rs')
if [ -n "$CHANGED" ]; then
    git add $CHANGED
fi

echo "✓ fmt done"
