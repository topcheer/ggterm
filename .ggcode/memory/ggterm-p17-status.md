## GGTerm Phase 17 — COMPLETE

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace` = **1340 tests ALL PASS** (2 ignored)

### Phase 17 Tasks
| Task | Owner | Status | Description |
|------|-------|--------|-------------|
| P17-A | me | DONE | OSC 10/11/12 dynamic colors: set/query fg/bg/cursor, parse_xcolor(), 8 tests |
| P17-B | me | DONE | Combining chars: Cell gains combining: Vec<char>, put_printable_char merges zero-width marks, 5 tests |
| P17-C | dd_dev (team) | DONE | URL click & hover: detect_url_at_position(), open_url(), hovered_link field, Cmd+Click opens |
| P17-D | ggcxf_dev (team) | DONE | Status bar toggle: Ctrl+Shift+B, status_bar_visible field, borrow conflict fix |
| P17-E | pre-existing | DONE | OSC 133 exit code: last_exit_code() + last_command_succeeded() already on Terminal |

### Key Architecture Changes
1. **Cell**: Added `combining: Vec<char>` field for zero-width marks (é, ü, emoji modifiers)
2. **Terminal**: Added dynamic_fg/dynamic_bg fields for OSC 10/11 color overrides
3. **parse_xcolor()**: Parses rgb:RR/GG/BB and #RRGGBB color specs
4. **color_for_index()**: 16-color palette RGB lookup
5. **DesktopApp**: Added hovered_link + status_bar_visible fields, super_key in ModsState
6. **mouse.rs**: detect_url_at_position(), find_urls(), open_url(), is_url_char()

### Commits
- 93d5cd1 — feat: P17-A OSC 10/11/12 dynamic colors + P17-C URL click & hover
- 3e50635 — feat: P17-B combining character support + P17-D status bar toggle

### Test Count Growth
- Phase 16 complete: 1317 tests
- Phase 17 complete: 1340 tests (+23)

### Team Coordination
- dd_dev: P17-C (mouse.rs + window.rs mouse handler) — URL detection + open
- ggcxf_dev: P17-D (window.rs render_frame + status_bar toggle) — Ctrl+Shift+B
- Me: P17-A (term/mod.rs OSC handler), P17-B (cell.rs + term/mod.rs put_printable_char)
- All parallel, minimal file conflicts (resolved by section-based editing)