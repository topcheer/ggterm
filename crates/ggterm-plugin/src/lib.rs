//! # GGTerm Plugin System
//!
//! Extensible plugin architecture for GGTerm terminal emulator.
//!
//! Plugins hook into the terminal's input/output pipeline, observe
//! command execution, and react to theme/resize events.
//!
//! ## Supported Runtimes
//!
//! - **Native**: Rust plugins implementing the [`Plugin`] trait
//! - **Lua** (feature = `lua`): mlua-based sandboxed runtime
//! - **WASM** (feature = `wasm`): wasmtime-based sandboxed runtime
//!
//! ## Quick Start
//!
//! ```no_run
//! use ggterm_plugin::{PluginManager, NativePlugin};
//! use ggterm_plugin::hooks::{HookType, HookResult};
//!
//! let mut mgr = PluginManager::new();
//! let plugin = NativePlugin::new("my-plugin")
//!     .hook(HookType::OnInput)
//!     .on_hook(|hook, ctx| {
//!         HookResult::transform("HELLO")
//!     })
//!     .build();
//! mgr.register(Box::new(plugin)).unwrap();
//! ```

pub mod config;
pub mod hooks;
#[cfg(feature = "lua")]
pub mod lua;
pub mod manager;
pub mod plugin;
#[cfg(feature = "wasm")]
pub mod wasm;

// Re-export key types for convenience
pub use config::{PluginConfig, PluginLoader, PluginManifest, PluginType};
pub use hooks::{Hook, HookResult, HookResultAggregator, HookType};
#[cfg(feature = "lua")]
pub use lua::LuaPlugin;
pub use manager::PluginManager;
pub use plugin::{
    NativePlugin, NativePluginBuilder, Plugin, PluginContext, PluginContextBuilder, PluginError,
    PluginStats, native,
};
#[cfg(feature = "wasm")]
pub use wasm::WasmPlugin;
