//! Configuration system for GGTerm.
//!
//! Loads TOML config from `~/.ggterm/config.toml`, applies settings to the
//! running application, and supports hot-reload via file-system watching.
//!
//! ## Config file format
//!
//! ```toml
//! [appearance]
//! theme = "dark"
//! font_family = "monospace"
//! font_size = 14
//! cell_width = 8
//! cell_height = 16
//!
//! [terminal]
//! scrollback_lines = 10000
//! shell = "/bin/zsh"
//!
//! [ai]
//! enabled = false
//! api_endpoint = "https://api.openai.com/v1"
//! model = "gpt-4o-mini"
//! ```

use std::path::{Path, PathBuf};

#[cfg(feature = "config-watch")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "config-watch")]
use std::sync::Arc;

use thiserror::Error;

// ─── Config structs ──────────────────────────────────────────────────────

/// Top-level GGTerm configuration.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Appearance settings (theme, font, cell dimensions).
    pub appearance: AppearanceConfig,
    /// Terminal behaviour settings (scrollback, shell).
    pub terminal: TerminalConfig,
    /// AI engine settings.
    pub ai: AiConfig,
}

/// Appearance / rendering configuration.
#[derive(Debug, Clone)]
pub struct AppearanceConfig {
    /// Theme name: `"dark"`, `"light"`, `"solarized"`, etc.
    pub theme: String,
    /// Font family name (resolved by glyphon).
    pub font_family: String,
    /// Font size in pixels.
    pub font_size: u32,
    /// Cell width in pixels.
    pub cell_width: u32,
    /// Cell height in pixels.
    pub cell_height: u32,
}

/// Terminal behaviour configuration.
#[derive(Debug, Clone)]
pub struct TerminalConfig {
    /// Maximum scrollback lines retained in history.
    pub scrollback_lines: usize,
    /// Shell program path. If empty, uses `$SHELL` or falls back to `/bin/sh`.
    pub shell: String,
}

/// AI engine configuration.
#[derive(Debug, Clone)]
pub struct AiConfig {
    /// Whether the AI bridge is active at startup.
    pub enabled: bool,
    /// LLM API endpoint URL.
    pub api_endpoint: String,
    /// Model identifier to use for suggestions.
    pub model: String,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: "dark".to_string(),
            font_family: "monospace".to_string(),
            font_size: 14,
            cell_width: 8,
            cell_height: 16,
        }
    }
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            scrollback_lines: 10_000,
            shell: String::new(),
        }
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_endpoint: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o-mini".to_string(),
        }
    }
}

// ─── TOML parsing ────────────────────────────────────────────────────────

/// Internal serde-compatible structs matching the TOML file structure.
mod raw {
    use serde::Deserialize;

    #[derive(Debug, Default, Deserialize)]
    #[serde(default)]
    pub struct Config {
        pub appearance: Appearance,
        pub terminal: Terminal,
        pub ai: Ai,
    }

    #[derive(Debug, Default, Deserialize)]
    #[serde(default)]
    pub struct Appearance {
        pub theme: Option<String>,
        pub font_family: Option<String>,
        pub font_size: Option<u32>,
        pub cell_width: Option<u32>,
        pub cell_height: Option<u32>,
    }

    #[derive(Debug, Default, Deserialize)]
    #[serde(default)]
    pub struct Terminal {
        pub scrollback_lines: Option<usize>,
        pub shell: Option<String>,
    }

    #[derive(Debug, Default, Deserialize)]
    #[serde(default)]
    pub struct Ai {
        pub enabled: Option<bool>,
        pub api_endpoint: Option<String>,
        pub model: Option<String>,
    }
}

impl Config {
    /// Parse a TOML string into a `Config`, applying defaults for missing keys.
    pub fn from_toml_str(s: &str) -> Result<Self, ConfigError> {
        let raw: raw::Config = toml::from_str(s).map_err(ConfigError::Parse)?;
        Ok(Self::from_raw(raw))
    }

