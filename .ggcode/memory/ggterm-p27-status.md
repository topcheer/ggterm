## GGTerm Phase 27 — COMPLETE

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace --lib` = **1582 tests ALL PASS** (7 ignored)

### Phase 27 Tasks (7 tasks)
| Task | Status | Description |
|------|--------|-------------|
| P27-A | DONE | Text selection highlight: semi-transparent blue UiRect overlay on selected cells |
| P27-B | DONE | Double-click word select + triple-click line select (400ms timeout, click_count tracking) |
| P27-C | DONE | Right-click context menu: 6 actions (Copy/Paste/SelectAll/Search/Clear/Reset), SDF rounded rendering |
| P27-D | DONE | Smooth inertial scrolling: SmoothScroller with exponential decay, trackpad momentum |
| P27-E | DEFERRED | macOS vibrancy: NSVisualEffectView FFI crashes on ARM64, code exists but disabled |
| P27-F | DONE | Select-to-copy (already existed) + window_focused tracking for cursor style |
| P27-G | DONE | Scroll-to-bottom indicator (↓ pill) + Ctrl+Shift+End shortcut |

### New Files
- `crates/ggterm-app/src/context_menu.rs` — ContextMenuState + 6 actions (8 tests)
- `crates/ggterm-app/src/smooth_scroll.rs` — SmoothScroller with velocity decay (8 tests)
- `crates/ggterm-app/src/vibrancy.rs` — macOS NSVisualEffectView (disabled, ARM64 crash)

### Key Architecture
1. **Selection highlight**: UiRect fill with (0.3, 0.55, 0.95, 0.30) RGBA, renders per-row selection rects
2. **Double-click**: click_count + last_click_time + last_click_pos on DesktopApp, select_word_at/select_line_at scan grid display rows
3. **Context menu**: hit_test() pixel position → item index, execute_context_menu_action() dispatches to existing methods
4. **Smooth scroll**: add_lines/add_pixels set target, tick() returns integer delta each frame, exponential decay interpolation
5. **Scroll indicator**: grid.is_scrolled() check in render.rs → blue pill with ↓ arrow

### Commits
- e84dbe4 — feat: Phase 27 — selection highlight, double-click, context menu, smooth scroll
- 5098bf3 — feat: P27-E macOS window vibrancy (NSVisualEffectView)
- 7e2f067 — feat: P27-G scroll-to-bottom indicator + Ctrl+Shift+End shortcut
- 187e612 — fix: disable vibrancy to prevent ARM64 crash
- e7e2088 — fix: comment out vibrancy call to avoid invalid cfg

### Test Count Growth
- Phase 26 complete: 1566 tests
- Phase 27 complete: 1582 tests (+16: 8 context_menu + 8 smooth_scroll)

### P27-E Known Issue
The raw FFI objc_msgSend approach for NSVisualEffectView crashes on ARM64 macOS because NSRect (4 doubles) return values are handled differently. The vibrancy.rs module contains the full implementation but is commented out. To re-enable, need to add objc2-app-kit features ["NSView", "NSVisualEffectView", "NSWindow"] to Cargo.toml and use the typed API instead of raw FFI.