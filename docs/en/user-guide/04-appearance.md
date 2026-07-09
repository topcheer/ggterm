# Part 4: Themes, Fonts & Appearance

## Themes

### 9 Built-in Themes + Auto

| Theme | Background | Style |
|-------|-----------|-------|
| `dark` | Dark gray | Default dark theme |
| `light` | White | Bright environments |
| `dracula` | Dark purple | Popular dark theme |
| `solarized-dark` | Deep blue | Developer focus |
| `solarized-light` | Warm cream | Reading |
| `gruvbox` | Earthy dark | Retro warm |
| `nord` | Arctic blue | Clean minimal |
| `tokyo-night` | Deep navy | Night coding |
| `catppuccin-mocha` | Soft brown | Warm dark |
| `auto` | Follows OS | Seamless switching |

### Theme Controls

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+Alt+T` | Cycle through themes |
| `Ctrl+Shift+T` | Cycle themes (alternative) |

### Auto Theme

When `theme = "auto"`, GGTerm detects the OS appearance:
- **macOS**: AppleInterfaceStyle
- **Linux**: GTK_THEME / gsettings
- **Windows**: Registry check

### Dynamic Colors (OSC 10/11/12)

Programs can override theme colors at runtime:
- `OSC 10` — Set/query foreground color
- `OSC 11` — Set/query background color
- `OSC 12` — Set/query cursor color
- `OSC 104/110/111/112` — Reset to defaults

### Custom Palette (OSC 4)

Programs like base16-shell, wal, and pywal can set custom 16-color palettes:
- `OSC 4 ; N ; rgb:RR/GG/BB` — Set palette color N
- `OSC 104 ; N` — Reset palette color N
- Renderer applies overrides to indexed colors

## Fonts

### Font Controls

| Shortcut | Action |
|----------|--------|
| `Ctrl+=` | Zoom in (font size +1.5px) |
| `Ctrl+-` | Zoom out (font size -1.5px) |
| `Ctrl+0` | Reset to default font size |
| `Ctrl+Shift+Wheel` | Zoom font with mouse wheel |

### Platform Default Fonts

| Platform | Font |
|----------|------|
| macOS | Menlo (Regular only — Bold variant lacks box-drawing glyphs) |
| Linux | DejaVu Sans Mono |
| Windows | Cascadia Mono |

**Bold text**: Distinguished by bright color, not font weight (xterm/Alacritty standard).

**CJK fallback**: `Shaping::Advanced` enables automatic font fallback for CJK characters.

### Cell Dimensions

- Cell width = exact float advance of 'M' (no rounding)
- Cell height = font size in pixels
- Real pixel dimensions reported via CSI 14t/15t/16t

## Background Opacity

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+Alt+]` | Increase opacity (+5%) |
| `Ctrl+Shift+Alt+[` | Decrease opacity (-5%) |

Opacity range: 0.0 (fully transparent) to 1.0 (fully opaque). Toast notification shows percentage.

Config: `[appearance] background_opacity = 0.85`

## Window Controls

| Shortcut | Action |
|----------|--------|
| `F11` | Toggle fullscreen |
| `Ctrl+Shift+Enter` | Toggle maximized |
| `Ctrl+Shift+Alt+A` | Toggle always-on-top |
| `Ctrl+Shift+B` | Toggle status bar |

### Transparent Titlebar (macOS)

On macOS, the titlebar is made transparent for a seamless look.

## Cursor

### Cursor Styles

Config: `[appearance] cursor_style = "block"`

Options: `block`, `underline`, `bar`

Programs can change cursor style via DECSCUSR (CSI N q).

### Cursor Blink

Config: `[appearance] cursor_blink = true`

- Blink uses sine-wave alpha for smooth fade
- Blink resets on user input
- Blink phase shared with SGR 5 blink text rendering

### Cursor Line Highlight

Config: `[appearance] cursor_line_highlight = false`

Highlights the entire line where the cursor is positioned (like Vim's `cursorline`).

### Cursor Effects

Via Command Palette:
- **cursor.trail** — Cursor leaves a particle trail
- **cursor.glow** — Cursor has a glow effect
- **cursor.none** — Disable cursor effects

## Status Bar

Toggle: `Ctrl+Shift+B`

The status bar shows:
- Cursor position (row:col)
- Tab count
- Current directory (from OSC 7)
- Remote host (SSH indicator from OSC 1337)
- Running command + timer
- Progress percentage (from OSC 9;4)
- Broadcast mode indicator
- Recording indicator
- Pane zoom indicator
- Bell indicator
- Sound toggle indicator
- Selection word count
- Config error indicator

## Profiles

Profiles allow switching between appearance configurations:

```toml
[profiles.develop]
theme = "nord"
font_size = 12

[profiles.present]
theme = "light"
font_size = 18
```

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+Alt+F` | Cycle config profile |
| `Ctrl+Shift+Alt+P` | Cycle profiles (alternative) |

## Settings Panel

| Shortcut | Action |
|----------|--------|
| `Ctrl+,` | Open Settings panel |

Navigate settings with arrow keys, edit values inline.

## Debug Overlays

| Shortcut | Action |
|----------|--------|
| `F1` | Toggle debug overlay (FPS, cell counts, pane info) |
| `Ctrl+Shift+G` | Toggle performance monitor |

## Per-Pane Rendering

Each pane maintains independent renderer state:
- Reverse video mode (DECSCNM)
- Dynamic foreground/background colors (OSC 10/11)
- Underline color (SGR 58)
- Blink text phase (SGR 5)
