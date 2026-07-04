## GGTerm Phase 49 — UX Polish & Keybinding Fixes

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test` = **1917 tests ALL PASS** (7 ignored)
- LOC: ~63,000

### Commits This Session
| Commit | Description |
|--------|-------------|
| 9da90f9 | feat: terminal mode reset on exit + scroll percentage indicator |
| d564c2b | fix: copy_cwd keybinding conflict with command palette |
| 7c39a25 | improve: scrollbar now theme-aware (visible on light themes) |

### Features Delivered

1. **Terminal Mode Reset on Exit** — sends reset sequences to all PTYs before closing:
   - Bracketed paste off, mouse tracking off, SGR/URXVT mouse off
   - Cursor keys normal, cursor visible, keypad numeric
   - Soft reset (DECSTR)
   - Prevents shells from being stuck in special modes

2. **Scroll Percentage Indicator** — shows "↓ NN%" instead of raw line count

3. **Copy CWD Keybinding Fix** — Ctrl+Shift+P was bound to both copy_cwd and command palette toggle. Changed copy_cwd to Ctrl+Shift+Alt+P.

4. **Theme-aware Scrollbar** — scrollbar was hardcoded white, invisible on light themes. Now uses dark color on light themes.