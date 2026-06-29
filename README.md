# GGTerm

A GPU-accelerated, AI-native, cross-platform terminal emulator built in Rust.

## Goals

- **Fast**: wgpu GPU rendering, damage-only updates, zero-copy parsing
- **AI-native**: shell integration (OSC 133), command blocks, AI suggestions
- **Cross-platform**: macOS, Linux, Windows (desktop) + iOS, Android (mobile)
- **Configurable**: TOML config with hot-reload, plugin system (Lua + WASM)
- **Extensible**: WASM + Lua plugin system, hook into all I/O
- **Customizable**: Multiple themes (dark, light, dracula) + multi-tab support
- **Runnable**: Standalone binary with CLI args, shell integration, mouse + keyboard

## Architecture

Core-Shell design: terminal logic in pure Rust, rendering decoupled.

```
Platform Shell (wgpu / Flutter)
    ↓
AI Engine (LLM, shell markers)
    ↓
App Layer (Events, Tabs, Themes, AI Bridge)
    ↓
Terminal Core (VTE, Grid, PTY)  ← ggterm-core crate
    ↓
Platform Abstraction (ConPTY / POSIX)
```

### Crate Structure

| Crate | Description |
|-------|-------------|
| `ggterm-core` | VTE parser, Grid model, Terminal state machine, PTY, CommandBlock |
| `ggterm-render` | Renderer trait, ConsoleRenderer (ANSI), Theme system |
| `ggterm-render-wgpu` | GPU renderer using wgpu + glyphon |
| `ggterm-ai` | AI engine: context extraction, prompt templates, LLM client |
| `ggterm-app` | App event loop, input encoding, tabs, themes, AI bridge, window |

## Features

### Phase 1-4: Core Terminal
- Full VTE parser (keyboard, mouse, paste, escape sequences)
- Grid model with scrollback, CJK/emoji support
- GPU-accelerated text rendering (wgpu + glyphon)
- PTY integration (portable-pty)
- Shell integration (OSC 133 command markers)
- AI engine (OpenAI-compatible streaming, explain/suggest/error-help/nl2cmd)

### Phase 5: Modern UI
- **Themes**: 3 built-in themes (Dark, Light, Dracula) with hot-swap
- **Tabs**: Multi-session terminal management with dirty tracking
- **AI Bridge**: Background AI requests without blocking the terminal
- **Extended Events**: Tab/theme/AI events in the main event loop

### Phase 6: Plugin System
- **Lua Runtime**: Lua 5.4 plugins with hooks (input, output, render, command)
- **WASM Runtime**: WebAssembly plugins via Wasmoth
- **Plugin Manager**: Lifecycle, permissions, loading/unloading

### Phase 8: Production
- **Config System**: TOML config (`~/.ggterm/config.toml`) with hot-reload
- **Config Watch**: File-system watcher for live config changes
- **Error Handling**: Unified error types via thiserror
- **Command Navigation**: OSC 133 block navigation with status bar

### Phase 9: Desktop Terminal
- **Binary CLI**: `ggterm` binary with clap (--cols, --rows, --shell, --theme, --font-size, -v)
- **Shell Integration Auto-Injection**: OSC 133 hooks auto-injected for bash/zsh/fish
- **Dynamic Window Title**: OSC 0/2 sequences set the window title
- **Mouse Support**: SGR mouse (1006), wheel scroll, click-drag selection, clipboard copy
- **Keyboard Refinement**: Full keymap (Ctrl/Alt/Shift/F-keys/nav keys, Ctrl+punctuation)
- **PTY Enhancement**: Env vars (GGTERM=1), spawn args, shell integration wiring
- **Config Startup Loading**: `~/.ggterm/config.toml` loaded on launch, CLI overrides config

## Status

| Phase | Description | Status |
|-------|-------------|--------|
| 1 | Terminal Core (VTE, Grid, PTY, Rendering) | Done |
| 2 | VT Compatibility (alt screen, CSI, charsets, scroll) | Done |
| 3 | Shell Integration (OSC 133, CommandBlock) | Done |
| 4 | AI Engine (context, prompts, LLM client) | Done |
| 5 | Modern UI (themes, tabs, AI bridge) | Done |
| 6 | Plugin System (WASM + Lua) | Done |
| 7 | Mobile (Flutter + SSH) | Planned |
| 8 | Production (config, docs, thiserror) | Done |
| 9 | Desktop Terminal (binary, mouse, keyboard, resize) | Done |

## Usage

