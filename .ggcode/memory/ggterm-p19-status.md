## GGTerm Phase 19 — COMPLETE

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace` = **1490 tests ALL PASS** (2 ignored)

### Phase 19 Tasks (8 tasks, all done)
| Task | Owner | Status | Description |
|------|-------|--------|-------------|
| P19-A | dd_dev | DONE (25cb26d) | Application menu (MenuAction + Mutex queue) + AboutDialog |
| P19-B | gg_dev | DONE (bccecda) | SplitTree + PaneSession: split_h/v, focus, remove, resize |
| P19-C | ggcxf_dev | DONE (19a2696) | TabBarState + SettingsState (7 fields, nav, cycle) |
| P19-D | me | DONE (6c30abe) | Platform packaging (Cargo.bundle.toml, Makefile, Info.plist) |
| P19-E | me | DONE (5a61ee4) | OSC 10/11 dynamic colors wired to renderer |
| P19-F | dd_dev | DONE (03aa599) | native_menu.rs data layer (action_to_tag/tag_to_action, 13 tests) |
| P19-G | me+dd_dev+ggcxf | DONE (03aa599) | Overlay rendering: OverlayRect/OverlayTextSpec, push_rect, prepare/draw_overlay |
| P19-H | dd_dev | DONE (bccecda, 20f9ea5) | 26 integration tests for splits/tab_bar/settings_ui/about_dialog |

### Key Architecture
1. **OverlayRect { x, y, w, h, color }** — pixel coords → NDC vertices
2. **OverlayTextSpec { text, left, top, color }** — glyphon TextArea merge
3. **push_rect()** — 6 vertices per rect (2 triangles), 5-float stride
4. **PaneSession** — TabSession contains Vec<Option<PaneSession>> + SplitTree
5. **MenuAction** — 16-variant enum with thread-safe action queue
6. **native_menu.rs** — data layer (action_to_tag/tag_to_action/parse_accelerator), install = logging stub

### Commits (P19)
- 25cb26d — P19-A application menu + about dialog
- 19a2696 — P19-B/C splits.rs, settings_ui.rs, tab_bar.rs
- 03aa599 — P19-G overlay + native menu data + settings/about wiring
- 5e3cfd9 — P19-G overlay rendering tests (6)
- 5dee092 — overlay rendering tests
- bccecda — P19-H integration tests
- 20f9ea5 — P19-H integration tests + renderer dedup + native_menu cleanup
- b8440b1..2ad3131 — README Phase 12-19 docs

### Test Count Growth
- Phase 18 complete: 1343 tests
- Phase 19 complete: 1490 tests (+147)

### Next: Phase 20
**P20-A: Multi-pane viewport rendering** — highest priority
- SplitTree::areas(bounds) calculates per-pane Rect
- render_frame needs to iterate pane_ids() and render each pane's grid into its sub-rect
- Requires converter.rs offset parameter + renderer viewport/scissor clip
- Currently only active pane renders full window (splits are invisible to user)