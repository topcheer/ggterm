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
}
