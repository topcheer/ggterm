//! Lua plugin runtime — sandboxed Lua 5.4 via [`mlua`].
//!
//! A [`LuaPlugin`] loads a Lua script that returns a table describing the
//! plugin's metadata and hook callbacks. The Lua state is sandboxed: unsafe
//! functions (`os.execute`, `io.popen`, `require`, `debug`, ...) are removed
//! before user code runs.

use mlua::{Function, Lua, Table, Value};

use crate::hooks::{Hook, HookResult, HookType};
use crate::plugin::{Plugin, PluginContext, PluginError};

/// A plugin backed by a sandboxed Lua 5.4 script.
pub struct LuaPlugin {
    name: String,
    version: String,
    registered_hooks: Vec<HookType>,
    lua: Lua,
    plugin_table: Table,
    has_init: bool,
}

impl LuaPlugin {
    /// Create a Lua plugin from source code.
    pub fn from_source(source: &str) -> Result<Self, PluginError> {
        let lua = create_sandboxed_lua()?;

        let result: Value = lua
            .load(source)
            .set_name("plugin.lua")
            .eval()
            .map_err(|e| PluginError::Lua(format!("load error: {e}")))?;

        let table = match result {
            Value::Table(t) => t,
            _ => {
                return Err(PluginError::Lua(
                    "plugin script must return a table".to_string(),
                ));
            }
        };

        let name: String = table
            .get("name")
            .map_err(|e| PluginError::Lua(format!("missing 'name': {e}")))?;
        let version: String = table
            .get("version")
            .map_err(|e| PluginError::Lua(format!("missing 'version': {e}")))?;

        if name.is_empty() {
            return Err(PluginError::Lua(
                "plugin 'name' must not be empty".to_string(),
            ));
        }

        let registered_hooks = parse_hooks_table(&table)?;
        let has_init: bool = table.get("init").map(|_: Value| true).unwrap_or(false);

        Ok(Self {
            name,
            version,
            registered_hooks,
            lua,
            plugin_table: table,
            has_init,
        })
    }

    /// Load a Lua plugin from a file.
    pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self, PluginError> {
        let source = std::fs::read_to_string(path.as_ref()).map_err(|e| {
            PluginError::Lua(format!("failed to read {}: {e}", path.as_ref().display()))
        })?;
        Self::from_source(&source)
    }

    fn hook_fn_name(hook_type: HookType) -> &'static str {
        match hook_type {
            HookType::OnInput => "on_input",
            HookType::OnOutput => "on_output",
            HookType::OnCommandStart => "on_command_start",
            HookType::OnCommandEnd => "on_command_end",
            HookType::OnResize => "on_resize",
            HookType::OnThemeChange => "on_theme_change",
        }
    }

    fn build_hook_arg(&self, hook: &Hook) -> Value {
        let lua = &self.lua;
        match hook {
            Hook::OnInput(text) | Hook::OnOutput(text) | Hook::OnCommandStart(text) => {
                match lua.create_string(text) {
                    Ok(s) => Value::String(s),
                    Err(_) => Value::Nil,
                }
            }
            Hook::OnCommandEnd { command, exit_code } => match lua.create_table() {
                Ok(t) => {
                    let cmd_str = lua.create_string(command).ok();
                    if let Some(s) = cmd_str {
                        let _ = t.set("command", s);
                    }
                    let _ = t.set("exit_code", *exit_code);
                    Value::Table(t)
                }
                Err(_) => Value::Nil,
            },
            Hook::OnResize { cols, rows } => match lua.create_table() {
                Ok(t) => {
                    let _ = t.set("cols", *cols);
                    let _ = t.set("rows", *rows);
                    Value::Table(t)
                }
                Err(_) => Value::Nil,
            },
            Hook::OnThemeChange { from, to } => match lua.create_table() {
                Ok(t) => {
                    if let Ok(s) = lua.create_string(from) {
                        let _ = t.set("from", s);
                    }
                    if let Ok(s) = lua.create_string(to) {
                        let _ = t.set("to", s);
                    }
                    Value::Table(t)
                }
                Err(_) => Value::Nil,
            },
        }
    }
}

