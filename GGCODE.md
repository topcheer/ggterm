# GGCODE.md

> Durable project guidance for AI coding agents working on GGTerm.
> Verify and update when conventions change.

## Project Overview

**GGTerm** is a GPU-accelerated, AI-native, cross-platform terminal emulator written in Rust. It features a multi-tab, multi-pane desktop application (winit + wgpu) and a mobile FFI bridge for Flutter integration (iOS/Android).

- **Language:** Rust (edition 2024)
- **Toolchain:** stable channel (rustfmt + clippy required)
- **Repository:** https://github.com/topcheer/ggterm
- **LOC:** ~56,000 lines across 9 crates

## Workspace Layout

```
crates/
  ggterm-core/          — VTE parser, grid model, terminal state, PTY transport trait
  ggterm-render/        — Render trait, themes, cursor state, console fallback
  ggterm-render-wgpu/   — wgpu GPU renderer (glyphon text + SDF UI shaders + decorations)
  ggterm-app/           — Desktop app: winit event loop, window, handlers, config, splits, tabs
  ggterm-ai/            — AI engine bridge (OpenAI-compatible API client)
  ggterm-plugin/        — Plugin manager (Lua + WASM runtimes)
  ggterm-ssh/           — SSH transport (russh 0.61, async→sync bridge)
  ggterm-ffi/           — C-ABI FFI for Flutter/mobile (session lifecycle, transport pump)
mobile/                 — Flutter app (Dart FFI bindings, iOS/Android)
shell/                  — Shell integration scripts (bash/zsh/fish OSC 133)
docs/                   — Architecture notes
examples/               — Config example, plugin examples
assets/                 — Logo, icons, desktop files
```

## Feature Flags

Desktop features (ggterm-app):
- `desktop` — winit window + wgpu rendering + PTY (required for binary)
- `ai` — AI assistant integration
- `plugin` — Plugin manager framework
- `plugin-lua` — Lua plugin runtime (implies `plugin`)
- `plugin-wasm` — WASM plugin runtime (implies `plugin`)
- `config-watch` — Hot-reload config via filesystem watching

FFI features (ggterm-ffi):
- `ssh` — SSH transport support for mobile
- `ai` — AI engine for mobile

**Standard desktop feature set:** `desktop ai plugin plugin-lua config-watch`

## Validation Commands

```bash
# Build (debug)
make build                          # cargo build --features "$(TAGS)" --bin ggterm

# Release build
make release                        # cargo build --release --features "$(TAGS)" --bin ggterm

# Run all tests
make test                           # cargo test --features "$(TAGS)" --workspace

# Lint (must be zero warnings)
make clippy                         # cargo clippy --features "$(TAGS)" --workspace -- -D warnings

# Format check
make fmt                            # cargo fmt --all -- --check

# Format fix
make fmt-fix                        # cargo fmt --all

# Test specific crate
cargo test -p ggterm-core --lib
cargo test --features "desktop ai plugin plugin-lua config-watch" -p ggterm-app --lib
cargo test -p ggterm-ffi --features ssh --lib

# Full workspace test (all features including ssh)
cargo test --features "desktop ai plugin plugin-lua config-watch ssh" --workspace --lib
```

**TAGS** = `desktop,ai,plugin,plugin-lua,config-watch`

## Architecture

### DesktopApp (ggterm-app)
- `DesktopApp` is the main application struct with `sessions: Vec<TabSession>` + `active: usize`
- Each `TabSession` contains `panes: Vec<Option<PaneSession>>` + `SplitTree` for split layout
- Each `PaneSession` wraps an `App` (terminal + grid) + `PtySession` + metadata
- Window code split into `window/{mod, handlers, actions, render}.rs`
- Multi-pane rendering uses `PaneRenderSpec` with per-pane renderer state (reverse_video, dynamic colors, underline_color)
- Event loop: `about_to_wait()` pumps PTY → processes bytes → flushes terminal responses → checks dirty/bell/blink → conditionally redraws

### Terminal Core (ggterm-core)
- `Terminal` struct owns: grid, cursor, modes, SGR state, response_buffer, command_marks
- `Grid` owns: rows, scrollback, scroll region, dirty flags, display_offset
- `Cell` is Clone (not Copy) due to `hyperlink: Option<String>` and `combining: Vec<char>`
- VTE parser is a Paul Williams state machine in `vte/parser.rs`
- Protocol responses (DA1, DSR, DECRQM, XTVERSION, OSC 4, ENQ) stored in `response_buffer`, flushed via `TabSession::flush_responses()`

