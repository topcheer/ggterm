-- hello.lua — Example GGTerm Lua plugin
--
-- Demonstrates the plugin API: lifecycle hooks, input/output observation,
-- and OSC command tracking.
--
-- Install: copy to ~/.ggterm/plugins/hello.lua

local greeting = "Hello from GGTerm plugins!"

return {
    name = "hello",
    version = "1.0.0",
    hooks = { "on_input", "on_output", "on_command_start", "on_command_end" },

    -- Called once when the plugin is initialized.
    init = function(ctx)
        -- ctx is a table with: cwd, last_command, last_exit_code, cols, rows, theme_name
        -- We can't print to the terminal directly (sandboxed), but we can
        -- store state for later use.
    end,

    -- Called when the user types input (before it goes to the PTY).
    -- Return "allow" to pass through, "deny" to block, or
    -- { "transform", "new_text" } to modify.
    on_input = function(text)
        return "allow"
    end,

    -- Called when terminal output is rendered (read-only).
    on_output = function(text)
        return "allow"
    end,

    -- Called when a command starts (OSC 133;C mark).
    on_command_start = function(command)
        return "allow"
    end,

    -- Called when a command finishes (OSC 133;D mark).
    on_command_end = function(command, exit_code)
        return "allow"
    end,
}
