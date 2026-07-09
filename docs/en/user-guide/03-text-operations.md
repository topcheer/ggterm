# Part 3: Text Selection, Copy & Paste

## Text Selection

### Selection Modes

| Action | Result |
|--------|--------|
| Click + Drag | Normal text selection |
| `Alt` + Click + Drag | Block (rectangular) selection |
| Double-click | Select word |
| Triple-click | Select entire line |

### Keyboard-Driven Selection

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+A` | Select all text |
| `Shift+Arrows` | Extend selection by character |
| `Ctrl+Shift+Left/Right` | Extend selection by word |

### Selection Highlight

Selected text is highlighted with a semi-transparent blue overlay. For block selection, per-row rectangles are rendered.

### Selection Word Count

When text is selected, the status bar shows character and word count:
- `SEL:42c/7w` — 42 characters, 7 words
- `w` suffix omitted when 0 words (whitespace-only selection)

## Copy

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+C` | Copy selection to clipboard |
| `Ctrl+Insert` | Copy selection (Linux/Windows convention) |
| `Ctrl+Shift+Alt+H` | Copy selection as HTML (with colors) |
| `Ctrl+Shift+Alt+O` | Copy last command output (uses OSC 133 marks) |
| `Ctrl+Shift+Alt+P` | Copy current working directory path |

Additional copy commands via Command Palette:
- **Copy visible text** — copies only the visible screen (no scrollback)
- **Copy as Markdown** — converts terminal output to Markdown format
- **Copy as HTML** — preserves colors and formatting

### Smart Copy Trimming

Leading and trailing empty lines are automatically stripped from copied text.

## Paste

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+V` | Paste from clipboard |
| `Shift+Insert` | Paste (Linux/Windows convention) |
| Middle-click | Paste selection (X11-style) |

### Bracketed Paste

When the shell supports bracketed paste (most modern shells do):
- Pasted content is wrapped with `ESC[200~ ... ESC[201~`
- The shell can handle multi-line paste safely

### Safe Paste

When bracketed paste is NOT supported:
- Trailing newlines are stripped to prevent accidental command execution
- Toast notification: "Pasted first line (N lines stripped)"

## Search

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+F` | Toggle floating search bar |
| `Enter` | Next match |
| `Shift+Enter` | Previous match |
| `Tab` (in search) | Toggle case sensitivity |
| `Shift+Tab` (in search) | Toggle regex search mode |
| `Up/Down` (in search) | Navigate search history |
| `Esc` | Close search bar |

### Search Features

- **Floating search bar** with match counter (e.g., "3/12 matches")
- **Search history**: last 20 queries saved, navigable with Up/Down
- **Case toggle**: case-sensitive or insensitive
- **Regex mode**: full regular expression support
- **Highlighting**: matches highlighted in the terminal grid

Additional search commands via Command Palette:
- **Search selection** — search using the currently selected text
- **Search on web** — search selected text in browser

## Export

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+Alt+S` | Save scrollback to text file (`~/ggterm-export-{timestamp}.txt`) |
| `Ctrl+Shift+Alt+E` | Export terminal as HTML (with colors) |

Export commands via Command Palette:
- **Export scrollback** — plain text file
- **Export as HTML** — preserves colors, formatting, hyperlinks

## Terminal Lock

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+Alt+L` | Toggle terminal lock (read-only mode) |

When locked, all keyboard input is blocked. Useful when you want to read output without accidentally typing.

## Scrolling

### Keyboard Scrolling

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+Space` | Toggle scrollback browse mode (less-style: j/k/G/g/d/u/q) |
| `Shift+PageUp` | Scroll up one page |
| `Shift+PageDown` | Scroll down one page |
| `Shift+Home` | Scroll to top of scrollback |
| `Shift+End` | Scroll to bottom |
| `Ctrl+Shift+End` | Scroll to bottom (alternative) |
| `Ctrl+Shift+Alt+Up` | Scroll to mark (OSC 1337 SetMark) |

### Mouse Scrolling

- **Scroll wheel**: Scroll scrollback history
- **Shift+Scroll**: Sync-scroll all panes simultaneously
- **Scrollbar**: Click or drag the thin scrollbar on the right edge

### Scroll Indicators

- **Scroll-to-bottom pill**: Blue pill with down arrow appears when scrolled up; click to jump to bottom
- **Percentage indicator**: Shows scroll position as percentage (e.g., "45%")
- **Line count**: Shows line count when scrolled >99 lines (e.g., "127 lines")

### Smooth Inertial Scrolling

Trackpad momentum is supported with exponential decay interpolation for smooth scrolling.

### Alternate Screen Scroll

In full-screen apps (vim, less, htop) without mouse tracking:
- Mouse wheel is automatically converted to arrow key presses (DECSET 7727)
- Shift+wheel bypasses this and scrolls the viewport
