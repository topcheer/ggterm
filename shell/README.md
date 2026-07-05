# ggterm Shell Integration

Shell integration scripts that emit [OSC 133 semantic prompt marks](https://gitlab.freedesktop.org/Per_Bothner/specifications/blob/master/proposals/semantic-prompts.md), enabling ggterm to:

- **Detect command boundaries** — know exactly which output belongs to which command
- **Track exit codes** — see whether each command succeeded or failed
- **Navigate command blocks** — jump between commands with keyboard shortcuts
- **Mark prompt/output regions** — visually distinguish prompts, commands, and output

## OSC 133 Protocol

| Mark | Sequence | When |
|------|----------|------|
| **A** — PromptStart | `OSC 133;A ST` | Before the shell draws the prompt |
| **B** — CommandStart | `OSC 133;B ST` | After user presses Enter, before command runs |
| **C** — OutputStart | `OSC 133;C ST` | Before command output begins |
| **D** — CommandEnd | `OSC 133;D;exitcode ST` | After command finishes, with exit code |

`ST` = string terminator (`BEL` `\a` or `ESC \\`)

### Command Lifecycle

```
A (prompt start)
  └─ prompt text displayed
B (command start — Enter pressed)
  └─ command text
C (output start)
  └─ command output lines
D;exitcode (command end)
```

## Installation

### Bash

```bash
# Add to ~/.bashrc
echo "source $(pwd)/shell/bash.sh" >> ~/.bashrc
```

Or manually edit `~/.bashrc`:
```bash
# ggterm shell integration
source /path/to/ggterm/shell/bash.sh
```

### Zsh

```zsh
# Add to ~/.zshrc
echo "source $(pwd)/shell/zsh.zsh" >> ~/.zshrc
```

Or manually edit `~/.zshrc`:
```zsh
# ggterm shell integration
source /path/to/ggterm/shell/zsh.zsh
```

### Fish

```fish
# Add to ~/.config/fish/config.fish
echo "source $(pwd)/shell/fish.fish" >> ~/.config/fish/config.fish
```

Or manually edit `~/.config/fish/config.fish`:
```fish
# ggterm shell integration
source /path/to/ggterm/shell/fish.fish
```

## Verification

After sourcing the script, run a command and check if ggterm detects command blocks:

```bash
echo "hello"
```

In ggterm, you should see:
- Command block boundary markers
- Exit code display (0 for success)
- Keyboard navigation between commands

## How It Works

### Bash

Bash has no native `preexec` hook. We use:
- **`PROMPT_COMMAND`**: Chained with any existing value. Runs before each prompt → emits `C`, `D;exitcode`, `A`.
- **`trap ... DEBUG`**: Fires before each command → emits `B`. This is the standard workaround used by most terminal emulators (iTerm2, Kitty, WezTerm).

### Zsh

Zsh has native `precmd` and `preexec` hooks:
- **`precmd_functions`**: Runs before each prompt → emits `C`, `D;exitcode`, `A`.
- **`preexec_functions`**: Runs after Enter, before command → emits `B`.
- Uses `add-zsh-hook` when available for clean composition.

### Fish

Fish has event-based hooks:
- **`fish_prompt`**: Wrapped to emit `A` before the original prompt.
- **`fish_preexec` event**: Emits `B` (command start) and `C` (output start) when a command is submitted.
- **`fish_postexec` event**: Emits `D;exitcode` when a command finishes.

## Conflict Avoidance

All three shells (bash, zsh, fish) detect common sources of duplicate OSC 133 marks and skip integration when found:

- **Starship**: `$STARSHIP_SHELL_INTEGRATION` set
- **iTerm2**: `$ITERM_SESSION_ID` set
- **Apple Terminal**: `$TERM_PROGRAM == Apple_Terminal`
- **Warp**: `$WARP_HONOR_PS1` set
- **Powerlevel10k** (zsh only): `$POWERLEVEL9K_INSTANT_PROMPT` set and not `quiet`

If you use a tool not listed here that sends OSC 133 marks, set one of these environment variables to prevent duplicate marks.

## Compatibility

These scripts are compatible with:
- Bash 4.0+
- Zsh 5.0+
- Fish 3.3+ (for `fish_preexec`/`fish_postexec` events)

## Files

```
shell/
├── bash.sh      # Bash integration
├── zsh.zsh      # Zsh integration
├── fish.fish    # Fish integration
└── README.md    # This file
```
