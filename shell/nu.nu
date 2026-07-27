# ggterm shell integration for Nushell
# Emits OSC 133 marks for prompt/command/output boundaries.
#
# Install:
#   # Add to ~/.config/nushell/config.nu:
#   source /path/to/ggterm/shell/nu.nu
#
# Protocol: https://gitlab.freedesktop.org/Per_Bothner/specifications/blob/master/proposals/semantic-prompts.md

# Only enable once
if 'GGTERM_SHELL_INTEGRATION_NU' in $env {
    return
}
$env.GGTERM_SHELL_INTEGRATION_NU = '1'

# ── Conflict detection ──
# Skip if another tool already sends OSC 133 marks.
if 'STARSHIP_SHELL_INTEGRATION' in $env { return }
if 'ITERM_SESSION_ID' in $env { return }
if 'WARP_HONOR_PS1' in $env { return }
if 'WEZTERM_EXECUTABLE' in $env { return }
if ($env | get TERM_PROGRAM | default '') == 'ghostty' { return }

# ── OSC 133 helpers ──

def ggterm-osc133-A [] { print -n "\x1b]133;A\a" }   # prompt start
def ggterm-osc133-B [] { print -n "\x1b]133;B\a" }   # command start
def ggterm-osc133-C [] { print -n "\x1b]133;C\a" }   # output start
def ggterm-osc133-D [] {
    # D includes the exit code of the last command
    let ec = ($env | get LAST_EXIT_CODE | default 0)
    print -n $"\x1b]133;D;($ec)\a"
}

# ── Register hooks ──

# Ensure $env.config.hooks exists
if ($env.config | get hooks) == null {
    $env.config.hooks = {}
}

# pre_prompt: fires before each prompt is drawn.
# At this point the previous command has finished.
$env.config.hooks.pre_prompt = (
    $env.config.hooks
        | get pre_prompt
        | default []
        | append {||
            ggterm-osc133-D   # D: end previous command
            ggterm-osc133-A   # A: start new prompt

            # OSC 7: report current working directory for CWD tracking.
            # Enables new tab/split to inherit CWD, and status bar display.
            let host = (hostname | str trim)
            print -n $"\x1b]7;file://($host)($env.PWD)\a"
        }
)

# pre_execution: fires after Enter is pressed, before command starts.
$env.config.hooks.pre_execution = (
    $env.config.hooks
        | get pre_execution
        | default []
        | append {||
            ggterm-osc133-B   # B: command start
            ggterm-osc133-C   # C: output start
        }
)

# Emit initial prompt start on shell launch
ggterm-osc133-A
