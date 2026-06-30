## GGTerm P16-B: Config Hot-Reload Theme/Font Switching — COMPLETE

### Files Changed
- `crates/ggterm-app/src/window.rs` only

### What Was Done
1. **DesktopApp struct**: Added `last_applied_theme: String` and `last_applied_font_size: f32` fields for change detection
2. **Constructor**: Initializes both fields from config_mgr (or defaults to "dark" / DEFAULT_FONT_SIZE)
3. **about_to_wait() config poll**: When `poll_reload()` detects changes:
   - Theme change → `set_by_name()` + `apply_theme_to_renderer()`
   - Font size change → `set_base_size()` + `apply_font_size()`
   - Scrollback → always updated via `grid_mut().set_scrollback()`
4. **Borrow fix**: Config values extracted to local variables before mutable self calls

### Build State
- clippy CLEAN, fmt CLEAN
- **1317 tests ALL PASS** (0 failed, 2 ignored)