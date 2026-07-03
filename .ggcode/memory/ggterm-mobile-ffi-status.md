## GGTerm Mobile FFI Bridge — COMPLETE (e817a3c)

### Build State
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo clippy -p ggterm-ffi --features ssh -- -D warnings` = CLEAN
- `cargo test -p ggterm-ffi --features ssh --lib` = **49 tests ALL PASS**
- `cargo fmt --all -- --check` = CLEAN

### What Was Done

**Rust FFI (crates/ggterm-ffi/src/transport.rs — NEW):**
- Session lifecycle: `ggterm_session_create(cols, rows)` → session ID, `ggterm_session_destroy(id)`
- Terminal operations: `session_process_bytes`, `session_send_input`, `session_take_input`, `session_read_cells`, `session_dimensions`, `session_cursor`, `session_resize`, `session_take_bell`
- Transport pump: `ggterm_transport_pump(id)` reads from transport → processes bytes → returns count
- Transport flush: `ggterm_transport_flush(id)` sends queued input to transport
- Transport status: `ggterm_transport_is_alive(id)` 
- SSH connect: `ggterm_ssh_connect(id, host, port, user, password)` + `ggterm_ssh_connect_key(id, host, port, user, key_path)` (feature-gated)
- Echo transport: `ggterm_echo_connect(id)` for testing without SSH server
- Error reporting: `ggterm_last_error()` returns C string
- Global session registry via `OnceLock<Mutex<HashMap<u32, MobileSession>>>`

**Dart FFI Bindings (mobile/lib/ffi/ — NEW):**
1. `types.dart` — GGTermCell TypedStruct, CellFlags bits, ColorCodec (pack/unpack), AnsiPalette (16-color resolution)
2. `ffi_bindings.dart` — GgtermFfi class maps all 18 C-ABI functions via dart:ffi lookupFunction
3. `session_manager.dart` — SessionManager wrapper with ScreenSnapshot, GGTermCellData, SshConnectionParams

**Flutter Integration (mobile/lib/ — REWRITTEN):**
- `main.dart` — SessionManager lifecycle, real SSH/echo connect flow
- `connection_screen.dart` — SSH form + Echo Test button
- `terminal_screen.dart` — 30fps Timer render loop, real cell data with RGB colors, cursor block rendering, auto-resize on layout change, transport status indicator (green/red dot), keyboard bar toggle
- `keyboard_bar.dart` — Fixed Ctrl-C/D/Z to send control characters (0x03/0x04/0x1A)
- `pubspec.yaml` — Replaced flutter_rust_bridge with `ffi: ^2.1.0`

### Architecture
```
Flutter (Dart)                     Rust (ggterm-ffi)
┌─────────────────┐                ┌──────────────────────┐
│ TerminalScreen  │ ──render────── │ SessionManager       │
│  (CustomPaint)  │                │  ┌─────────────────┐ │
│ SessionManager  │ ──dart:ffi───→ │  │ MobileSession   │ │
│  .pumpAndFlush  │                │  │  TerminalHandle │ │
│  .getSnapshot   │ ←──cells────── │  │  Transport      │ │
│  .sendInput     │ ──input──────→ │  │   (SSH/Echo)    │ │
│  .resize        │                │  └─────────────────┘ │
└─────────────────┘                └──────────────────────┘
```

### Key Design Decisions
1. **dart:ffi direct** instead of flutter_rust_bridge — simpler, no codegen, more portable
2. **Global session registry** in Rust via OnceLock<Mutex<HashMap>> — sessions persist across FFI calls
3. **Echo transport** — tests mobile app without SSH server (just echoes input back)
4. **CStr import** is cfg(feature="ssh")-gated because only SSH connect functions use CStr
5. **TerminalTransport** trait from ggterm-core is always available — no cfg gate needed on MobileSession.transport field

### Commit
- e817a3c — feat: mobile FFI bridge — transport layer + Dart bindings + Flutter integration

### Test Count
- 49 FFI tests (session lifecycle, pump, echo, multi-session, cursor, bell, resize)
- Total workspace: unchanged at 1750 lib tests

### Files Changed (10 files, +1621 lines)
- crates/ggterm-ffi/src/transport.rs (NEW — 600+ lines)
- crates/ggterm-ffi/src/lib.rs (+1 line — pub mod transport)
- mobile/lib/ffi/types.dart (NEW)
- mobile/lib/ffi/ffi_bindings.dart (NEW)
- mobile/lib/ffi/session_manager.dart (NEW)
- mobile/lib/main.dart (REWRITTEN)
- mobile/lib/connection_screen.dart (REWRITTEN)
- mobile/lib/terminal_screen.dart (REWRITTEN)
- mobile/lib/keyboard_bar.dart (FIXED)
- mobile/pubspec.yaml (UPDATED)