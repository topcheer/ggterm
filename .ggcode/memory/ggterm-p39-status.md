## GGTerm Session — Background Opacity + Ctrl+Shift+Wheel Font Zoom

### Build State
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test` = **1806 tests ALL PASS** (7 ignored)

### Commits This Session
| Commit | Description |
|--------|-------------|
| e5f8233 | feat: background opacity/transparency support |
| 5c3f244 | feat: opacity controls in command palette |
| b99b7c7 | feat: Ctrl+Shift+Wheel font zoom |
| 6342049 | docs: add opacity and Ctrl+Shift+Wheel shortcuts to README |

### Features Delivered (3 new features)

1. **Background Opacity** (config + runtime)
   - AppearanceConfig.background_opacity: f32 (0.0=transparent, 1.0=opaque)
   - TOML: `[appearance] background_opacity = 0.85`
   - Runtime: Ctrl+Shift+Alt+[ (decrease) / ] (increase), 5% steps
   - wgpu surface uses PostMultiplied alpha mode
   - winit window created with `with_transparent(true)`
   - bg_color changed from [f64; 3] to [f64; 4] throughout render pipeline
   - Toast notification with percentage on change
   - 4 config tests (default, parse, clamp, export round-trip)
   - Command palette: "Increase/Decrease Background Opacity"

2. **Ctrl+Shift+Wheel Font Zoom** (VS Code / iTerm2 style)
   - Mouse wheel with Ctrl+Shift adjusts font size
   - Wheel up = zoom in, wheel down = zoom out
   - Toast feedback with font size

3. **Documentation Updates**
   - README: transparency feature, new shortcuts in keyboard table
   - config.example.toml: `background_opacity` with documentation

### Key Architecture Changes
1. **AppearanceConfig** gains `background_opacity: f32` field (default 1.0)
2. **raw::Appearance** gains `background_opacity: Option<f32>` for serde
3. **gpu.rs**: `render_frame()` and `render_multi_pane_frame()` now take `[f64; 4]` bg_color
4. **render.rs**: bg_color array extended to 4 elements (RGBA)
5. **SurfaceConfiguration**: prefers `PostMultiplied` alpha mode when available
6. **Window**: `with_transparent(true)` for compositing
7. **handlers.rs**: Ctrl+Shift+Wheel early-returns before normal scroll handling
8. **command_palette.rs**: 2 new opacity commands registered
9. **shortcut_help.rs**: 2 new entries for opacity shortcuts