    /// Load config from a file path.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::Io(path.display().to_string(), e))?;
        Self::from_toml_str(&contents)
    }

    /// Load config from the default location (`~/.ggterm/config.toml`).
    /// Returns `Ok(Config::default())` when the file does not exist.
    pub fn load_default() -> Result<Self, ConfigError> {
        if let Some(home) = home_dir() {
            let path = home.join(".ggterm").join("config.toml");
            if path.exists() {
                return Self::load(&path);
            }
        }
        Ok(Self::default())
    }

    fn from_raw(raw: raw::Config) -> Self {
        let mut config = Self::default();

        if let Some(v) = raw.appearance.theme {
            config.appearance.theme = v;
        }
        if let Some(v) = raw.appearance.font_family {
            config.appearance.font_family = v;
        }
        if let Some(v) = raw.appearance.font_size {
            config.appearance.font_size = v;
        }
        if let Some(v) = raw.appearance.cell_width {
            config.appearance.cell_width = v;
        }
        if let Some(v) = raw.appearance.cell_height {
            config.appearance.cell_height = v;
        }

        if let Some(v) = raw.terminal.scrollback_lines {
            config.terminal.scrollback_lines = v;
        }
        if let Some(v) = raw.terminal.shell {
            config.terminal.shell = v;
        }

        if let Some(v) = raw.ai.enabled {
            config.ai.enabled = v;
        }
        if let Some(v) = raw.ai.api_endpoint {
            config.ai.api_endpoint = v;
        }
        if let Some(v) = raw.ai.model {
            config.ai.model = v;
        }

        config
    }
}

// ─── Config manager (hot-reload) ─────────────────────────────────────────

/// Callback type invoked when config is reloaded.
pub type ConfigChangeCallback = Box<dyn Fn(&Config) + Send>;

/// Manages loading and hot-reloading the GGTerm configuration.
pub struct ConfigManager {
    config: Config,
    config_path: Option<PathBuf>,
    on_change: Option<ConfigChangeCallback>,

    /// File system watcher for auto-reload (behind `config-watch` feature).
    #[cfg(feature = "config-watch")]
    watcher: Option<notify::RecommendedWatcher>,

    /// Set to `true` by the watcher callback when the config file changes.
    #[cfg(feature = "config-watch")]
    reload_pending: Arc<AtomicBool>,
}

