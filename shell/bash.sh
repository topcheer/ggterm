# ggterm shell integration for Bash
# Emits OSC 133 marks for prompt/command/output boundaries.
#
# Install:
#   echo 'source /path/to/ggterm/shell/bash.sh' >> ~/.bashrc
#
# Protocol: https://gitlab.freedesktop.org/Per_Bothner/specifications/blob/master/proposals/semantic-prompts.md

# Only enable once
if [ -n "$GGTERM_SHELL_INTEGRATION_BASH" ]; then
    return 0
fi
GGTERM_SHELL_INTEGRATION_BASH=1

# ── Conflict detection ──
# Skip if another tool already sends OSC 133 marks.
__ggterm_osc133_already_handled() {
    # Starship sends OSC 133 when shell integration is enabled
    if [ -n "$STARSHIP_SHELL_INTEGRATION" ]; then
        return 0
    fi
    # iTerm2 shell integration
    if [ -n "$ITERM_SESSION_ID" ]; then
        return 0
    fi
    # Warp terminal
    if [ -n "$WARP_HONOR_PS1" ]; then
        return 0
    fi
    # Check if PROMPT_COMMAND already contains OSC 133
    if echo "$PROMPT_COMMAND" | grep -q "133;A" 2>/dev/null; then
        return 0
    fi
    return 1
}

if __ggterm_osc133_already_handled; then
    return 0
fi

# ── OSC 133 helpers ──

# A: prompt start — before shell draws the prompt
__ggterm_osc133_A() { printf '\e]133;A\a'; }

# B: command start — user pressed Enter, command is about to run
__ggterm_osc133_B() { printf '\e]133;B\a'; }

# C: output start — command output is about to begin
__ggterm_osc133_C() { printf '\e]133;C\a'; }

# D: command end — command finished, with exit code
# Args: $1 = exit code (defaults to $?)
__ggterm_osc133_D() {
    local ec="${1:-$?}"
    printf '\e]133;D;%s\a' "$ec"
}

# ── State ──
# Track whether we've already seen a preexec for the current command cycle.
# This prevents the DEBUG trap from double-firing inside PROMPT_COMMAND.
__ggterm_preexec_done=0

# ── Pre-command hook (fires before each command via DEBUG trap) ──
__ggterm_preexec() {
    # Skip during command completion
    [ -n "$COMP_LINE" ] && return

    # Only fire once per command cycle
    [ "$__ggterm_preexec_done" = "1" ] && return
    __ggterm_preexec_done=1

    # B: command start, C: output start
    __ggterm_osc133_B
    __ggterm_osc133_C
}

# ── Pre-prompt hook (fires before each prompt via PROMPT_COMMAND) ──
__ggterm_precmd() {
    local ec=$?

    # D: end the previous command (if there was one)
    if [ "$__ggterm_preexec_done" = "1" ]; then
        __ggterm_osc133_D "$ec"
    fi

    # Reset for next cycle
    __ggterm_preexec_done=0

    # A: prompt start
    __ggterm_osc133_A
}

# ── Integration with Bash hooks ──

# Chain PROMPT_COMMAND (preserve existing value)
if [ -n "$PROMPT_COMMAND" ]; then
    __ggterm_saved_prompt_command="$PROMPT_COMMAND"
    PROMPT_COMMAND='__ggterm_precmd; __ggterm_saved_prompt_command'
else
    PROMPT_COMMAND='__ggterm_precmd'
fi

# Use DEBUG trap for preexec (Bash has no native preexec hook)
# This is the standard technique used by iTerm2, Kitty, WezTerm, etc.
trap '__ggterm_preexec' DEBUG

# Emit initial prompt start on shell launch
__ggterm_osc133_A
