## GGTerm Phase 30 — 3 Features Delivered

### Build State
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test` = **1735 tests ALL PASS** (7 ignored)
- 52,000+ lines Rust across 8 crates

### Phase 30 Commits
| Commit | Description |
|--------|-------------|
| d7f14ca | P30-A: Visual scrollbar with click/drag support |
| 2a0b238 | P30-B: Tab rename + tab bar click-to-switch + toast notifications |
| eb00d9f | docs: Phase 30 README shortcuts |

### Phase 30 Features
| Task | Description | Interaction |
|------|-------------|-------------|
| P30-A | Scrollbar: thin 4px bar on right showing scroll position | Click or drag scrollbar |
| P30-B | Tab bar click-to-switch + double-click rename | Click tab, Double-click tab |
| P30-B | "+" new tab button clickable | Click + in tab bar |
| P30-C | Toast notifications: "Copied N chars" with fade | Auto after copy |

### Architecture Notes
- P30-A: scroll_to_scrollbar_pos() converts pixel Y → display_offset delta
- P30-B: renaming_tab: Option<usize> + rename_text: String intercept keyboard
- P30-C: toast: Option<(String, u32)> with 120-frame (~2s) countdown in about_to_wait
- Scrollbar auto-hides when scrollback_len == 0
- copy_selection_to_clipboard changed from &self to &mut self (toast trigger)