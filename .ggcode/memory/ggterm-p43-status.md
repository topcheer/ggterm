## GGTerm Session — OSC 4 Palette Overrides + Scrollback Fix + Wide Char Fix

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test` = **1856 tests ALL PASS** (7 ignored)
- LOC: 55,076

### Commits This Session (5 commits)
| Commit | Description |
|--------|-------------|
| c86d437 | feat: bell indicator on background tabs (bell emoji on tab title) |
| 018dbcc | fix: skip wide-character spacers when copying text (CJK/emoji) |
| 1d002bb | fix: preserve scrollback position when new output arrives |
| cd44c31 | improve: scroll indicator shows line count when scrolled up |
| 52c4b1d | feat: OSC 4 custom palette override support (base16-shell compatibility) |

### Key Feature: OSC 4 Custom Palette (52c4b1d)
Programs like base16-shell, wal, and pywal set custom terminal colors via OSC 4.
Previously this was a no-op. Now:
- Terminal stores `palette_overrides: HashMap<u8, (u8, u8, u8)>`
- OSC 4 SET stores overrides; OSC 4 ? returns overridden values
- OSC 104 resets specific or all overrides (was also a no-op)
- Renderer applies overrides to Color::Indexed cells during rendering
- `resolve_palette_color()` accessor for indexed color resolution
- 4 new tests

### Key Fix: Scrollback Position Preservation (1d002bb)
When scrolled up reading history, new output no longer snaps viewport to bottom.

### Test Count Growth
- Previous: 1840 tests
- Current: 1856 tests (+16)