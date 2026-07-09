# Part 5: AI Assistant

## AI Features

GGTerm integrates an AI assistant that can explain terminal output, suggest commands, and convert natural language to shell commands.

### AI Shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+E` | Explain current output |
| `Ctrl+Shift+S` | Suggest next command |
| `Ctrl+Shift+H` | Help — ask anything about the terminal |
| `Ctrl+Shift+N` | Natural language to command |
| `Esc` | Dismiss AI overlay |
| `Tab` (in AI overlay) | Insert suggested command into terminal |
| `Ctrl+Enter` (in AI overlay) | Execute suggested command immediately |

### AI Configuration

```toml
[ai]
enabled = true
api_endpoint = "https://api.openai.com/v1"
model = "gpt-4"
```

The AI engine uses an OpenAI-compatible API client, so any compatible endpoint works.

### AI Context

When triggering AI features, GGTerm builds context from:
- Current terminal output (visible screen)
- Current working directory (OSC 7)
- Running command (OSC 133 marks)
- Exit code (OSC 133 D mark)

### Command Palette AI Commands

Via Command Palette (`Ctrl+Shift+P`):
- `ai.explain` — Explain output
- `ai.suggest` — Suggest command
- `ai.help` — General help

## Shell Switcher

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+L` | Quick shell switcher |

Opens a dropdown to switch between installed shells (bash, zsh, fish, etc.) in the current pane.

## Command History Sidebar

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+Y` | Toggle command history sidebar |

Features:
- Full history of commands run in the current session
- Synced from OSC 133 marks
- Shows exit code (green/red indicator)
- Click a command to re-run it

## Command Navigation

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+Up/Down` | Navigate between command blocks |

Uses OSC 133 marks to jump between previous command output blocks.

## Snippets

Snippet manager stores frequently used commands:
- CRUD via Command Palette
- TOML persistence
- Placeholder fill (e.g., `$USER`, `$HOST`)
- Accessed via Command Palette

## Broadcast Input

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+Alt+B` | Cycle broadcast mode |

Three modes:
1. **None** — Input goes to active pane only (default)
2. **AllPanes** — Input sent to all panes in the active tab
3. **AllTabs** — Input sent to all tabs' active panes

Status bar shows `BCAST:AllPanes` or `BCAST:AllTabs` when active.

Additional broadcast commands via Command Palette:
- Send `Ctrl+C` to all panes
- Reset all terminals

## Session Recording

Record terminal sessions in asciinema v2 format:
- Start/stop via Command Palette
- Status bar shows `REC` indicator
- Output: `.cast` file

## Workspaces

Workspaces separate groups of tabs:

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+Alt+W` | Cycle workspace |

Via Command Palette:
- `workspace.next` — Switch to next workspace
- `workspace.prev` — Switch to previous workspace
- `workspace.add` — Create new workspace

## Perf Monitor

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+G` | Toggle performance monitor |

Shows overlay with FPS, memory, cell counts, and PID information.

## Sound

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+M` | Toggle sound |

When enabled, terminal bell (`\a`) plays an audible sound. Status bar shows `SND` indicator.

## File Preview

When dragging files over the terminal, a preview card appears showing:
- File icon (by category: code, image, archive, etc.)
- File name and size
- Category-specific color

## Color Picker

When hovering over ANSI color sequences, a color swatch appears showing the hex value.

## Context Menu

Right-click in the terminal content area:

| Action | Description |
|--------|-------------|
| Copy | Copy selection |
| Paste | Paste from clipboard |
| Select All | Select all text |
| Search | Open search bar |
| Clear | Clear screen + scrollback |
| Reset | Reset terminal (RIS) |
| Split Horizontal | Split left/right |
| Split Vertical | Split top/bottom |

## Notifications

- **Desktop notifications**: OSC 9 (iTerm2) and OSC 777 (urxvt) protocols
- **Progress reports**: OSC 9;4 shows percentage in status bar
- **Bell**: Visual flash + optional sound