impl Plugin for LuaPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> {
        if !self.has_init {
            return Ok(());
        }

        let lua = &self.lua;
        let ctx_table = lua
            .create_table()
            .map_err(|e| PluginError::Lua(format!("init ctx: {e}")))?;

        if let Some(ref cwd) = ctx.cwd
            && let Ok(s) = lua.create_string(cwd)
        {
            let _ = ctx_table.set("cwd", s);
        }
        let _ = ctx_table.set("cols", ctx.cols);
        let _ = ctx_table.set("rows", ctx.rows);
        if let Ok(s) = lua.create_string(&ctx.theme_name) {
            let _ = ctx_table.set("theme", s);
        }

        // Call init function if present.
        let init_fn: Option<Function> = self.plugin_table.get("init").ok();
        if let Some(f) = init_fn {
            f.call::<Value>(ctx_table)
                .map_err(|e| PluginError::Lua(format!("init callback: {e}")))?;
        }

        Ok(())
    }

    fn hooks(&self) -> &[HookType] {
        &self.registered_hooks
    }

    fn handle_hook(&mut self, hook: &Hook, _ctx: &PluginContext) -> HookResult {
        let hook_type = hook.hook_type();

        // Fast check: is this hook registered?
        if !self.registered_hooks.contains(&hook_type) {
            return HookResult::Allow;
        }

        let fn_name = Self::hook_fn_name(hook_type);
        let lua = &self.lua;

        // Check if the callback function exists.
        let func: Function = match self.plugin_table.get(fn_name) {
            Ok(f) => f,
            Err(_) => return HookResult::Allow,
        };

        // Build the argument.
        let arg = self.build_hook_arg(hook);

        // Use a pcall wrapper to safely call and capture multi-return.
        let wrapper_src = r#"
            return function(f, arg)
                local results = table.pack(pcall(f, arg))
                local ok = results[1]
                if not ok then
                    return { error = tostring(results[2]) }
                end
                return {
                    action = results[2],
                    data1 = results[3],
                    data2 = results[4],
                }
            end
        "#;

        let wrapper: Function = match lua.load(wrapper_src).eval() {
            Ok(w) => w,
            Err(e) => {
                log::warn!("Lua plugin '{}' wrapper error: {e}", self.name);
                return HookResult::Allow;
            }
        };

        let ret: Table = match wrapper.call((func, arg)) {
            Ok(t) => t,
            Err(e) => {
                log::warn!("Lua plugin '{}' hook call error: {e}", self.name);
                return HookResult::Allow;
            }
        };

        // Check for runtime error from pcall.
        let err: Option<String> = ret.get("error").ok();
        if let Some(e) = err {
            log::warn!("Lua plugin '{}' hook '{fn_name}' error: {e}", self.name);
            return HookResult::Allow;
        }

        // Parse the action.
        let action: Value = ret.get("action").unwrap_or(Value::Nil);
        match action {
            Value::Nil => HookResult::Allow,
            Value::Boolean(b) => {
                if b {
                    HookResult::Allow
                } else {
                    HookResult::Deny
                }
            }
            Value::String(s) => {
                let action_str = match s.to_str() {
                    Ok(v) => v.to_string(),
                    Err(_) => return HookResult::Allow,
                };
                match action_str.as_str() {
                    "allow" | "ok" | "" => HookResult::Allow,
                    "deny" | "block" => HookResult::Deny,
                    "transform" => {
                        let data1: String = ret.get("data1").unwrap_or_default();
                        HookResult::Transform(data1)
                    }
                    "annotate" => {
                        let key: String = ret.get("data1").unwrap_or_default();
                        let value: String = ret.get("data2").unwrap_or_default();
                        HookResult::Annotate(key, value)
                    }
                    other => {
                        log::warn!("Lua plugin '{}' unknown return: '{other}'", self.name);
                        HookResult::Allow
                    }
                }
            }
            _ => HookResult::Allow,
        }
    }
}

