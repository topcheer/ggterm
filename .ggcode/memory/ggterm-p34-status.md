## GGTerm Phase 34+ — Bug Fixes & Layout Reset

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace --lib` = **1740 tests ALL PASS** (7 ignored)
- LOC: 52,660

### Phase 34 Commits
| Commit | Description |
|--------|-------------|
| abfbe76 | fix: overlay text double-rendered in multi-pane causing ghosting |
| 1304f8a | improve: pane borders and inactive bg now theme-aware |
| 9b5cd1a | fix: header/footer background color now follows terminal theme |
| 325a8af | improve: clean tab format + fix multi-byte string slicing |
| 4a55f29 | feat: reset layout to single pane (Ctrl+Shift+Alt+N) |
| 21ed757 | improve: new-tab + button now theme-aware |
| 8c33538 | fix: unify single/multi-pane rendering + always show tab bar |
| 8e024b5 | fix: "+" new-tab button text invisible on dark theme |
| 2d2dbc8 | fix: session restore now opt-in, immediate save on pane/tab close |
| 5e0709b | docs: document restore_session option in config.example.toml |
| 638d9c1 | fix: tab bar text invisible — UI backgrounds drawn on top of text |
| 4c54969 | fix: 100% CPU usage when idle — add sleep in event loop |
| 1c55b07 | feat: add "x" close button on each tab + reduce idle CPU |
| c6a5088 | feat: middle-click on tab to close it (browser-style) |
| 9e756a2 | feat: show current directory in status bar (from OSC 7) |
| 765e8a8 | docs: update README with latest features and test count |
| e71eb86 | feat: refined GGTerm logo — cleaner, scalable icon design |
| d218ef9 | feat: set window icon from embedded logo PNG |
| 933b695 | feat: tab titles now sync with running program name (OSC 0/2) |
| d44c1f3 | improve: about dialog header accent bar + ">_" logo symbol |

### Key Bug Fixes
1. **Tab bar text invisible** (638d9c1): overlay text rendered FIRST then UI backgrounds ON TOP. Fix: reversed draw order.
2. **100% CPU when idle** (4c54969): winit Poll mode calls about_to_wait in tight loop. Fix: sleep 50ms when idle.
3. **Session persistence unwanted** (2d2dbc8): Default OFF, immediate save on pane/tab close.
4. **Overlay text ghosting** (abfbe76): overlay_text consumed by prepare_grid in multi-pane. Fix: save/clear/restore.

### New Features
1. **Tab close button "x"** — click to close, middle-click also closes
2. **CWD in status bar** — from OSC 7 tracking
3. **Tab title sync** — tabs show running program name (vim, htop, etc.)
4. **Reset layout** — Ctrl+Shift+Alt+N clears to single pane
5. **GGTerm logo** — SVG + PNG (32-1024px) + macOS .icns + window icon
6. **Configurable session restore** — `[terminal] restore_session = true`

### Architecture Notes
- **Overlay rendering order**: backgrounds (tab bar, status bar) → text — text must be LAST
- **Idle loop**: sleep 50ms when no content dirty
- **Tab title sync**: in about_to_wait, iterate sessions and sync terminal().title() to session.title
- **Unified render path**: all rendering goes through render_multi_pane_frame() even for single pane
