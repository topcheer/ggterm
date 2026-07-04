## GGTerm Phase 48 — Robustness & Protocol Completeness

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test` = **1917 tests ALL PASS** (7 ignored)
- LOC: ~63,000

### Commits This Session
| Commit | Description |
|--------|-------------|
| 24b5c02 | feat: DECRQSS DECSCA/DECSCUSR + scroll line count indicator |
| 36c2b8a | test: add robustness edge case tests for terminal core |

### Features Delivered
1. **DECRQSS DECSCA** — programs can query character protection attribute
2. **DECRQSS DECSCUSR** — programs can query cursor style (0-6)
3. **Scroll indicator improvement** — shows "↓ N lines" when scrolled >99 lines
4. **8 new robustness tests** — empty input, partial escapes, NUL bytes, 1x1 resize, growth, invalid UTF-8, multiple resets, many CSI params

### Test Count Growth
- Previous: 1904 tests
- Current: 1917 tests (+13)

### Key Architecture
- DECRQSS handled via DCS ($ q): selectors "m" (SGR), "r" (DECSTBM), "\"q" (DECSCA), " q" (DECSCUSR)
- Scroll indicator dynamically sizes based on text length