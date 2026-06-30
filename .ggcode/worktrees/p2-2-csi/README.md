# GGTerm

A GPU-accelerated, AI-native, cross-platform terminal emulator built in Rust.

## Goals

- **Fast**: wgpu GPU rendering, damage-only updates, zero-copy parsing
- **AI-native**: shell integration (OSC 133), command blocks, AI suggestions
- **Cross-platform**: macOS, Linux, Windows (desktop) + iOS, Android (mobile)
- **Extensible**: WASM + Lua plugin system

## Architecture

Core-Shell design: terminal logic in pure Rust, rendering decoupled.

```
Platform Shell (wgpu / Flutter)
    ↓
AI Engine (LLM, shell markers)
    ↓
Terminal Core (VTE, Grid, PTY)  ← this crate
    ↓
Platform Abstraction (ConPTY / POSIX)
```

## Status

Phase 1: Terminal Core (in progress)

## License

MIT OR Apache-2.0
