## P9-D: Mouse Support — COMPLETE

### Final Stats
- **1107 tests** ALL PASS (desktop ai plugin plugin-lua config-watch), Clippy CLEAN
- +111 new tests (23 mouse.rs, 7 term/modes, 5 grid/viewport, rest existing)

### Files Changed
1. `crates/ggterm-core/src/term/mod.rs` — 6 mouse mode fields + DECSET handling + 5 accessors + 7 tests
2. `crates/ggterm-core/src/grid/mod.rs` — display_offset + viewport scroll + 5 tests
3. `crates/ggterm-app/src/mouse.rs` — NEW: SGR encoder + selection state (~430 lines, 23 tests)
4. `crates/ggterm-app/src/window.rs` — mouse event wiring + handlers + clipboard copy
5. `crates/ggterm-app/src/lib.rs` — pub mod mouse

### Key API
```rust
// Terminal mouse mode accessors
pub fn mouse_tracking_enabled(&self) -> bool      // any of 1000/1002/1003
pub fn mouse_sgr_enabled(&self) -> bool            // mode 1006
pub fn mouse_urxvt_enabled(&self) -> bool          // mode 1015
pub fn mouse_any_event_enabled(&self) -> bool      // mode 1003
pub fn mouse_button_event_enabled(&self) -> bool   // mode 1002

// Grid viewport scroll
pub fn scroll_up_viewport(&mut self, n: usize)     // towards older scrollback
pub fn scroll_down_viewport(&mut self, n: usize)   // towards active bottom
pub fn reset_viewport(&mut self)
pub fn display_row(&self, row: usize) -> Option<&Row>
pub fn display_cell(&self, col: usize, row: usize) -> Option<&Cell>

// Mouse encoding (mouse.rs)
encode_sgr_press/release/motion(MouseEvent) -> String
encode_legacy(MouseEvent) -> Option<Vec<u8>>
encode_urxvt(MouseEvent, pressed) -> String
encode_mouse_event(ev, sgr, urxvt, pressed) -> Option<Vec<u8>>
```

### Design
- Mouse tracking ON (1000/1002/1003): all events → PTY encoded as SGR/URXVT/legacy
- Mouse tracking OFF: wheel → scrollback viewport scroll, click-drag → text selection → clipboard copy
- display_offset auto-resets when new PTY output scrolls (in scroll_up())
- Clipboard: pbcopy on macOS, stub on other platforms