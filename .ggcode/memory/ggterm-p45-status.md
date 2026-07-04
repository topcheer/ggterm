## GGTerm Phase 45 — Protocol Completeness & UX Polish

### Build State
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test` = **1872 tests ALL PASS** (7 ignored)
- LOC: ~56,000

### Commits This Session (all sessions combined)
| Commit | Description |
|--------|-------------|
| 550a082 | feat: CSI 16t character cell size report + scrollbar alt screen fix |
| cfa4294 | fix: hide scrollbar in alternate screen mode (vim/less/htop) |
| f91b4ef | feat: add 5 new commands to command palette |
| ce67b57 | feat: add --working-directory CLI option |
| baaa15d | improve: DECRQM ANSI mode responses for autowrap and cursor blink |
| 1c6a2e9 | feat: SGR 21 (double underline) + SGR 53/55 (overline) support |
| 5d3b118 | fix: reset cursor blink on user input |
| be940b7 | feat: scroll-to-mark shortcut (Ctrl+Shift+Alt+Up) |
| 05255d3 | feat: DA3 tertiary device attributes + sanitize OSC 0/2 title |

### Test Count: 1872 (+13 from baseline 1859)