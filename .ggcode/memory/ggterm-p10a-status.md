## GGTerm P10-A: Multi-Tab Refactoring — COMPLETE

### Commits
- `e7f9095`: feat(app): P10-A DesktopApp multi-tab refactoring
- `fe184bf`: fmt fix

### What Was Done
DesktopApp struct refactored from single session to multi-tab:
- `sessions: Vec<TabSession>` + `active: usize` replaces `app`/`pty`/`_event_tx`/`encoder`
- Constructor creates initial TabSession with shell integration + config
- Accessor methods: `active_session()`, `active_session_mut()`, `shell()`
- Tab management: `open_tab()`, `close_tab()`, `switch_tab()`, `next_tab()`, `prev_tab()`
- Keyboard shortcuts: Ctrl+T (new), Ctrl+W (close), Alt+1-9 (switch), Ctrl+Tab/Ctrl+Shift+Tab (cycle)
- All `self.app`/`self.pty` references replaced with `active_session()` / `active_session_mut()`
- `render_frame` borrow conflict resolved by indexing `self.sessions[self.active]` directly

### Key Design Decisions
- `render_frame`: Cannot use `self.active_session().app().grid()` because it borrows all of `self`, preventing `&mut self.gpu`. Fix: copy `self.active` to a local var, then index `self.sessions[active]` directly.
- Tab keyboard shortcuts use `if let` chains (stable in Rust 2024): `if self.mods.ctrl && !self.mods.shift && let PhysicalKey::Code(code) = &event.physical_key`
- PTY exit check: `!self.active_session_mut().is_alive()` replaces old `if let Some(pty) = self.pty && !pty.is_alive()`

### Build State
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo fmt --all -- --check` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace` = **1232 tests ALL PASS** (0 failed, 2 ignored)