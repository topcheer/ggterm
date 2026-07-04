## GGTerm Phase 42 — DECSC Fix + Hyperlink Underline + Scroll Preserve + SSH Fingerprint

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test` = **1846 tests ALL PASS** (7 ignored)

### Commits This Session (5 commits)
| Commit | Description |
|--------|-------------|
| 3e4e579 | fix: DECSC/DECRC (ESC 7/8) now saves/restores full terminal state (SGR attrs, charset, autowrap) |
| bf19c1a | improve: OSC 8 hyperlinks now render with underline (like web links) |
| e99a772 | fix: preserve scroll position on terminal resize (was resetting to bottom) |
| 83c82fe | docs: update config.example.toml with all themes and copy_cwd keybinding |
| 479f1b2 | improve: SSH server key fingerprint logging (SHA256:base64 format) |

### Features Delivered

1. **DECSC/DECRC Full State Save** (3e4e579) — Bug fix
   - ESC 7 now saves: cursor position + pending_wrap, SGR attributes (fg, bg, underline_color, flags), character set (G0, G1, active_g1), autowrap mode
   - ESC 8 restores all saved state; without prior save, resets cursor to (0,0)
   - SCP/RCP (CSI s/u) retains old cursor-only behavior
   - 4 new tests

2. **OSC 8 Hyperlink Underline** (bf19c1a) — UI improvement
   - Hyperlinked cells now render with underline even if UNDERLINE flag not set
   - Combined with existing blue tint → proper link appearance

3. **Scroll Position Preservation on Resize** (e99a772) — UX fix
   - Previously resizing the window reset display_offset to 0, losing user's scroll position
   - Now saves and restores display_offset, clamped to new scrollback size
   - 2 new tests

4. **SSH Server Key Fingerprint** (479f1b2) — Security improvement
   - SSH connections now compute SHA-256 fingerprint of server public key
   - Logged at info level for security auditing
   - Removed TODO comment, replaced with working implementation
   - Standard OpenSSH "SHA256:base64" format
   - Includes self-contained base64 encoder (no new dependency)

5. **Config Example Update** (83c82fe) — Documentation
   - Updated theme list to include nord, tokyo-night, catppuccin-mocha
   - Added copy_cwd keybinding documentation

### Test Count Growth
- Previous: 1840 tests
- Current: 1846 tests (+6)

### Key Files Modified
1. `crates/ggterm-core/src/term/mod.rs` — DecscState struct, ESC 7/8 full save/restore, 4 tests
2. `crates/ggterm-core/src/grid/mod.rs` — resize preserves display_offset, 2 tests
3. `crates/ggterm-render-wgpu/src/converter.rs` — hyperlink underline rendering
4. `crates/ggterm-ssh/src/lib.rs` — server key fingerprint, base64 encoder
5. `config.example.toml` — updated themes list