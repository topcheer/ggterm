//! Plugin configuration — TOML manifest loading and plugin discovery.
//!
//! Plugins are configured via a TOML file (default: `~/.ggterm/plugins.toml`).
//! Each entry specifies the plugin name, type (native/lua/wasm), path,
//! and optional settings.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::plugin::PluginError;

/// Type of plugin runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginType {
    /// Native Rust plugin (compiled into the binary).
    Native,
    /// Lua script plugin (requires `lua` feature).
    Lua,
    /// WebAssembly module plugin (requires `wasm` feature).
    Wasm,
}

impl PluginType {
    /// Parse from a string.
    pub fn parse(s: &str) -> Result<Self, PluginError> {
        match s.to_lowercase().as_str() {
            "native" | "rust" => Ok(Self::Native),
            "lua" => Ok(Self::Lua),
            "wasm" | "webassembly" => Ok(Self::Wasm),
            other => Err(PluginError::Config(format!(
                "unknown plugin type: '{other}'. Expected: native, lua, or wasm"
            ))),
        }
    }

    /// String representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Lua => "lua",
            Self::Wasm => "wasm",
        }
    }
}

impl std::fmt::Display for PluginType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Configuration for a single plugin.
#[derive(Debug, Clone)]
pub struct PluginConfig {
    /// Plugin name (must be unique).
    pub name: String,
    /// Plugin type (native/lua/wasm).
    pub plugin_type: PluginType,
    /// Path to the plugin file (for Lua/Wasm).
    pub path: Option<PathBuf>,
    /// Whether the plugin is enabled.
    pub enabled: bool,
    /// Plugin-specific settings.
    pub settings: HashMap<String, String>,
}

impl PluginConfig {
    /// Create a new native plugin config.
    pub fn native(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            plugin_type: PluginType::Native,
            path: None,
            enabled: true,
            settings: HashMap::new(),
        }
    }

    /// Create a new Lua plugin config.
    pub fn lua(name: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            plugin_type: PluginType::Lua,
            path: Some(path.into()),
            enabled: true,
            settings: HashMap::new(),
        }
    }

    /// Create a new WASM plugin config.
    pub fn wasm(name: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            plugin_type: PluginType::Wasm,
            path: Some(path.into()),
            enabled: true,
            settings: HashMap::new(),
        }
    }

    /// Set enabled state.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Add a setting.
    pub fn with_setting(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.settings.insert(key.into(), value.into());
        self
    }
}

/// Plugin manifest — the full configuration file.
#[derive(Debug, Clone, Default)]
pub struct PluginManifest {
    /// All plugin configurations.
    pub plugins: Vec<PluginConfig>,
}

impl PluginManifest {
    /// Create an empty manifest.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a plugin config.
    pub fn add(&mut self, config: PluginConfig) {
        self.plugins.push(config);
    }

    /// Get a plugin config by name.
    pub fn get(&self, name: &str) -> Option<&PluginConfig> {
        self.plugins.iter().find(|p| p.name == name)
    }

    /// Get all enabled plugins.
    pub fn enabled(&self) -> impl Iterator<Item = &PluginConfig> {
        self.plugins.iter().filter(|p| p.enabled)
    }

    /// Number of plugins.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Whether the manifest is empty.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

/// Loader for plugin configurations.
pub struct PluginLoader {
    /// Base directory for relative plugin paths.
    base_dir: PathBuf,
}

impl PluginLoader {
    /// Create a new loader with the given base directory.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// Create a loader using the default config directory (`~/.ggterm`).
    pub fn default_dir() -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        Self::new(home.join(".ggterm"))
    }

    /// Get the base directory.
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Validate a plugin config — check that the path exists for Lua/Wasm.
    pub fn validate(&self, config: &PluginConfig) -> Result<(), PluginError> {
        match config.plugin_type {
            PluginType::Native => Ok(()), // No path needed
            PluginType::Lua | PluginType::Wasm => {
                let path = config.path.as_ref().ok_or_else(|| {
                    PluginError::Config(format!(
                        "plugin '{}' ({}) requires a path",
                        config.name, config.plugin_type
                    ))
                })?;

                let full_path = if path.is_absolute() {
                    path.clone()
                } else {
                    self.base_dir.join(path)
                };

                if !full_path.exists() {
                    return Err(PluginError::Config(format!(
                        "plugin '{}' path does not exist: {}",
                        config.name,
                        full_path.display()
                    )));
                }

                Ok(())
            }
        }
    }

