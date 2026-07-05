# ggterm shell integration for Zsh
# Emits OSC 133 marks for prompt/command/output boundaries.
#
# Install:
#   echo 'source /path/to/ggterm/shell/zsh.zsh' >> ~/.zshrc
#
# Protocol: https://gitlab.freedesktop.org/Per_Bothner/specifications/blob/master/proposals/semantic-prompts.md

# Only enable once
if [[ -n "$GGTERM_SHELL_INTEGRATION_ZSH" ]]; then
    return 0
fi
GGTERM_SHELL_INTEGRATION_ZSH=1

# ── Conflict detection ──
# If starship, powerlevel10k, or another tool already sends OSC 133
# marks, skip our integration to avoid duplicate/conflicting marks
# (which causes issues like the spinner always spinning).
__ggterm_osc133_already_handled() {
    # Starship sends OSC 133 when add_newline + shell integration is on
    if [[ -n "$STARSHIP_SHELL_INTEGRATION" ]]; then
        return 0
    fi
    # Powerlevel10k instant prompt sends its own marks
    if [[ -n "$POWERLEVEL9K_INSTANT_PROMPT" ]] \
       && [[ "$POWERLEVEL9K_INSTANT_PROMPT" != "quiet" ]]; then
        return 0
    fi
    # oh-my-zsh terminal-app plugin on macOS
    if [[ -n "$TERM_PROGRAM" && "$TERM_PROGRAM" == "Apple_Terminal" ]]; then
        return 0
    fi
    # iTerm2 shell integration
    if [[ -n "$ITERM_SESSION_ID" ]]; then
        return 0
    fi
    # Warp terminal
    if [[ -n "$WARP_HONOR_PS1" ]]; then
        return 0
    fi
    return 1
}

if __ggterm_osc133_already_handled; then
    return 0
fi

# ── OSC 133 helpers ──

__ggterm_osc133_A() { printf '\e]133;A\a'; }   # prompt start
__ggterm_osc133_B() { printf '\e]133;B\a'; }   # command start
__ggterm_osc133_C() { printf '\e]133;C\a'; }   # output start
__ggterm_osc133_D() {                             # command end
    local ec="${1:-$?}"
    printf '\e]133;D;%s\a' "$ec"
}

# ── Zsh hook functions ──

# precmd: runs before each prompt is drawn.
# At this point the previous command has finished.
__ggterm_zsh_precmd() {
    local ec=$?
    __ggterm_osc133_D "$ec"   # D: end previous command
    __ggterm_osc133_A          # A: start new prompt
}

# preexec: runs after Enter is pressed, before command starts.
__ggterm_zsh_preexec() {
    __ggterm_osc133_B          # B: command start
    __ggterm_osc133_C          # C: output start
}

# ── Register hooks (use add-zsh-hook when available) ──
autoload -Uz add-zsh-hook 2>/dev/null
if typeset -f add-zsh-hook >/dev/null 2>&1; then
    add-zsh-hook precmd  __ggterm_zsh_precmd
    add-zsh-hook preexec __ggterm_zsh_preexec
else
    # Fallback: append to zsh hook arrays
    precmd_functions+=(__ggterm_zsh_precmd)
    preexec_functions+=(__ggterm_zsh_preexec)
fi

# Emit initial prompt start on shell launch
__ggterm_osc133_A
