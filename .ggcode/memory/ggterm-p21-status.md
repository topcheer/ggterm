## GGTerm Phase 21 — COMPLETE

### Build State (Final)
- `cargo fmt --all -- --check` = CLEAN
- `cargo clippy --features "desktop ai plugin plugin-lua config-watch" --workspace -- -D warnings` = CLEAN
- `cargo test --features "desktop ai plugin plugin-lua config-watch" --workspace` = **1543 tests ALL PASS** (2 ignored)

### Commits
- `711fa14` — feat: Phase 21 — drag-to-resize splits, session persistence, config validation, dirty rect repaint (12 files, +1244 lines)
- `205fa1e` — docs: README Phase 20-21 update + P21-G status bar error indicator
- `b8fbe19` — feat: P21-G status bar config error indicator + README Phase 21 update

### Phase 21 Tasks (5 tasks, all done)
| Task | Owner | Description |
|------|-------|-------------|
| P21-A | me (PM) | Drag-to-resize split separators: separator_at_point(), set_ratio_at_point(), drag_resize field, try_start_separator_drag() |
| P21-B | gg_dev | Session persistence: session.rs (710 lines), SessionData/SplitNodeData serde, save/load/clear_session |
| P21-C | ggcxf_dev | Config validation: Config::validate(), ConfigError::Validation, SettingsState error_message |
| P21-D | dd_dev | Dirty rect partial repaint: PaneRenderSpec.needs_prepare, render_pane_to_pass(needs_prepare), PaneSession.needs_reprepare |
| P21-G | ggcxf_dev | Status bar !ERROR! indicator: config_error field, set/clear/has_config_error methods, 7 tests |

### Test Count Growth
- Phase 20 complete: 1493 tests
- Phase 21 complete: 1543 tests (+50)