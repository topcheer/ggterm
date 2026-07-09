# Part 2: Tabs, Panes & Splits

## Tabs

### Tab Management

| Shortcut | Action |
|----------|--------|
| `Ctrl+T` | Open new tab |
| `Ctrl+W` | Close current tab |
| `Alt+1-9` | Switch to tab N |
| `Ctrl+Tab` | Next tab |
| `Ctrl+Shift+Tab` | Previous tab |
| `Ctrl+Shift+\`` | Toggle last tab (switch between two most recent) |
| `Ctrl+Shift+T` | Reopen last closed tab |
| `Ctrl+Shift+N` | Open new window |
| `Ctrl+Shift+I` | Rename current tab |
| `Ctrl+Shift+PageUp` | Move tab left |
| `Ctrl+Shift+PageDown` | Move tab right |
| `Ctrl+Shift+Alt+D` | Duplicate current tab (same shell + cwd) |
| `Ctrl+Shift+Alt+W` | Close all other tabs |

### Tab Interactions

- **Click tab**: Switch to that tab
- **Double-click tab**: Rename it
- **Middle-click tab**: Close it (browser-style)
- **Drag tab**: Reorder among siblings
- **Right-click tab**: Context menu (Close, Close Others, Close Right, Pin/Unpin, Split)
- **Click "+"**: Dropdown menu (New Tab, Split Horizontal, Split Vertical)

### Tab Pinning

Pin a tab via Command Palette to prevent accidental close:
- Pinned tabs show a pin indicator
- `Ctrl+W` is ignored on pinned tabs
- Unpin via Command Palette to close

### Tab Title Sync

Tab titles automatically sync with the running program:
- Shows program name from OSC 0/2 (e.g., "vim", "htop", "less")
- Falls back to shell name (e.g., "zsh", "bash")
- Shows bell indicator when a background tab receives a bell
- Shows `(alt)` when in alternate screen mode

## Split Panes

### Creating Splits

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+D` | Split horizontal (left | right) |
| `Ctrl+Shift+\` | Split vertical (top / bottom) |

New panes inherit the active pane's working directory (from OSC 7).

### Pane Navigation

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+[` | Focus previous pane |
| `Ctrl+Shift+]` | Focus next pane |
| `Alt+H` | Focus left pane (vim-style) |
| `Alt+J` | Focus down pane (vim-style) |
| `Alt+K` | Focus up pane (vim-style) |
| `Alt+L` | Focus right pane (vim-style) |

- **Click pane**: Switch focus to that pane
- **Mouse wheel over pane**: Scroll that pane's content

### Pane Operations

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+X` | Swap active pane content with next |
| `Ctrl+Shift+Z` | Toggle pane zoom (maximize/restore) |
| `Ctrl+Shift+Alt+Arrows` | Adjust split ratio |
| `Ctrl+Shift+Alt+B` | Balance split panes (even spacing) |
| `Ctrl+Shift+Alt+N` | Reset layout to single pane |

### Pane Zoom

`Ctrl+Shift+Z` toggles zoom mode:
- When zoomed: active pane fills the entire window
- Pane borders are hidden
- Mouse focus is locked to the active pane
- Separator drag is disabled
- Status bar shows `ZOOM` indicator

### Separator Drag

- Drag the separator between panes to resize them
- Separator drag is disabled when zoomed

### Multi-Pane Rendering

- Each pane renders its own terminal grid independently
- Active pane has a bright blue border
- Inactive panes have dim borders
- Pane gap: 6px between panes
- Scissor rect ensures content doesn't bleed across pane boundaries
