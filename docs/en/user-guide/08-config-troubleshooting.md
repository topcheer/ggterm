# Part 8: Configuration, Plugins & Troubleshooting

## Configuration File

Location: `~/.ggterm/config.toml`

### All Config Options

```toml
[appearance]
theme = "dark"                  # See themes list below
font_family = "monospace"        # Font family name
font_size = 14                   # Font size in pixels
cell_width = 8                   # Cell width in pixels
cell_height = 16                 # Cell height in pixels
cursor_style = "block"           # block | underline | bar
cursor_blink = true              # Cursor blink on/off
background_opacity = 1.0         # 0.0 transparent to 1.0 opaque
padding = 8                      # Content padding in pixels
cursor_line_highlight = false    # Highlight cursor line (Vim-style)
word_chars = ""                  # Extra word characters for selection

[terminal]
scrollback_lines = 10000         # Max scrollback history
shell = ""                       # Empty = $SHELL or /bin/sh
restore_session = false           # Restore tabs/splits on startup

[ai]
enabled = false
api_endpoint = ""
model = ""

[plugins]
enabled = false
directory = "~/.ggterm/plugins"

[keybindings]
# Customize keyboard shortcuts (see below)
new_tab = "Ctrl+T"
close_tab = "Ctrl+W"
new_split_horizontal = "Ctrl+Shift+D"
new_split_vertical = "Ctrl+Shift+\"
focus_next_pane = "Ctrl+Shift+]"
focus_prev_pane = "Ctrl+Shift+["
copy = "Ctrl+Shift+C"
paste = "Ctrl+Shift+V"
search = "Ctrl+Shift+F"
toggle_fullscreen = "F11"
zoom_in = "Ctrl+="
zoom_out = "Ctrl+-"
zoom_reset = "Ctrl+0"
reset_terminal = "Ctrl+Shift+R"
clear_screen = "Ctrl+Shift+K"
select_all = "Ctrl+Shift+A"
cycle_theme = "Ctrl+Shift+T"
open_url = "Ctrl+Shift+U"
command_palette = "Ctrl+Shift+P"
copy_cwd = "Ctrl+Shift+Alt+P"

[profiles.develop]
# Optional per-profile overrides
theme = "nord"
font_size = 12

[profiles.present]
theme = "light"
font_size = 18
```

### Config Management Shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+,` (Cmd+,) | Open config file in editor |
| `Ctrl+Shift+O` | Open config file (alternative) |
| `Ctrl+Shift+J` | Edit shell config (.bashrc/.zshrc) |
| `Ctrl+Shift+Alt+E` | Export config to clipboard (TOML) |
| `Ctrl+Shift+Alt+I` | Import config from clipboard |
| `Ctrl+Shift+Alt+R` | Reset config to defaults |
| `Ctrl+Shift+Alt+L` | Reload config from file |
| `Ctrl+,` | Open Settings panel |

### Hot-Reload

With the `config-watch` feature, changes to `config.toml` are detected automatically:
- Theme changes apply instantly
- Font size changes apply instantly
- Scrollback line limit updates
- Toast notification: "Config reloaded"

## Keybinding Customization

All keybindings can be customized in the `[keybindings]` section. Key format:

- Single keys: `F11`, `Escape`, `Tab`, `Enter`
- Modified keys: `Ctrl+T`, `Ctrl+Shift+D`, `Alt+H`
- Special: `Ctrl+Shift+/`, `Ctrl+Shift+\`

## Plugins

### Lua Plugins

```toml
[plugins]
enabled = true
directory = "~/.ggterm/plugins"
```

Example plugin:
```lua
-- ~/.ggterm/plugins/hello.lua
function on_load()
    print("Hello from GGTerm plugin!")
end

function on_resize(cols, rows)
    -- React to terminal resize
end
```

### Plugin Lifecycle

1. Plugins are loaded on startup from the configured directory
2. `on_load()` is called when the plugin is loaded
3. Lua runtime via `mlua`

## Session Persistence

```toml
[terminal]
restore_session = false  # default: clean startup
# restore_session = true  # restore tabs/splits from last session
```

- Session is saved immediately when a pane or tab closes
- On startup with `restore_session = true`, tabs/splits/working directories are restored
- Window position and size are also persisted

## SSH Configuration

GGTerm reads SSH configuration from:
- `~/.ssh/config` (importable via Command Palette)
- Connection manager stores entries in TOML
- Supports password and public key authentication

## Terminal Protocol Support

GGTerm implements a comprehensive set of terminal protocols:

| Protocol | Examples | Status |
|----------|---------|--------|
| SGR | Bold, italic, underline, blink, strikethrough, overline | Full |
| Cursor | CSI A/B/C/D/E/F/G/H, SCP/RCP, DECSC/DECRC | Full |
| Erase | ED, EL, DECSED (selective) | Full |
| Scroll | SU, SD, DECSET 7727 (alt scroll) | Full |
| Modes | DECSET 1/5/6/7/12/25/47/1000-1006/1015-1016/1047-1049/2004/2026/2027 | Full |
| OSC | 0/2/4/7/8/9/10-12/52/104/110-112/133/1337/9;4 | Full |
| DCS | XTGETTCAP, DECRQSS | Full |
| DA | DA1/DA2/DA3 | Full |
| DSR | Cursor position, status, window state | Full |
| DECRQM | All standard + private modes | Full |
| Kitty keyboard | CSI > u push/pop, CSI = u | Full |
| Character sets | G0/G1, US/UK/special graphics | Full |
| DECSCUSR | Cursor shape change (6 styles) | Full |
| Alt screen | DECSET 47/1047/1049 with grid save/restore | Full |

## Troubleshooting

### Font Issues

**Box-drawing characters show as squares (tofu):**
- macOS: Menlo Regular is used (not Bold) because Menlo Bold lacks box-drawing glyphs
- Bold is shown via bright color, not weight

**CJK characters not rendering:**
- Ensure `Shaping::Advanced` is enabled (default)
- Install CJK fonts on your system

### Terminal Stuck in Wrong Mode

If the shell behaves oddly after GGTerm crashes:
```bash
reset   # or: stty sane
```

GGTerm sends reset sequences on normal exit:
- Bracketed paste off
- Mouse tracking off
- Cursor keys normal
- Cursor visible
- Keypad numeric
- Soft reset (DECSTR)

### High CPU Usage When Idle

GGTerm sleeps 50ms when no redraw is needed. If CPU usage is high:
- Check for background processes producing terminal output
- Disable cursor blink: `cursor_blink = false`
- Check if `config-watch` is triggering excessive reloads

### Session Not Restoring

Set `restore_session = true` in config.toml. Session is saved on tab/pane close and on app exit.

### Tab Bar Text Invisible

This was a known bug (fixed). Overlay rendering order: backgrounds first, then text.

### Window Position Not Persisting

Window geometry is saved with session data. Enable `restore_session = true`.

### SSH Connection Issues

- Server key fingerprint is logged for verification
- Both password and public key auth supported
- Non-blocking I/O prevents UI freezes during connection

### P2P Connection Issues

- Ensure both devices are online
- Check firewall settings (QUIC uses UDP)
- Try the manual ticket entry if QR scanning fails
- iroh relay fallback handles most NAT scenarios

### Getting Help

- Press `Ctrl+Shift+/` for in-app shortcut help
- Press `Ctrl+Shift+H` for AI-powered help
- Check logs: `ggterm -vv` for debug output
- GitHub Issues: https://github.com/topcheer/ggterm/issues
