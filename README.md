# GGTerm

A GPU-accelerated, AI-native, cross-platform terminal emulator built in Rust.

## Goals

- **Fast**: wgpu GPU rendering, damage-only updates, zero-copy parsing
- **AI-native**: shell integration (OSC 133), command blocks, AI suggestions
- **Cross-platform**: macOS, Linux, Windows (desktop) + iOS, Android (mobile)
- **Extensible**: WASM + Lua plugin system
- **Customizable**: Multiple themes (dark, light, dracula) + multi-tab support

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

## Status

| Phase | Description | Status |
|-------|-------------|--------|
| 1 | Terminal Core (VTE, Grid, PTY, Rendering) | ✅ Complete |
| 2 | VT Compatibility (alt screen, CSI, charsets, scroll) | ✅ Complete |
| 3 | Shell Integration (OSC 133, CommandBlock) | ✅ Complete |
| 4 | AI Engine (context, prompts, LLM client) | ✅ Complete |
| 5 | Modern UI (themes, tabs, AI bridge) | ✅ Complete |
| 6 | Plugin System (WASM + Lua) | Planned |
| 7 | Mobile (Flutter + SSH) | Planned |
| 8 | Production (packaging, CI, docs) | Planned |

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

## Building

```bash
# Headless (no GPU required)
cargo build

# Desktop (wgpu GPU rendering)
cargo build --features desktop

# With AI integration
cargo build --features ai

# All features
cargo build --features "desktop ai"

# Run tests
cargo test --features "desktop ai" --workspace
```

## License

MIT OR Apache-2.0
