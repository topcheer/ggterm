## GGTerm Phase 16 — COMPLETE

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace` = **1317 tests ALL PASS** (2 ignored)

### Phase 16 Tasks (4 tasks, all done)
| Task | Owner | Status | Description |
|------|-------|--------|-------------|
| P16-A | me | DONE | Wire search highlights to renderer: render_frame() now calls renderer.set_highlights() with search match positions |
| P16-B | gg_dev (team) | DONE | Config hot-reload theme/font/scrollback switching: last_applied_theme + last_applied_font_size fields, about_to_wait() detects changes |
| P16-C | me | DONE | Remove dead code: make_cell() and advance_cursor() + unused Cell import |
| P16-D | me | DONE | Window title enhancement: (alt) indicator for alt-screen, [BELL] indicator, Terminal::is_alt_screen() accessor |

### Key Changes
1. **render_frame()**: Converts SearchMatch(abs_row, col, len) → (visible_row, col_start, col_end) and passes to renderer.set_highlights() before GPU render
2. **about_to_wait()**: Config reload now compares old vs new theme/font/scrollback and applies changes
3. **Terminal**: Added `is_alt_screen()` public accessor
4. **Window title**: Multi-tab format `[tab1] [vim* (alt)] [BELL]`, single-tab `(alt)` and `[BELL]` suffixes
5. **Dead code removed**: make_cell(), advance_cursor() functions + Cell import

### Commits
- ebed3d2 — feat: P16-A wire search highlights to renderer + P16-C dead code cleanup
- c23b285 — feat: P16-D window title enhancement — alt-screen + bell indicators
- (gg_dev's P16-B was already in working tree, no separate commit)

### Team Coordination
- gg_dev: P16-B (window.rs about_to_wait config poll + DesktopApp struct fields)
- Me: P16-A (render_frame), P16-C (term/mod.rs cleanup), P16-D (title enhancement)
- No file conflicts — sequential work on different window.rs sections