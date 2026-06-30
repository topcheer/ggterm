## GGTerm Phase 24 — COMPLETE

### Commits
- `32af7ce` — feat: Phase 24 — protocol completeness & developer experience
- `64fca45` — docs: update README with Phase 22-24 status

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace --lib` = **1429 tests ALL PASS** (7 ignored)

### Phase 24 Tasks (6 tasks, all done)
| Task | Owner | Tests | Description |
|------|-------|-------|-------------|
| P24-A | me | 2 | Synchronized output (DECSET 2026) — is_synchronized() for batch rendering |
| P24-B | me | 2 | Text reflow on resize (DECSET 2027) — reflow_enabled() mode (default: true) |
| P24-C | me | — | Debug overlay — F1 toggle, FPS counter, cell counts in window title |
| P24-D | me | 6 | DECSCA/DECSED selective erase — PROTECTED flag, 3 erase modes |
| P24-E | me | 4 | OSC 9/777 desktop notifications — iTerm2 + urxvt protocols, macOS osascript |
| P24-F | me | — | Full workspace verification + commit |

### Key Architecture Changes
1. **CellFlags::PROTECTED** (0x400) — new bitflag for DECSCA protected cells
2. **Modes.synchronized_output** — DECSET 2026, renderer can defer updates
3. **Modes.reflow** — DECSET 2027 (default true), controls grid reflow on resize
4. **Terminal.protected_attr** — current DECSCA state, applied to new cells in put_printable_char
5. **Terminal.pending_notification** — (title, body) pair from OSC 9/777
6. **DECSED handler** — `b'J' if is_private` must come BEFORE regular `b'J'` in CSI match
7. **Debug overlay** — debug_visible, frame_count, current_fps fields on DesktopApp

### Test Count Growth
- Phase 23 complete: 1615 tests (desktop features)
- Phase 24 complete: 1429 lib tests (mobile+desktop lib, +12 in ggterm-core)
- ggterm-core: 294 → 306 (+12)

### Files Changed
1. `crates/ggterm-core/src/grid/cell.rs` — PROTECTED flag
2. `crates/ggterm-core/src/term/mod.rs` — Modes, DECSET 2026/2027, DECSCA, DECSED, OSC 9/777, 12 tests
3. `crates/ggterm-app/src/window/mod.rs` — debug_visible, FPS counter, notification polling
4. `crates/ggterm-app/src/window/handlers.rs` — F1 debug toggle
5. `crates/ggterm-app/src/window/actions.rs` — poll_notification()
6. `README.md` — Phase 22-24 status table
