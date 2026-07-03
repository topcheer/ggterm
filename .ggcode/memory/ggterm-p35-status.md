## GGTerm Phase 35 — Session Persistence Fix + Unified Rendering

### Build State
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace --lib` = **1740 tests ALL PASS** (7 ignored)

### Commits
| Commit | Description |
|--------|-------------|
| 2d2dbc8 | fix: session restore now opt-in, immediate save on pane/tab close |
| 5e0709b | docs: document restore_session option in config.example.toml |

### Key Changes
1. **`restore_session` config option** (default: false) — Session restore is now opt-in via `[terminal] restore_session = true` in config.toml
2. **Immediate session save** — session.json written immediately when pane/tab closes, not just on app exit
3. **Clear stale session** — when restore_session=false, session.json is deleted on exit
4. **Deleted stale session.json** that was causing unwanted 2-pane startup

### Config Example
```toml
[terminal]
restore_session = false  # default: clean startup
# restore_session = true  # restore tabs/splits from last session
```
