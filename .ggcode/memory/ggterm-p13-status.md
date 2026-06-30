## GGTerm Phase 13 — COMPLETE (P13-F deferred to Phase 14)

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace` = **1295 tests ALL PASS** (2 ignored)

### Phase 13 Tasks
| Task | Owner | Status | Description |
|------|-------|--------|-------------|
| P13-A | gg_dev (me) | DONE | SGR attrs: DIM (60% brightness), HIDDEN (fg=bg), STRIKETHROUGH (wgpu pipeline) |
| P13-B | gg_dev (me) | DONE | OSC 8 hyperlinks: Cell gains Option<String>, Terminal applies to printed cells |
| P13-C | dd_dev (team) | DONE | KeybindingsConfig: 13 customizable actions, parse_keybinding(), TOML [keybindings] |
| P13-D | ggcxf_dev (team) | DONE | StatusBar: cursor pos, tab count, bell/search/AI flags, format() method |
| P13-E | gg_dev (me) | DONE | Cross-platform clipboard: Linux X11 (xclip/xsel) + Wayland (wl-copy/wl-paste) |
| P13-F | — | DEFERRED | Module extraction (window.rs 1723 lines → separate modules). Phase 14. |

### Key Architecture Changes
1. **Cell struct**: Changed from Copy to Clone (hyperlink: Option<String>)
2. **grid/row.rs**: copy_within replaced with clone-based shifts (Cell no longer Copy)
3. **TextRun**: Added strikethrough field
4. **prepare_decorations()**: Unified underline + strikethrough vertex generation
5. **upload_vertices()**: Free function for GPU buffer creation
6. **DisplayServer enum**: Auto-detects macOS/Wayland/X11 for clipboard

### Commits
- 2047c67 — feat: Phase 13 — terminal completeness & UX
- 7ef1d1f — test: OSC 8 hyperlink tests (6 tests by gg_dev)
- f2dc7f9 — docs: README update for Phase 13

### Test Count Growth
- Phase 12 complete: 1268 tests
- Phase 13 complete: 1295 tests (+27)