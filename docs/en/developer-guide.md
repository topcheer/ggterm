# GGTerm Developer Guide

> For contributors and plugin developers

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

All SGR/CSI/OSC handlers are in `crates/ggterm-core/src/term/mod.rs`.

1. **CSI handler** — Add to the `csi()` method
2. **OSC handler** — Add to the `osc()` method
3. **ESC handler** — Add to the `esc()` method
4. **Test** — Add tests in the same file

## Adding a New Theme

Themes are defined in `crates/ggterm-render/src/theme.rs`.

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

Register in `by_name()`, `builtin_names()`, and `cycle_next()`.

## Window Module Guide

```
window/mod.rs       — DesktopApp struct, constructor, ApplicationHandler
window/handlers.rs  — Event handlers (keyboard, mouse, resize, IME)
window/actions.rs   — Business logic (tab/split/clipboard/theme/session)
window/render.rs    — Rendering (render_frame, multi-pane, overlays)
```

### Adding a Keyboard Shortcut

1. Add the handler in `window/handlers.rs`
2. Add the action method in `window/actions.rs`
3. Register in shortcut help (`shortcut_help.rs`)

### Borrow Checker Patterns

**Problem**: `self.active_session().app().grid()` borrows all of `&self`.

**Solution**: Direct field access:
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

- **No `.unwrap()` in non-test code** — use `unwrap_or_else(|e| e.into_inner())` for locks
- **`cargo fmt --all` before every commit**
- **Clippy must pass with `-D warnings`**
- **Cell is Clone not Copy** — use `.clone()` explicitly
- **Read before edit** — always `read_file` first

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

Press `F1` for debug overlay, `Ctrl+Shift+G` for performance monitor.

## FFI Development

### Adding a New C-ABI Function

1. Declare in `crates/ggterm-ffi/src/lib.rs` or `transport.rs`
2. Implement (use `unwrap_or_else(|e| e.into_inner())` for locks)
3. Add Dart binding in `mobile/lib/ffi/ffi_bindings.dart`
4. Update C header in `mobile/ios/RustLib/ggterm_ffi.h`

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

- **"module not found"**: Ensure module is declared in `lib.rs`
- **Clippy let chains**: Only `&&` supported, never `||`
- **Font rendering**: Menlo Bold lacks box-drawing chars; always use Weight::NORMAL

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

1. Fork the repository
2. Create a feature branch
3. Run `make fmt && make clippy && make test`
4. Commit with conventional messages (`feat:`, `fix:`, `docs:`)
5. Create a Pull Request

### Pull Request Checklist

- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --features "..." --workspace -- -D warnings` passes
- [ ] `cargo test --features "..." --workspace` passes
- [ ] No new `.unwrap()` in non-test code
