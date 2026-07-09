# Guía para desarrolladores de GGTerm

> Para contribuyentes y desarrolladores de plugins

## Configuración del entorno de desarrollo

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

## Añadir un nuevo protocolo de terminal

Todos los handlers de SGR/CSI/OSC se encuentran en `crates/ggterm-core/src/term/mod.rs`.

1. **CSI handler** — Añadir al método `csi()`
2. **OSC handler** — Añadir al método `osc()`
3. **ESC handler** — Añadir al método `esc()`
4. **Test** — Añadir tests en el mismo archivo

## Añadir un nuevo tema

Los temas se definen en `crates/ggterm-render/src/theme.rs`.

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

Registrar en `by_name()`, `builtin_names()` y `cycle_next()`.

## Guía del módulo window

```
window/mod.rs       — DesktopApp struct, constructor, ApplicationHandler
window/handlers.rs  — Event handlers (keyboard, mouse, resize, IME)
window/actions.rs   — Business logic (tab/split/clipboard/theme/session)
window/render.rs    — Rendering (render_frame, multi-pane, overlays)
```

### Añadir un atajo de teclado

1. Añadir el handler en `window/handlers.rs`
2. Añadir el método de acción en `window/actions.rs`
3. Registrar en la ayuda de atajos (`shortcut_help.rs`)

### Patrones para el borrow checker

**Problema**: `self.active_session().app().grid()` toma prestado todo `&self`.

**Solución**: Acceso directo a campos:
```rust
let active = self.active;
let grid = &self.sessions[active].app().grid();
```

## Desarrollo móvil

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

## Estilo de código

- **No usar `.unwrap()` en código que no sean tests** — usar `unwrap_or_else(|e| e.into_inner())` para locks
- **`cargo fmt --all` antes de cada commit**
- **Clippy debe pasar con `-D warnings`**
- **Cell es Clone, no Copy** — usar `.clone()` explícitamente
- **Leer antes de editar** — siempre usar `read_file` primero

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

## Pipeline CI/CD

| Trigger | Workflow | Acción |
|---------|----------|--------|
| Push a main / PR | `ci.yml` | fmt + clippy + test + build |
| Tag `v*` | `release-desktop.yml` | macOS .dmg + Linux .deb + Windows .zip |
| Tag `v*` | `release-mobile.yml` | Android .apk + iOS .ipa |

### Crear un release

```bash
git add -A
git commit -m "release: vX.Y.Z"
git tag vX.Y.Z
git push origin main --tags
```

## Depuración

```bash
ggterm -v     # info
ggterm -vv    # debug
ggterm -vvv   # trace
```

Pulsa `F1` para el debug overlay, `Ctrl+Shift+G` para el performance monitor.

## Desarrollo FFI

### Añadir una nueva función C-ABI

1. Declarar en `crates/ggterm-ffi/src/lib.rs` o `transport.rs`
2. Implementar (usar `unwrap_or_else(|e| e.into_inner())` para locks)
3. Añadir el binding Dart en `mobile/lib/ffi/ffi_bindings.dart`
4. Actualizar el header C en `mobile/ios/RustLib/ggterm_ffi.h`

## Desarrollo de plugins

### Plugin Lua

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

## Problemas comunes

- **"module not found"**: Asegurarse de que el módulo esté declarado en `lib.rs`
- **Clippy let chains**: Solo se soporta `&&`, nunca `||`
- **Renderizado de fuentes**: Menlo Bold carece de caracteres box-drawing; usar siempre Weight::NORMAL

## Ubicaciones de archivos clave

| Qué | Dónde |
|-----|-------|
| Protocolos de terminal | `crates/ggterm-core/src/term/mod.rs` |
| VTE parser | `crates/ggterm-core/src/vte/parser.rs` |
| Grid model | `crates/ggterm-core/src/grid/mod.rs` |
| Temas | `crates/ggterm-render/src/theme.rs` |
| GPU pipeline | `crates/ggterm-render-wgpu/src/lib.rs` |
| DesktopApp | `crates/ggterm-app/src/window/mod.rs` |
| Event handlers | `crates/ggterm-app/src/window/handlers.rs` |
| Sistema de configuración | `crates/ggterm-app/src/config.rs` |
| Funciones FFI | `crates/ggterm-ffi/src/lib.rs` |
| Entry point CLI | `crates/ggterm-app/src/bin/ggterm.rs` |

## Contribuir

1. Hacer un fork del repositorio
2. Crear una rama de feature
3. Ejecutar `make fmt && make clippy && make test`
4. Hacer commit con mensajes convencionales (`feat:`, `fix:`, `docs:`)
5. Crear un Pull Request

### Checklist del Pull Request

- [ ] `cargo fmt --all -- --check` pasa
- [ ] `cargo clippy --features "..." --workspace -- -D warnings` pasa
- [ ] `cargo test --features "..." --workspace` pasa
- [ ] No hay nuevos `.unwrap()` en código que no sean tests
