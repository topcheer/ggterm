## GGTerm Phase 47 — Protocol Completeness & Correctness

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace --lib` = **1904 tests ALL PASS** (7 ignored)
- LOC: ~62,000

### Commits This Session
| Commit | Description |
|--------|-------------|
| 6f5062b | feat: LNM (mode 20) + OSC 9;4 progress report |
| 0113fd0 | feat: OSC 9;4 progress display in status bar |
| 0132d2c | feat: text reflow on resize (DECSET 2027) |
| e45b623 | fix: DECSTR (CSI ! p) now properly soft-resets instead of hard reset |
| c128a36 | fix: DSR cursor position report now respects origin mode |
| 71417af | fix: GPU surface lost/occluded errors on startup |
| 635910f | fix: 100% CPU idle + DECSEL selective erase + origin mode clamp |

### Features Delivered

1. **LNM (Line Feed/New Line Mode, ANSI mode 20)** — when enabled, LF produces CR+LF
2. **OSC 9;4 Progress Report** — programs report task progress (0-100%), shown in status bar
3. **Text Reflow on Resize (DECSET 2027)** — growing window pulls scrollback back into view
4. **DECSTR Soft Reset Fix** — was hard-resetting (destroying scrollback); now properly soft-resets
5. **DSR Origin Mode Fix** — cursor position report now respects scroll region in origin mode
6. **Surface Lost Recovery** — wgpu surface Lost/Occluded states now auto-recover instead of crashing
7. **100% CPU Idle Fix** (CRITICAL) — content_dirty was never cleared, making the idle sleep dead code
8. **DECSEL (CSI ? K)** — selective erase in line respecting PROTECTED attribute
9. **Origin Mode CUP Clamp** — cursor position now clamps to scroll region bottom in origin mode

### Key Architecture Changes
1. **Grid::reflow_resize()** — new method that pulls from scrollback when height increases
2. **Terminal::resize()** — uses reflow_resize when modes.reflow is enabled
3. **DECSTR** — now resets cursor/SGR/modes/charset but preserves scrollback/grid
4. **Modes.new_line_mode: bool** — new field for LNM (mode 20)
5. **Terminal.progress: Option<f32>** — OSC 9;4 progress state
6. **StatusBar.progress: Option<f32>** — shows percentage in status bar
7. **TabSession::clear_content_dirty()** — called after render to fix 100% CPU bug
8. **gpu.rs** — surface Lost/Occluded auto-reconfigure instead of error

### Test Count Growth
- Previous: 1888 tests
- Current: 1904 tests (+16)