// ── Sandbox creation ──

fn create_sandboxed_lua() -> Result<Lua, PluginError> {
    let lua = Lua::new();
    let globals = lua.globals();

    // Remove dangerous os.* functions
    if let Ok(os_table) = globals.get::<Table>("os") {
        let _ = os_table.set("execute", Value::Nil);
        let _ = os_table.set("exit", Value::Nil);
        let _ = os_table.set("remove", Value::Nil);
        let _ = os_table.set("rename", Value::Nil);
        let _ = os_table.set("tmpname", Value::Nil);
        let _ = os_table.set("getenv", Value::Nil);
    }

    // Remove dangerous io.* functions
    if let Ok(io_table) = globals.get::<Table>("io") {
        let _ = io_table.set("popen", Value::Nil);
        let _ = io_table.set("open", Value::Nil);
        let _ = io_table.set("write", Value::Nil);
        let _ = io_table.set("read", Value::Nil);
        let _ = io_table.set("lines", Value::Nil);
    }

    // Remove package loading
    if let Ok(pkg_table) = globals.get::<Table>("package") {
        let _ = pkg_table.set("loadlib", Value::Nil);
        let _ = pkg_table.set("searchpath", Value::Nil);
        let _ = pkg_table.set("path", "");
        let _ = pkg_table.set("cpath", "");
    }

    // Remove require, dofile, loadfile, debug
    let _ = globals.set("require", Value::Nil);
    let _ = globals.set("dofile", Value::Nil);
    let _ = globals.set("loadfile", Value::Nil);
    let _ = globals.set("debug", Value::Nil);

    // Provide a restricted ggterm API table.
    let ggterm_api = lua
        .create_table()
        .map_err(|e| PluginError::Lua(format!("create ggterm api: {e}")))?;

    let _ = ggterm_api.set("version", "0.1.0");
    let _ = globals.set("ggterm", ggterm_api);

    Ok(lua)
}