### Renderer (ggterm-render-wgpu)
- `GlyphonRenderer` uses glyphon for text shaping + wgpu for GPU rendering
- Per-run grid alignment: each text run positioned at exact column (start_col × cell_width)
- SDF shaders: `shaders/ui.wgsl` for rounded rectangles (tab bar, status bar, dialogs)
- Decoration pipeline: underlines + strikethroughs with configurable color (SGR 58)
- `row_to_runs()` converts grid rows to glyphon TextAreas with theme color resolution
- Blink text (SGR 5) uses shared cursor blink phase
- `render_multi_pane_frame()`: one render pass, scissor per pane, overlay pass last

### Config (ggterm-app/src/config.rs)
- TOML format at `~/.ggterm/config.toml`
- `ConfigManager`: load, reload (hot via `notify`), validate, export/import TOML
- Sections: `[appearance]`, `[terminal]`, `[ai]`, `[keybindings]`, `[profiles]`
- Validation: font_size 6-32, cell dimensions 4-32, theme name whitelist

### Mobile FFI (ggterm-ffi)
- C-ABI functions for session lifecycle, byte processing, cell reading
- Global session registry via `OnceLock<Mutex<HashMap<u32, MobileSession>>>`
- Mutex locks use `unwrap_or_else(|e| e.into_inner())` for panic safety
- Flutter bindings in `mobile/lib/ffi/` (dart:ffi direct, no flutter_rust_bridge)

## Key Conventions

1. **Before editing a file, always `read_file` first** — copy exact lines into `old_text` for edits
2. **Never use `.unwrap()` in non-test code** — use `unwrap_or_else(|e| e.into_inner())` for locks, `if let Some()` for options
3. **Clippy must pass with `-D warnings`** — zero warnings allowed
4. **`cargo fmt --all` before every commit**
5. **SGR/CSI/OSC handlers go in `term/mod.rs`** — the Terminal struct's `csi()`, `esc()`, `osc()` methods
6. **Renderer changes need both `converter.rs` (text) and `lib.rs` (GPU pipeline)**
7. **Multi-pane rendering: each pane gets its own `PaneRenderSpec` with per-pane terminal state**
8. **Protocol responses must be flushed**: `TabSession::flush_responses()` in `about_to_wait()`
9. **Cell is Clone not Copy** — grid operations use `clone()` not copy semantics
10. **VTE parser must consume string sequences** (DCS/SOS/PM/APC) — never print their payloads

## Config File

Location: `~/.ggterm/config.toml` (see `config.example.toml` for all options)

Key settings:
- `appearance.theme`: dark, light, dracula, solarized-dark, solarized-light, gruvbox, nord, tokyo-night, catppuccin-mocha
- `terminal.scrollback_lines`: max scrollback history (default 10000)
- `terminal.restore_session`: restore tabs/splits on startup (default false)
- `ai.enabled`: enable AI features
- `[keybindings]`: customizable keyboard shortcuts

## Testing

- ~2300 tests across all crates (580+ in core, 1282 in app, 65+ in FFI)
- Core tests: terminal protocol (CSI, OSC, SGR), grid operations, VTE parsing
- App tests: config, splits, tab bar, command palette, snippets, broadcast, search
- FFI tests: session lifecycle, echo transport, multi-session, cursor, bell, combining chars, DECCKM
- Env-var tests in ggterm-ai use a shared Mutex to prevent parallel races

## Commit Convention

```
feat: <description>
fix: <description>
refactor: <description>
docs: <description>
improve: <description>
test: <description>
perf: <description>
```

Always append: `Co-Authored-By: ggcode <noreply@ggcode.dev>`

## Current Phase Status

Active development through Phase 55+. Recent work focused on:
- Mobile DECCKM support: arrow keys/Home/End respect application cursor mode for vim/less/htop
- Mobile CJK word selection: Unicode property classes in double-tap regex
- Mobile selection text: trailing whitespace stripping per line
- Desktop search bar UX: red border on no-match, corrected hint text
- Desktop first-run: auto-creates documented config.toml on first launch
- Desktop dead code: StatusBar 14 dead fields + 6 cached fields removed
- Desktop performance: grid clone eliminated in search refresh, log_enabled! guard
- CJK correctness: char count (not byte count) in all copy/paste toasts
- Combining character correctness across all text paths (renderer, FFI, extraction, export)
- CLI flags: -e, --hold, --config, --fullscreen, --maximize
- Shell integration OSC 7 CWD tracking (bash/zsh/fish)
- Code quality: no panic-prone unwrap() in non-test code, helper function extraction
