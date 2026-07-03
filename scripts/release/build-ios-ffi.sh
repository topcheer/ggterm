#!/bin/bash
# build-ios-ffi.sh — Build Rust FFI static library for iOS
# Usage: scripts/release/build-ios-ffi.sh [target]
#   target: aarch64-apple-ios-sim (default), aarch64-apple-ios, x86_64-apple-ios
set -euo pipefail

TARGET=${1:-aarch64-apple-ios-sim}
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

CARGO="$HOME/.cargo/bin/cargo"
if [ ! -f "$CARGO" ]; then
    CARGO="cargo"
fi

# Add iOS targets
rustup target add aarch64-apple-ios-sim aarch64-apple-ios x86_64-apple-ios 2>/dev/null || true

echo "Building ggterm-ffi for iOS ($TARGET)..."
cd "$REPO_ROOT"
"$CARGO" build -p ggterm-ffi --target "$TARGET" --release

# Copy to iOS project
DEST_DIR="$REPO_ROOT/mobile/ios/RustLib"
mkdir -p "$DEST_DIR"
cp "$REPO_ROOT/target/$TARGET/release/libggterm_ffi.a" "$DEST_DIR/"

echo "Done: $DEST_DIR/libggterm_ffi.a"
ls -lh "$DEST_DIR/libggterm_ffi.a"
