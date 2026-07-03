## GGTerm Phase 36 — Tab/Menu UX Polish

### Build State
- cargo fmt CLEAN, cargo clippy CLEAN, 1750 tests ALL PASS (7 ignored)

### Commits This Session
| Commit | Description |
|--------|-------------|
| 814638e | improve: tab bar auto-fill width + cross-platform icon + macOS Dock icon |
| 4862f9d | fix: tab bar mouse hit-test mismatch — + button unclickable |
| 4f96f71 | fix: mouse cursor must be on top half of tab — DPI scale bug |
| 6c87f8e | fix: tab bar mouse coordinate mismatch on Retina displays |
| 4117102 | fix: tabs now truly fill full window width — remove 220px max cap |
| 3642bc5 | feat: "+" button opens dropdown menu — New Tab, Split Horizontal/Vertical |
| 8375e3d | improve: position + dropdown centered below the button |
| df3255f | improve: tab hover feedback — brighten on hover, show close button on hover |
| 1838e6e | fix: + dropdown right-aligned to button edge, no longer clipped |
| ee68528 | fix: dropdown menu wider (240px) + theme-aware colors |
| 6701eb5 | improve: wider dropdown (280px), separator, larger + click area, more padding |
| 9777184 | fix: tab titles no longer truncated prematurely |
| a79fa1f | improve: context menu wider with border, split actions, proper clear/reset |
| 6823a17 | fix: menu borders invisible — theme_bg multiplier too low |
| d9bfbb9 | improve: unify tab right-click menu styling with other menus |
| 10702b2 | improve: close button uses × glyph, turns red on hover (browser-style) |
| 968e1a9 | improve: middle-click on + button area opens new tab (browser-style) |

### Key Fixes
1. Tab auto-fill width: compute_layout divides available_width equally among tabs
2. DPI coordinate fix: compute_layout uses renderer.cell_height() (physical px) not hardcoded 14
3. cursor_pos: winit 0.30 CursorMoved delivers physical pixels — no scale_factor multiply needed
4. Menu borders: fixed bright accent (0.45,0.52,0.68) instead of theme_bg*2.2 (invisible on dark)
5. Clear vs Reset: Clear sends ESC[H ESC[2J Ctrl+L for fresh prompt; Reset sends RIS (ESC c)
6. Close button: uses × (U+00D7), turns red on direct hover

### New Files
- `crates/ggterm-app/src/new_tab_menu.rs` — NewTabMenuAction + NewTabMenuState (10 tests)