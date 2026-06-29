//! Plugin trait and core types.
//!
//! The [`Plugin`] trait is the central abstraction. All plugin runtimes
//! (Lua, WASM, native Rust) implement this trait so the
//! [`PluginManager`](crate::manager::PluginManager) treats them uniformly.

use std::collections::HashMap;
use std::fmt;

use crate::hooks::{Hook, HookResult};

/// Read-only snapshot of terminal state passed to plugins.
#[derive(Debug, Clone)]
pub struct PluginContext {
    pub cwd: Option<String>,
    pub last_command: Option<String>,
    pub last_exit_code: Option<i32>,
    pub cols: usize,
    pub rows: usize,
    pub theme_name: String,
    pub recent_output: Option<String>,
    pub metadata: HashMap<String, String>,
}

impl PluginContext {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            cwd: None,
            last_command: None,
            last_exit_code: None,
            cols,
            rows,
            theme_name: "dark".to_string(),
            recent_output: None,
            metadata: HashMap::new(),
        }
    }

    pub fn builder(cols: usize, rows: usize) -> PluginContextBuilder {
        PluginContextBuilder {
            inner: Self::new(cols, rows),
        }
    }

    pub fn last_command_succeeded(&self) -> bool {
        self.last_exit_code == Some(0)
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.metadata.get(key).map(|s| s.as_str())
    }
}

impl Default for PluginContext {
    fn default() -> Self {
        Self::new(80, 24)
    }
}

pub struct PluginContextBuilder {
    inner: PluginContext,
}

impl PluginContextBuilder {
    pub fn cwd(mut self, cwd: impl Into<String>) -> Self {
        self.inner.cwd = Some(cwd.into());
        self
    }
    pub fn last_command(mut self, cmd: impl Into<String>) -> Self {
        self.inner.last_command = Some(cmd.into());
        self
    }
    pub fn last_exit_code(mut self, code: i32) -> Self {
        self.inner.last_exit_code = Some(code);
        self
    }
    pub fn theme(mut self, name: impl Into<String>) -> Self {
        self.inner.theme_name = name.into();
        self
    }
    pub fn recent_output(mut self, out: impl Into<String>) -> Self {
        self.inner.recent_output = Some(out.into());
        self
    }
    pub fn metadata(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.inner.metadata.insert(k.into(), v.into());
        self
    }
    pub fn build(self) -> PluginContext {
        self.inner
    }
}

/// Errors during plugin operations.
#[derive(Debug, Clone)]
pub enum PluginError {
    Init(String),
    Execution(String),
    Config(String),
    NotFound(String),
    AlreadyExists(String),
    #[cfg(feature = "lua")]
    Lua(String),
    #[cfg(feature = "wasm")]
    Wasm(String),
    Permission(String),
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Init(m) => write!(f, "plugin init error: {m}"),
            Self::Execution(m) => write!(f, "plugin execution error: {m}"),
            Self::Config(m) => write!(f, "plugin config error: {m}"),
            Self::NotFound(n) => write!(f, "plugin not found: {n}"),
            Self::AlreadyExists(n) => write!(f, "plugin already exists: {n}"),
            #[cfg(feature = "lua")]
            Self::Lua(m) => write!(f, "lua plugin error: {m}"),
            #[cfg(feature = "wasm")]
            Self::Wasm(m) => write!(f, "wasm plugin error: {m}"),
            Self::Permission(m) => write!(f, "plugin permission denied: {m}"),
        }
    }
}

impl std::error::Error for PluginError {}

/// Runtime statistics for a plugin.
#[derive(Debug, Clone, Default)]
pub struct PluginStats {
    pub hooks_called: u64,
    pub denials: u64,
    pub transforms: u64,
    pub annotations: u64,
    pub errors: u64,
}

impl PluginStats {
    pub fn record(&mut self, result: &HookResult) {
        self.hooks_called += 1;
        match result {
            HookResult::Deny => self.denials += 1,
            HookResult::Transform(_) => self.transforms += 1,
            HookResult::Annotate(_, _) => self.annotations += 1,
            HookResult::Allow => {}
        }
    }

