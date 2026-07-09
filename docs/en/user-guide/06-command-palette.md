# Part 6: Command Palette

The Command Palette (`Ctrl+Shift+P`) provides fuzzy-search access to all GGTerm actions.

## Using the Command Palette

1. Press `Ctrl+Shift+P` to open
2. Type to fuzzy-search commands
3. `Up/Down` to navigate results
4. `Enter` to execute
5. `Esc` to close

## Complete Command List

### Tab Management

| Command | Action |
|---------|--------|
| `tab.new` | New tab |
| `tab.close` | Close tab |
| `tab.next` | Next tab |
| `tab.prev` | Previous tab |
| `tab.toggle_last` | Toggle last tab |
| `tab.rename` | Rename tab |
| `tab.move_left` | Move tab left |
| `tab.move_right` | Move tab right |
| `tab.duplicate` | Duplicate tab |
| `tab.close_others` | Close other tabs |
| `tab.toggle_pin` | Pin/Unpin tab |
| `tab.reopen_closed` | Reopen closed tab |
| `window.new` | New window |

### Split Panes

| Command | Action |
|---------|--------|
| `split.horizontal` | Split horizontal |
| `split.vertical` | Split vertical |
| `split.focus_next` | Focus next pane |
| `split.focus_prev` | Focus previous pane |
| `split.zoom` | Toggle pane zoom |
| `split.balance` | Balance panes |
| `split.swap` | Swap pane content |
| `split.close` | Close current pane |

### Terminal Operations

| Command | Action |
|---------|--------|
| `terminal.clear` | Clear screen |
| `terminal.clear_all` | Clear screen + scrollback |
| `terminal.reset` | Reset terminal (RIS) |
| `terminal.reset_all` | Reset all terminals |
| `terminal.select_all` | Select all text |
| `terminal.copy` | Copy selection |
| `terminal.copy_cwd` | Copy current directory |
| `terminal.paste` | Paste |
| `terminal.search` | Search scrollback |
| `terminal.open_url` | Open URL at cursor |
| `terminal.save_scrollback` | Save scrollback to file |
| `terminal.export_html` | Export as HTML |
| `terminal.copy_as_html` | Copy as HTML |
| `terminal.copy_last_output` | Copy last command output |
| `terminal.copy_visible` | Copy visible text |
| `terminal.copy_markdown` | Copy as Markdown |
| `terminal.toggle_lock` | Toggle terminal lock |
| `terminal.scroll_mode` | Toggle scrollback browse mode |
| `terminal.open_in_finder` | Open cwd in Finder/Explorer |
| `terminal.open_shell_config` | Edit shell config (.bashrc/.zshrc) |
| `terminal.import_ssh` | Import SSH hosts from ~/.ssh/config |
| `terminal.edit_selection` | Edit selected text |
| `terminal.run_selection` | Run selected text as command |
| `terminal.search_selection` | Search selected text on web |
| `terminal.send_ctrl_c_all` | Send Ctrl+C to all panes |
| `terminal.new_session` | New SSH session |

### Appearance

| Command | Action |
|---------|--------|
| `theme.cycle` | Cycle theme |
| `font.zoom_in` | Zoom in |
| `font.zoom_out` | Zoom out |
| `font.zoom_reset` | Reset font size |
| `opacity.increase` | Increase opacity |
| `opacity.decrease` | Decrease opacity |
| `view.toggle_cursor_line` | Toggle cursor line highlight |

### Window

| Command | Action |
|---------|--------|
| `view.fullscreen` | Toggle fullscreen |
| `view.maximize` | Toggle maximized |
| `view.status_bar` | Toggle status bar |
| `window.always_on_top` | Toggle always-on-top |
| `settings.open` | Open Settings panel |
| `config.open` | Open config file |
| `config.reload` | Reload config |

### AI

| Command | Action |
|---------|--------|
| `ai.explain` | Explain output |
| `ai.suggest` | Suggest command |
| `ai.help` | AI help |

### Sessions & Profiles

| Command | Action |
|---------|--------|
| `session.save` | Save session |
| `session.profile` | Cycle profiles |
| `ssh.manager` | Open SSH connection manager |

### Cursor Effects

| Command | Action |
|---------|--------|
| `cursor.trail` | Enable cursor particle trail |
| `cursor.glow` | Enable cursor glow |
| `cursor.none` | Disable cursor effects |

### Other

| Command | Action |
|---------|--------|
| `perf.toggle` | Toggle performance monitor |
| `sound.toggle` | Toggle sound |
| `shell.switch` | Open shell switcher |
| `workspace.next` | Next workspace |
| `workspace.prev` | Previous workspace |
| `workspace.add` | Add workspace |
