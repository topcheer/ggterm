# Phase 5: Modern UI — Design Document

## Overview

Phase 5 adds **Modern UI features** to GGTerm: themes, tab management, and AI
integration. These features transform GGTerm from a single-session terminal
into a multi-functional, customizable terminal emulator.

## Tasks Completed

| Task | Module | Tests | Description |
|------|--------|-------|-------------|
| P5-A | `render/theme.rs` + `app/theme.rs` | 38 | Theme system with 3 built-in themes + hot-swap |
| P5-B | `app/tabs.rs` | 40 | Multi-session tab management |
| P5-C | `app/ai_bridge.rs` | 22 | AI engine integration (feature-gated) |
| P5-D | `app/event.rs` + `app.rs` | 22 | Extended events + app-level integration |
| **Total** | | **~122 new tests** | |

## Architecture

### P5-A: Theme System

**Files:**
- `crates/ggterm-render/src/theme.rs` — Extended `RenderTheme` + `ThemeManager`
- `crates/ggterm-app/src/theme.rs` — `AppTheme` with callback support

**Design:**
- `RenderTheme` struct holds: default fg/bg, cursor fg/bg/style, 16-color palette, selection bg
- 3 built-in themes: `dark_default()`, `light_default()`, `dracula()`
- `by_name()` class-method for lookup (case-insensitive)
- `ThemeManager` tracks current theme + name, provides `set_by_name()`
- `AppTheme` wraps `ThemeManager` with `on_change` callback and `cycle_next()`

**Theme resolution pipeline:**
```
Cell color → Theme.resolve_fg/bg() → (u8, u8, u8) RGB
  Color::Default → theme.default_fg/bg
  Color::Indexed(0-15) → theme.palette[n]
  Color::Indexed(16-231) → 6x6x6 color cube
  Color::Indexed(232-255) → grayscale ramp
  Color::Rgb(r,g,b) → passthrough
```

### P5-B: Tab Management

**File:** `crates/ggterm-app/src/tabs.rs`

**Design:**
- `TabManager` is a pure state container (no Terminal/PTY ownership)
- Each tab tracked as `TabInfo { id, title, dirty, cols, rows }`
- Operations: `open_tab()`, `close_tab(idx)`, `switch_tab(idx)`, `next_tab()`, `prev_tab()`
- Dirty flag tracks background tabs with unseen output
- Tab IDs are monotonic (never reused)
- Configurable `max_tabs` (default 10)

**Why pure state container?**
The actual Terminal/PtySession instances are managed at the DesktopApp level
(window.rs). TabManager provides the bookkeeping that both the app and the
desktop layer share. This makes it trivially testable without PTY.

### P5-C: AI Bridge

**File:** `crates/ggterm-app/src/ai_bridge.rs` (feature = "ai")

**Design:**
- `AIBridge` owns an `AIEngine` (from ggterm-ai crate)
- On `request()`, engine ownership transfers to a background thread
- Thread executes the AI request and sends `(engine, response)` back via mpsc
- `poll_result()` returns the response and restores engine ownership
- Only one request in-flight at a time (`is_busy()` guard)
- Supports all 4 actions: Explain, Suggest, ErrorHelp, NL2Command

**Why engine ownership transfer?**
`AIEngine` owns `Box<dyn LLMProvider>` which cannot be cloned. Instead of
adding `Arc<Mutex<>>` complexity, we transfer ownership to the worker thread
and receive it back with the result. The main thread can't execute requests
while one is pending, which is the desired behavior (show "thinking..." UI).

### P5-D: Extended Events

**File:** `crates/ggterm-app/src/event.rs`

New event variants:
```
AppEvent::NewTab           // Open a new tab
AppEvent::CloseTab(Option) // Close tab by index or active
AppEvent::SwitchTab(usize) // Switch to tab index
AppEvent::NextTab          // Next tab (wraps)
AppEvent::PrevTab          // Previous tab (wraps)
AppEvent::SetTheme(String) // Set theme by name
AppEvent::CycleTheme       // Cycle to next theme
AppEvent::AIRequest(Action) // Request AI action [ai feature]
AppEvent::AIResponse(String) // AI response arrived [ai feature]
AppEvent::AIError(String)    // AI request failed [ai feature]
```

All new events are handled in `App::handle_event()`. The App struct now
includes `theme: AppTheme`, `tabs: TabManager`, and optionally
`last_ai_response: Option<String>`.

## Feature Flags

| Flag | Dependencies | Enabled Modules |
|------|-------------|-----------------|
| (default) | ggterm-core, ggterm-render | app, event, input, tabs, theme, command_nav |
| `desktop` | + winit, wgpu, portable-pty | + gpu, keymap, window |
| `ai` | + ggterm-ai | + ai_bridge |

## Test Coverage

- **767 tests total** across the workspace (up from 620 in Phase 4)
- All tests pass with `-race` (no data races)
- Zero warnings in new code

## Future Work

- **P5-F**: Render tab bar in window.rs (visual tab bar at top of terminal)
- **P5-G**: Keyboard shortcuts (Ctrl+T new tab, Ctrl+W close, Alt+1-9 switch)
- **P5-H**: Theme file loading (TOML/JSON config files)
- **P5-I**: AI overlay rendering (show AI response in a popup panel)
