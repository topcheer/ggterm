## GGTerm Phase 20 — COMPLETE

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace` = **1493 tests ALL PASS** (2 ignored)

### Phase 20 Tasks (4 tasks, all done)
| Task | Owner | Commit | Description |
|------|-------|--------|-------------|
| P20-A | dd_dev | a4e617b | Multi-pane viewport rendering: viewport_offset, render_pane_to_pass, render_overlays_to_pass, PaneRenderSpec, render_multi_pane_frame |
| P20-B | me | c0c31df | Pane border overlays: 1px separators, active/inactive border colors via OverlayRect |
| P20-D | gg_dev | 0ef7fe9 | Mouse pane focus: maybe_switch_pane_focus(), wheel routing, fixed Ctrl+Shift+S → Ctrl+Shift+\ |
| P20-E | ggcxf_dev | a0ba444 | README keyboard shortcut table: split shortcuts, non-configurable note |

### Key Architecture: Multi-Pane Rendering (P20-A)
1. **GlyphonRenderer.viewport_offset: (f32, f32)** — shifts all TextArea positions + decoration vertices
2. **render_pane_to_pass()** — prepares grid + draws text + decorations (NO overlay)
3. **render_overlays_to_pass()** — resets offset to (0,0), draws overlay (borders, tab bar, settings)
4. **PaneRenderSpec** — { grid, cursor, offset_x, offset_y, width, height }
5. **render_multi_pane_frame()** — ONE render pass, iterate panes with scissor rect + viewport offset, then overlay
6. **window.rs render_frame()** — pane_count > 1 → build PaneRenderSpec list from SplitTree::areas()

### Key Architecture: Pane Borders (P20-B)
- Active pane: bright blue (0.4, 0.55, 0.85)
- Inactive panes: dim (0.15, 0.15, 0.2)
- 4 OverlayRect per pane (top/bottom/left/right, 1px each)
- Uses tree.is_single() to skip when only 1 pane

### Split Keyboard Shortcuts (Final)
- Ctrl+Shift+D — split horizontal (left/right)
- Ctrl+Shift+\ — split vertical (top/bottom) [changed from Ctrl+Shift+S]
- Ctrl+Shift+[ / ] — focus prev/next pane
- Ctrl+Shift+Alt+arrows — adjust split ratio

### Commits
- c0c31df — feat: P20-B pane border overlays
- 0ef7fe9 — feat: P20-D mouse pane focus + split shortcut fix
- a0ba444 — docs: P20-E README keyboard shortcuts
- a4e617b — feat: P20-A multi-pane viewport rendering

### Test Count Growth
- Phase 19 complete: 1490 tests
- Phase 20 complete: 1493 tests (+3)