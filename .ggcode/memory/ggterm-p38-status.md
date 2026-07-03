## GGTerm Session — Blink Text, Keypad Mode, Shift+Insert, FFI Test Fix

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test` = **1767 tests ALL PASS** (7 ignored)
- LOC: ~55,000

### Commits This Session
| Commit | Description |
|--------|-------------|
| 0e4d1de | fix: SSH transport — non-blocking buffered I/O prevents UI freeze |
| 03589b1 | fmt: fix formatting in SSH lib |
| 58c2dc6 | feat: blink text rendering, keypad application mode, Shift+Insert paste |

### Features Delivered (4 changes)

1. **Blink Text Rendering (SGR 5)** — text with blink attribute fades alpha based on cursor blink timer
   - TextRun gains `blink: bool` field in converter.rs
   - GlyphonRenderer gains `blink_phase: f32` field + `set_blink_phase()` method
   - When blink_phase > 0.5, blink runs get alpha=0 (invisible)
   - Wired from cursor_blink alpha in render.rs

2. **Keypad Application Mode (DECPAM/DECPNM)** — ESC = / ESC > support
   - Modes struct gains `keypad_app: bool` field
   - Terminal esc() handler: b'=' sets keypad_app=true, b'>' sets false
   - InputEncoder gains `keypad_app_mode` field + `set_keypad_app_mode()` setter
   - handlers.rs syncs cursor_keys_app() and keypad_app() from terminal before encoding
   - Public accessors: `cursor_keys_app()`, `keypad_app()` on Terminal

3. **Shift+Insert Paste** — now works on ALL platforms (was Linux/Windows only)
   - Changed condition from `!cfg!(target_os = "macos")` to unconditional

4. **FFI Test Isolation Fix** — t_destroy_cleans_up no longer uses hardcoded count=1
   - Was `assert_eq!(ggterm_session_count(), 1)` → failed with parallel tests
   - Now uses relative `count_before` pattern like other tests

### SSH Non-Blocking I/O (from earlier this session)
- Rewrote SshSession to use background tokio task + shared buffers
- read() drains Arc<Mutex<Vec<u8>>> — instant, non-blocking
- write() pushes to mpsc channel — instant, non-blocking
- resize() pushes to mpsc channel — instant, non-blocking
- Background task: 5ms polling read loop + non-blocking write/resize drain
- Eliminates all block_on() calls from the critical FFI path

### Test Count Growth
- Previous: 1750 tests
- Current: 1767 tests (+3 in ggterm-core: DECPAM, DECPNM, SGR blink)
- ggterm-ffi: 49 tests (fixed test isolation, was intermittently failing)
