//! Plugin integration — bridges PluginManager with the App event loop.
//!
//! When the `plugin` feature is enabled, the App owns an optional
//! [`PluginBridge`] (wrapping a PluginManager) and dispatches hooks at key
//! event-loop points:
//!
//! | Hook | Trigger | Result Handling |
//! |------|---------|-----------------|
//! | `OnInput` | `Keyboard` event, before PTY write | `Deny` skips write, `Transform` replaces bytes |
//! | `OnOutput` | `PtyBytes` event, after parse | Read-only (result ignored) |
//! | `OnResize` | `Resize` event | Read-only |
//! | `OnThemeChange` | `SetTheme` / `CycleTheme` | Read-only |
//! | `OnCommandStart` | OSC 133 mark C detected | Read-only |
//! | `OnCommandEnd` | OSC 133 mark D detected | Read-only |

use ggterm_plugin::{Hook, HookResult, Plugin, PluginContext, PluginManager};

/// Wrapper that owns a PluginManager and provides convenience dispatch methods.
///
/// Stored as `Option<PluginBridge>` on the App struct.
pub struct PluginBridge {
    manager: PluginManager,
}

impl PluginBridge {
    /// Create a new empty bridge.
    pub fn new() -> Self {
        Self {
            manager: PluginManager::new(),
        }
    }

    /// Create from an existing PluginManager (e.g. loaded from config).
    pub fn from_manager(manager: PluginManager) -> Self {
        Self { manager }
    }

    /// Access the inner PluginManager.
    pub fn manager(&self) -> &PluginManager {
        &self.manager
    }

    /// Access the inner PluginManager (mutable).
    pub fn manager_mut(&mut self) -> &mut PluginManager {
        &mut self.manager
    }

    /// Register a plugin.
    pub fn register(&mut self, plugin: Box<dyn Plugin>) -> Result<(), ggterm_plugin::PluginError> {
        self.manager.register(plugin)
    }

    /// Unregister a plugin by name.
    pub fn unregister(&mut self, name: &str) {
        let _ = self.manager.unregister(name);
    }

    /// Enable a plugin.
    pub fn enable(&mut self, name: &str) {
        let _ = self.manager.set_enabled(name, true);
    }

    /// Disable a plugin.
    pub fn disable(&mut self, name: &str) {
        let _ = self.manager.set_enabled(name, false);
    }

    /// Number of registered plugins.
    pub fn count(&self) -> usize {
        self.manager.count()
    }

    // ── Hook dispatch helpers ──

    /// Dispatch OnInput. Returns aggregated result.
    pub fn dispatch_input(&mut self, text: &str, ctx: &PluginContext) -> HookResult {
        let hook = Hook::input(text);
        self.manager.dispatch(&hook, ctx)
    }

    /// Dispatch OnOutput (read-only — result ignored).
    pub fn dispatch_output(&mut self, text: &str, ctx: &PluginContext) {
        let hook = Hook::output(text);
        let _ = self.manager.dispatch(&hook, ctx);
    }

    /// Dispatch OnResize (read-only).
    pub fn dispatch_resize(&mut self, cols: usize, rows: usize, ctx: &PluginContext) {
        let hook = Hook::resize(cols, rows);
        let _ = self.manager.dispatch(&hook, ctx);
    }

    /// Dispatch OnThemeChange (read-only).
    pub fn dispatch_theme_change(&mut self, from: &str, to: &str, ctx: &PluginContext) {
        let hook = Hook::theme_change(from, to);
        let _ = self.manager.dispatch(&hook, ctx);
    }

    /// Dispatch OnCommandStart (read-only).
    pub fn dispatch_command_start(&mut self, command: &str, ctx: &PluginContext) {
        let hook = Hook::command_start(command);
        let _ = self.manager.dispatch(&hook, ctx);
    }

    /// Dispatch OnCommandEnd (read-only).
    pub fn dispatch_command_end(&mut self, command: &str, exit_code: i32, ctx: &PluginContext) {
        let hook = Hook::command_end(command, exit_code);
        let _ = self.manager.dispatch(&hook, ctx);
    }