### Headless (testing)
```rust
use ggterm_app::App;
use ggterm_app::event::AppEvent;

let (mut app, tx) = App::new(80, 24);
tx.send(AppEvent::PtyBytes(b"Hello World".to_vec())).unwrap();
app.pump();
assert!(app.output().contains("Hello World"));
```

### Desktop (GPU rendering)
```rust
use ggterm_app::window::{DesktopApp, DesktopConfig};

let config = DesktopConfig::default()
    .with_title("GGTerm")
    .with_size(120, 36);

DesktopApp::run(config).expect("failed to run terminal");
```

### AI Integration (feature = "ai")
```rust
use ggterm_app::ai_bridge::{AIBridge, AIRequest};
use ggterm_ai::{AIContext, Action};

let mut bridge = AIBridge::with_mock("This command lists files.");
let ctx = AIContext::from_terminal(&terminal);
bridge.request(AIRequest::new(Action::Explain, ctx));

// Poll for result in event loop
if let Some(response) = bridge.poll_result() {
    println!("AI: {:?}", response.result);
}
```

## Configuration

GGTerm reads settings from `~/.ggterm/config.toml` with hot-reload support.
See [`docs/config.md`](docs/config.md) for the full reference, or copy
[`config.example.toml`](config.example.toml) to get started.

```toml
[appearance]
theme = "dark"
font_size = 14

[terminal]
scrollback_lines = 10000
shell = "/bin/zsh"

[ai]
enabled = false
model = "gpt-4o-mini"
```

## Command Navigation

Jump between command blocks with keyboard shortcuts. GGTerm auto-injects
OSC 133 shell integration hooks when spawning shells — no manual setup needed.

For shells that need manual integration:

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+Up` | Previous command block |
| `Ctrl+Shift+Down` | Next command block |
| `Ctrl+Shift+H` | Toggle status bar |

See [`docs/command-nav.md`](docs/command-nav.md) for details.

## Binary Usage

```bash
# Build the binary
cargo build --features desktop

# Default 80x24 terminal
./target/debug/ggterm

# Custom dimensions and shell
./target/debug/ggterm --cols 120 --rows 40 --shell /bin/zsh

# Custom theme and font size
./target/debug/ggterm --theme solarized --font-size 15

# Show help
./target/debug/ggterm --help

# Verbose logging
./target/debug/ggterm -v
```

### CLI Options

| Option | Default | Description |
|--------|---------|-------------|
| `--cols <N>` | 80 | Initial terminal column count |
| `--rows <N>` | 24 | Initial terminal row count |
| `--shell <PATH>` | `$SHELL` | Shell program to execute |
| `--title <TITLE>` | "GGTerm" | Initial window title |
| `--theme <NAME>` | dark | Theme: dark, light, solarized |
| `--font-size <N>` | 14 | Font size in pixels |
| `-v` | — | Verbose logging (env_logger) |

CLI options override `~/.ggterm/config.toml` values.

## Building

```bash
# Headless (no GPU required)
cargo build

# Desktop (wgpu GPU rendering)
cargo build --features desktop

# With AI + plugins
cargo build --features "desktop ai plugin plugin-lua"

# All features
cargo build --features "desktop ai plugin plugin-lua plugin-wasm"

# Run the terminal!
cargo run --features desktop

# With CLI options
cargo run --features desktop -- --cols 120 --rows 40 --shell /bin/zsh

# Run tests (1151 tests with all features)
cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace
```

## Status

**1151 tests passing** (2 ignored PTY integration tests).

| Feature | Status | Tests |
|---------|--------|-------|
| VTE Parser | Done | 58 |
| Grid Model | Done | 116 |
| Terminal State Machine | Done | 136 |
| PTY Integration | Done | 16 |
| Renderer (Console + GPU) | Done | 49 |
| App + Events + Input | Done | 295 |
| Plugin System (Lua + WASM) | Done | 132 |
| Shell Integration (OSC 133) | Done | 12 |
| Command Navigation | Done | 32 |
| Config System (TOML + Hot-reload) | Done | 15 |
| Config File Watch | Done | 10 |
| Error Handling (thiserror) | Done | — |
| Binary CLI (clap) | Done | — |
| Shell Integration Auto-Injection | Done | 11 |
| Mouse Support (SGR + Selection) | Done | 23 |
| Keyboard Refinement | Done | 63 |
| PTY Enhancement (env + args) | Done | 4 |
| Config Startup Loading | Done | 7 |
| Resize Enhancement (debounce) | Done | 26 |

## License

MIT OR Apache-2.0
