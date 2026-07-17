# ggterm shell integration for Fish
# Emits OSC 133 marks for prompt/command/output boundaries.
#
# Install:
#   # Add to ~/.config/fish/config.fish:
#   source /path/to/ggterm/shell/fish.fish
#
# Protocol: https://gitlab.freedesktop.org/Per_Bothner/specifications/blob/master/proposals/semantic-prompts.md

# Only enable once
# NOTE: In fish, `exit` kills the entire shell session!
# We must use a conditional block instead of early return.
if not set -q GGTERM_SHELL_INTEGRATION_FISH
    set -g GGTERM_SHELL_INTEGRATION_FISH 1

    # ── Conflict detection ──
    # Skip if another tool already sends OSC 133 marks.
    set -l _ggterm_skip 0
    if set -q STARSHIP_SHELL_INTEGRATION
        set _ggterm_skip 1
    end
    if set -q ITERM_SESSION_ID
        set _ggterm_skip 1
    end
    if test "$TERM_PROGRAM" = "Apple_Terminal"
        set _ggterm_skip 1
    end
    if set -q WARP_HONOR_PS1
        set _ggterm_skip 1
    end
    # WezTerm sends its own OSC 133 marks
    if set -q WEZTERM_EXECUTABLE
        set _ggterm_skip 1
    end
    # Ghostty terminal
    if test "$TERM_PROGRAM" = "ghostty"
        set _ggterm_skip 1
    end

    if test "$_ggterm_skip" = 0

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
            set -l real_status $status
            set -l ec $argv[1]
            if test -z "$ec"
                set ec $real_status
            end
            printf '\e]133;D;%s\a' "$ec"
        end

        # ── Fish event handlers ──

        # fish_prompt: wrap the original to emit A before prompt text
        functions --copy fish_prompt __ggterm_saved_fish_prompt 2>/dev/null

        function fish_prompt
            __ggterm_osc133_A
            __ggterm_saved_fish_prompt

            # OSC 7: report current working directory for CWD tracking.
            # Enables new tab/split to inherit CWD, and status bar display.
            printf '\e]7;file://%s%s\a' (hostname) "$PWD"
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

    end
end