    /// Resolve a plugin path relative to the base directory.
    pub fn resolve_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.base_dir.join(path)
        }
    }

    /// Auto-discover plugins in the base directory.
    ///
    /// Looks for:
    /// - `*.lua` files → Lua plugins
    /// - `*.wasm` files → WASm plugins
    pub fn discover(&self) -> Vec<PluginConfig> {
        let mut found = Vec::new();

        if !self.base_dir.exists() {
            return found;
        }

        if let Ok(entries) = std::fs::read_dir(&self.base_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }

                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    let name = path
                        .file_stem()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                        .to_string();

                    match ext {
                        "lua" => found.push(PluginConfig::lua(name, path)),
                        "wasm" => found.push(PluginConfig::wasm(name, path)),
                        _ => {}
                    }
                }
            }
        }

        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── PluginType ──

    #[test]
    fn t_plugin_type_from_str_native() {
        assert_eq!(PluginType::parse("native").unwrap(), PluginType::Native);
        assert_eq!(PluginType::parse("rust").unwrap(), PluginType::Native);
    }

    #[test]
    fn t_plugin_type_from_str_lua() {
        assert_eq!(PluginType::parse("lua").unwrap(), PluginType::Lua);
        assert_eq!(PluginType::parse("LUA").unwrap(), PluginType::Lua);
    }

    #[test]
    fn t_plugin_type_from_str_wasm() {
        assert_eq!(PluginType::parse("wasm").unwrap(), PluginType::Wasm);
        assert_eq!(PluginType::parse("webassembly").unwrap(), PluginType::Wasm);
    }

    #[test]
    fn t_plugin_type_from_str_invalid() {
        assert!(PluginType::parse("python").is_err());
        assert!(PluginType::parse("").is_err());
    }

    #[test]
    fn t_plugin_type_as_str() {
        assert_eq!(PluginType::Native.as_str(), "native");
        assert_eq!(PluginType::Lua.as_str(), "lua");
        assert_eq!(PluginType::Wasm.as_str(), "wasm");
    }

    #[test]
    fn t_plugin_type_display() {
        assert_eq!(format!("{}", PluginType::Native), "native");
    }

    // ── PluginConfig ──

    #[test]
    fn t_config_native() {
        let c = PluginConfig::native("test");
        assert_eq!(c.name, "test");
        assert_eq!(c.plugin_type, PluginType::Native);
        assert!(c.path.is_none());
        assert!(c.enabled);
    }

    #[test]
    fn t_config_lua() {
        let c = PluginConfig::lua("my-plugin", "/path/to/plugin.lua");
        assert_eq!(c.name, "my-plugin");
        assert_eq!(c.plugin_type, PluginType::Lua);
        assert_eq!(
            c.path.as_ref().unwrap().to_str(),
            Some("/path/to/plugin.lua")
        );
    }

    #[test]
    fn t_config_wasm() {
        let c = PluginConfig::wasm("wasm-plugin", "/path/to/plugin.wasm");
        assert_eq!(c.plugin_type, PluginType::Wasm);
        assert!(c.path.is_some());
    }

    #[test]
    fn t_config_with_enabled() {
        let c = PluginConfig::native("test").with_enabled(false);
        assert!(!c.enabled);
    }

    #[test]
    fn t_config_with_settings() {
        let c = PluginConfig::native("test")
            .with_setting("key1", "value1")
            .with_setting("key2", "value2");
        assert_eq!(c.settings.len(), 2);
        assert_eq!(c.settings.get("key1").map(|s| s.as_str()), Some("value1"));
    }

    #[test]
    fn t_config_builder_chain() {
        let c = PluginConfig::lua("test", "test.lua")
            .with_enabled(false)
            .with_setting("timeout", "30");
        assert!(!c.enabled);
        assert_eq!(c.settings.get("timeout").map(|s| s.as_str()), Some("30"));
    }

    // ── PluginManifest ──

    #[test]
    fn t_manifest_new() {
        let m = PluginManifest::new();
        assert!(m.is_empty());
    }

    #[test]
    fn t_manifest_add() {
        let mut m = PluginManifest::new();
        m.add(PluginConfig::native("a"));
        m.add(PluginConfig::native("b"));
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn t_manifest_get() {
        let mut m = PluginManifest::new();
        m.add(PluginConfig::native("alpha"));
        let p = m.get("alpha").unwrap();
        assert_eq!(p.name, "alpha");
    }

    #[test]
    fn t_manifest_get_not_found() {
        let m = PluginManifest::new();
        assert!(m.get("nonexistent").is_none());
    }

    #[test]
    fn t_manifest_enabled() {
        let mut m = PluginManifest::new();
        m.add(PluginConfig::native("a"));
        m.add(PluginConfig::native("b").with_enabled(false));
        m.add(PluginConfig::native("c"));
        let enabled: Vec<_> = m.enabled().collect();
        assert_eq!(enabled.len(), 2);
    }

    // ── PluginLoader ──

    #[test]
    fn t_loader_new() {
        let loader = PluginLoader::new("/tmp/test");
        assert_eq!(loader.base_dir(), Path::new("/tmp/test"));
    }

    #[test]
    fn t_loader_validate_native() {
        let loader = PluginLoader::new("/tmp");
        let config = PluginConfig::native("test");
        assert!(loader.validate(&config).is_ok());
    }

    #[test]
    fn t_loader_validate_lua_missing_path() {
        let loader = PluginLoader::new("/tmp");
        let mut config = PluginConfig::lua("test", "/nonexistent/file.lua");
        config.path = None; // Remove path
        let result = loader.validate(&config);
        assert!(result.is_err());
    }

    #[test]
    fn t_loader_validate_lua_nonexistent_file() {
        let loader = PluginLoader::new("/tmp");
        let config = PluginConfig::lua("test", "/nonexistent/file.lua");
        let result = loader.validate(&config);
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("does not exist"));
    }

    #[test]
    fn t_loader_resolve_path_absolute() {
        let loader = PluginLoader::new("/base");
        let resolved = loader.resolve_path(Path::new("/absolute/path.lua"));
        assert_eq!(resolved, PathBuf::from("/absolute/path.lua"));
    }

    #[test]
    fn t_loader_resolve_path_relative() {
        let loader = PluginLoader::new("/base");
        let resolved = loader.resolve_path(Path::new("relative/path.lua"));
        assert_eq!(resolved, PathBuf::from("/base/relative/path.lua"));
    }

    #[test]
    fn t_loader_discover_empty_dir() {
        // Use a temp dir that doesn't exist
        let loader = PluginLoader::new("/nonexistent/dir/for/testing");
        let found = loader.discover();
        assert!(found.is_empty());
    }

    #[test]
    fn t_loader_discover_with_files() {
        // Create a temp directory with test files
        let temp = std::env::temp_dir().join("ggterm_plugin_test");
        std::fs::create_dir_all(&temp).unwrap();

        // Create test files
        std::fs::write(temp.join("test1.lua"), "-- test lua").unwrap();
        std::fs::write(temp.join("test2.wasm"), b"\0asm").unwrap();
        std::fs::write(temp.join("readme.txt"), "not a plugin").unwrap();

        let loader = PluginLoader::new(&temp);
        let found = loader.discover();

        assert_eq!(found.len(), 2); // test1.lua + test2.wasm
        assert!(
            found
                .iter()
                .any(|p| p.name == "test1" && p.plugin_type == PluginType::Lua)
        );
        assert!(
            found
                .iter()
                .any(|p| p.name == "test2" && p.plugin_type == PluginType::Wasm)
        );

        // Cleanup
        std::fs::remove_dir_all(&temp).ok();
    }
}