    /// Initialize all registered plugins.
    pub fn init_all(
        &mut self,
        ctx: &PluginContext,
    ) -> Result<(), (String, ggterm_plugin::PluginError)> {
        self.manager.init_all(ctx)
    }

    /// Shut down all plugins (called on App drop).
    pub fn shutdown_all(&mut self) {
        self.manager.shutdown_all();
    }
}

impl Default for PluginBridge {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a PluginContext from current app state.
pub fn build_context(cols: usize, rows: usize, theme_name: &str) -> PluginContext {
    let mut ctx = PluginContext::new(cols, rows);
    ctx.theme_name = theme_name.to_string();
    ctx
}

/// Expand a `~`-prefixed path to an absolute path using the HOME env var.
fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return std::path::Path::new(&home)
            .join(rest)
            .to_string_lossy()
            .into_owned();
    }
    path.to_string()
}

/// Load Lua plugins from a directory into a PluginBridge.
///
/// Scans `dir` for `*.lua` files, loads each as a `LuaPlugin`, and registers it.
/// Returns the bridge with all successfully loaded plugins. Files that fail to
/// load are silently skipped (logged at debug level).
///
/// When the `plugin-lua` feature is not enabled, this is a no-op.
pub fn load_plugins(bridge: &mut PluginBridge, dir: &str) -> usize {
    let expanded = expand_tilde(dir);
    let path = std::path::Path::new(&expanded);

    if !path.is_dir() {
        return 0;
    }

    load_plugins_from_dir(bridge, path)
}

#[cfg(feature = "plugin-lua")]
fn load_plugins_from_dir(bridge: &mut PluginBridge, path: &std::path::Path) -> usize {
    let mut count = 0;
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let entry_path = entry.path();
        if entry_path.extension().is_none_or(|ext| ext != "lua") {
            continue;
        }

        match ggterm_plugin::LuaPlugin::from_file(&entry_path) {
            Ok(plugin) => {
                let name = plugin.name().to_string();
                match bridge.register(Box::new(plugin)) {
                    Ok(()) => {
                        log::debug!("Loaded plugin: {}", name);
                        count += 1;
                    }
                    Err(e) => {
                        log::warn!("Failed to register plugin {}: {}", name, e);
                    }
                }
            }
            Err(e) => {
                log::warn!("Failed to load Lua plugin {}: {}", entry_path.display(), e);
            }
        }
    }

    count
}

