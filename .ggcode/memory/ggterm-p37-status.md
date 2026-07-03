## GGTerm Session — Pane Zoom + Tab Reorder + URL Open + Command Palette

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace --lib` = **1750 tests ALL PASS**
- LOC: 54,575

### Commits This Session
| Commit | Description |
|--------|-------------|
| 1c0439f | feat: pane zoom mode (Ctrl+Shift+Z) + config reload toast |
| 1a28b4b | improve: ZOOM indicator in status bar when pane zoom is active |
| e8d25e1 | feat: Ctrl+Shift+PageUp/Down to reorder tabs |
| 9095e5e | feat: Ctrl+Shift+U to open URL at cursor position |
| 1b82baa | improve: add pane zoom, open URL, tab reorder to command palette |

### Features Delivered (5 new features)

1. **Pane Zoom Mode (Ctrl+Shift+Z)** — tmux-style zoom
   - When toggled, active pane renders at full window size
   - Pane borders hidden, mouse focus locked to active pane
   - Separator drag disabled when zoomed
   - ZOOM indicator shown in status bar
   - `pane_zoomed: bool` field on DesktopApp

2. **Config Reload Toast** — shows "Config reloaded" on successful hot-reload
   - Added to about_to_wait config poll handler

3. **Tab Reorder (Ctrl+Shift+PageUp/Down)** — keyboard tab reordering
   - Move current tab left/right
   - Uses existing `move_tab(from, to)` method

4. **Open URL at Cursor (Ctrl+Shift+U)** — keyboard URL opening
   - Checks hovered link → OSC 8 hyperlink → plain-text URL
   - Shows toast with opened URL or "No URL at cursor"

5. **Command Palette Expansion** — 5 new commands registered
   - split.zoom, terminal.open_url, tab.move_left, tab.move_right
   - All dispatch correctly from command palette

### Key Architecture Changes
1. **DesktopApp.pane_zoomed: bool** — new field for zoom state
2. **render.rs** — zoom-aware rendering: skips borders, renders only active pane at full bounds
3. **handlers.rs** — maybe_switch_pane_focus and try_start_separator_drag skip when zoomed
4. **StatusBar.pane_zoomed: bool** — ZOOM indicator in format() and format_segments()
5. **actions.rs** — toggle_pane_zoom() and open_url_at_cursor() methods
6. **ShortcutHelpState** — 3 new entries (pane zoom, tab reorder, open URL)
