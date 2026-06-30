## GGTerm P15-B: New Built-in Themes — COMPLETE

### Files Changed
- `crates/ggterm-render/src/theme.rs` only

### 4 New Themes
1. **light()** — bg=(250,250,250), fg=(40,40,40)
2. **solarized_dark()** — bg=base03(0,43,54), fg=base0(131,148,150), shared SOLARIZED_PALETTE
3. **solarized_light()** — bg=base3(253,246,227), fg=base00(101,123,131), shared SOLARIZED_PALETTE
4. **gruvbox()** — bg=bg0(40,40,40), fg=fg0(235,219,178), full 16-color gruvbox palette

### API Updates
- `by_name()`: supports solarized-dark, solarized-light, gruvbox (also underscore variants)
- `builtin_names()`: returns 6 themes now (was 3)
- `ThemeManager::cycle_next()`: cycles through all 6 themes with wrap-around
- `SOLARIZED_PALETTE` const: shared 16-color palette for both solarized variants

### Tests: 9 new (36 total in ggterm-render, all pass)
- t_light_theme_colors, t_solarized_dark_colors, t_solarized_light_colors, t_gruvbox_colors
- t_by_name_new_themes, t_by_name_solarized_underscore
- t_builtin_names_includes_new (verifies 6 themes)
- t_cycle_next_wraps_around, t_cycle_next_visits_all_themes

### Build State
- `cargo clippy -p ggterm-render -- -D warnings` = CLEAN
- `cargo test -p ggterm-render` = 36 passed, 0 failed
- Full workspace: ggterm-app has pre-existing AlphaMode error (not from this change)