#[cfg(not(feature = "plugin-lua"))]
fn load_plugins_from_dir(_bridge: &mut PluginBridge, _path: &std::path::Path) -> usize {
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use ggterm_plugin::{HookResult, HookType, NativePlugin};

    /// A simple mock plugin that registers for all hooks and returns Allow.
    fn make_recorder(name: &str) -> NativePlugin {
        NativePlugin::new(name)
            .version("1.0.0")
            .hook(HookType::OnInput)
            .hook(HookType::OnOutput)
            .hook(HookType::OnResize)
            .hook(HookType::OnThemeChange)
            .build()
    }

    #[test]
    fn t_bridge_create_empty() {
        let bridge = PluginBridge::new();
        assert_eq!(bridge.count(), 0);
    }

    #[test]
    fn t_bridge_register_and_count() {
        let mut bridge = PluginBridge::new();
        bridge.register(Box::new(make_recorder("p1"))).unwrap();
        assert_eq!(bridge.count(), 1);
    }

    #[test]
    fn t_bridge_unregister() {
        let mut bridge = PluginBridge::new();
        bridge.register(Box::new(make_recorder("p1"))).unwrap();
        assert_eq!(bridge.count(), 1);
        bridge.unregister("p1");
        assert_eq!(bridge.count(), 0);
    }

    #[test]
    fn t_bridge_dispatch_input_allow() {
        let mut bridge = PluginBridge::new();
        bridge.register(Box::new(make_recorder("p1"))).unwrap();
        let ctx = build_context(80, 24, "dark");
        bridge.init_all(&ctx).unwrap();
        let result = bridge.dispatch_input("ls", &ctx);
        assert!(matches!(result, HookResult::Allow));
    }

    #[test]
    fn t_bridge_dispatch_output_no_panic() {
        let mut bridge = PluginBridge::new();
        bridge.register(Box::new(make_recorder("p1"))).unwrap();
        let ctx = build_context(80, 24, "dark");
        bridge.init_all(&ctx).unwrap();
        bridge.dispatch_output("hello world", &ctx);
    }

    #[test]
    fn t_bridge_dispatch_resize_no_panic() {
        let mut bridge = PluginBridge::new();
        let ctx = build_context(80, 24, "dark");
        bridge.dispatch_resize(120, 40, &ctx);
    }

    #[test]
    fn t_bridge_dispatch_theme_change_no_panic() {
        let mut bridge = PluginBridge::new();
        let ctx = build_context(80, 24, "dark");
        bridge.dispatch_theme_change("dark", "light", &ctx);
    }

    #[test]
    fn t_bridge_shutdown_no_panic() {
        let mut bridge = PluginBridge::new();
        bridge.register(Box::new(make_recorder("p1"))).unwrap();
        let ctx = build_context(80, 24, "dark");
        bridge.init_all(&ctx).unwrap();
        bridge.shutdown_all();
    }

    #[test]
    fn t_build_context_basic() {
        let ctx = build_context(80, 24, "dracula");
        assert_eq!(ctx.cols, 80);
        assert_eq!(ctx.rows, 24);
        assert_eq!(ctx.theme_name, "dracula");
    }

    #[test]
    fn t_bridge_multiple_plugins() {
        let mut bridge = PluginBridge::new();
        bridge.register(Box::new(make_recorder("p1"))).unwrap();
        bridge.register(Box::new(make_recorder("p2"))).unwrap();
        bridge.register(Box::new(make_recorder("p3"))).unwrap();
        assert_eq!(bridge.count(), 3);

        let ctx = build_context(80, 24, "dark");
        bridge.init_all(&ctx).unwrap();
        let result = bridge.dispatch_input("test", &ctx);
        assert!(matches!(result, HookResult::Allow));
    }

    #[test]
    fn t_bridge_from_manager() {
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(make_recorder("native"))).unwrap();
        let bridge = PluginBridge::from_manager(mgr);
        assert_eq!(bridge.count(), 1);
    }

    // ── P23-D: load_plugins + tilde expansion tests ──────────────────

    #[test]
    fn t_expand_tilde_no_home() {
        // When HOME is set, ~/foo should expand.
        let expanded = super::expand_tilde("~/plugins");
        // Should NOT start with ~ if HOME is set.
        if std::env::var_os("HOME").is_some() {
            assert!(!expanded.starts_with('~'));
        }
    }

    #[test]
    fn t_expand_tilde_absolute_path() {
        // Absolute paths should be unchanged.
        let expanded = super::expand_tilde("/usr/local/plugins");
        assert_eq!(expanded, "/usr/local/plugins");
    }

    #[test]
    fn t_expand_tilde_relative_path() {
        // Relative paths should be unchanged.
        let expanded = super::expand_tilde("plugins");
        assert_eq!(expanded, "plugins");
    }

    #[test]
    fn t_load_plugins_nonexistent_dir() {
        let mut bridge = PluginBridge::new();
        let count = load_plugins(&mut bridge, "/nonexistent/path/that/does/not/exist");
        assert_eq!(count, 0);
        assert_eq!(bridge.count(), 0);
    }

    #[test]
    fn t_load_plugins_empty_dir() {
        let dir = std::env::temp_dir().join("ggterm_test_empty_plugins");
        let _ = std::fs::create_dir_all(&dir);
        let mut bridge = PluginBridge::new();
        let count = load_plugins(&mut bridge, dir.to_str().unwrap());
        assert_eq!(count, 0);
        assert_eq!(bridge.count(), 0);
        let _ = std::fs::remove_dir(&dir);
    }

    #[cfg(feature = "plugin-lua")]
    #[test]
    fn t_load_plugins_lua_file() {
        let dir = std::env::temp_dir().join("ggterm_test_lua_plugins");
        let _ = std::fs::create_dir_all(&dir);

        // Write a minimal valid Lua plugin.
        let lua_source = r#"
return {
    name = "test-hello",
    version = "1.0.0",
    hooks = { "on_input", "on_output" },
    on_input = function(text)
        return "allow"
    end,
    on_output = function(text)
        return "allow"
    end,
}
"#;
        let plugin_path = dir.join("hello.lua");
        std::fs::write(&plugin_path, lua_source).unwrap();

        let mut bridge = PluginBridge::new();
        let count = load_plugins(&mut bridge, dir.to_str().unwrap());
        assert_eq!(count, 1);
        assert_eq!(bridge.count(), 1);

        let _ = std::fs::remove_file(&plugin_path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[cfg(feature = "plugin-lua")]
    #[test]
    fn t_load_plugins_skips_non_lua_files() {
        let dir = std::env::temp_dir().join("ggterm_test_mixed_plugins");
        let _ = std::fs::create_dir_all(&dir);

        std::fs::write(dir.join("readme.txt"), "not a plugin").unwrap();
        std::fs::write(dir.join("config.json"), "{}").unwrap();

        let mut bridge = PluginBridge::new();
        let count = load_plugins(&mut bridge, dir.to_str().unwrap());
        assert_eq!(count, 0);
        assert_eq!(bridge.count(), 0);

        let _ = std::fs::remove_file(dir.join("readme.txt"));
        let _ = std::fs::remove_file(dir.join("config.json"));
        let _ = std::fs::remove_dir(&dir);
    }

    #[cfg(feature = "plugin-lua")]
    #[test]
    fn t_load_plugins_invalid_lua_skipped() {
        let dir = std::env::temp_dir().join("ggterm_test_bad_plugins");
        let _ = std::fs::create_dir_all(&dir);

        // Invalid Lua that doesn't return a table.
        std::fs::write(dir.join("bad.lua"), "return 42").unwrap();

        let mut bridge = PluginBridge::new();
        let count = load_plugins(&mut bridge, dir.to_str().unwrap());
        assert_eq!(count, 0);
        assert_eq!(bridge.count(), 0);

        let _ = std::fs::remove_file(dir.join("bad.lua"));
        let _ = std::fs::remove_dir(&dir);
    }

    #[cfg(feature = "plugin-lua")]
    #[test]
    fn t_load_plugins_multiple_files() {
        let dir = std::env::temp_dir().join("ggterm_test_multi_plugins");
        let _ = std::fs::create_dir_all(&dir);

        for i in 0..3 {
            let src = format!(
                r#"
return {{
    name = "plugin-{}",
    version = "1.0.0",
    hooks = {{}},
}}
"#,
                i
            );
            std::fs::write(dir.join(format!("p{}.lua", i)), src).unwrap();
        }

        let mut bridge = PluginBridge::new();
        let count = load_plugins(&mut bridge, dir.to_str().unwrap());
        assert_eq!(count, 3);
        assert_eq!(bridge.count(), 3);

        for i in 0..3 {
            let _ = std::fs::remove_file(dir.join(format!("p{}.lua", i)));
        }
        let _ = std::fs::remove_dir(&dir);
    }

    #[cfg(feature = "plugin-lua")]
    #[test]
    fn t_loaded_lua_plugin_dispatches_input() {
        let dir = std::env::temp_dir().join("ggterm_test_dispatch_plugins");
        let _ = std::fs::create_dir_all(&dir);

        let lua_source = r#"
return {
    name = "echo-plugin",
    version = "1.0.0",
    hooks = { "on_input" },
    on_input = function(text)
        return "allow"
    end,
}
"#;
        std::fs::write(dir.join("echo.lua"), lua_source).unwrap();

        let mut bridge = PluginBridge::new();
        let count = load_plugins(&mut bridge, dir.to_str().unwrap());
        assert_eq!(count, 1);

        let ctx = build_context(80, 24, "dark");
        bridge.init_all(&ctx).unwrap();
        let result = bridge.dispatch_input("hello", &ctx);
        assert!(matches!(result, HookResult::Allow));

        let _ = std::fs::remove_file(dir.join("echo.lua"));
        let _ = std::fs::remove_dir(&dir);
    }
}
