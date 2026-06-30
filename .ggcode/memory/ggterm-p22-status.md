## GGTerm Phase 22 — COMPLETE

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace` = **1572 tests ALL PASS** (2 ignored)

### Commits
- `6dec870` — feat: Phase 22 — session lifecycle, profiles, OSC 7 cwd, drag & drop, window extraction
- `d1f08d2` — docs: update test count to 1572+

### Phase 22 Tasks (5 tasks, all done)
| Task | Owner | Description |
|------|-------|-------------|
| P22-A | me | Session save/restore: capture_session(), save_session_on_exit(), restore_from_plan(), CloseRequested saves on exit, constructor restores on startup |
| P22-B | gg_dev + me | Window module extraction: window.rs (2852 lines) → window/{mod.rs, handlers.rs, actions.rs, render.rs} |
| P22-C | ggcxf_dev | Profile system: Profile struct, profiles HashMap, apply_profile(), cycle_profile(), status_bar @profile display, 19 tests |
| P22-D | dd_dev | OSC 7 cwd tracking: parse_osc7_cwd(), percent_decode(), Terminal.cwd field, PaneSession cwd, 12 tests |
| P22-E | me | Drag & drop: WindowEvent::DroppedFile → quote_shell_path() → write to PTY |

### Key Architecture Changes
1. **window/mod.rs** — DesktopApp struct, constructor, run() loop, ApplicationHandler impl, session restore in constructor
2. **window/handlers.rs** — keyboard, mouse, cursor, resize event handlers (914 lines)
3. **window/actions.rs** — tab/split/clipboard/theme/font/session/drag-drop operations (457 lines)
4. **window/render.rs** — render_frame, multi-pane rendering, overlay composition (400+ lines)
5. **session.rs** — capture_session() uses SplitTree::active() and capture_split_tree()
6. **config.rs** — Profile struct with Optional overrides, profiles: HashMap<String, Profile>
7. **status_bar.rs** — profile_name field, @profile display in format()
8. **term/mod.rs** — OSC 7 handler, parse_osc7_cwd(), cwd: Option<PathBuf>
9. **tab_session.rs** — PaneSession cwd tracking, cwd() accessor per pane

### Integration Fixes Applied
1. window.rs vs window/mod.rs conflict → deleted window.rs
2. tab_session.rs AppEvent::Data → AppEvent::PtyBytes (dd_dev's test used nonexistent variant)
3. 3 clippy empty-line-after-doc-comment errors in extracted files
4. Stray doc comment lines from gg_dev's extraction (merged comments on wrong functions)

### Test Count Growth
- Phase 21 complete: 1543 tests
- Phase 22 complete: 1572 tests (+29)