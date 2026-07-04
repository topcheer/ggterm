## GGTerm Phase 41 — Block Selection + Search History + Safe Paste

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test` = **1840 tests ALL PASS** (7 ignored)
- LOC: ~59,700

### Commits (4 commits)
| Commit | Description |
|--------|-------------|
| 8a6f18e | feat: block selection (Alt+drag), OSC 104/110/111/112 color reset |
| 9e40c5f | feat: search history navigation (Up/Down arrows in search bar) |
| fceb618 | feat: safe paste — strip trailing newlines when bracketed paste is off |
| 0ae961d | feat: CSI 11t window state report + minimum tab width |

### Features Delivered (5 new features)

1. **Block/Rectangular Selection (Alt+Drag)**
   - MouseSelection gains `block_mode: bool` field + `start_block()` / `block_rect()` methods
   - Alt+click-drag selects a rectangular region for copying columnar data
   - Block rendering: per-row rectangles in render.rs
   - Block copy: column-by-column text extraction with "Copied N chars (block)" toast
   - 5 tests in mouse.rs

2. **Search History Navigation (Up/Down)**
   - SearchState gains `history: Vec<String>`, `history_idx`, `saved_query` fields
   - Queries saved on close (deduplicated, max 20 entries)
   - Up arrow → older queries, Down arrow → newer, restores partial query
   - 4 tests in search.rs

3. **Safe Paste (Newline Stripping)**
   - When bracketed paste is OFF and clipboard has newlines, strips trailing newlines
   - Shows toast "Pasted first line (N lines stripped)" for multi-line pastes
   - Prevents accidental command execution from clipboard paste

4. **OSC 104/110/111/112 Color Reset**
   - OSC 104: consume palette reset (no-op, palette fixed)
   - OSC 110: reset dynamic foreground (dynamic_fg = None)
   - OSC 111: reset dynamic background (dynamic_bg = None)
   - OSC 112: reset dynamic cursor (dynamic_cursor = None)
   - 4 tests in term/mod.rs

5. **CSI 11t Window State + Min Tab Width**
   - CSI 11t: respond with CSI 1t (not iconified) — xterm windowops extension
   - Tab bar: enforce 80px minimum tab width for readability with many tabs
   - 1 test for CSI 11t

### Test Count Growth
- Phase 40 complete: 1826 tests
- Phase 41 complete: 1840 tests (+14)

### Key Files Modified
1. `crates/ggterm-app/src/mouse.rs` — block_mode field, start_block(), block_rect(), 5 tests
2. `crates/ggterm-app/src/search.rs` — history fields, history_prev/next(), 4 tests
3. `crates/ggterm-app/src/window/actions.rs` — block copy, safe paste
4. `crates/ggterm-app/src/window/handlers.rs` — Alt+drag block selection, Up/Down search history
5. `crates/ggterm-app/src/window/render.rs` — block selection rendering
6. `crates/ggterm-app/src/shortcut_help.rs` — Alt+Drag entry
7. `crates/ggterm-app/src/tab_bar.rs` — 80px min tab width
8. `crates/ggterm-core/src/term/mod.rs` — OSC 104/110/111/112, CSI 11t, 5 tests