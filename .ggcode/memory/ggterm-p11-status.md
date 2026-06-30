## GGTerm Phase 11 — COMPLETE

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace` = **1263 tests ALL PASS** (2 ignored)

### Phase 11 Tasks (6 tasks, all done)
| Task | Commit | Tests | Description |
|------|--------|-------|-------------|
| P11-A: Font Zoom | 7d6dbc2 | 14 | Ctrl+=/-/0, FontZoom module, set_font_size() |
| P11-B: Terminal Utilities | fa8297b | 12 | Ctrl+Shift+C/K/R/A, terminal_actions.rs |
| P11-C: Fullscreen | a5dc344 | — | F11 fullscreen, Ctrl+Shift+Enter maximize |
| P11-D: Theme Renderer | 4a6a0c5 | — | set_theme(), cycle_theme(), Ctrl+Shift+T |
| P11-E: Bell Support | 3f37a71 | 5 | BEL detection, take_bell(), visual bell |
| P11-F: Documentation | 6d26ceb | — | README keyboard shortcuts reference |

### New Keyboard Shortcuts Added
| Shortcut | Action |
|----------|--------|
| Ctrl+= | Zoom in (font size +1.5px) |
| Ctrl+- | Zoom out (font size -1.5px) |
| Ctrl+0 | Reset font size |
| Ctrl+Shift+C | Copy selection to clipboard |
| Ctrl+Shift+K | Clear screen + scrollback |
| Ctrl+Shift+R | Reset terminal (RIS) |
| Ctrl+Shift+A | Select all text |
| Ctrl+Shift+T | Cycle through themes |
| F11 | Toggle fullscreen |
| Ctrl+Shift+Enter | Toggle maximized |

### New Files
- `crates/ggterm-app/src/font.rs` — FontZoom state (14 tests)
- `crates/ggterm-app/src/terminal_actions.rs` — clear/reset/select_all (12 tests)

### Modified Files
- `crates/ggterm-core/src/term/mod.rs` — bell field + take_bell() + 5 tests
- `crates/ggterm-render-wgpu/src/lib.rs` — theme field, set_theme(), set_font_size(), current_theme()
- `crates/ggterm-app/src/window.rs` — all keyboard shortcuts + apply_theme_to_renderer() + cycle_theme() + apply_font_size() + poll_bell() + toggle_fullscreen() + toggle_maximized()
- `crates/ggterm-app/src/lib.rs` — added font + terminal_actions modules
- `README.md` — Phase 10/11 features + keyboard shortcuts reference table

### Test Count Growth
- Phase 10 complete: 1232 tests
- Phase 11 complete: 1263 tests (+31 new)
- 6 crates, ~30,000+ lines Rust

### Phase 11 Commits
- fa8297b — feat(app): P11-B terminal utility shortcuts
- a5dc344 — feat(app): P11-C fullscreen & window controls
- 4a6a0c5 — feat(app): P11-D theme application to renderer + font size API
- 7d6dbc2 — feat(app): P11-A font customization & live zoom
- 3f37a71 — feat(app): P11-E notification & bell support
- 6d26ceb — docs: P11-F README update