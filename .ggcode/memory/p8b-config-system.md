P8-B Config system implemented:
- crates/ggterm-app/src/config.rs
- Config struct: AppearanceConfig (theme, font_family, font_size, cell_width, cell_height), TerminalConfig (scrollback_lines, shell), AiConfig (enabled, api_endpoint, model)
- TOML parsing via serde + toml workspace deps
- ConfigManager: load_default(), load_from(path), reload() with change detection, on_change callback
- Default config path: ~/.ggterm/config.toml
- 15 tests, all pass. Commit: 10b737a
- workspace deps added: serde = { version = "1", features = ["derive"] }, toml = "0.8"