## GGTerm Phase 32 — COMPLETE

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace --lib` = **1737 tests ALL PASS** (7 ignored)
- LOC: 52,401

### Phase 32 Commits
| Commit | Description |
|--------|-------------|
| 175f4ed | feat: drag-to-reorder tabs with live position swap |
| f37f2ef | feat: scroll-to-bottom indicator + smart copy trimming |
| b3f796d | improve: remove stale dead_code fields and methods |
| 952fc08 | feat: floating search bar with case toggle and match counter |

### Phase 32 Features (4 changes)
| Feature | Description | Interaction |
|---------|-------------|-------------|
| Tab drag reordering | Click+drag a tab to reorder among siblings | Mouse drag in tab bar |
| Scroll-to-bottom indicator | Blue pill shows when scrolled up | Click indicator or Ctrl+Shift+End |
| Smart copy trimming | Strips leading/trailing empty lines | Automatic on copy |
| Floating search bar | Visible search input with case toggle + match count | Ctrl+Shift+F, Tab for case toggle |

### Test Count Growth
- Phase 31 complete: 1735 tests
- Phase 32 complete: 1737 tests (+2)

### Dead Code Cleanup
- Removed stale `drag_tab` field (replaced by `dragging_tab`)
- Removed stale `tab_close_hovered` field
- Removed unused `start_tab_drag()` and `tab_index_at_x()` methods
- Removed `#[allow(dead_code)]` from `move_tab()` (now used)
