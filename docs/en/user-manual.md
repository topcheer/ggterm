# GGTerm User Manual

> **Version:** Phase 55+ | **Platform:** macOS, Linux, Windows, iOS, Android

## Installation

### Build from Source

```bash
# Prerequisites: Rust stable, clang, pkg-config

# Clone
git clone https://github.com/topcheer/ggterm.git
cd ggterm

# Build and run
cargo run --features "desktop ai plugin plugin-lua config-watch" --bin ggterm

# Release build
cargo build --release --features "desktop ai plugin plugin-lua config-watch" --bin ggterm
# Binary: target/release/ggterm
```

### Pre-built Releases

Download from [GitHub Releases](https://github.com/topcheer/ggterm/releases):
- **macOS**: Universal .dmg (Apple Silicon + Intel)
- **Linux**: .deb package or tarball
- **Windows**: .zip

## CLI Usage

```bash
ggterm [OPTIONS]

Options:
  -c, --cols <N>            Initial columns (default: 80)
  -r, --rows <N>            Initial rows (default: 24)
  -s, --shell <PATH>        Shell path (default: $SHELL)
  -t, --title <TITLE>       Window title (default: "GGTerm")
      --theme <NAME>        Color theme (default: "dark")
      --font-size <PX>      Font size in pixels (default: 16)
      --cell-width <PX>     Cell width (default: 8)
  -w, --working-directory <DIR>  Start shell in this directory
  -C, --config <PATH>       Custom config file
  -e, --execute <CMD...>    Execute command instead of interactive shell
      --hold                Keep terminal open after command exits
      --fullscreen          Start in fullscreen
      --maximize            Start maximized
  -v                        Verbose logging (-v info, -vv debug, -vvv trace)
```

### Examples

```bash
# Default terminal
ggterm

# Large terminal with zsh
ggterm --cols 120 --rows 40 --shell /bin/zsh

# Dracula theme, font size 18
ggterm --theme dracula --font-size 18

# Run vim and hold after exit
ggterm -e vim --hold

# Start fullscreen
ggterm --fullscreen
```

## Configuration

Config file: `~/.ggterm/config.toml` (see `config.example.toml` for all options).

### Appearance

```toml
[appearance]
theme = "dark"                # dark, light, dracula, solarized-dark,
                              # solarized-light, gruvbox, nord,
                              # tokyo-night, catppuccin-mocha, auto
font_family = "monospace"
font_size = 14
cursor_style = "block"         # block | underline | bar
background_opacity = 1.0       # 0.0 transparent to 1.0 opaque
cursor_blink = true            # Cursor blink on/off
```

### Terminal

```toml
[terminal]
scrollback_lines = 10000
shell = ""                     # Empty = $SHELL or /bin/sh
restore_session = false        # Restore tabs/splits on startup
```

### AI

```toml
[ai]
enabled = true
api_endpoint = "https://api.openai.com/v1"
model = "gpt-4"
```

### Profiles

```toml
[profiles.develop]
theme = "nord"
font_size = 12

[profiles.present]
theme = "light"
font_size = 18
```

Cycle profiles with `Ctrl+Shift+Alt+P`.

### Keybindings

Customize keyboard shortcuts in `[keybindings]` section. See `config.example.toml` for all options.

## Themes

9 built-in themes + auto mode:

| Theme | Background | Best for |
|-------|-----------|----------|
| `dark` | Dark gray | General use |
| `light` | White | Daytime / bright environments |
| `dracula` | Dark purple | Popular dark theme |
| `solarized-dark` | Deep blue | Developer focus |
| `solarized-light` | Warm cream | Reading |
| `gruvbox` | Earthy dark | Retro warm |
| `nord` | Arctic blue | Clean minimal |
| `tokyo-night` | Deep navy | Night coding |
| `catppuccin-mocha` | Soft brown | Warm dark |
| `auto` | Follows OS | Seamless switching |

Cycle themes: `Ctrl+Shift+T`

## Keyboard Shortcuts

### Tabs

| Shortcut | Action |
|----------|--------|
| `Ctrl+T` | New tab |
| `Ctrl+W` | Close tab |
| `Alt+1-9` | Switch to tab N |
| `Ctrl+Tab` | Next tab |
| `Ctrl+Shift+Tab` | Previous tab |
| `Ctrl+Shift+PageUp` | Move tab left |
| `Ctrl+Shift+PageDown` | Move tab right |
| `Ctrl+Shift+Alt+D` | Duplicate tab |
| `Ctrl+Shift+Alt+W` | Close other tabs |

### Split Panes

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+D` | Split horizontal (left/right) |
| `Ctrl+Shift+\` | Split vertical (top/bottom) |
| `Ctrl+Shift+[` | Focus previous pane |
| `Ctrl+Shift+]` | Focus next pane |
| `Alt+H/J/K/L` | Vim-style pane navigation |
| `Ctrl+Shift+X` | Swap pane content |
| `Ctrl+Shift+Z` | Toggle pane zoom |
| `Ctrl+Shift+Alt+Arrows` | Adjust split ratio |
| `Ctrl+Shift+Alt+N` | Reset to single pane |

### Font & Theme

| Shortcut | Action |
|----------|--------|
| `Ctrl+=` | Zoom in (font size +1.5px) |
| `Ctrl+-` | Zoom out (font size -1.5px) |
| `Ctrl+0` | Reset font size |
| `Ctrl+Shift+Wheel` | Zoom font with mouse wheel |
| `Ctrl+Shift+T` | Cycle themes |
| `Ctrl+Shift+Alt+P` | Cycle profiles |

### Terminal Operations

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+C` | Copy selection |
| `Ctrl+Shift+V` | Paste from clipboard |
| `Shift+Insert` | Paste (cross-platform) |
| `Ctrl+Shift+K` | Clear screen + scrollback |
| `Ctrl+Shift+R` | Reset terminal (RIS) |
| `Ctrl+Shift+A` | Select all text |
| `Ctrl+Shift+U` | Open URL at cursor |
| `Ctrl+Shift+Alt+S` | Export scrollback to file |
| `Ctrl+Shift+Alt+Up` | Scroll to mark |
| `Ctrl+Shift+End` | Scroll to bottom |

### Search

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+F` | Toggle search bar |
| `Enter` | Next match |
| `Shift+Enter` | Previous match |
| `Tab` (in search) | Toggle case sensitivity |
| `Up/Down` (in search) | Search history |

### AI Assistant

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+E` | Explain current output |
| `Ctrl+Shift+S` | Suggest command |
| `Ctrl+Shift+H` | Help |
| `Ctrl+Shift+N` | Natural language → command |
| `Esc` | Dismiss AI overlay |

### Window & Display

| Shortcut | Action |
|----------|--------|
| `F11` | Toggle fullscreen |
| `Ctrl+Shift+Enter` | Toggle maximized |
| `Ctrl+Shift+B` | Toggle status bar |
| `F1` | Toggle debug overlay |
| `Ctrl+Shift+G` | Toggle performance monitor |

### Advanced

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+P` | Command palette |
| `Ctrl+Shift+/` | Shortcut help overlay |
| `Ctrl+Shift+L` | Shell switcher |
| `Ctrl+Shift+Y` | Command history sidebar |
| `Ctrl+Shift+M` | Toggle sound (bell) |
| `Ctrl+Shift+Alt+B` | Cycle broadcast mode |
| `Ctrl+Shift+Alt+P` | Copy current working directory |
| `Ctrl+Shift+Alt+E` | Export config to clipboard |
| `Ctrl+Shift+Alt+[` | Decrease opacity |
| `Ctrl+Shift+Alt+]` | Increase opacity |
| `Ctrl+Shift+Alt+Q` | Toggle P2P share (QR code) |

### Mouse

| Action | Result |
|--------|--------|
| Click+Drag | Text selection |
| Alt+Click+Drag | Block (rectangular) selection |
| Double-click | Select word |
| Triple-click | Select line |
| Middle-click | Paste selection |
| Cmd/Ctrl+Click | Open URL/hyperlink |
| Scroll wheel | Scroll scrollback |
| Shift+Scroll | Sync-scroll all panes |
| Ctrl+Shift+Scroll | Zoom font |

## P2P Terminal Sharing

Share your desktop terminal with a mobile device via QR code.

### Desktop (Host)

1. Press `Ctrl+Shift+Alt+Q` to open the share overlay
2. A QR code appears with your connection ticket
3. Scan the QR code with the mobile app (or copy the ticket string)
4. Once connected, the mobile device mirrors your terminal
5. Press `Esc` or `Ctrl+Shift+Alt+Q` to close sharing

### Mobile (Client)

1. Tap **Scan QR** in the connection screen
2. Point camera at the desktop QR code
3. Terminal output appears on mobile
4. Type on mobile keyboard to send input

## Mobile App

### Connection Options

| Option | Description |
|--------|-------------|
| SSH | Connect to remote server via SSH (host, port, user, password) |
| Echo Test | Diagnostic — echoes typed characters back (no server needed) |
| Scan QR | P2P connect to desktop terminal via QR code |
| Share Terminal | P2P host mode (Android only — requires local shell) |

### iOS vs Android

- **iOS**: SSH + P2P client (Scan QR) only — no local terminal
- **Android**: All features including local shell + P2P host

## Shell Integration

GGTerm auto-injects OSC 133 marks for command detection in bash, zsh, and fish.

Enable manually by adding to your shell config:

```bash
# bash (~/.bashrc)
source /path/to/ggterm/shell/bash.sh

# zsh (~/.zshrc)
source /path/to/ggterm/shell/zsh.zsh

# fish (~/.config/fish/config.fish)
source /path/to/ggterm/shell/fish.fish
```

## Plugin System

```toml
[plugins]
enabled = true
directory = "~/.ggterm/plugins"
```

Lua plugins:
```lua
-- ~/.ggterm/plugins/hello.lua
print("Hello from GGTerm plugin!")
```

## Troubleshooting

### Font Issues

**Box-drawing characters show as squares (tofu):**
- On macOS, GGTerm uses Menlo Regular (not Bold) because Menlo Bold lacks box-drawing glyphs.
- Bold text is shown via bright color instead of weight.

**CJK characters not rendering:**
- Ensure `Shaping::Advanced` is enabled (default).
- Install CJK fonts on your system.

### Terminal Stuck in Wrong Mode

If the shell behaves oddly after GGTerm crashes:
```bash
reset   # or: stty sane
```

GGTerm sends reset sequences on normal exit (bracketed paste off, mouse off, cursor keys normal, soft reset).

### High CPU Usage When Idle

GGTerm sleeps 50ms when no redraw is needed. If CPU usage is high:
- Check for background processes producing terminal output
- Disable cursor blink: `cursor_blink = false` in config
- Check if `config-watch` is triggering excessive reloads

### Session Not Restoring

Set `restore_session = true` in config.toml. Session is saved on tab/pane close and on app exit.
