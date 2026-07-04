## GGTerm Phase 46 — Protocol Completeness & Tab Management

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace --lib` = **1888 tests ALL PASS** (7 ignored)
- LOC: ~60,800

### Commits (5 commits)
| Commit | Description |
|--------|-------------|
| 7a016b6 | feat: kitty keyboard protocol, DCS/XTGETTCAP parser, duplicate/close-other tabs |
| 1a4cec4 | feat: DECRQSS status string query (SGR + DECSTBM) via DCS |
| b0450d0 | feat: OSC 1337 SetUserVar + bare hostname URL detection |
| 2acca03 | feat: additional XTWINOPS modes (13t, 15t, 19t) |

### Features Delivered

1. **Kitty Keyboard Protocol** (CSI >/< u push/pop, CSI = u set/query)
   - Flag stack for modern TUI apps (nvim, kakoune)
   - 4 bit flags: disambiguate, report events, report alt keys, report all as escapes

2. **DCS Parser Rewrite**
   - Proper param/intermediate/string state machine (was: discard-all)
   - DcsEntry → DcsParam/DcsIntermediate → DcsString → DcsEsc → dispatch
   - Perform trait gains dcs() callback

3. **XTGETTCAP** (DCS + q)
   - Responds to TN (terminal name), Co (colors), RGB (truecolor), BG, FG
   - Hex encode/decode for capability names and values
   - Enables tmux/nvim terminal detection

4. **DECRQSS** (DCS $ q selector ST)
   - SGR query ("m"): reports current SGR attribute flags
   - DECSTBM query ("r"): reports scroll region (top;bottom)

5. **Tab Management**
   - Duplicate tab (Ctrl+Shift+Alt+D) — same shell + cwd
   - Close other tabs (Ctrl+Shift+Alt+W) — keeps only active tab
   - Both in command palette + shortcut help

6. **OSC 1337 SetUserVar**
   - Stores user variables (tmux integration)
   - Terminal::user_var() accessor

7. **Bare Hostname URL Detection**
   - github.com/user/repo, example.com:8080/api detected as clickable
   - 21 common TLDs, requires path/port after hostname

8. **XTWINOPS Extensions**
   - CSI 13t (window position), CSI 15t (screen pixels), CSI 19t (screen chars)

### Key Architecture Changes
1. **VTE Parser**: New DcsEntry/DcsParam/DcsIntermediate states for proper DCS parsing
2. **Perform trait**: gains dcs(intermediates, params, final_byte, data) method
3. **Modes struct**: gains kitty_keyboard: u32 field
4. **Terminal**: gains kitty_kb_stack, user_vars fields
5. **InputEncoder**: no changes (kitty encoding not yet wired to output)

### Test Count Growth
- Previous: 1872 tests
- Current: 1888 tests (+16)