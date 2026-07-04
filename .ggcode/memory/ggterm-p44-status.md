## GGTerm Phase 44+ — Protocol Completeness & UX Polish

### Build State
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test` = **1859 tests ALL PASS** (7 ignored)
- LOC: ~55,400

### Recent Commits
| Commit | Description |
|--------|-------------|
| 9b1751b | improve: scroll indicator shows percentage for large scrollback |
| 2634653 | fix: OSC 8 hyperlink underline now uses blue tint color |
| 40a34bf | improve: DECRQM ANSI mode 20 (LNM) response + min window size |
| 7923bda | feat: CSI 21t title query + minimum window size enforcement |
| e090922 | feat: detect git:// and ssh:// URLs in terminal output |
| a8657a7 | feat: DECREQTPARM + resize size toast |
| 52c4b1d | feat: OSC 4 custom palette override support (base16-shell) |
| cd44c31 | improve: scroll indicator shows line count when scrolled up |
| 1d002bb | fix: preserve scrollback position when new output arrives |
| 018dbcc | fix: skip wide-character spacers when copying text |
| c86d437 | feat: bell indicator on background tabs |
| 479f1b2 | improve: SSH server key fingerprint logging |

### Key Features
1. OSC 4 palette overrides (base16-shell/wal/pywal compatibility)
2. DECREQTPARM, CSI 21t title query, DECRQM mode 20
3. git:// and ssh:// URL detection
4. Scrollback position preservation on new output
5. Hyperlink underline color consistency fix
6. Scroll indicator with percentage display
7. Minimum window size enforcement