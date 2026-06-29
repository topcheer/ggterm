# Command Navigation

GGTerm's command navigation feature lets you jump between command blocks
using keyboard shortcuts. It's powered by the [OSC 133 shell integration
protocol](https://gitlab.freedesktop.org/Per_Bothner/specifications/blob/master/proposals/semantic-prompts.md).

## Prerequisites

Install the shell integration script for your shell:

```bash
# bash
echo 'source /path/to/ggterm/shell/bash.sh' >> ~/.bashrc

# zsh
echo 'source /path/to/ggterm/shell/zsh.zsh' >> ~/.zshrc

# fish
echo 'source /path/to/ggterm/shell/fish.fish' >> ~/.config/fish/config.fish
```

## OSC 133 Protocol

The shell emits semantic markers that GGTerm parses:

| Mark | Sequence | When |
|------|----------|------|
| **PromptStart** | `OSC 133;A ST` | Before the prompt is drawn |
| **CommandStart** | `OSC 133;B ST` | After Enter, before command execution |
| **OutputStart** | `OSC 133;C ST` | Before command output |
| **CommandEnd** | `OSC 133;D;exitcode ST` | After command finishes |

`ST` = string terminator (`BEL` or `ESC \`)

### Command Lifecycle

```
[A] $ ls -la           <- prompt
[B]                     <- Enter pressed
[C] total 42            <- output begins
drwxr-xr-x  5 user staff 160 Jan 1 12:00 src
-rw-r--r--  1 user staff 42 Jan 1 12:00 README.md
[D;0]                   <- command finished, exit code 0
```

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+Up` | Jump to previous command block |
| `Ctrl+Shift+Down` | Jump to next command block |
| `Ctrl+Shift+H` | Toggle command navigation status bar |

## Status Bar

When command navigation is active, a status bar appears at the bottom of
the terminal showing:

- **Command text** of the selected block (truncated if long)
- **Exit status** with color coding:
  - Green = success (exit code 0)
  - Red = failure (non-zero exit code)
  - Yellow = running (no exit code yet)

## CommandBlock API

Developers can access command blocks programmatically:

```rust
use ggterm_core::Terminal;

let terminal = Terminal::new(80, 24);
// ... after processing PTY output with OSC 133 marks ...

for block in terminal.command_blocks() {
    println!("Command at row {}: {:?} (exit: {:?})",
        block.prompt_row,
        block.is_success(),
        block.exit_code,
    );
}

// Check last command status
if terminal.last_command_succeeded() {
    println!("Last command succeeded!");
}

// Get last exit code
if let Some(code) = terminal.last_exit_code() {
    println!("Exit code: {}", code);
}
```
