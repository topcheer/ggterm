## GGTerm P20-A: Multi-Pane Viewport Rendering — COMPLETE

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace --lib` = **1239 tests ALL PASS** (0 failed)

### What Was Done
Multi-pane rendering: each split pane's grid is rendered to its own sub-region of the surface.

### Key Architecture Changes
1. **GlyphonRenderer viewport_offset** — `viewport_offset: (f32, f32)` field shifts all TextArea positions and decoration vertices. `set_viewport_offset(x, y)` sets the offset before `prepare_grid*()`. Overlay rendering is NOT offset (uses absolute screen coords).

2. **Render method split** — `render_to_pass()` (full render, backward compat) split into:
   - `render_pane_to_pass()` — prepares grid + decorations, draws text + decorations (NO overlay)
   - `render_overlays_to_pass()` — resets offset to (0,0), prepares + draws overlays only

3. **gpu.rs render_multi_pane_frame()** — Creates ONE render pass:
   - For each pane: `set_scissor_rect(pane area)` → `set_viewport_offset(pane x,y)` → `render_pane_to_pass()`
   - Reset scissor to full screen → `render_overlays_to_pass()`

4. **PaneRenderSpec struct** — `{ grid: &Grid, cursor: &CursorState, offset_x: u32, offset_y: u32, width: u32, height: u32 }`

5. **window.rs render_frame()** — Checks `pane_count() > 1`:
   - Multi-pane: `SplitTree::areas(bounds)` → build `PaneRenderSpec` list → `gpu.render_multi_pane_frame()`
   - Single-pane: existing `gpu.render_frame()` unchanged

### Commit
- `a4e617b` — feat: P20-A multi-pane viewport rendering

### Files Changed
1. `crates/ggterm-render-wgpu/src/lib.rs` — viewport_offset field + set_viewport_offset() + render_pane_to_pass() + render_overlays_to_pass() + offset in prepare_grid_with_dirty/prepare_decorations
2. `crates/ggterm-app/src/gpu.rs` — PaneRenderSpec struct + render_multi_pane_frame() method
3. `crates/ggterm-app/src/window.rs` — render_frame() multi-pane branch

### Test Count Growth
- Phase 19 complete: 1236 tests
- P20-A complete: 1239 tests (+3 viewport offset tests)

### Other P20 Tasks (by team)
- P20-B: Pane border overlays (PM) — DONE (commit c0c31df)
- P20-D: Mouse pane focus (gg_dev) — DONE
- P20-C: ggcxf_dev available