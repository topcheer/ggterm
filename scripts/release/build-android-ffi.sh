#!/bin/bash
# build-android-ffi.sh — Build Rust FFI static library for Android
# Usage: scripts/release/build-android-ffi.sh [architecture]
#   architecture: arm64-v8a (default), armeabi-v7a, x86_64
set -euo pipefail

ARCH=${1:-arm64-v8a}
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

# Check for cargo-ndk
if ! command -v cargo-ndk >/dev/null 2>&1; then
    echo "Installing cargo-ndk..."
    cargo install cargo-ndk
fi

# Add Android targets
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android

# Build for target architecture
case "$ARCH" in
    arm64-v8a) NDK_TARGET="aarch64-linux-android" ;;
    armeabi-v7a) NDK_TARGET="armv7-linux-androideabi" ;;
    x86_64) NDK_TARGET="x86_64-linux-android" ;;
    *) echo "Unknown arch: $ARCH"; exit 1 ;;
esac

echo "Building ggterm-ffi for $ARCH ($NDK_TARGET)..."
cargo ndk -t "$NDK_TARGET" build -p ggterm-ffi --release --features ssh,p2p

# Copy to JniLibs directory structure
JNILIB_DIR="$REPO_ROOT/mobile/android/app/src/main/jniLibs/$ARCH"
mkdir -p "$JNILIB_DIR"
cp "$REPO_ROOT/target/$NDK_TARGET/release/libggterm_ffi.so" "$JNILIB_DIR/"

echo "Done: $JNILIB_DIR/libggterm_ffi.so"
ls -lh "$JNILIB_DIR/libggterm_ffi.so"
