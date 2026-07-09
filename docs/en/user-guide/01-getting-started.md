# Part 1: Getting Started

## Installation

### Build from Source

```bash
git clone https://github.com/topcheer/ggterm.git
cd ggterm

# Debug build
cargo run --features "desktop ai plugin plugin-lua config-watch" --bin ggterm

# Release build (faster startup, optimized)
cargo build --release --features "desktop ai plugin plugin-lua config-watch" --bin ggterm
# Binary: target/release/ggterm
```

### Pre-built Releases

Download from [GitHub Releases](https://github.com/topcheer/ggterm/releases):
- **macOS**: Universal .dmg (Apple Silicon + Intel)
- **Linux**: .deb package or tarball
- **Windows**: .zip

## Command-Line Interface

```bash
ggterm [OPTIONS]

Options:
  -c, --cols <N>              Initial columns (default: 80)
  -r, --rows <N>              Initial rows (default: 24)
  -s, --shell <PATH>          Shell path (default: $SHELL)
  -t, --title <TITLE>         Window title (default: "GGTerm")
      --theme <NAME>          Color theme (default: "dark")
      --font-size <PX>        Font size in pixels (default: 16)
      --cell-width <PX>       Cell width (default: 8)
  -w, --working-directory <DIR>  Start shell in this directory
  -C, --config <PATH>         Custom config file path
  -e, --execute <CMD...>      Execute command instead of interactive shell
      --hold                  Keep terminal open after command exits
      --fullscreen            Start in fullscreen mode
      --maximize              Start maximized
  -v                          Verbose logging (-v info, -vv debug, -vvv trace)
```

### CLI Examples

```bash
# Default terminal
ggterm

# Large terminal with zsh
ggterm --cols 120 --rows 40 --shell /bin/zsh

# Dracula theme, font size 18
ggterm --theme dracula --font-size 18

# Run vim and hold after exit
ggterm -e vim --hold

# Start fullscreen in specific directory
ggterm --fullscreen --working-directory ~/projects

# Custom config file
ggterm --config ~/.config/ggterm/custom.toml
```

## Configuration File

Location: `~/.ggterm/config.toml`

```toml
[appearance]
theme = "dark"                # 9 themes + auto
font_family = "monospace"
font_size = 14
cursor_style = "block"         # block | underline | bar
cursor_blink = true
background_opacity = 1.0       # 0.0 transparent to 1.0 opaque
# padding = 8                 # Content padding in pixels
# cursor_line_highlight = false
# word_chars = ""             # Extra word characters for selection

[terminal]
scrollback_lines = 10000
shell = ""                     # Empty = $SHELL or /bin/sh
restore_session = false        # Restore tabs/splits on startup

[ai]
enabled = false
api_endpoint = ""
model = ""

[plugins]
enabled = false
directory = "~/.ggterm/plugins"

[keybindings]
# See Part 8: Configuration

[profiles.develop]
# Optional overrides per profile
theme = "nord"
font_size = 12
```

## First Run

1. GGTerm starts with your default shell in a single tab
2. Shell integration (OSC 133) is auto-injected for bash/zsh/fish
3. The config file is created at `~/.ggterm/config.toml` on first use
4. Press `Ctrl+Shift+/` anytime to see all keyboard shortcuts

## Shell Integration

GGTerm auto-injects OSC 133 marks for:
- Command detection (prompt/command/output boundaries)
- Exit code tracking
- Command history sidebar
- "Copy last command output" feature

Manual setup (if auto-injection fails):

```bash
# bash (~/.bashrc)
source /path/to/ggterm/shell/bash.sh

# zsh (~/.zshrc)
source /path/to/ggterm/shell/zsh.zsh

# fish (~/.config/fish/config.fish)
source /path/to/ggterm/shell/fish.fish
```
