## GGTerm Phase 31 — 3 Features Delivered

### Build State
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test` = **1735 tests ALL PASS** (7 ignored)
- 52,248 lines Rust across 8 crates

### Phase 31 Commits
| Commit | Description |
|--------|-------------|
| 8dffa1c | feat: new tabs and split panes inherit working directory (OSC 7) |
| d741e33 | feat: window position and size persistence across restarts |
| 79382a5 | feat: profile cycling and config export shortcuts |
| cb5bbb0 | improve: add new shortcuts to help overlay |

### Phase 31 Features
| Task | Description | Interaction |
|------|-------------|-------------|
| P31-A | CWD inheritance: new tabs/splits open in active pane's OSC 7 cwd | Automatic when shell reports cwd |
| P31-B | Window geometry persistence: position+size saved/restored | Automatic on close/startup |
| P31-C | Profile cycling: Ctrl+Shift+Alt+P switches profiles with toast | Ctrl+Shift+Alt+P |
| P31-C | Config export: Ctrl+Shift+Alt+E exports TOML to clipboard | Ctrl+Shift+Alt+E |

### Key Architecture
1. PtySession::open_with_cwd() — new method with optional cwd parameter
2. PaneSession::new_with_cwd() + TabSession::new_with_cwd() / split_horizontal_with_cwd / split_vertical_with_cwd
3. SessionData gains window_x/y/width/height (serde Optional fields)
4. capture_session() saves outer_position() + inner_size() as logical pixels
5. ConfigManager gains config_mut() accessor for profile application
6. cycle_profile + export_config methods on DesktopApp with toast feedback