impl ConfigManager {
    /// Create a new manager with default configuration (no file loaded).
    pub fn new() -> Self {
        Self {
            config: Config::default(),
            config_path: None,
            on_change: None,
            #[cfg(feature = "config-watch")]
            watcher: None,
            #[cfg(feature = "config-watch")]
            reload_pending: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Create a manager and load config from the default location.
    pub fn load_default() -> Result<Self, ConfigError> {
        let config = Config::load_default()?;
        let path = default_config_path();
        Ok(Self {
            config,
            config_path: path,
            on_change: None,
            #[cfg(feature = "config-watch")]
            watcher: None,
            #[cfg(feature = "config-watch")]
            reload_pending: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Create a manager and load config from a specific file path.
    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        let config = Config::load(path)?;
        Ok(Self {
            config,
            config_path: Some(path.to_path_buf()),
            on_change: None,
            #[cfg(feature = "config-watch")]
            watcher: None,
            #[cfg(feature = "config-watch")]
            reload_pending: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Register a callback fired when the config is reloaded.
    pub fn on_change(&mut self, f: ConfigChangeCallback) {
        self.on_change = Some(f);
    }

    /// Get the current configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Attempt to reload the config from the same file path.
    ///
    /// Returns `Ok(true)` if the config changed, `Ok(false)` if unchanged,
    /// `Err` if the reload failed (previous config is retained).
    pub fn reload(&mut self) -> Result<bool, ConfigError> {
        let path = match &self.config_path {
            Some(p) => p.clone(),
            None => return Ok(false),
        };
        let new_config = Config::load(&path)?;
        let changed = new_config.appearance.theme != self.config.appearance.theme
            || new_config.appearance.font_size != self.config.appearance.font_size
            || new_config.appearance.cell_width != self.config.appearance.cell_width
            || new_config.appearance.cell_height != self.config.appearance.cell_height
            || new_config.terminal.scrollback_lines != self.config.terminal.scrollback_lines
            || new_config.terminal.shell != self.config.terminal.shell
            || new_config.ai.enabled != self.config.ai.enabled;

        self.config = new_config;
        if changed && let Some(ref f) = self.on_change {
            f(&self.config);
        }
        Ok(changed)
    }

    /// Get the config file path, if one was loaded.
    pub fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }

    // ── File system watching (config-watch feature) ─────────────────────

    /// Start watching the config file for changes.
    ///
    /// When the file is modified, a flag is set.  Call [`poll_reload`]
    /// from your event loop to perform the actual reload.
    ///
    /// Requires the `config-watch` feature.
    #[cfg(feature = "config-watch")]
    pub fn watch(&mut self) -> Result<(), ConfigError> {
        use notify::{RecommendedWatcher, RecursiveMode, Watcher};

        let path = match &self.config_path {
            Some(p) => p.clone(),
            None => {
                return Err(ConfigError::Watch(
                    "no config path loaded".to_string(),
                ));
            }
        };

        // Set up the callback — stores a flag for the main loop to pick up.
        let flag = self.reload_pending.clone();
        let watcher = RecommendedWatcher::new(
            move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res
                    && matches!(
                        event.kind,
                        notify::EventKind::Modify(_) | notify::EventKind::Create(_)
                    )
                {
                    flag.store(true, Ordering::SeqCst);
                }
            },
            notify::Config::default(),
        )
        .map_err(|e| ConfigError::Watch(e.to_string()))?;

        let mut watcher = watcher;
        watcher
            .watch(&path, RecursiveMode::NonRecursive)
            .map_err(|e| ConfigError::Watch(e.to_string()))?;

        self.watcher = Some(watcher);
        log::info!("Watching config file: {}", path.display());
        Ok(())
    }

    /// Stop watching the config file.
    ///
    /// Requires the `config-watch` feature.
    #[cfg(feature = "config-watch")]
    pub fn stop_watch(&mut self) {
        if self.watcher.take().is_some() {
            self.reload_pending.store(false, Ordering::SeqCst);
            log::info!("Stopped watching config file");
        }
    }

    /// Check whether the file watcher is active.
    ///
    /// Requires the `config-watch` feature.
    #[cfg(feature = "config-watch")]
    pub fn is_watching(&self) -> bool {
        self.watcher.is_some()
    }

    /// Poll for pending config reloads.
    ///
    /// If the watcher detected a file change, this calls [`reload`]
    /// and returns `Ok(true)`.  Returns `Ok(false)` when no change
    /// is pending.
    ///
    /// Requires the `config-watch` feature.
    #[cfg(feature = "config-watch")]
    pub fn poll_reload(&mut self) -> Result<bool, ConfigError> {
        if self.reload_pending.swap(false, Ordering::SeqCst) {
            return self.reload();
        }
        Ok(false)
    }
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────

/// Returns the user's home directory, if determinable.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

/// Returns the default config file path (`~/.ggterm/config.toml`), if home is known.
fn default_config_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".ggterm").join("config.toml"))
}

/// Configuration errors.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// TOML parse error.
    #[error("config parse error: {0}")]
    Parse(#[from] toml::de::Error),
    /// File I/O error (path, source).
    #[error("config I/O error ({0}): {1}")]
    Io(String, std::io::Error),
    /// File-watch error (from the `notify` crate).
    #[cfg(feature = "config-watch")]
    #[error("config watch error: {0}")]
    Watch(String),
}

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.appearance.theme, "dark");
        assert_eq!(config.appearance.font_size, 14);
        assert_eq!(config.terminal.scrollback_lines, 10_000);
        assert!(!config.ai.enabled);
    }

    #[test]
    fn test_parse_full_config() {
        let toml = r#"
[appearance]
theme = "light"
font_family = "JetBrains Mono"
font_size = 16
cell_width = 9
cell_height = 18

[terminal]
scrollback_lines = 5000
shell = "/bin/bash"

[ai]
enabled = true
api_endpoint = "http://localhost:8080/v1"
model = "llama-3"
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert_eq!(config.appearance.theme, "light");
        assert_eq!(config.appearance.font_family, "JetBrains Mono");
        assert_eq!(config.appearance.font_size, 16);
        assert_eq!(config.appearance.cell_width, 9);
        assert_eq!(config.appearance.cell_height, 18);
        assert_eq!(config.terminal.scrollback_lines, 5000);
        assert_eq!(config.terminal.shell, "/bin/bash");
        assert!(config.ai.enabled);
        assert_eq!(config.ai.api_endpoint, "http://localhost:8080/v1");
        assert_eq!(config.ai.model, "llama-3");
    }

    #[test]
    fn test_parse_empty_uses_defaults() {
        let config = Config::from_toml_str("").unwrap();
        assert_eq!(config.appearance.theme, "dark");
        assert_eq!(config.appearance.font_size, 14);
        assert_eq!(config.terminal.scrollback_lines, 10_000);
        assert!(!config.ai.enabled);
    }

    #[test]
    fn test_parse_partial_config() {
        let toml = r#"
[appearance]
theme = "solarized"
font_size = 12
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert_eq!(config.appearance.theme, "solarized");
        assert_eq!(config.appearance.font_size, 12);
        // Unspecified fields keep defaults
        assert_eq!(config.appearance.font_family, "monospace");
        assert_eq!(config.terminal.scrollback_lines, 10_000);
    }

