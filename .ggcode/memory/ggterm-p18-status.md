## GGTerm Phase 18 — COMPLETE

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace` = **1343 tests ALL PASS** (2 ignored)

### Phase 18 Tasks
| Task | Status | Description |
|------|--------|-------------|
| P18-A: DPI-aware rendering | DONE | LogicalSize window, PhysicalSize surface, ScaleFactorChanged handler, surface clamped to max_texture_dimension_2d |
| P18-B: Font measurement | DONE | Menlo(macOS)/DejaVu Sans Mono(Linux)/Cascodia Mono(Windows), measure_cell_width() via glyphon shaping, line_height=font_size |
| P18-C: PTY/Renderer sync | DONE | Renderer derives cols/rows from surface, sessions resized immediately after renderer creation |
| P18-D: Per-run grid alignment | DONE | Each text run positioned at exact grid column (left = start_col × cell_w_f32), wide chars force run split |
| P18-E: Bold weight fix | DONE | Removed Weight::BOLD (Menlo Bold missing box-drawing glyphs), bold via bright color only |

### Critical Architecture: Per-Run Rendering (P18-D)
- `row_to_runs()` splits at wide char (CJK/emoji) boundaries — each wide char gets its own run
- `TextRun` has `start_col: usize` field for absolute positioning
- Each run → independent glyphon Buffer → TextArea(left = start_col × cell_w_f32, top = row × cell_h)
- `cell_w_f32` = exact float advance of 'M' (no rounding) — matches glyphon's natural positioning
- No `letter_spacing` — the font's natural advance IS the cell width
- `Shaping::Advanced` for CJK font fallback

### Key Bug: Menlo Bold Missing Glyphs
- Menlo Bold (in Menlo.ttc) does NOT contain box-drawing chars (U+2500-257F)
- Logo with `.Bold(true)` → tofu/squares
- Fix: Always use Weight::NORMAL, bold distinguished by bright color (xterm/Alacritty standard)

### Multi-Platform
- Fonts: Menlo (macOS) / DejaVu Sans Mono (Linux) / Cascadia Mono (Windows)
- Clipboard: macOS (pbpaste/pbcopy), Linux X11 (xclip/xsel), Linux Wayland (wl-copy/wl-paste), Windows (powershell/clip)
- DisplayServer enum: Macos, Wayland, X11, Windows, Unsupported

### Commits
- 7f0f4c8 — feat: P18 DPI-aware rendering, per-run grid alignment, multi-platform support
- 33e660f — fix: remove Weight::BOLD — Menlo Bold missing box-drawing glyphs
- 17518aa — docs: fix outdated set_font_size doc comment

### Test Count
- Phase 17 complete: 1340 tests (before P18 changes in term/mod.rs)
- Phase 18 complete: 1343 tests (+3 from converter/renderer changes)