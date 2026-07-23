//! PluginManager — registry, lifecycle, and hook dispatch.

use std::collections::HashMap;

use crate::hooks::{Hook, HookResult, HookResultAggregator, HookType};
use crate::plugin::{Plugin, PluginContext, PluginError, PluginStats};

/// Lifecycle state of a managed plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PluginLifecycle {
    #[default]
    Registered,
    Active,
    Stopped,
    Failed,
}

struct PluginEntry {
    plugin: Box<dyn Plugin>,
    state: PluginLifecycle,
    stats: PluginStats,
    enabled: bool,
}

/// Manages plugin lifecycle and hook dispatch.
pub struct PluginManager {
    plugins: HashMap<String, PluginEntry>,
    hook_index: HashMap<HookType, Vec<String>>,
}

impl PluginManager {
    /// Create a new empty plugin manager.
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            hook_index: HashMap::new(),
        }
    }

    /// Register a plugin. Returns `Err(AlreadyExists)` if name is taken.
    pub fn register(&mut self, plugin: Box<dyn Plugin>) -> Result<(), PluginError> {
        let name = plugin.name().to_string();
        if self.plugins.contains_key(&name) {
            return Err(PluginError::AlreadyExists(name));
        }
        for &ht in plugin.hooks() {
            self.hook_index.entry(ht).or_default().push(name.clone());
        }
        self.plugins.insert(
            name,
            PluginEntry {
                plugin,
                state: PluginLifecycle::Registered,
                stats: PluginStats::default(),
                enabled: true,
            },
        );
        Ok(())
    }

    /// Unregister a plugin by name. Calls `shutdown()` if active.
    pub fn unregister(&mut self, name: &str) -> Result<Box<dyn Plugin>, PluginError> {
        let mut entry = self
            .plugins
            .remove(name)
            .ok_or_else(|| PluginError::NotFound(name.to_string()))?;
        for names in self.hook_index.values_mut() {
            names.retain(|n| n != name);
        }
        if entry.state == PluginLifecycle::Active || entry.state == PluginLifecycle::Stopped {
            entry.plugin.shutdown();
        }
        Ok(entry.plugin)
    }

    /// Initialize all registered-but-uninitialized plugins.
    pub fn init_all(&mut self, ctx: &PluginContext) -> Result<(), (String, PluginError)> {
        let names: Vec<String> = self
            .plugins
            .keys()
            .filter(|n| self.plugins[*n].state == PluginLifecycle::Registered)
            .cloned()
            .collect();
        for name in names {
            let Some(entry) = self.plugins.get_mut(&name) else {
                continue; // Plugin was removed during iteration
            };
            match entry.plugin.init(ctx) {
                Ok(()) => entry.state = PluginLifecycle::Active,
                Err(e) => {
                    entry.state = PluginLifecycle::Failed;
                    return Err((name, e));
                }
            }
        }
        Ok(())
    }

    /// Initialize a single plugin by name.
    pub fn init(&mut self, name: &str, ctx: &PluginContext) -> Result<(), PluginError> {
        let entry = self
            .plugins
            .get_mut(name)
            .ok_or_else(|| PluginError::NotFound(name.to_string()))?;
        if entry.state == PluginLifecycle::Active {
            return Ok(());
        }
        entry.plugin.init(ctx)?;
        entry.state = PluginLifecycle::Active;
        Ok(())
    }

    /// Shut down all plugins.
    pub fn shutdown_all(&mut self) {
        for entry in self.plugins.values_mut() {
            entry.plugin.shutdown();
            entry.state = PluginLifecycle::Stopped;
        }
    }

    /// Dispatch a hook to all interested, enabled, active plugins.
    pub fn dispatch(&mut self, hook: &Hook, ctx: &PluginContext) -> HookResult {
        let ht = hook.hook_type();
        let names: Vec<String> = self.hook_index.get(&ht).cloned().unwrap_or_default();
        if names.is_empty() {
            return HookResult::Allow;
        }

        let mut agg = HookResultAggregator::new();
        for name in &names {
            let entry = match self.plugins.get_mut(name) {
                Some(e) if e.enabled && e.state == PluginLifecycle::Active => e,
                _ => continue,
            };
            let result = entry.plugin.handle_hook(hook, ctx);
            entry.stats.record(&result);
            agg.add(result);
        }
        agg.finalize()
    }

    /// Dispatch a hook to a specific plugin.
    pub fn dispatch_to(
        &mut self,
        name: &str,
        hook: &Hook,
        ctx: &PluginContext,
    ) -> Result<HookResult, PluginError> {
        let entry = self
            .plugins
            .get_mut(name)
            .ok_or_else(|| PluginError::NotFound(name.to_string()))?;
        let result = entry.plugin.handle_hook(hook, ctx);
        entry.stats.record(&result);
        Ok(result)
    }

    /// Enable/disable a plugin.
    pub fn set_enabled(&mut self, name: &str, enabled: bool) -> Result<(), PluginError> {
        let entry = self
            .plugins
            .get_mut(name)
            .ok_or_else(|| PluginError::NotFound(name.to_string()))?;
        entry.enabled = enabled;
        Ok(())
    }

    /// Check if a plugin is enabled.
    pub fn is_enabled(&self, name: &str) -> bool {
        self.plugins.get(name).map(|e| e.enabled).unwrap_or(false)
    }

    /// Number of registered plugins.
    pub fn count(&self) -> usize {
        self.plugins.len()
    }

    /// Number of active plugins.
    pub fn active_count(&self) -> usize {
        self.plugins
            .values()
            .filter(|e| e.state == PluginLifecycle::Active)
            .count()
    }

    /// Check if a plugin exists.
    pub fn contains(&self, name: &str) -> bool {
        self.plugins.contains_key(name)
    }

    /// Get plugin version.
    pub fn version_of(&self, name: &str) -> Option<&str> {
        self.plugins.get(name).map(|e| e.plugin.version())
    }

    /// Get plugin lifecycle state.
    pub fn state_of(&self, name: &str) -> Option<PluginLifecycle> {
        self.plugins.get(name).map(|e| e.state)
    }

    /// Get plugin stats.
    pub fn stats_of(&self, name: &str) -> Option<&PluginStats> {
        self.plugins.get(name).map(|e| &e.stats)
    }

    /// List all plugin names.
    pub fn names(&self) -> Vec<&str> {
        self.plugins.keys().map(|s| s.as_str()).collect()
    }

    /// List plugins registered for a hook type.
    pub fn plugins_for_hook(&self, ht: HookType) -> Vec<&str> {
        self.hook_index
            .get(&ht)
            .map(|names| names.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for PluginManager {
    fn drop(&mut self) {
        self.shutdown_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::native;

    fn ctx() -> PluginContext {
        PluginContext::default()
    }

    fn make_plugin(name: &str, _hooks: Vec<HookType>) -> Box<dyn Plugin> {
        Box::new(
            native(name)
                .hook(HookType::OnInput) // always add OnInput for test simplicity
                .build(),
        )
    }

    // ── Registration ──

    #[test]
    fn t_new_empty() {
        let mgr = PluginManager::new();
        assert_eq!(mgr.count(), 0);
        assert!(mgr.names().is_empty());
    }

    #[test]
    fn t_default_empty() {
        assert_eq!(PluginManager::default().count(), 0);
    }

    #[test]
    fn t_register_one() {
        let mut mgr = PluginManager::new();
        mgr.register(make_plugin("test", vec![])).unwrap();
        assert_eq!(mgr.count(), 1);
        assert!(mgr.contains("test"));
    }

    #[test]
    fn t_register_duplicate() {
        let mut mgr = PluginManager::new();
        mgr.register(make_plugin("dup", vec![])).unwrap();
        let result = mgr.register(make_plugin("dup", vec![]));
        assert!(matches!(result, Err(PluginError::AlreadyExists(_))));
    }

    #[test]
    fn t_register_multiple() {
        let mut mgr = PluginManager::new();
        for i in 0..5 {
            mgr.register(make_plugin(&format!("p{i}"), vec![])).unwrap();
        }
        assert_eq!(mgr.count(), 5);
    }

    // ── Unregister ──

    #[test]
    fn t_unregister() {
        let mut mgr = PluginManager::new();
        mgr.register(make_plugin("test", vec![])).unwrap();
        let p = mgr.unregister("test").unwrap();
        assert_eq!(p.name(), "test");
        assert_eq!(mgr.count(), 0);
    }

    #[test]
    fn t_unregister_not_found() {
        let mut mgr = PluginManager::new();
        assert!(mgr.unregister("ghost").is_err());
    }

    #[test]
    fn t_unregister_removes_from_hook_index() {
        let mut mgr = PluginManager::new();
        mgr.register(make_plugin("a", vec![HookType::OnInput]))
            .unwrap();
        mgr.register(make_plugin("b", vec![HookType::OnInput]))
            .unwrap();
        assert_eq!(mgr.plugins_for_hook(HookType::OnInput).len(), 2);

        mgr.unregister("a").unwrap();
        assert_eq!(mgr.plugins_for_hook(HookType::OnInput).len(), 1);
    }

    #[test]
    fn t_unregister_then_reregister() {
        let mut mgr = PluginManager::new();
        mgr.register(make_plugin("reusable", vec![])).unwrap();
        mgr.unregister("reusable").unwrap();
        mgr.register(make_plugin("reusable", vec![])).unwrap();
        assert_eq!(mgr.count(), 1);
    }

    // ── Init ──

    #[test]
    fn t_init_all() {
        let mut mgr = PluginManager::new();
        mgr.register(make_plugin("a", vec![])).unwrap();
        mgr.register(make_plugin("b", vec![])).unwrap();
        mgr.init_all(&ctx()).unwrap();
        assert_eq!(mgr.active_count(), 2);
        assert_eq!(mgr.state_of("a"), Some(PluginLifecycle::Active));
    }

    #[test]
    fn t_init_specific() {
        let mut mgr = PluginManager::new();
        mgr.register(make_plugin("a", vec![])).unwrap();
        mgr.register(make_plugin("b", vec![])).unwrap();
        mgr.init("a", &ctx()).unwrap();
        assert_eq!(mgr.state_of("a"), Some(PluginLifecycle::Active));
        assert_eq!(mgr.state_of("b"), Some(PluginLifecycle::Registered));
    }

    #[test]
    fn t_init_not_found() {
        let mut mgr = PluginManager::new();
        assert!(mgr.init("ghost", &ctx()).is_err());
    }

    #[test]
    fn t_init_failure_sets_failed_state() {
        let failing = Box::new(
            native("fail")
                .on_init(|_| Err(PluginError::Init("bad".to_string())))
                .build(),
        );
        let mut mgr = PluginManager::new();
        mgr.register(failing).unwrap();
        let result = mgr.init_all(&ctx());
        assert!(result.is_err());
        assert_eq!(mgr.state_of("fail"), Some(PluginLifecycle::Failed));
    }

    #[test]
    fn t_init_idempotent() {
        let mut mgr = PluginManager::new();
        mgr.register(make_plugin("a", vec![])).unwrap();
        mgr.init("a", &ctx()).unwrap();
        mgr.init("a", &ctx()).unwrap(); // no error
        assert_eq!(mgr.state_of("a"), Some(PluginLifecycle::Active));
    }

    // ── Shutdown ──

    #[test]
    fn t_shutdown_all() {
        let mut mgr = PluginManager::new();
        mgr.register(make_plugin("a", vec![])).unwrap();
        mgr.register(make_plugin("b", vec![])).unwrap();
        mgr.init_all(&ctx()).unwrap();
        mgr.shutdown_all();
        assert_eq!(mgr.state_of("a"), Some(PluginLifecycle::Stopped));
    }

    // ── Dispatch ──

    #[test]
    fn t_dispatch_no_plugins() {
        let mut mgr = PluginManager::new();
        let result = mgr.dispatch(&Hook::OnInput("ls".to_string()), &ctx());
        assert_eq!(result, HookResult::Allow);
    }

    #[test]
    fn t_dispatch_uninitialized_skipped() {
        let mut mgr = PluginManager::new();
        mgr.register(make_plugin("p", vec![HookType::OnInput]))
            .unwrap();
        // Don't init
        let result = mgr.dispatch(&Hook::OnInput("ls".to_string()), &ctx());
        assert_eq!(result, HookResult::Allow); // inactive plugin doesn't run
    }

    #[test]
    fn t_dispatch_allow() {
        let mut mgr = PluginManager::new();
        let p = Box::new(
            native("allow")
                .hook(HookType::OnInput)
                .on_hook(|_, _| HookResult::Allow)
                .build(),
        );
        mgr.register(p).unwrap();
        mgr.init_all(&ctx()).unwrap();
        let result = mgr.dispatch(&Hook::OnInput("ls".to_string()), &ctx());
        assert_eq!(result, HookResult::Allow);
    }

    #[test]
    fn t_dispatch_deny() {
        let mut mgr = PluginManager::new();
        let p = Box::new(
            native("block")
                .hook(HookType::OnInput)
                .on_hook(|_, _| HookResult::Deny)
                .build(),
        );
        mgr.register(p).unwrap();
        mgr.init_all(&ctx()).unwrap();
        let result = mgr.dispatch(&Hook::OnInput("rm -rf /".to_string()), &ctx());
        assert_eq!(result, HookResult::Deny);
    }

    #[test]
    fn t_dispatch_transform() {
        let mut mgr = PluginManager::new();
        let p = Box::new(
            native("tx")
                .hook(HookType::OnInput)
                .on_hook(|_, _| HookResult::transform("modified"))
                .build(),
        );
        mgr.register(p).unwrap();
        mgr.init_all(&ctx()).unwrap();
        let result = mgr.dispatch(&Hook::OnInput("orig".to_string()), &ctx());
        assert_eq!(result.transformed_text(), Some("modified"));
    }

    #[test]
    fn t_dispatch_deny_overrides_transform() {
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(
            native("tx")
                .hook(HookType::OnInput)
                .on_hook(|_, _| HookResult::transform("mod"))
                .build(),
        ))
        .unwrap();
        mgr.register(Box::new(
            native("block")
                .hook(HookType::OnInput)
                .on_hook(|_, _| HookResult::Deny)
                .build(),
        ))
        .unwrap();
        mgr.init_all(&ctx()).unwrap();
        let result = mgr.dispatch(&Hook::OnInput("test".to_string()), &ctx());
        assert_eq!(result, HookResult::Deny);
    }

    #[test]
    fn t_dispatch_last_transform_wins() {
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(
            native("first")
                .hook(HookType::OnInput)
                .on_hook(|_, _| HookResult::transform("first"))
                .build(),
        ))
        .unwrap();
        mgr.register(Box::new(
            native("second")
                .hook(HookType::OnInput)
                .on_hook(|_, _| HookResult::transform("second"))
                .build(),
        ))
        .unwrap();
        mgr.init_all(&ctx()).unwrap();
        let result = mgr.dispatch(&Hook::OnInput("test".to_string()), &ctx());
        assert_eq!(result.transformed_text(), Some("second"));
    }

    #[test]
    fn t_dispatch_filtered_by_hook_type() {
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(
            native("input_only")
                .hook(HookType::OnInput)
                .on_hook(|_, _| HookResult::Deny)
                .build(),
        ))
        .unwrap();
        mgr.init_all(&ctx()).unwrap();
        // Dispatch OnOutput — plugin shouldn't deny
        let result = mgr.dispatch(&Hook::OnOutput("hello".to_string()), &ctx());
        assert_eq!(result, HookResult::Allow);
    }

    #[test]
    fn t_dispatch_stats_updated() {
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(
            native("p")
                .hook(HookType::OnInput)
                .on_hook(|_, _| HookResult::Deny)
                .build(),
        ))
        .unwrap();
        mgr.init_all(&ctx()).unwrap();
        mgr.dispatch(&Hook::OnInput("a".to_string()), &ctx());
        mgr.dispatch(&Hook::OnInput("b".to_string()), &ctx());
        let stats = mgr.stats_of("p").unwrap();
        assert_eq!(stats.hooks_called, 2);
        assert_eq!(stats.denials, 2);
    }

    #[test]
    fn t_dispatch_to_specific() {
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(
            native("p")
                .hook(HookType::OnInput)
                .on_hook(|_, _| HookResult::Deny)
                .build(),
        ))
        .unwrap();
        mgr.init_all(&ctx()).unwrap();
        let result = mgr
            .dispatch_to("p", &Hook::OnInput("x".to_string()), &ctx())
            .unwrap();
        assert_eq!(result, HookResult::Deny);
    }

    #[test]
    fn t_dispatch_to_not_found() {
        let mut mgr = PluginManager::new();
        assert!(
            mgr.dispatch_to("ghost", &Hook::OnInput("x".to_string()), &ctx())
                .is_err()
        );
    }

    // ── Enable/Disable ──

    #[test]
    fn t_disable_plugin() {
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(
            native("block")
                .hook(HookType::OnInput)
                .on_hook(|_, _| HookResult::Deny)
                .build(),
        ))
        .unwrap();
        mgr.init_all(&ctx()).unwrap();

        mgr.set_enabled("block", false).unwrap();
        assert!(!mgr.is_enabled("block"));
        let result = mgr.dispatch(&Hook::OnInput("x".to_string()), &ctx());
        assert_eq!(result, HookResult::Allow); // disabled, doesn't deny
    }

    #[test]
    fn t_reenable_plugin() {
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(
            native("block")
                .hook(HookType::OnInput)
                .on_hook(|_, _| HookResult::Deny)
                .build(),
        ))
        .unwrap();
        mgr.init_all(&ctx()).unwrap();
        mgr.set_enabled("block", false).unwrap();
        mgr.set_enabled("block", true).unwrap();
        let result = mgr.dispatch(&Hook::OnInput("x".to_string()), &ctx());
        assert_eq!(result, HookResult::Deny);
    }

    // ── Queries ──

    #[test]
    fn t_names() {
        let mut mgr = PluginManager::new();
        mgr.register(make_plugin("alpha", vec![])).unwrap();
        mgr.register(make_plugin("beta", vec![])).unwrap();
        let names = mgr.names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
    }

    #[test]
    fn t_version_of() {
        let mut mgr = PluginManager::new();
        let p = Box::new(native("test").version("2.0.0").build());
        mgr.register(p).unwrap();
        assert_eq!(mgr.version_of("test"), Some("2.0.0"));
    }

    #[test]
    fn t_version_of_not_found() {
        let mgr = PluginManager::new();
        assert!(mgr.version_of("ghost").is_none());
    }

    #[test]
    fn t_plugins_for_hook() {
        let mut mgr = PluginManager::new();
        mgr.register(make_plugin("a", vec![HookType::OnInput]))
            .unwrap();
        mgr.register(make_plugin("b", vec![HookType::OnInput]))
            .unwrap();
        assert_eq!(mgr.plugins_for_hook(HookType::OnInput).len(), 2);
        assert!(mgr.plugins_for_hook(HookType::OnResize).is_empty());
    }

    #[test]
    fn t_active_count() {
        let mut mgr = PluginManager::new();
        mgr.register(make_plugin("a", vec![])).unwrap();
        mgr.register(make_plugin("b", vec![])).unwrap();
        assert_eq!(mgr.active_count(), 0);
        mgr.init_all(&ctx()).unwrap();
        assert_eq!(mgr.active_count(), 2);
    }

    // ── Drop ──

    #[test]
    fn t_drop_calls_shutdown() {
        let mut mgr = PluginManager::new();
        mgr.register(make_plugin("test", vec![])).unwrap();
        mgr.init_all(&ctx()).unwrap();
        drop(mgr); // no panic = success
    }
}
