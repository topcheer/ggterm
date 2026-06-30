## GGTerm P10-B: Clipboard Integration — COMPLETE

### Build State
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace` = **1232 tests ALL PASS** (2 ignored)

### P10-B: Clipboard Integration (gg_dev)

**Files changed:**
1. `crates/ggterm-app/src/clipboard.rs` (NEW) — read_clipboard() (pbpaste), set_clipboard_bytes() (pbcopy), bracket_paste()
2. `crates/ggterm-core/src/term/mod.rs` — OSC 52 handler in osc(), decode_base64(), pending_clipboard_set field, take_pending_clipboard_set() accessor, bracketed_paste() accessor
3. `crates/ggterm-app/src/window.rs`:
   - `paste_from_clipboard()` — reads clipboard, wraps with bracketed paste markers, writes to PTY
   - `poll_osc52_clipboard()` — polls pending OSC 52 clipboard set in about_to_wait
   - Ctrl+Shift+V handler in keyboard input
   - Middle-click paste in mouse handler
   - Refactored copy_selection_to_clipboard to use clipboard::set_clipboard_bytes()

**15 new tests:**
- OSC 52 parsing: 5 tests (BEL/ST terminated, empty=clear, no selector, take clears)
- Base64 decode: 3 tests (basic, empty, padding)
- bracketed_paste accessor: 1 test
- clipboard.rs: 5 tests (bracket_paste variants, read/write doesn't panic)

### Key API
```rust
// Terminal (ggterm-core)
pub fn take_pending_clipboard_set(&mut self) -> Option<Vec<u8>>
pub fn bracketed_paste(&self) -> bool
fn decode_base64(input: &str) -> Option<Vec<u8>>

// Clipboard module (ggterm-app)
pub fn read_clipboard() -> Option<String>
pub fn set_clipboard_bytes(data: &[u8])
pub fn bracket_paste(text: &str, bracketed: bool) -> Vec<u8>
```

### OSC 52 Design
- Programs send `OSC 52 ; c ; <base64> ST` → Terminal decodes base64 → stores in `pending_clipboard_set`
- Event loop `about_to_wait()` calls `poll_osc52_clipboard()` → applies to system clipboard via pbcopy
- Empty payload = clear clipboard
- Query (read) not implemented yet (security: requires user confirmation)