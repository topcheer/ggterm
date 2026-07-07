# GGTerm

A GPU-accelerated, AI-native, cross-platform terminal emulator built in Rust.

[![CI](https://github.com/topcheer/ggterm/actions/workflows/ci.yml/badge.svg)](https://github.com/topcheer/ggterm/actions/workflows/ci.yml)
[![Release (Desktop)](https://github.com/topcheer/ggterm/actions/workflows/release-desktop.yml/badge.svg)](https://github.com/topcheer/ggterm/actions/workflows/release-desktop.yml)
[![Release (Mobile)](https://github.com/topcheer/ggterm/actions/workflows/release-mobile.yml/badge.svg)](https://github.com/topcheer/ggterm/actions/workflows/release-mobile.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-stable-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows%20%7C%20iOS%20%7C%20Android-lightgrey.svg)](#download)

> **2094+ tests passing** | 9 crates | ~70,000 lines of Rust

## Download

Pre-built binaries are available on the [Releases page](https://github.com/topcheer/ggterm/releases).

| Platform | Format | Architecture |
|----------|--------|-------------|
| macOS | `.dmg` (universal) | Apple Silicon + Intel |
| Linux | `.deb` / `.AppImage` | x86_64 |
| Windows | `.zip` / `.msi` | x86_64 |
| Android | `.apk` | arm64-v8a |
| iOS | `.ipa` | arm64 (Apple Silicon simulator) |

## Quick Start

### Build from source

```bash
git clone https://github.com/topcheer/ggterm.git
cd ggterm

# Desktop — build and run
cargo run --features "desktop ai plugin plugin-lua config-watch"

# Or use the Makefile
make build    # debug build
make release  # optimized build
make run      # build + run
```

### Binary CLI

```bash
# Default terminal
ggterm

# Custom size, shell, theme
ggterm --cols 120 --rows 40 --shell /bin/zsh --theme solarized-dark --font-size 15

# Verbose logging
ggterm -v
```

## Goals

- **Fast**: wgpu GPU rendering, damage-only updates, zero-copy VTE parsing
- **AI-native**: shell integration (OSC 133), command blocks, AI suggestions
- **Cross-platform**: macOS, Linux, Windows (desktop) + iOS, Android (mobile via SSH)
- **Configurable**: TOML config with hot-reload, 9 built-in themes, custom keybindings
- **Extensible**: Lua + WASM plugin system with I/O hooks
- **Production-ready**: session persistence, profiles, multi-pane splits, clipboard, search

## Architecture

```
Platform Shell (wgpu / Flutter Mobile)
         |
   App Layer (Events, Tabs, Splits, Themes, AI Bridge, Config)
         |
Terminal Core (VTE Parser, Grid, PTY, Input Encoding)
         |
Platform Abstraction (POSIX PTY / ConPTY / SSH Transport)
```

### Crate Structure

| Crate | LOC | Description |
|-------|-----|-------------|
| `ggterm-core` | ~8,000 | VTE parser, Grid model, Terminal state machine, PTY, transport trait |
| `ggterm-render` | ~1,500 | Renderer trait, ConsoleRenderer (ANSI), Theme system (9 themes) |
| `ggterm-render-wgpu` | ~3,500 | GPU renderer using wgpu + glyphon, SDF UI shaders |
| `ggterm-ai` | ~2,000 | AI engine: context extraction, prompt templates, LLM streaming client |
| `ggterm-app` | ~35,000 | Desktop app: winit event loop, window/tabs/splits, overlays, mouse, config |
| `ggterm-ffi` | ~1,500 | C-ABI FFI bindings for Flutter mobile integration |
| `ggterm-ssh` | ~1,000 | SSH transport via russh (password + key auth) |
| `ggterm-plugin` | ~3,000 | Lua 5.4 + WASM plugin runtime with hook dispatch |

For detailed design docs, see:
- [Architecture & Command Navigation](docs/command-nav.md)
- [Configuration Reference](docs/config.md)

## Features

### Terminal Core
- Full VTE parser (CSI, OSC, DCS, escape sequences, charsets)
- Grid model with scrollback, CJK/emoji wide chars, combining marks
- Alt-screen support (DECSET 47/1047/1049) with grid swap + cursor save
- SGR attributes: bold, dim, italic, underline, strikethrough, blink, hidden
- OSC 8 hyperlinks with visual rendering (blue + underline)
- Dynamic colors via OSC 10/11/12 (fg/bg/cursor)
- DECSCA/DECSED selective erase with protected cells
- Synchronized output (DECSET 2026) and text reflow (DECSET 2027)
- SGR pixel mouse mode (DECSET 1016)

### Desktop UI
- **GPU Rendering**: wgpu + glyphon with DPI-aware per-run grid alignment
- **Multi-Tab**: Ctrl+T/W, Alt+1-9, Ctrl+Tab, drag-to-reorder, tab rename
- **Multi-Pane Splits**: horizontal/vertical splits, drag-to-resize, pane zoom (Ctrl+Shift+Z)
- **9 Themes**: dark, light, dracula, solarized-dark/light, gruvbox, nord, tokyo-night, catppuccin-mocha
- **Search**: floating search bar with case toggle + match highlighting (Ctrl+Shift+F)
- **Clipboard**: Ctrl+Shift+V paste, OSC 52 sync, middle-click paste, select-to-copy
- **Font Zoom**: Ctrl+=/-/0 or Ctrl+Shift+Wheel (VS Code style)
- **Background Opacity**: configurable transparency (Ctrl+Shift+Alt+[/])
- **Settings Panel**: live config editing with validation (Ctrl+,)
- **Command Palette**: fuzzy search 30+ commands (Ctrl+Shift+P)
- **Status Bar**: cursor pos, cwd, profile, broadcast mode, bell indicator (Ctrl+Shift+B)
- **Notifications**: OSC 9/777 desktop notifications + bell sound (Ctrl+Shift+M)
- **Session Persistence**: save/restore tab+pane layout (opt-in via config)

### AI Integration
- Shell integration auto-injection (OSC 133) for bash/zsh/fish
- AI assistant overlay: explain, suggest, error help, natural-language-to-command
- OpenAI-compatible streaming LLM client
- Command block navigation (Ctrl+Shift+Up/Down)
- Command history sidebar (Ctrl+Shift+Y)

### Mobile (Flutter)
- Cross-compiled Rust FFI (C-ABI) with dart:ffi bindings
- SSH remote terminal (password + key authentication)
- Local shell on Android (proot) and iOS (proot-distro)
- Echo transport for testing without SSH server
- 60fps adaptive render loop with RGB cell rendering + cursor block
- Custom keyboard bar with Ctrl/Alt/Shift/Esc/Tab/Arrow keys
- Pinch-to-zoom font size, two-finger scroll scrollback
- Double-tap word select, triple-tap line select
- Long-press menu: copy word/line, paste, open URL, send Tab
- Screen wakelock (stays awake during active sessions)
- Connection history with swipe-to-delete + quick reconnect
- 9 built-in themes (persisted across restarts)
- Scrollbar indicator showing scrollback position
- Command history in input bar (Up/Down arrow navigation)
- Form auto-focus with Next/Send keyboard actions
- Connect elapsed timer ("Connecting… 3s")

### Plugin System
- Lua 5.4 runtime with hooks (input, output, render, command)
- WASM plugin runtime (via Wasmoth)
- Plugin manager: lifecycle, permissions, loading/unloading
- Config-driven activation (`[plugins]` in config.toml)

## Configuration

GGTerm reads `~/.ggterm/config.toml` (or `%USERPROFILE%\.ggterm\config.toml` on Windows)
with hot-reload support. See [`config.example.toml`](config.example.toml) for all options.

```toml
[appearance]
theme = "dark"               # 9 built-in themes
font_family = "monospace"
font_size = 14
cell_width = 8
cell_height = 16
background_opacity = 1.0     # 0.0 (transparent) to 1.0 (opaque)
padding = 8                  # content padding in pixels

[terminal]
scrollback_lines = 10000
shell = "/bin/zsh"
bell_mode = "visual"         # none | visual | sound
restore_session = false      # restore tabs/splits from last session

[ai]
enabled = false
api_endpoint = "https://api.openai.com/v1"
model = "gpt-4o-mini"

[plugins]
enabled = false
directory = "~/.ggterm/plugins"

[keybindings]
# All customizable — see "Custom Keybindings" below
new_tab = "Ctrl+T"
close_tab = "Ctrl+W"
paste = "Ctrl+Shift+V"
# ...
```

### Themes

| Theme | Description |
|-------|-------------|
| `dark` | Default dark theme |
| `light` | Light theme |
| `dracula` | Dracula color scheme |
| `solarized-dark` | Solarized dark palette |
| `solarized-light` | Solarized light palette |
| `gruvbox` | Gruvbox community theme |
| `nord` | Nord color palette |
| `tokyo-night` | Tokyo Night theme |
| `catppuccin-mocha` | Catppuccin Mocha flavor |

## Keyboard Shortcuts

> Shortcuts marked (*) are customizable via `[keybindings]` in config.toml.

### Tab & Pane Management

| Shortcut | Action |
|----------|--------|
| `Ctrl+T` | New tab (*) |
| `Ctrl+W` | Close tab |
| `Alt+1-9` | Switch to tab N |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Cycle tabs |
| `Ctrl+Shift+PageUp/Down` | Reorder tab left/right |
| `Ctrl+Shift+D` | Split horizontal |
| `Ctrl+Shift+\` | Split vertical |
| `Ctrl+Shift+[` / `]` | Focus prev/next pane |
| `Ctrl+Shift+X` | Swap active pane with next |
| `Ctrl+Shift+Z` | Toggle pane zoom |
| `Ctrl+Shift+Alt+Arrows` | Adjust split ratio |
| `Alt+H/J/K/L` | Vim-style pane navigation |
| `Drag separator` | Resize split |

### Terminal & Clipboard

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+C` | Copy selection (*) |
| `Ctrl+Shift+V` | Paste from clipboard (*) |
| `Ctrl+Shift+K` | Clear screen + scrollback (*) |
| `Ctrl+Shift+R` | Reset terminal (RIS) (*) |
| `Ctrl+Shift+U` | Open URL at cursor |
| `Ctrl+Click` | Open file path / URL |
| `Shift+Insert` | Paste (cross-platform) |
| `Alt+Drag` | Block/rectangular selection |

### Font & Display

| Shortcut | Action |
|----------|--------|
| `Ctrl+=` / `Ctrl+-` / `Ctrl+0` | Zoom in / out / reset (*) |
| `Ctrl+Shift+Wheel` | Mouse wheel font zoom |
| `Ctrl+Shift+Alt+[` / `]` | Opacity down / up |
| `Ctrl+Shift+T` | Cycle themes (*) |
| `F11` | Toggle fullscreen (*) |
| `Ctrl+Shift+Enter` | Toggle maximized |

### Search & Navigation

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+F` | Toggle search bar (*) |
| `Ctrl+Shift+Space` | Scrollback browse mode (j/k/G/g/d/u/q) |
| `Ctrl+Shift+Up/Down` | Navigate command blocks |
| `Ctrl+Shift+Alt+Up` | Scroll to mark (OSC 1337) |
| `Ctrl+Shift+Alt+S` | Export scrollback to file |
| `Ctrl+Shift+End` | Scroll to bottom |

### AI & Productivity

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+E/S/H/N` | AI explain / suggest / help / nl2cmd |
| `Ctrl+Shift+P` | Command palette |
| `Ctrl+Shift+/` | Keyboard shortcut help |
| `Ctrl+Shift+G` | Toggle perf monitor |
| `Ctrl+Shift+M` | Toggle sound |
| `Ctrl+Shift+B` | Toggle status bar |
| `Ctrl+Shift+Y` | Toggle command history |
| `Ctrl+,` | Toggle settings panel |
| `Ctrl+Shift+Alt+B` | Cycle broadcast mode |
| `Ctrl+Shift+Alt+N` | Reset to single pane |
| `Ctrl+Shift+Alt+F` | Cycle profiles |
| `Ctrl+Shift+Alt+P` | Copy current working directory |
| `Ctrl+Shift+Alt+E` | Export config to clipboard |
| `Ctrl+Shift+J` | Edit shell config (.bashrc/.zshrc) |
| `Ctrl+Shift+Alt+H` | Copy selection as HTML (with colors) |
| `Ctrl+Shift+,` | Open GGTerm config file |
| `Ctrl+Shift+U` | Open URL at cursor |
| `Ctrl+Shift+X` | Swap pane content |
| `Ctrl+Shift+PageUp/Down` | Reorder tabs |

### Custom Keybindings

```toml
[keybindings]
paste = "Ctrl+Shift+Insert"    # Remap paste
new_tab = "Ctrl+N"             # Remap new tab
fullscreen = "F2"              # Use F2 for fullscreen
```

**Format:** `"Modifier+Key"` (modifiers: Ctrl, Shift, Alt; key: A-Z, 0-9, punctuation, F1-F24)

## Building

```bash
# Quick build (debug)
make build

# Optimized release
make release

# Run tests
make test

# Lint
make clippy
make fmt

# Platform packaging
make macos       # .app bundle
make linux       # .deb package
make appimage    # AppImage
make windows     # .msi installer
```

## Development

### Project Structure

```
ggterm/
  crates/
    ggterm-core/         # Terminal engine (VTE, Grid, PTY)
    ggterm-render/       # Rendering trait + theme system
    ggterm-render-wgpu/  # GPU renderer (wgpu + glyphon)
    ggterm-ai/           # AI engine + LLM client
    ggterm-app/          # Desktop app (winit + window + tabs + splits)
    ggterm-ffi/          # Mobile FFI (C-ABI for Flutter)
    ggterm-ssh/          # SSH transport (russh)
    ggterm-plugin/       # Lua + WASM plugins
  mobile/                # Flutter mobile app (iOS + Android)
  assets/                # Icons, .desktop, Info.plist
  shell/                 # Shell integration scripts (bash/zsh/fish)
  docs/                  # Design docs
  scripts/               # Release build scripts
```

### CI/CD

| Workflow | Trigger | Purpose |
|----------|---------|---------|
| [`ci.yml`](.github/workflows/ci.yml) | push/PR | fmt, clippy, test, build (3 platforms), FFI test, Flutter analyze |
| [`release-desktop.yml`](.github/workflows/release-desktop.yml) | tag `v*` | Build macOS/Linux/Windows binaries + GitHub Release |
| [`release-mobile.yml`](.github/workflows/release-mobile.yml) | tag `v*` | Build Android APK + iOS IPA with Rust FFI cross-compile |

### Conventions

- No `unwrap()` in production code — use `expect()` with context or proper error handling
- Run `cargo fmt --all` before every commit
- Commit messages: `feat:` / `fix:` / `refactor:` / `docs:` / `test:` / `chore:`
- Always include `Co-Authored-By: ggcode <noreply@ggcode.dev>` trailer

## License

MIT OR Apache-2.0
