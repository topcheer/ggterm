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
