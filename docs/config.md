# GGTerm Configuration

GGTerm reads its configuration from `~/.ggterm/config.toml` (TOML format).
If the file does not exist, built-in defaults are used.

## File Location

| Platform | Path |
|----------|------|
| macOS / Linux | `~/.ggterm/config.toml` |
| Windows | `%USERPROFILE%\.ggterm\config.toml` |

## Hot Reload

GGTerm watches the config file for changes and applies them at runtime
without restarting. Theme switches, scrollback limit changes, and AI
settings take effect immediately.

## Configuration Reference

### `[appearance]`

Visual rendering settings.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `theme` | string | `"dark"` | Theme name: `dark`, `light`, `solarized` |
| `font_family` | string | `"monospace"` | Font family name (resolved by glyphon) |
| `font_size` | integer | `14` | Font size in pixels |
| `cell_width` | integer | `8` | Cell width in pixels |
| `cell_height` | integer | `16` | Cell height in pixels |

### `[terminal]`

Terminal behaviour settings.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `scrollback_lines` | integer | `10000` | Maximum scrollback history lines |
| `shell` | string | `""` | Shell program path. Empty = use `$SHELL` or `/bin/sh` |

### `[ai]`

AI engine settings.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | boolean | `false` | Enable AI features at startup |
| `api_endpoint` | string | `"https://api.openai.com/v1"` | LLM API endpoint URL |
| `model` | string | `"gpt-4o-mini"` | Model identifier |

## Example Configuration

```toml
# ~/.ggterm/config.toml

[appearance]
theme = "solarized"
font_family = "JetBrains Mono"
font_size = 15
cell_width = 9
cell_height = 18

[terminal]
scrollback_lines = 50000
shell = "/usr/bin/zsh"

[ai]
enabled = true
api_endpoint = "https://api.openai.com/v1"
model = "gpt-4o"
```

## Partial Configuration

All fields are optional. Missing keys fall back to defaults:

```toml
# Only override theme and font size — everything else uses defaults
[appearance]
theme = "light"
font_size = 16
```

## Error Handling

If the config file contains invalid TOML, GGTerm logs a warning and
falls back to default values. The application does not crash on config
errors.