    pub fn record_error(&mut self) {
        self.errors += 1;
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// The core plugin trait.
///
/// All plugins — Lua scripts, WASM modules, or native Rust — implement this.
pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError>;
    fn shutdown(&mut self) {}
    fn hooks(&self) -> &[crate::hooks::HookType];
    fn handle_hook(&mut self, hook: &Hook, ctx: &PluginContext) -> HookResult;
}

/// Type alias for the hook handler closure.
pub type HookHandler = Box<dyn Fn(&Hook, &PluginContext) -> HookResult + Send + Sync>;
/// Type alias for the init closure.
pub type InitFn = Box<dyn Fn(&PluginContext) -> Result<(), PluginError> + Send + Sync>;

/// Native Rust plugin backed by closures.
pub struct NativePlugin {
    name: String,
    version: String,
    registered_hooks: Vec<crate::hooks::HookType>,
    handler: HookHandler,
    init_fn: Option<InitFn>,
}

impl Plugin for NativePlugin {
    fn name(&self) -> &str {
        &self.name
    }
    fn version(&self) -> &str {
        &self.version
    }
    fn init(&mut self, ctx: &PluginContext) -> Result<(), PluginError> {
        if let Some(ref f) = self.init_fn {
            f(ctx)?;
        }
        Ok(())
    }
    fn shutdown(&mut self) {}
    fn hooks(&self) -> &[crate::hooks::HookType] {
        &self.registered_hooks
    }
    fn handle_hook(&mut self, hook: &Hook, ctx: &PluginContext) -> HookResult {
        (self.handler)(hook, ctx)
    }
}

pub struct NativePluginBuilder {
    name: String,
    version: String,
    hooks: Vec<crate::hooks::HookType>,
    handler: Option<HookHandler>,
    init_fn: Option<InitFn>,
}

impl NativePluginBuilder {
    pub fn version(mut self, v: impl Into<String>) -> Self {
        self.version = v.into();
        self
    }
    pub fn hook(mut self, h: crate::hooks::HookType) -> Self {
        self.hooks.push(h);
        self
    }
    pub fn on_hook<F>(mut self, f: F) -> Self
    where
        F: Fn(&Hook, &PluginContext) -> HookResult + Send + Sync + 'static,
    {
        self.handler = Some(Box::new(f));
        self
    }
    pub fn on_init<F>(mut self, f: F) -> Self
    where
        F: Fn(&PluginContext) -> Result<(), PluginError> + Send + Sync + 'static,
    {
        self.init_fn = Some(Box::new(f));
        self
    }
    pub fn build(self) -> NativePlugin {
        NativePlugin {
            name: self.name,
            version: self.version,
            registered_hooks: self.hooks,
            handler: self.handler.unwrap_or(Box::new(|_, _| HookResult::Allow)),
            init_fn: self.init_fn,
        }
    }
}

/// Convenience function to create a native plugin builder.
pub fn native(name: impl Into<String>) -> NativePluginBuilder {
    NativePlugin::new(name)
}

impl NativePlugin {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(name: impl Into<String>) -> NativePluginBuilder {
        NativePluginBuilder {
            name: name.into(),
            version: "0.1.0".to_string(),
            hooks: vec![],
            handler: None,
            init_fn: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::HookType;

    #[test]
    fn t_context_new() {
        let ctx = PluginContext::new(120, 40);
        assert_eq!(ctx.cols, 120);
        assert_eq!(ctx.rows, 40);
        assert_eq!(ctx.theme_name, "dark");
        assert!(ctx.cwd.is_none());
    }

    #[test]
    fn t_context_default() {
        let ctx = PluginContext::default();
        assert_eq!(ctx.cols, 80);
        assert_eq!(ctx.rows, 24);
    }

    #[test]
    fn t_context_builder() {
        let ctx = PluginContext::builder(80, 24)
            .cwd("/home/user")
            .last_command("ls")
            .last_exit_code(0)
            .theme("dracula")
            .recent_output("total 0")
            .metadata("key", "val")
            .build();
        assert_eq!(ctx.cwd.as_deref(), Some("/home/user"));
        assert_eq!(ctx.last_command.as_deref(), Some("ls"));
        assert_eq!(ctx.last_exit_code, Some(0));
        assert_eq!(ctx.theme_name, "dracula");
        assert!(ctx.last_command_succeeded());
        assert_eq!(ctx.get("key"), Some("val"));
    }

    #[test]
    fn t_context_last_command_failed() {
        let ctx = PluginContext::builder(80, 24).last_exit_code(1).build();
        assert!(!ctx.last_command_succeeded());
    }

    #[test]
    fn t_context_last_command_no_exit_code() {
        let ctx = PluginContext::new(80, 24);
        assert!(!ctx.last_command_succeeded());
    }

    #[test]
    fn t_plugin_error_display() {
        let e = PluginError::Init("bad".to_string());
        assert!(format!("{e}").contains("bad"));
        let e = PluginError::NotFound("foo".to_string());
        assert!(format!("{e}").contains("foo"));
        let e = PluginError::AlreadyExists("bar".to_string());
        assert!(format!("{e}").contains("bar"));
        let e = PluginError::Permission("denied".to_string());
        assert!(format!("{e}").contains("denied"));
    }

    #[test]
    fn t_plugin_stats_default() {
        let s = PluginStats::default();
        assert_eq!(s.hooks_called, 0);
        assert_eq!(s.denials, 0);
    }

    #[test]
    fn t_plugin_stats_record() {
        let mut s = PluginStats::default();
        s.record(&HookResult::Allow);
        s.record(&HookResult::Deny);
        s.record(&HookResult::Transform("x".to_string()));
        s.record(&HookResult::Annotate("k".to_string(), "v".to_string()));
        assert_eq!(s.hooks_called, 4);
        assert_eq!(s.denials, 1);
        assert_eq!(s.transforms, 1);
        assert_eq!(s.annotations, 1);
    }

    #[test]
    fn t_plugin_stats_record_error() {
        let mut s = PluginStats::default();
        s.record_error();
        s.record_error();
        assert_eq!(s.errors, 2);
    }

    #[test]
    fn t_plugin_stats_reset() {
        let mut s = PluginStats::default();
        s.record(&HookResult::Deny);
        s.record_error();
        s.reset();
        assert_eq!(s.hooks_called, 0);
        assert_eq!(s.errors, 0);
    }

    #[test]
    fn t_native_plugin_basic() {
        let mut p = NativePlugin::new("test")
            .version("1.0.0")
            .hook(HookType::OnInput)
            .on_hook(|_, _| HookResult::Deny)
            .build();
        assert_eq!(p.name(), "test");
        assert_eq!(p.version(), "1.0.0");
        assert_eq!(p.hooks(), &[HookType::OnInput]);

        let ctx = PluginContext::default();
        p.init(&ctx).unwrap();

        let hook = Hook::input("ls");
        let result = p.handle_hook(&hook, &ctx);
        assert_eq!(result, HookResult::Deny);
    }

    #[test]
    fn t_native_plugin_allow_by_default() {
        let mut p = NativePlugin::new("passive").build();
        let ctx = PluginContext::default();
        p.init(&ctx).unwrap();
        let result = p.handle_hook(&Hook::output("hello"), &ctx);
        assert_eq!(result, HookResult::Allow);
    }

    #[test]
    fn t_native_plugin_transform() {
        let mut p = NativePlugin::new("tx")
            .hook(HookType::OnInput)
            .on_hook(|hook, _| match hook {
                Hook::OnInput(t) => HookResult::Transform(t.to_uppercase()),
                _ => HookResult::Allow,
            })
            .build();
        let ctx = PluginContext::default();
        let result = p.handle_hook(&Hook::input("hello"), &ctx);
        assert_eq!(result.transformed_text(), Some("HELLO"));
    }

    #[test]
    fn t_native_plugin_init_failure() {
        let mut p = NativePlugin::new("fail")
            .on_init(|_| Err(PluginError::Init("boom".to_string())))
            .build();
        let result = p.init(&PluginContext::default());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("boom"));
    }

    #[test]
    fn t_native_plugin_multiple_hooks() {
        let p = NativePlugin::new("multi")
            .hook(HookType::OnInput)
            .hook(HookType::OnOutput)
            .hook(HookType::OnCommandEnd)
            .build();
        assert_eq!(p.hooks().len(), 3);
    }

    #[test]
    fn t_native_plugin_shutdown_no_panic() {
        let mut p = NativePlugin::new("cleanup").build();
        p.init(&PluginContext::default()).unwrap();
        p.shutdown();
    }

    #[test]
    fn t_native_plugin_annotate() {
        let mut p = NativePlugin::new("annotator")
            .hook(HookType::OnCommandEnd)
            .on_hook(|_, _| HookResult::annotate("runtime", "42ms"))
            .build();
        let result = p.handle_hook(&Hook::command_end("ls", 0), &PluginContext::default());
        match result {
            HookResult::Annotate(k, v) => {
                assert_eq!(k, "runtime");
                assert_eq!(v, "42ms");
            }
            _ => panic!("expected Annotate"),
        }
    }
}
