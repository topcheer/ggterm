## GGTerm Session — Scrollback Export, OSC 1337, Vim Nav, OSC 12

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace --lib` = **1826 tests ALL PASS** (7 ignored)
- LOC: ~59,500

### Commits This Session (7 commits)
| Commit | Description |
|--------|-------------|
| 40abd42 | feat: CI/CD pipelines for desktop + mobile, documentation consolidation |
| 45e1a46 | feat: scrollback export to file (Ctrl+Shift+Alt+S) |
| a2090cc | feat: OSC 1337 iTerm2 shell integration (CurrentDir, RemoteHost, SetMark, ClearScrollback) |
| 62e9cef | feat: SSH remote host indicator in status bar (OSC 1337 RemoteHost) |
| 633e530 | feat: vim-style pane navigation (Alt+H/J/K/L) |
| ebad002 | feat: OSC 12 dynamic cursor color tracking (set + query) |

### Features Delivered (5 new features)

1. **Scrollback Export to File** (Ctrl+Shift+Alt+S)
   - Grid::export_text() exports entire scrollback + visible screen as plain text
   - Saves to ~/ggterm-export-{timestamp}.txt with toast notification
   - 3 tests in grid/mod.rs

2. **OSC 1337 iTerm2 Shell Integration**
   - CurrentDir=<path> — updates cwd (complements OSC 7)
   - RemoteHost=user@host — tracks remote SSH host
   - SetMark — records scrollback mark row
   - ClearScrollback — clears scrollback history
   - Terminal gains remote_host + mark_row fields
   - 4 tests in term/mod.rs

3. **SSH Remote Host in Status Bar**
   - StatusBar gains remote_host field, shows "SSH:user@host" segment
   - Wired from Terminal::remote_host() in about_to_wait
   - 1 test in status_bar.rs

4. **Vim-Style Pane Navigation** (Alt+H/J/K/L)
   - Alt+J/L → next pane, Alt+H/K → prev pane
   - Compatible with vim-tmux-navigator muscle memory
   - Added to shortcut help overlay

5. **OSC 12 Dynamic Cursor Color**
   - Terminal gains dynamic_cursor: Option<Color> field
   - OSC 12 set now properly stores color (was no-op)
   - OSC 12 query returns stored color (was falling back to fg)
   - 2 tests in term/mod.rs

### CI/CD (from earlier in session)
- ci.yml: fmt, clippy, test (Linux+macOS), FFI test, build (Linux+macOS+Windows), Flutter analyze, coverage
- release-desktop.yml: macOS universal .dmg, Linux .deb, Windows .zip (tag v*)
- release-mobile.yml: Android APK + iOS IPA (tag v*)
- 3 release scripts: build-macos-app.sh, build-android-ffi.sh, build-ios-ffi.sh
- .gitignore hardened for credential safety
- README rewritten with badges, download table, architecture diagram

### Test Count Growth
- Previous: 1816 tests
- Current: 1826 tests (+10)
