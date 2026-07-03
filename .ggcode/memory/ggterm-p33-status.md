## GGTerm Phase 33+ — Bug Fixes & UX Polish

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace --lib` = **1740 tests ALL PASS** (7 ignored)
- LOC: 52,564

### Recent Commits
| Commit | Description |
|--------|-------------|
| 765e8a8 | docs: update README with latest features and test count |
| 9e756a2 | feat: show current directory in status bar (from OSC 7) |
| c6a5088 | feat: middle-click on tab to close it (browser-style) |
| 1c55b07 | feat: add "x" close button on each tab + reduce idle CPU |
| 4c54969 | fix: 100% CPU usage when idle — add sleep in event loop |
| 638d9c1 | fix: tab bar text invisible — UI backgrounds drawn on top of text |
| 5e0709b | docs: document restore_session option in config.example.toml |
| 2d2dbc8 | fix: session restore now opt-in, immediate save on pane/tab close |

### Critical Bug Fixes
1. **Tab bar text invisible** (638d9c1): In `render_overlays_to_pass()`, overlay text was rendered FIRST, then UI background rectangles (tab bar, status bar) were drawn ON TOP, hiding the text. Fix: Reversed draw order — backgrounds first, text last.
2. **100% CPU when idle** (4c54969): winit's Poll mode calls about_to_wait() in a tight loop. Without sleep, the app consumed ~100% CPU even when idle. Fix: Sleep 50ms when no redraw is needed.
3. **Session persistence unwanted** (2d2dbc8): Session restore was always on and saved only on exit. Fix: Added `restore_session` config option (default: false), save immediately on pane/tab close, clear old session file when disabled.

### New Features
1. **Tab close button "x"** (1c55b07): Renders "x" on each tab when 2+ tabs exist. Click closes the tab.
2. **Middle-click tab close** (c6a5088): Browser-style middle-click on any tab closes it.
3. **CWD in status bar** (9e756a2): Shows current directory basename from OSC 7 cwd tracking.
4. **Configurable session restore**: `[terminal] restore_session = true/false` in config.toml.

### Architecture Notes
- **Overlay rendering order**: backgrounds (tab bar, status bar) → text (tab titles, status segments) — text must be LAST
- **Idle loop**: When no content is dirty, sleep 50ms before returning from about_to_wait()
- **Session persistence**: save_session_on_exit() now checks restore_session config, clears file if disabled
