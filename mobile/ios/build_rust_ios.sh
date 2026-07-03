#!/bin/bash
# build_rust_ios.sh — Build Rust FFI for iOS simulator and copy to project
set -e

REPO_ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
RUST_DIR="$REPO_ROOT"
TARGET_DIR="$RUST_DIR/target/aarch64-apple-ios-sim/release"
DEST_DIR="$REPO_ROOT/mobile/ios/RustLib"

echo "Building ggterm-ffi for iOS simulator (aarch64-apple-ios-sim)..."

# Use rustup's cargo (Homebrew cargo doesn't know about rustup targets)
CARGO="$HOME/.cargo/bin/cargo"
if [ ! -f "$CARGO" ]; then
    CARGO="cargo"
fi

cd "$RUST_DIR"
"$CARGO" build -p ggterm-ffi --target aarch64-apple-ios-sim --release

echo "Copying static library to iOS project..."
mkdir -p "$DEST_DIR"
cp "$TARGET_DIR/libggterm_ffi.a" "$DEST_DIR/"

echo "Done: $(ls -lh "$DEST_DIR/libggterm_ffi.a" | awk '{print $5}')"
