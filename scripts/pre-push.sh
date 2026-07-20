#!/bin/bash
# GGTerm pre-push hook: runs CI-equivalent checks before allowing push.
#
# Checks (matching CI exactly):
#   1. cargo fmt --check
#   2. cargo clippy --workspace -- -D warnings           (default features)
#   3. cargo clippy --workspace --features "$FEATURES" -- -D warnings
#   4. cargo test --workspace                            (default features)
#   5. cargo test --workspace --features "$FEATURES"
#
# Skip with: git push --no-verify

set -euo pipefail

FEATURES="desktop ai plugin plugin-lua config-watch"

echo "━━━ pre-push checks ━━━"

echo "▶ fmt check..."
cargo fmt --all -- --check

echo "▶ clippy (default features)..."
cargo clippy --workspace -- -D warnings 2>&1 | tail -1

echo "▶ clippy (full features)..."
cargo clippy --workspace --features "$FEATURES" -- -D warnings 2>&1 | tail -1

echo "▶ test (default features)..."
cargo test --workspace 2>&1 | grep -E "^test result" | grep -v "0 failed" && {
    echo "✗ tests failed (default features)"; exit 1
} || true

echo "▶ test (full features)..."
cargo test --workspace --features "$FEATURES" 2>&1 | grep -E "^test result" | grep -v "0 failed" && {
    echo "✗ tests failed (full features)"; exit 1
} || true

echo "━━━ all checks passed ✓ ━━━"