fn parse_hooks_table(table: &Table) -> Result<Vec<HookType>, PluginError> {
    let hooks_val: Value = table.get("hooks").unwrap_or(Value::Nil);

    match hooks_val {
        Value::Nil => Ok(HookType::all().to_vec()),
        Value::Table(t) => {
            let mut hooks = Vec::new();
            for pair in t.pairs::<Value, Value>() {
                let (_, v) =
                    pair.map_err(|e| PluginError::Lua(format!("hooks array error: {e}")))?;
                if let Value::String(s) = v {
                    let name = s
                        .to_str()
                        .map_err(|e| PluginError::Lua(format!("hook name not UTF-8: {e}")))?;
                    let name = name.to_string();
                    let ht = match name.as_str() {
                        "on_input" => HookType::OnInput,
                        "on_output" => HookType::OnOutput,
                        "on_command_start" => HookType::OnCommandStart,
                        "on_command_end" => HookType::OnCommandEnd,
                        "on_resize" => HookType::OnResize,
                        "on_theme_change" => HookType::OnThemeChange,
                        other => {
                            return Err(PluginError::Lua(format!("unknown hook type: '{other}'")));
                        }
                    };
                    hooks.push(ht);
                }
            }
            Ok(hooks)
        }
        _ => Err(PluginError::Lua(
            "'hooks' must be an array of strings".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Basic creation ──

    #[test]
    fn t_lua_plugin_basic() {
        let src = r#"return { name = "test-basic", version = "1.0.0" }"#;
        let p = LuaPlugin::from_source(src).unwrap();
        assert_eq!(p.name(), "test-basic");
        assert_eq!(p.version(), "1.0.0");
    }

    #[test]
    fn t_lua_plugin_missing_name() {
        let result = LuaPlugin::from_source(r#"return { version = "1.0" }"#);
        assert!(result.is_err());
    }

    #[test]
    fn t_lua_plugin_missing_version() {
        let result = LuaPlugin::from_source(r#"return { name = "test" }"#);
        assert!(result.is_err());
    }

    #[test]
    fn t_lua_plugin_empty_name() {
        let result = LuaPlugin::from_source(r#"return { name = "", version = "1" }"#);
        assert!(result.is_err());
    }

    #[test]
    fn t_lua_plugin_not_a_table() {
        let result = LuaPlugin::from_source(r#"return 42"#);
        assert!(result.is_err());
    }

    #[test]
    fn t_lua_plugin_syntax_error() {
        let result = LuaPlugin::from_source(r#"return { name ="#);
        assert!(result.is_err());
    }

    // ── Hooks registration ──

    #[test]
    fn t_hooks_explicit() {
        let src = r#"
            return {
                name = "h", version = "1",
                hooks = {"on_input", "on_output"},
            }
        "#;
        let p = LuaPlugin::from_source(src).unwrap();
        assert_eq!(p.hooks(), &[HookType::OnInput, HookType::OnOutput]);
    }

    #[test]
    fn t_hooks_all_when_unspecified() {
        let p = LuaPlugin::from_source(r#"return { name="t", version="1" }"#).unwrap();
        assert_eq!(p.hooks().len(), HookType::all().len());
    }

    #[test]
    fn t_hooks_empty_array() {
        let src = r#"return { name="n", version="1", hooks={} }"#;
        let p = LuaPlugin::from_source(src).unwrap();
        assert_eq!(p.hooks().len(), 0);
    }

    #[test]
    fn t_hooks_unknown() {
        let src = r#"return { name="b", version="1", hooks={"nope"} }"#;
        assert!(LuaPlugin::from_source(src).is_err());
    }

    // ── Hook dispatch: Allow ──

    #[test]
    fn t_hook_allow_default() {
        let src = r#"
            return { name="p", version="1", hooks={"on_input"},
                on_input = function(d) end,
            }
        "#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        let ctx = PluginContext::default();
        assert_eq!(p.handle_hook(&Hook::input("hi"), &ctx), HookResult::Allow);
    }

    #[test]
    fn t_hook_nil_return() {
        let src = r#"
            return { name="p", version="1", hooks={"on_output"},
                on_output = function(d) return nil end,
            }
        "#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        assert_eq!(
            p.handle_hook(&Hook::output("hi"), &PluginContext::default()),
            HookResult::Allow
        );
    }

    #[test]
    fn t_hook_no_callback() {
        let src = r#"return { name="p", version="1", hooks={"on_input"} }"#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        let ctx = PluginContext::default();
        assert_eq!(p.handle_hook(&Hook::input("x"), &ctx), HookResult::Allow);
    }

    #[test]
    fn t_hook_not_registered() {
        let src = r#"
            return { name="p", version="1", hooks={"on_input"},
                on_output = function(d) return "deny" end,
            }
        "#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        let ctx = PluginContext::default();
        // on_output defined but not registered → Allow
        assert_eq!(p.handle_hook(&Hook::output("x"), &ctx), HookResult::Allow);
    }

    // ── Hook dispatch: Deny ──

    #[test]
    fn t_hook_deny() {
        let src = r#"
            return { name="p", version="1", hooks={"on_input"},
                on_input = function(d) return "deny" end,
            }
        "#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        let ctx = PluginContext::default();
        assert_eq!(p.handle_hook(&Hook::input("rm"), &ctx), HookResult::Deny);
    }

    #[test]
    fn t_hook_boolean_false() {
        let src = r#"
            return { name="p", version="1", hooks={"on_input"},
                on_input = function(d) return false end,
            }
        "#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        assert_eq!(
            p.handle_hook(&Hook::input("x"), &PluginContext::default()),
            HookResult::Deny
        );
    }

    // ── Hook dispatch: Transform ──

    #[test]
    fn t_hook_transform() {
        let src = r#"
            return { name="p", version="1", hooks={"on_input"},
                on_input = function(d) return "transform", d:upper() end,
            }
        "#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        let ctx = PluginContext::default();
        let result = p.handle_hook(&Hook::input("hello"), &ctx);
        assert_eq!(result, HookResult::Transform("HELLO".to_string()));
    }

    // ── Hook dispatch: Annotate ──

    #[test]
    fn t_hook_annotate() {
        let src = r#"
            return { name="p", version="1", hooks={"on_command_end"},
                on_command_end = function(info)
                    if info.exit_code ~= 0 then
                        return "annotate", "failed", "true"
                    end
                end,
            }
        "#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        let ctx = PluginContext::default();

        assert_eq!(
            p.handle_hook(&Hook::command_end("ls", 0), &ctx),
            HookResult::Allow
        );

        let result = p.handle_hook(&Hook::command_end("ls", 1), &ctx);
        match result {
            HookResult::Annotate(k, v) => {
                assert_eq!(k, "failed");
                assert_eq!(v, "true");
            }
            other => panic!("expected Annotate, got {other:?}"),
        }
    }

    #[test]
    fn t_hook_resize_data() {
        let src = r#"
            return { name="p", version="1", hooks={"on_resize"},
                on_resize = function(info)
                    return "annotate", "cols", tostring(info.cols)
                end,
            }
        "#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        let ctx = PluginContext::default();
        let result = p.handle_hook(&Hook::resize(120, 40), &ctx);
        match result {
            HookResult::Annotate(k, v) => {
                assert_eq!(k, "cols");
                assert_eq!(v, "120");
            }
            other => panic!("expected Annotate, got {other:?}"),
        }
    }

    #[test]
    fn t_hook_theme_change_data() {
        let src = r#"
            return { name="p", version="1", hooks={"on_theme_change"},
                on_theme_change = function(info)
                    return "annotate", "to", info.to
                end,
            }
        "#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        let ctx = PluginContext::default();
        let result = p.handle_hook(&Hook::theme_change("dark", "dracula"), &ctx);
        match result {
            HookResult::Annotate(k, v) => {
                assert_eq!(k, "to");
                assert_eq!(v, "dracula");
            }
            other => panic!("expected Annotate, got {other:?}"),
        }
    }

    // ── Error handling ──

    #[test]
    fn t_hook_runtime_error_caught() {
        let src = r#"
            return { name="p", version="1", hooks={"on_input"},
                on_input = function(d) error("boom!") end,
            }
        "#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        let ctx = PluginContext::default();
        assert_eq!(p.handle_hook(&Hook::input("x"), &ctx), HookResult::Allow);
    }

    #[test]
    fn t_hook_unknown_return() {
        let src = r#"
            return { name="p", version="1", hooks={"on_input"},
                on_input = function(d) return "frobnicate" end,
            }
        "#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        let ctx = PluginContext::default();
        assert_eq!(p.handle_hook(&Hook::input("x"), &ctx), HookResult::Allow);
    }

    // ── Init ──

    #[test]
    fn t_init_callback() {
        let src = r#"
            return { name="p", version="1", hooks={},
                init = function(ctx) end,
            }
        "#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        p.init(&PluginContext::default()).unwrap();
    }

    #[test]
    fn t_init_no_callback() {
        let src = r#"return { name="p", version="1", hooks={} }"#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        p.init(&PluginContext::default()).unwrap();
    }

    // ── Sandbox security ──

    #[test]
    fn t_sandbox_no_os_execute() {
        let src = r#"
            return { name="p", version="1", hooks={"on_input"},
                on_input = function(d) os.execute("echo hacked") end,
            }
        "#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        let ctx = PluginContext::default();
        assert_eq!(p.handle_hook(&Hook::input("x"), &ctx), HookResult::Allow);
    }

    #[test]
    fn t_sandbox_no_io_popen() {
        let src = r#"
            return { name="p", version="1", hooks={"on_output"},
                on_output = function(d) io.popen("whoami") end,
            }
        "#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        let ctx = PluginContext::default();
        assert_eq!(p.handle_hook(&Hook::output("x"), &ctx), HookResult::Allow);
    }

    #[test]
    fn t_sandbox_no_require() {
        let src = r#"
            return { name="p", version="1", hooks={"on_input"},
                on_input = function(d) require("os") end,
            }
        "#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        let ctx = PluginContext::default();
        assert_eq!(p.handle_hook(&Hook::input("x"), &ctx), HookResult::Allow);
    }

    #[test]
    fn t_sandbox_no_debug() {
        let src = r#"
            return { name="p", version="1", hooks={"on_input"},
                on_input = function(d) debug.getinfo(1) end,
            }
        "#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        let ctx = PluginContext::default();
        assert_eq!(p.handle_hook(&Hook::input("x"), &ctx), HookResult::Allow);
    }

    #[test]
    fn t_sandbox_no_dofile() {
        let src = r#"
            return { name="p", version="1", hooks={"on_input"},
                on_input = function(d) dofile("/etc/passwd") end,
            }
        "#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        let ctx = PluginContext::default();
        assert_eq!(p.handle_hook(&Hook::input("x"), &ctx), HookResult::Allow);
    }

    #[test]
    fn t_sandbox_ggterm_api() {
        let src = r#"
            return { name="p", version="1", hooks={"on_input"},
                on_input = function(d)
                    if ggterm and ggterm.version then
                        return "transform", ggterm.version
                    end
                    return "allow"
                end,
            }
        "#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        let ctx = PluginContext::default();
        let result = p.handle_hook(&Hook::input("check"), &ctx);
        match result {
            HookResult::Transform(s) => assert_eq!(s, "0.1.0"),
            other => panic!("expected Transform, got {other:?}"),
        }
    }

    // ── File loading ──

    #[test]
    fn t_from_file() {
        let path = std::env::temp_dir().join("ggterm_lua_test_from_file.lua");
        std::fs::write(&path, r#"return { name="f", version="2.0" }"#).unwrap();
        let p = LuaPlugin::from_file(&path).unwrap();
        assert_eq!(p.name(), "f");
        assert_eq!(p.version(), "2.0");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn t_from_file_not_found() {
        assert!(LuaPlugin::from_file("/nonexistent/x.lua").is_err());
    }

    // ── Complex plugin ──

    #[test]
    fn t_complex_multiple_hooks() {
        let src = r#"
            return {
                name = "multi", version = "1.0",
                hooks = {"on_input", "on_output", "on_command_end"},
                on_input = function(data)
                    if string.find(data, "rm") then return "deny" end
                    return "allow"
                end,
                on_output = function(data) return "allow" end,
                on_command_end = function(info)
                    if info.exit_code ~= 0 then
                        return "annotate", "error", "command failed"
                    end
                    return "allow"
                end,
            }
        "#;
        let mut p = LuaPlugin::from_source(src).unwrap();
        let ctx = PluginContext::default();
        p.init(&ctx).unwrap();

        assert_eq!(
            p.handle_hook(&Hook::input("rm -rf /"), &ctx),
            HookResult::Deny
        );
        assert_eq!(
            p.handle_hook(&Hook::input("ls -la"), &ctx),
            HookResult::Allow
        );
        assert_eq!(p.handle_hook(&Hook::output("hi"), &ctx), HookResult::Allow);
        let result = p.handle_hook(&Hook::command_end("ls", 1), &ctx);
        assert!(matches!(result, HookResult::Annotate(_, _)));
    }

    #[test]
    fn t_implements_plugin_trait() {
        let src = r#"return { name="trait", version="1", hooks={} }"#;
        let p = LuaPlugin::from_source(src).unwrap();
        fn check(p: &dyn Plugin) -> bool {
            !p.name().is_empty()
        }
        assert!(check(&p));
    }
}
