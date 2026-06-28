# ggterm shell integration for Fish
# Emits OSC 133 marks for prompt/command/output boundaries.
#
# Install:
#   # Add to ~/.config/fish/config.fish:
#   source /path/to/ggterm/shell/fish.fish
#
# Protocol: https://gitlab.freedesktop.org/Per_Bothner/specifications/blob/master/proposals/semantic-prompts.md

# Only enable once
if set -q GGTERM_SHELL_INTEGRATION_FISH
    exit 0
end
set -g GGTERM_SHELL_INTEGRATION_FISH 1

# ── OSC 133 helpers ──

function __ggterm_osc133_A   # prompt start
    printf '\e]133;A\a'
end

function __ggterm_osc133_B   # command start
    printf '\e]133;B\a'
end

function __ggterm_osc133_C   # output start
    printf '\e]133;C\a'
end

function __ggterm_osc133_D   # command end (arg: exit code)
    set -l ec $argv[1]
    if test -z "$ec"
        set ec $status
    end
    printf '\e]133;D;%s\a' "$ec"
end

# ── Fish event handlers ──

# fish_prompt: wrap the original to emit A before prompt text
functions --copy fish_prompt __ggterm_saved_fish_prompt 2>/dev/null

function fish_prompt
    __ggterm_osc133_A
    __ggterm_saved_fish_prompt
end

# fish_preexec: fires when user submits a command line
function __ggterm_on_preexec --on-event fish_preexec
    __ggterm_osc133_B
    __ggterm_osc133_C
end

# fish_postexec: fires when a command finishes
function __ggterm_on_postexec --on-event fish_postexec
    __ggterm_osc133_D $status
end

# Emit initial prompt start on shell launch
__ggterm_osc133_A