    #[test]
    fn test_parse_invalid_toml() {
        let result = Config::from_toml_str("not valid toml [[[[");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_invalid_type() {
        let toml = r#"
[appearance]
font_size = "not a number"
"#;
        let result = Config::from_toml_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_load_from_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("ggterm_test_config.toml");
        std::fs::write(
            &path,
            r#"
[appearance]
theme = "light"
font_size = 20

[terminal]
shell = "/usr/bin/fish"
"#,
        )
        .unwrap();

        let config = Config::load(&path).unwrap();
        assert_eq!(config.appearance.theme, "light");
        assert_eq!(config.appearance.font_size, 20);
        assert_eq!(config.terminal.shell, "/usr/bin/fish");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_nonexistent_fails() {
        let result = Config::load(Path::new("/nonexistent/path/config.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_default_missing_file() {
        // With a HOME pointing to a directory without config.toml,
        // load_default should return defaults.
        // This test relies on the real HOME not having a config.toml
        // (which is the case in CI / dev environments).
        let config = Config::load_default().unwrap_or_default();
        // Should be valid defaults
        assert_eq!(config.appearance.theme, "dark");
    }

    #[test]
    fn test_config_manager_new() {
        let mgr = ConfigManager::new();
        assert_eq!(mgr.config().appearance.theme, "dark");
        assert_eq!(mgr.config().terminal.scrollback_lines, 10_000);
    }

    #[test]
    fn test_config_manager_load_from() {
        let dir = std::env::temp_dir();
        let path = dir.join("ggterm_test_mgr.toml");
        std::fs::write(
            &path,
            r#"
[appearance]
theme = "solarized"
"#,
        )
        .unwrap();

        let mgr = ConfigManager::load_from(&path).unwrap();
        assert_eq!(mgr.config().appearance.theme, "solarized");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_config_manager_reload() {
        let dir = std::env::temp_dir();
        let path = dir.join("ggterm_test_reload.toml");

        // Write initial config
        std::fs::write(&path, "[appearance]\ntheme = \"dark\"\n").unwrap();
        let mut mgr = ConfigManager::load_from(&path).unwrap();
        assert_eq!(mgr.config().appearance.theme, "dark");

        // Overwrite with new theme
        std::fs::write(&path, "[appearance]\ntheme = \"light\"\n").unwrap();
        let changed = mgr.reload().unwrap();
        assert!(changed);
        assert_eq!(mgr.config().appearance.theme, "light");

        // Reload again — should report no change
        let changed = mgr.reload().unwrap();
        assert!(!changed);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_config_manager_reload_no_path() {
        let mut mgr = ConfigManager::new();
        let changed = mgr.reload().unwrap();
        assert!(!changed);
    }

    #[test]
    fn test_config_manager_on_change_callback() {
        let dir = std::env::temp_dir();
        let path = dir.join("ggterm_test_callback.toml");

        std::fs::write(&path, "[terminal]\nscrollback_lines = 1000\n").unwrap();
        let mut mgr = ConfigManager::load_from(&path).unwrap();

        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_clone = called.clone();
        mgr.on_change(Box::new(move |_| {
            called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        }));

        // Change scrollback
        std::fs::write(&path, "[terminal]\nscrollback_lines = 2000\n").unwrap();
        mgr.reload().unwrap();
        assert!(called.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(mgr.config().terminal.scrollback_lines, 2000);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_config_error_display() {
        let err = ConfigError::Parse(toml::from_str::<toml::Value>("bad").unwrap_err());
        assert!(err.to_string().contains("parse error"));

        let io_err = ConfigError::Io(
            "/tmp/test".to_string(),
            std::io::Error::new(std::io::ErrorKind::NotFound, "file not found"),
        );
        assert!(io_err.to_string().contains("/tmp/test"));
    }

    #[test]
    fn test_config_path() {
        let mgr = ConfigManager::new();
        assert!(mgr.config_path().is_none());

        let dir = std::env::temp_dir();
        let path = dir.join("ggterm_test_path.toml");
        std::fs::write(&path, "[appearance]\ntheme = \"dark\"\n").unwrap();

        let mgr = ConfigManager::load_from(&path).unwrap();
        assert_eq!(mgr.config_path(), Some(path.as_path()));

        let _ = std::fs::remove_file(&path);
    }
}

// ─── config-watch tests ─────────────────────────────────────────────────

#[cfg(all(test, feature = "config-watch"))]
mod watch_tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::thread;
    use std::time::Duration;

    fn wait_for(mgr: &ConfigManager, expected: bool, timeout_ms: u64) {
        let mut elapsed = 0u64;
        loop {
            if mgr.is_watching() == expected {
                return;
            }
            if elapsed >= timeout_ms {
                return;
            }
            thread::sleep(Duration::from_millis(50));
            elapsed += 50;
        }
    }

    #[test]
    fn test_watch_no_path_returns_err() {
        let mut mgr = ConfigManager::new();
        let result = mgr.watch();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no config"));
    }

    #[test]
    fn test_watch_with_path_succeeds() {
        let dir = std::env::temp_dir();
        let path = dir.join("ggterm_watch_start.toml");
        std::fs::write(&path, "[appearance]\ntheme = \"dark\"\n").unwrap();

        let mut mgr = ConfigManager::load_from(&path).unwrap();
        assert!(!mgr.is_watching());

        mgr.watch().unwrap();
        assert!(mgr.is_watching());

        mgr.stop_watch();
        assert!(!mgr.is_watching());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_stop_watch_without_start_is_noop() {
        let mut mgr = ConfigManager::new();
        mgr.stop_watch(); // must not panic
    }

    #[test]
    fn test_stop_watch_double_is_noop() {
        let dir = std::env::temp_dir();
        let path = dir.join("ggterm_watch_double.toml");
        std::fs::write(&path, "[appearance]\ntheme = \"dark\"\n").unwrap();

        let mut mgr = ConfigManager::load_from(&path).unwrap();
        mgr.watch().unwrap();
        mgr.stop_watch();
        mgr.stop_watch(); // must not panic

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_watch_restart() {
        let dir = std::env::temp_dir();
        let path = dir.join("ggterm_watch_restart.toml");
        std::fs::write(&path, "[appearance]\ntheme = \"dark\"\n").unwrap();

        let mut mgr = ConfigManager::load_from(&path).unwrap();
        mgr.watch().unwrap();
        mgr.stop_watch();
        mgr.watch().unwrap(); // restart
        assert!(mgr.is_watching());

        mgr.stop_watch();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_poll_reload_no_watch() {
        let mut mgr = ConfigManager::new();
        let changed = mgr.poll_reload().unwrap();
        assert!(!changed);
    }

    #[test]
    fn test_poll_reload_no_file_change() {
        let dir = std::env::temp_dir();
        let path = dir.join("ggterm_watch_poll.toml");
        std::fs::write(&path, "[appearance]\ntheme = \"dark\"\n").unwrap();

        let mut mgr = ConfigManager::load_from(&path).unwrap();
        mgr.watch().unwrap();

        // Drain any initial events from file creation.
        thread::sleep(Duration::from_millis(300));
        let _ = mgr.poll_reload();

        // No change after draining.
        let changed = mgr.poll_reload().unwrap();
        assert!(!changed);

        mgr.stop_watch();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_watch_triggers_reload_on_file_change() {
        let dir = std::env::temp_dir();
        let path = dir.join("ggterm_watch_real.toml");
        std::fs::write(&path, "[appearance]\ntheme = \"dark\"\n").unwrap();

        let mut mgr = ConfigManager::load_from(&path).unwrap();

        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_clone = called.clone();
        mgr.on_change(Box::new(move |_| {
            called_clone.store(true, Ordering::SeqCst);
        }));

        mgr.watch().unwrap();

        // Drain initial events.
        thread::sleep(Duration::from_millis(300));
        let _ = mgr.poll_reload();

        // Modify the config file.
        std::fs::write(&path, "[appearance]\ntheme = \"light\"\n").unwrap();

        // Wait for the watcher to detect the change.
        thread::sleep(Duration::from_millis(500));

        let changed = mgr.poll_reload().unwrap();
        assert!(changed, "poll_reload should report a change after file modification");
        assert_eq!(mgr.config().appearance.theme, "light");
        assert!(called.load(Ordering::SeqCst), "on_change callback should have been fired");

        mgr.stop_watch();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_watch_nonexistent_dir_returns_err() {
        let dir = std::env::temp_dir();
        let path = dir.join("nonexistent_subdir_12345").join("config.toml");

        let mut mgr = ConfigManager {
            config: Config::default(),
            config_path: Some(path),
            on_change: None,
            watcher: None,
            reload_pending: Arc::new(AtomicBool::new(false)),
        };

        let result = mgr.watch();
        assert!(result.is_err());
    }
}
