# GGTerm Developer Guide

> Contributor および plugin developer 向け

## Development Setup

```bash
git clone https://github.com/topcheer/ggterm.git
cd ggterm

# Debug build
make build

# Run tests (2,143 tests)
make test

# Lint (must be zero warnings)
make clippy

# Format check
make fmt
```

## Feature Flags

```bash
# Standard desktop
cargo build --features "desktop ai plugin plugin-lua config-watch" --bin ggterm

# Without config hot-reload
cargo build --features "desktop" --bin ggterm

# Mobile FFI (no desktop)
cargo build -p ggterm-ffi --features ssh

# P2P support
cargo build --features "desktop ai plugin plugin-lua config-watch p2p" --bin ggterm
```

## Adding a New Terminal Protocol

すべての SGR/CSI/OSC handler は `crates/ggterm-core/src/term/mod.rs` にあります。

1. **CSI handler** — `csi()` method に追加します
2. **OSC handler** — `osc()` method に追加します
3. **ESC handler** — `esc()` method に追加します
4. **Test** — 同じファイル内に test を追加します

## Adding a New Theme

Theme は `crates/ggterm-render/src/theme.rs` で定義されています。

```rust
pub fn my_theme() -> Theme {
    Theme {
        background: [20, 20, 30],
        foreground: [200, 200, 210],
        cursor: [255, 255, 255],
        selection_bg: [60, 80, 120],
        palette: DEFAULT_PALETTE,
    }
}
```

`by_name()`, `builtin_names()`, `cycle_next()` に登録してください。

## Window Module Guide

```
window/mod.rs       — DesktopApp struct, constructor, ApplicationHandler
window/handlers.rs  — Event handlers (keyboard, mouse, resize, IME)
window/actions.rs   — Business logic (tab/split/clipboard/theme/session)
window/render.rs    — Rendering (render_frame, multi-pane, overlays)
```

### Adding a Keyboard Shortcut

1. `window/handlers.rs` に handler を追加します
2. `window/actions.rs` に action method を追加します
3. Shortcut help（`shortcut_help.rs`）に登録します

### Borrow Checker Patterns

**問題**: `self.active_session().app().grid()` は `&self` 全体を borrow します。

**解決策**: Direct field access:
```rust
let active = self.active;
let grid = &self.sessions[active].app().grid();
```

## Mobile Development

### iOS Simulator

```bash
# Build Rust static lib (universal: arm64 + x86_64)
~/.cargo/bin/cargo build -p ggterm-ffi --target aarch64-apple-ios-sim --release --features "ssh p2p"
~/.cargo/bin/cargo build -p ggterm-ffi --target x86_64-apple-ios --release --features "ssh p2p"
lipo -create target/aarch64-apple-ios-sim/release/libggterm_ffi.a \
              target/x86_64-apple-ios/release/libggterm_ffi.a \
              -output mobile/ios/RustLib/libggterm_ffi.a

# Build and run Flutter
cd mobile && flutter run --debug
```

### Android

```bash
scripts/release/build-android-ffi.sh
cd mobile && flutter run
```

## Code Style

- **Test 以外のコードで `.unwrap()` を使用しない** — lock には `unwrap_or_else(|e| e.into_inner())` を使用してください
- **毎回の commit 前に `cargo fmt --all` を実行**
- **Clippy は `-D warnings` で pass する必要があります**
- **Cell は Copy ではなく Clone** — 明示的に `.clone()` を使用してください
- **編集前に読む** — 必ず `read_file` を先に行ってください

## Testing

```bash
# All tests
make test

# Specific crate
cargo test -p ggterm-core --lib
cargo test --features "desktop ai plugin plugin-lua config-watch" -p ggterm-app --lib

# Single test
cargo test --features "desktop" -p ggterm-core --lib -- test_osc52
```

## CI/CD Pipeline

| Trigger | Workflow | Action |
|---------|----------|--------|
| Push to main / PR | `ci.yml` | fmt + clippy + test + build |
| Tag `v*` | `release-desktop.yml` | macOS .dmg + Linux .deb + Windows .zip |
| Tag `v*` | `release-mobile.yml` | Android .apk + iOS .ipa |

### Creating a Release

```bash
git add -A
git commit -m "release: vX.Y.Z"
git tag vX.Y.Z
git push origin main --tags
```

## Debugging

```bash
ggterm -v     # info
ggterm -vv    # debug
ggterm -vvv   # trace
```

Debug overlay は `F1` を、performance monitor は `Ctrl+Shift+G` を押してください。

## FFI Development

### Adding a New C-ABI Function

1. `crates/ggterm-ffi/src/lib.rs` または `transport.rs` に宣言します
2. 実装します（lock には `unwrap_or_else(|e| e.into_inner())` を使用）
3. `mobile/lib/ffi/ffi_bindings.dart` に Dart binding を追加します
4. `mobile/ios/RustLib/ggterm_ffi.h` の C header を更新します

## Plugin Development

### Lua Plugin

```lua
-- ~/.ggterm/plugins/myplugin.lua
function on_load()
    print("Plugin loaded!")
end
```

```toml
[plugins]
enabled = true
directory = "~/.ggterm/plugins"
```

## Common Issues

- **"module not found"**: Module が `lib.rs` で宣言されていることを確認してください
- **Clippy let chains**: `&&` のみサポート、`||` は使用できません
- **Font rendering**: Menlo Bold は box-drawing chars を欠落しています。常に Weight::NORMAL を使用してください

## Key File Locations

| What | Where |
|------|-------|
| Terminal protocols | `crates/ggterm-core/src/term/mod.rs` |
| VTE parser | `crates/ggterm-core/src/vte/parser.rs` |
| Grid model | `crates/ggterm-core/src/grid/mod.rs` |
| Themes | `crates/ggterm-render/src/theme.rs` |
| GPU pipeline | `crates/ggterm-render-wgpu/src/lib.rs` |
| DesktopApp | `crates/ggterm-app/src/window/mod.rs` |
| Event handlers | `crates/ggterm-app/src/window/handlers.rs` |
| Config system | `crates/ggterm-app/src/config.rs` |
| FFI functions | `crates/ggterm-ffi/src/lib.rs` |
| CLI entry | `crates/ggterm-app/src/bin/ggterm.rs` |

## Contributing

1. Repository を fork します
2. Feature branch を作成します
3. `make fmt && make clippy && make test` を実行します
4. Conventional messages（`feat:`, `fix:`, `docs:`）で commit します
5. Pull Request を作成します

### Pull Request Checklist

- [ ] `cargo fmt --all -- --check` が pass する
- [ ] `cargo clippy --features "..." --workspace -- -D warnings` が pass する
- [ ] `cargo test --features "..." --workspace` が pass する
- [ ] Test 以外のコードに新たな `.unwrap()` がない
