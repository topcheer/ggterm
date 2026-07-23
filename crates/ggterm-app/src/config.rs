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
//!
//! [keybindings]
//! new_tab = "Ctrl+T"
//! close_tab = "Ctrl+W"
//! paste = "Ctrl+Shift+V"
//! search = "Ctrl+Shift+F"
//! fullscreen = "F11"
//! ```

use std::path::{Path, PathBuf};

#[cfg(feature = "config-watch")]
use std::sync::Arc;
#[cfg(feature = "config-watch")]
use std::sync::atomic::{AtomicBool, Ordering};

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
    /// Keyboard shortcut overrides.
    pub keybindings: KeybindingsConfig,
    /// Named configuration profiles (P22-C).
    ///
    /// Each profile can override `font_size`, `theme`, and `scrollback_lines`.
    /// Users define them under `[profiles.<name>]` in config.toml.
    pub profiles: std::collections::HashMap<String, Profile>,
    /// Plugin system configuration (P23-D).
    pub plugins: PluginConfig,
}

/// A named configuration profile (P22-C).
///
/// Fields that are `None` fall back to the base config value.
///
/// ```toml
/// [profiles.presentation]
/// font_size = 22
/// theme = "light"
///
/// [profiles.compact]
/// font_size = 10
/// scrollback_lines = 50000
/// ```
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct Profile {
    /// Override font size for this profile.
    pub font_size: Option<u32>,
    /// Override theme name for this profile.
    pub theme: Option<String>,
    /// Override scrollback lines for this profile.
    pub scrollback_lines: Option<usize>,
}

/// Plugin system configuration (P23-D).
///
/// ```toml
/// [plugins]
/// enabled = true
/// directory = "~/.ggterm/plugins"
/// ```
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PluginConfig {
    /// Whether the plugin system is enabled.
    #[serde(default = "default_plugins_enabled")]
    pub enabled: bool,
    /// Directory path for Lua plugins. Defaults to `~/.ggterm/plugins`.
    #[serde(default = "default_plugins_dir")]
    pub directory: String,
}

fn default_plugins_enabled() -> bool {
    false
}

fn default_plugins_dir() -> String {
    "~/.ggterm/plugins".to_string()
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            enabled: default_plugins_enabled(),
            directory: default_plugins_dir(),
        }
    }
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
    /// Default cursor style: "block", "underline", or "bar".
    pub cursor_style: String,
    /// Background opacity: 0.0 (fully transparent) to 1.0 (fully opaque).
    /// Values below 1.0 allow the desktop wallpaper or windows behind the
    /// terminal to show through. Default: 1.0 (opaque).
    pub background_opacity: f32,
    /// Terminal content padding in pixels (applied to all four sides).
    /// Default: 8 (matches the built-in CONTENT_PADDING).
    pub padding: u32,
    /// Whether the cursor blinks. Set to false for a steady cursor.
    /// Default: true.
    pub cursor_blink: bool,
    /// Whether to highlight the entire line where the cursor is positioned.
    /// Similar to Vim's `cursorline`. Default: false.
    pub cursor_line_highlight: bool,
    /// Custom ANSI 16-color palette (hex strings). Overrides theme palette.
    pub custom_palette: Option<[String; 16]>,
    /// Custom foreground color (hex). Overrides theme.
    pub custom_fg: Option<String>,
    /// Custom background color (hex). Overrides theme.
    pub custom_bg: Option<String>,
    /// Custom cursor color (hex). Overrides theme.
    pub custom_cursor: Option<String>,
    /// Custom selection background color (hex). Overrides theme.
    pub custom_selection: Option<String>,
}

/// Terminal behaviour configuration.
#[derive(Debug, Clone)]
pub struct TerminalConfig {
    /// Maximum scrollback lines retained in history.
    pub scrollback_lines: usize,
    /// Shell program path. If empty, uses `$SHELL` or falls back to `/bin/sh`.
    pub shell: String,
    /// Whether to restore the previous session (tabs, panes, splits) on startup.
    /// Default: false (clean single-pane start).
    pub restore_session: bool,
    /// Bell behavior: "none", "visual", or "sound".
    /// Default: "visual" (flash the terminal).
    pub bell_mode: String,
    /// When true, selecting text with the mouse automatically copies it to
    /// the system clipboard on mouse release (xterm/iTerm2 behaviour).
    /// Default: true.
    pub copy_on_select: bool,

    /// Extra characters treated as part of a word for double-click selection.
    /// Default: ".-/:@~+#?=&%$" (path and URL characters).
    /// Set to empty string to only select alphanumeric + underscore.
    pub word_chars: String,

    /// When true, shows a desktop notification when a long-running command
    /// finishes while the terminal window is unfocused.
    /// Default: true.
    pub notify_on_complete: bool,
    /// Minimum command duration (in seconds) to trigger a completion notification.
    /// Commands shorter than this threshold are silently ignored.
    /// Default: 10.
    pub min_notify_duration_secs: u64,
    /// Whether to auto-inject OSC 133 shell integration when spawning shells.
    /// Set to false to disable automatic prompt/command markers.
    /// Default: true.
    pub shell_integration: bool,
    /// Search engine URL template for 'Search Web' feature.
    /// The query replaces '%s'. Default: Google.
    /// Examples: "https://duckduckgo.com/?q=%s", "https://www.bing.com/search?q=%s"
    pub search_engine: String,
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
    /// API key for authentication. Can also be set via GGTERM_AI_API_KEY env var.
    pub api_key: String,
    /// Enable tool calling (run_command, read_file, list_files).
    /// Allows the AI to execute commands to gather context.
    pub enable_tools: bool,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: "dark".to_string(),
            font_family: "monospace".to_string(),
            font_size: 14,
            cell_width: 8,
            cell_height: 16,
            cursor_style: "block".to_string(),
            background_opacity: 1.0,
            padding: 8,
            cursor_blink: true,
            cursor_line_highlight: false,
            custom_palette: None,
            custom_fg: None,
            custom_bg: None,
            custom_cursor: None,
            custom_selection: None,
        }
    }
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            scrollback_lines: 10_000,
            shell: String::new(),
            restore_session: false,
            bell_mode: "visual".to_string(),
            copy_on_select: true,
            word_chars: ".-/:@~+#?=&%$".to_string(),
            notify_on_complete: true,
            min_notify_duration_secs: 10,
            shell_integration: true,
            search_engine: "https://www.google.com/search?q=%s".to_string(),
        }
    }
}

/// User-configurable keyboard shortcuts.
///
/// Each field is `None` by default, meaning the built-in shortcut is used.
/// When set to `Some("Ctrl+Shift+V")`, the [`parse_keybinding`] helper converts
/// the string into modifier flags + key name.
#[derive(Debug, Clone, Default)]
pub struct KeybindingsConfig {
    /// New tab.
    pub new_tab: Option<String>,
    /// Close tab.
    pub close_tab: Option<String>,
    /// Switch to tab 1.
    pub switch_tab_1: Option<String>,
    /// Paste from clipboard.
    pub paste: Option<String>,
    /// Copy selection to clipboard.
    pub copy: Option<String>,
    /// Toggle scrollback search.
    pub search: Option<String>,
    /// Zoom in (increase font).
    pub zoom_in: Option<String>,
    /// Zoom out (decrease font).
    pub zoom_out: Option<String>,
    /// Reset font zoom.
    pub zoom_reset: Option<String>,
    /// Toggle fullscreen.
    pub fullscreen: Option<String>,
    /// Clear screen + scrollback.
    pub clear: Option<String>,
    /// Reset terminal (RIS).
    pub reset: Option<String>,
    /// Cycle to next theme.
    pub cycle_theme: Option<String>,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_endpoint: "https://open.bigmodel.cn/api/paas/v4".to_string(),
            model: "glm-4-flash".to_string(),
            api_key: String::new(),
            enable_tools: true,
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
        pub keybindings: Keybindings,
        pub profiles: std::collections::HashMap<String, super::Profile>,
        pub plugins: super::PluginConfig,
    }

    #[derive(Debug, Default, Deserialize)]
    #[serde(default)]
    pub struct Appearance {
        pub theme: Option<String>,
        pub font_family: Option<String>,
        pub font_size: Option<u32>,
        pub cell_width: Option<u32>,
        pub cell_height: Option<u32>,
        pub cursor_style: Option<String>,
        pub background_opacity: Option<f32>,
        pub padding: Option<u32>,
        pub cursor_blink: Option<bool>,
        pub cursor_line_highlight: Option<bool>,
        pub colors: Option<Colors>,
    }

    #[derive(Debug, Default, Deserialize)]
    #[serde(default)]
    pub struct Colors {
        /// 16 ANSI colors as hex strings: [black, red, green, yellow, blue,
        /// magenta, cyan, white, bright_black, ..., bright_white].
        pub ansi: Option<Vec<String>>,
        /// Default foreground (text) color as hex string.
        pub foreground: Option<String>,
        /// Default background color as hex string.
        pub background: Option<String>,
        /// Cursor color as hex string.
        pub cursor: Option<String>,
        /// Selection background color as hex string.
        pub selection: Option<String>,
    }

    #[derive(Debug, Default, Deserialize)]
    #[serde(default)]
    pub struct Terminal {
        pub scrollback_lines: Option<usize>,
        pub shell: Option<String>,
        pub restore_session: Option<bool>,
        pub bell_mode: Option<String>,
        pub copy_on_select: Option<bool>,
        pub word_chars: Option<String>,
        pub notify_on_complete: Option<bool>,
        pub min_notify_duration_secs: Option<u64>,
        pub shell_integration: Option<bool>,
        pub search_engine: Option<String>,
    }

    #[derive(Debug, Default, Deserialize)]
    #[serde(default)]
    pub struct Ai {
        pub enabled: Option<bool>,
        pub api_endpoint: Option<String>,
        pub model: Option<String>,
        pub api_key: Option<String>,
        pub enable_tools: Option<bool>,
    }

    #[derive(Debug, Default, Deserialize)]
    #[serde(default)]
    pub struct Keybindings {
        pub new_tab: Option<String>,
        pub close_tab: Option<String>,
        pub switch_tab_1: Option<String>,
        pub paste: Option<String>,
        pub copy: Option<String>,
        pub search: Option<String>,
        pub zoom_in: Option<String>,
        pub zoom_out: Option<String>,
        pub zoom_reset: Option<String>,
        pub fullscreen: Option<String>,
        pub clear: Option<String>,
        pub reset: Option<String>,
        pub cycle_theme: Option<String>,
    }
}

impl Config {
    /// Parse a TOML string into a `Config`, applying defaults for missing keys.
    pub fn from_toml_str(s: &str) -> Result<Self, ConfigError> {
        let raw: raw::Config = toml::from_str(s).map_err(ConfigError::Parse)?;
        let config = Self::from_raw(raw);
        if let Err(ref e) = config.validate() {
            log::warn!("Config validation: {e} (values clamped to valid ranges)");
        }
        Ok(config)
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
            // First run: create a documented config file so users know
            // what options are available. This is a "best effort" — if
            // file creation fails, we just use defaults silently.
            let config = Self::default();
            if let Some(parent) = path.parent()
                && std::fs::create_dir_all(parent).is_ok()
            {
                let template = config.generate_documented_template();
                if std::fs::write(&path, template).is_ok() {
                    log::info!("Created default config at {}", path.display());
                }
            }
            return Ok(config);
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
            if !(6..=32).contains(&v) {
                log::warn!("font_size {v} out of range [6, 32], clamped");
            }
            config.appearance.font_size = v.clamp(6, 32);
        }
        if let Some(v) = raw.appearance.cell_width {
            if !(4..=32).contains(&v) {
                log::warn!("cell_width {v} out of range [4, 32], clamped");
            }
            config.appearance.cell_width = v.clamp(4, 32);
        }
        if let Some(v) = raw.appearance.cell_height {
            if !(4..=32).contains(&v) {
                log::warn!("cell_height {v} out of range [4, 32], clamped");
            }
            config.appearance.cell_height = v.clamp(4, 32);
        }
        if let Some(v) = raw.appearance.cursor_style {
            let normalized = v.to_lowercase();
            if matches!(normalized.as_str(), "block" | "underline" | "bar") {
                config.appearance.cursor_style = normalized;
            } else {
                log::warn!("Invalid cursor_style '{v}', expected: block, underline, bar");
            }
        }
        if let Some(v) = raw.appearance.background_opacity {
            if !(0.0..=1.0).contains(&v) {
                log::warn!("background_opacity {v} out of range [0.0, 1.0], clamped");
            }
            config.appearance.background_opacity = v.clamp(0.0, 1.0);
        }
        if let Some(v) = raw.appearance.padding {
            if v > 100 {
                log::warn!("padding {v} exceeds max 100, clamped");
            }
            config.appearance.padding = v.min(100);
        }
        if let Some(v) = raw.appearance.cursor_blink {
            config.appearance.cursor_blink = v;
        }
        if let Some(v) = raw.appearance.cursor_line_highlight {
            config.appearance.cursor_line_highlight = v;
        }
        // Apply custom color palette overrides.
        if let Some(colors) = raw.appearance.colors {
            config.appearance.custom_palette = colors
                .ansi
                .as_deref()
                .filter(|a| a.len() == 16)
                .map(|a| std::array::from_fn(|i| a[i].clone()));
            config.appearance.custom_fg = colors.foreground;
            config.appearance.custom_bg = colors.background;
            config.appearance.custom_cursor = colors.cursor;
            config.appearance.custom_selection = colors.selection;
        }

        if let Some(v) = raw.terminal.scrollback_lines {
            config.terminal.scrollback_lines = v;
        }
        if let Some(v) = raw.terminal.shell {
            config.terminal.shell = v;
        }
        if let Some(v) = raw.terminal.restore_session {
            config.terminal.restore_session = v;
        }
        if let Some(v) = raw.terminal.bell_mode {
            let mode = v.to_lowercase();
            if mode == "none" || mode == "visual" || mode == "sound" || mode == "both" {
                config.terminal.bell_mode = mode;
            }
        }
        if let Some(v) = raw.terminal.copy_on_select {
            config.terminal.copy_on_select = v;
        }
        if let Some(v) = raw.terminal.word_chars {
            config.terminal.word_chars = v;
        }
        if let Some(v) = raw.terminal.notify_on_complete {
            config.terminal.notify_on_complete = v;
        }
        if let Some(v) = raw.terminal.min_notify_duration_secs {
            config.terminal.min_notify_duration_secs = v.max(1);
        }
        if let Some(v) = raw.terminal.shell_integration {
            config.terminal.shell_integration = v;
        }
        if let Some(v) = raw.terminal.search_engine
            && v.contains('%')
            && v.contains('s')
        {
            config.terminal.search_engine = v;
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
        if let Some(v) = raw.ai.api_key {
            config.ai.api_key = v;
        }
        if let Some(v) = raw.ai.enable_tools {
            config.ai.enable_tools = v;
        }

        let kb = raw.keybindings;
        config.keybindings.new_tab = kb.new_tab;
        config.keybindings.close_tab = kb.close_tab;
        config.keybindings.switch_tab_1 = kb.switch_tab_1;
        config.keybindings.paste = kb.paste;
        config.keybindings.copy = kb.copy;
        config.keybindings.search = kb.search;
        config.keybindings.zoom_in = kb.zoom_in;
        config.keybindings.zoom_out = kb.zoom_out;
        config.keybindings.zoom_reset = kb.zoom_reset;
        config.keybindings.fullscreen = kb.fullscreen;
        config.keybindings.clear = kb.clear;
        config.keybindings.reset = kb.reset;
        config.keybindings.cycle_theme = kb.cycle_theme;

        // P22-C: Named profiles.
        config.profiles = raw.profiles;

        // P23-D: Plugin config.
        config.plugins = raw.plugins;

        config
    }

    // ── Export / Import / Reset ──────────────────────────────────────

    /// Export the current configuration as a TOML string.
    ///
    /// Produces a TOML document that round-trips through [`import_from_toml`].
    ///
    /// [`import_from_toml`]: Config::import_from_toml
    pub fn export_to_toml(&self) -> Result<String, ConfigError> {
        let mut root = toml::Table::new();

        // [appearance]
        let mut appearance = toml::Table::new();
        appearance.insert("theme".into(), self.appearance.theme.clone().into());
        appearance.insert(
            "font_family".into(),
            self.appearance.font_family.clone().into(),
        );
        appearance.insert(
            "font_size".into(),
            (self.appearance.font_size as i64).into(),
        );
        appearance.insert(
            "cell_width".into(),
            (self.appearance.cell_width as i64).into(),
        );
        appearance.insert(
            "cell_height".into(),
            (self.appearance.cell_height as i64).into(),
        );
        appearance.insert(
            "background_opacity".into(),
            (self.appearance.background_opacity as f64).into(),
        );
        appearance.insert("padding".into(), (self.appearance.padding as i64).into());
        appearance.insert("cursor_blink".into(), self.appearance.cursor_blink.into());
        appearance.insert(
            "cursor_line_highlight".into(),
            self.appearance.cursor_line_highlight.into(),
        );
        // [appearance.colors] — only export when custom colors are set.
        let has_colors = self.appearance.custom_palette.is_some()
            || self.appearance.custom_fg.is_some()
            || self.appearance.custom_bg.is_some()
            || self.appearance.custom_cursor.is_some()
            || self.appearance.custom_selection.is_some();
        if has_colors {
            let mut colors = toml::Table::new();
            if let Some(ref palette) = self.appearance.custom_palette {
                let ansi: Vec<toml::Value> = palette.iter().map(|c| c.clone().into()).collect();
                colors.insert("ansi".into(), ansi.into());
            }
            if let Some(ref v) = self.appearance.custom_fg {
                colors.insert("foreground".into(), v.clone().into());
            }
            if let Some(ref v) = self.appearance.custom_bg {
                colors.insert("background".into(), v.clone().into());
            }
            if let Some(ref v) = self.appearance.custom_cursor {
                colors.insert("cursor".into(), v.clone().into());
            }
            if let Some(ref v) = self.appearance.custom_selection {
                colors.insert("selection".into(), v.clone().into());
            }
            appearance.insert("colors".into(), colors.into());
        }
        root.insert("appearance".into(), appearance.into());

        // [terminal]
        let mut terminal = toml::Table::new();
        terminal.insert(
            "scrollback_lines".into(),
            (self.terminal.scrollback_lines as i64).into(),
        );
        terminal.insert("shell".into(), self.terminal.shell.clone().into());
        terminal.insert("bell_mode".into(), self.terminal.bell_mode.clone().into());
        terminal.insert("copy_on_select".into(), self.terminal.copy_on_select.into());
        terminal.insert("word_chars".into(), self.terminal.word_chars.clone().into());
        terminal.insert(
            "shell_integration".into(),
            self.terminal.shell_integration.into(),
        );
        terminal.insert(
            "search_engine".into(),
            self.terminal.search_engine.clone().into(),
        );
        root.insert("terminal".into(), terminal.into());

        // [ai]
        let mut ai = toml::Table::new();
        ai.insert("enabled".into(), self.ai.enabled.into());
        ai.insert("api_endpoint".into(), self.ai.api_endpoint.clone().into());
        ai.insert("model".into(), self.ai.model.clone().into());
        ai.insert("api_key".into(), self.ai.api_key.clone().into());
        ai.insert("enable_tools".into(), self.ai.enable_tools.into());
        root.insert("ai".into(), ai.into());

        // [keybindings] — only non-None entries
        let mut kb = toml::Table::new();
        let km = &self.keybindings;
        if let Some(ref v) = km.new_tab {
            kb.insert("new_tab".into(), v.clone().into());
        }
        if let Some(ref v) = km.close_tab {
            kb.insert("close_tab".into(), v.clone().into());
        }
        if let Some(ref v) = km.switch_tab_1 {
            kb.insert("switch_tab_1".into(), v.clone().into());
        }
        if let Some(ref v) = km.paste {
            kb.insert("paste".into(), v.clone().into());
        }
        if let Some(ref v) = km.copy {
            kb.insert("copy".into(), v.clone().into());
        }
        if let Some(ref v) = km.search {
            kb.insert("search".into(), v.clone().into());
        }
        if let Some(ref v) = km.zoom_in {
            kb.insert("zoom_in".into(), v.clone().into());
        }
        if let Some(ref v) = km.zoom_out {
            kb.insert("zoom_out".into(), v.clone().into());
        }
        if let Some(ref v) = km.zoom_reset {
            kb.insert("zoom_reset".into(), v.clone().into());
        }
        if let Some(ref v) = km.fullscreen {
            kb.insert("fullscreen".into(), v.clone().into());
        }
        if let Some(ref v) = km.clear {
            kb.insert("clear".into(), v.clone().into());
        }
        if let Some(ref v) = km.reset {
            kb.insert("reset".into(), v.clone().into());
        }
        if let Some(ref v) = km.cycle_theme {
            kb.insert("cycle_theme".into(), v.clone().into());
        }
        if !kb.is_empty() {
            root.insert("keybindings".into(), kb.into());
        }

        // [plugins]
        let mut plugins = toml::Table::new();
        plugins.insert("enabled".into(), self.plugins.enabled.into());
        plugins.insert("directory".into(), self.plugins.directory.clone().into());
        root.insert("plugins".into(), plugins.into());

        // [profiles.<name>]
        if !self.profiles.is_empty() {
            let mut profiles_map = toml::Table::new();
            for (name, profile) in &self.profiles {
                let mut pt = toml::Table::new();
                if let Some(v) = profile.font_size {
                    pt.insert("font_size".into(), (v as i64).into());
                }
                if let Some(ref v) = profile.theme {
                    pt.insert("theme".into(), v.clone().into());
                }
                if let Some(v) = profile.scrollback_lines {
                    pt.insert("scrollback_lines".into(), (v as i64).into());
                }
                profiles_map.insert(name.clone(), pt.into());
            }
            root.insert("profiles".into(), profiles_map.into());
        }

        let val = toml::Value::Table(root);
        toml::to_string_pretty(&val).map_err(ConfigError::Export)
    }

    /// Generate a user-friendly config template with explanatory comments.
    ///
    /// Used when creating a new config file for the first time.
    /// Unlike `export_to_toml()`, this includes inline comments that
    /// explain each setting, making it easy for new users to configure.
    pub fn generate_documented_template(&self) -> String {
        let mut s = String::new();
        s.push_str("# GGTerm configuration file\n");
        s.push_str("# Edit settings below, then save. Changes apply on reload.\n");
        s.push_str("# Full documentation: https://github.com/topcheer/ggterm\n\n");

        // Appearance
        s.push_str("[appearance]\n");
        s.push_str(&format!("theme = \"{}\"  # dark, light, dracula, solarized-dark, gruvbox, nord, tokyo-night, catppuccin-mocha\n", self.appearance.theme));
        s.push_str(&format!(
            "font_family = \"{}\"\n",
            self.appearance.font_family
        ));
        s.push_str(&format!(
            "font_size = {}  # 6-32\n",
            self.appearance.font_size
        ));
        s.push_str(&format!(
            "background_opacity = {:.1}  # 0.0 (transparent) to 1.0 (opaque)\n",
            self.appearance.background_opacity
        ));
        s.push_str(&format!(
            "padding = {}  # pixels around terminal content\n",
            self.appearance.padding
        ));
        s.push_str(&format!(
            "cursor_blink = {}\n",
            self.appearance.cursor_blink
        ));
        s.push_str(&format!(
            "cursor_line_highlight = {}\n\n",
            self.appearance.cursor_line_highlight
        ));

        // Terminal
        s.push_str("[terminal]\n");
        s.push_str(&format!(
            "scrollback_lines = {}  # max history lines kept in memory\n",
            self.terminal.scrollback_lines
        ));
        s.push_str(&format!(
            "shell = \"{}\"  # empty = use $SHELL\n",
            self.terminal.shell
        ));
        s.push_str(&format!(
            "bell_mode = \"{}\"  # none, visual, sound, both\n",
            self.terminal.bell_mode
        ));
        s.push_str(&format!(
            "copy_on_select = {}\n",
            self.terminal.copy_on_select
        ));
        s.push_str(&format!(
            "word_chars = \"{}\"  # chars treated as part of a word for double-click\n",
            self.terminal.word_chars
        ));
        s.push_str(&format!(
            "notify_on_complete = {}  # desktop notification when long commands finish\n\n",
            self.terminal.notify_on_complete
        ));

        // Keybindings
        s.push_str("[keybindings]\n");
        s.push_str("# All keybindings are optional. Remove lines to use defaults.\n");
        let km = &self.keybindings;
        if let Some(ref v) = km.new_tab {
            s.push_str(&format!("new_tab = \"{}\"\n", v));
        }
        if let Some(ref v) = km.close_tab {
            s.push_str(&format!("close_tab = \"{}\"\n", v));
        }
        if let Some(ref v) = km.paste {
            s.push_str(&format!("paste = \"{}\"\n", v));
        }
        if let Some(ref v) = km.copy {
            s.push_str(&format!("copy = \"{}\"\n", v));
        }
        if let Some(ref v) = km.search {
            s.push_str(&format!("search = \"{}\"\n", v));
        }
        if let Some(ref v) = km.zoom_in {
            s.push_str(&format!("zoom_in = \"{}\"\n", v));
        }
        if let Some(ref v) = km.zoom_out {
            s.push_str(&format!("zoom_out = \"{}\"\n", v));
        }
        if let Some(ref v) = km.zoom_reset {
            s.push_str(&format!("zoom_reset = \"{}\"\n", v));
        }
        if let Some(ref v) = km.fullscreen {
            s.push_str(&format!("fullscreen = \"{}\"\n", v));
        }
        if let Some(ref v) = km.clear {
            s.push_str(&format!("clear = \"{}\"\n", v));
        }
        if let Some(ref v) = km.reset {
            s.push_str(&format!("reset = \"{}\"\n", v));
        }
        if let Some(ref v) = km.cycle_theme {
            s.push_str(&format!("cycle_theme = \"{}\"\n", v));
        }

        s.push_str("\n# Custom colors (optional — overrides theme palette)\n");
        s.push_str("# [appearance.colors]\n");
        s.push_str("# ansi = [\"#000000\", \"#cc0000\", \"#4e9a06\", \"#c4a000\", \"#3465a4\", \"#75507b\", \"#06989a\", \"#d3d7cf\", \"#555753\", \"#ef2929\", \"#8ae234\", \"#fce94f\", \"#729fcf\", \"#ad7fa8\", \"#34e2e2\", \"#eeeeec\"]\n");
        s.push_str("# foreground = \"#c0caf5\"\n");
        s.push_str("# background = \"#1a1b26\"\n");
        s.push_str("# cursor = \"#c0caf5\"\n");
        s.push_str("# selection = \"#33467c\"\n");

        s
    }

    /// Import configuration from a TOML string.
    ///
    /// This is the inverse of [`export_to_toml`]. Any fields missing from the
    /// TOML string are filled with defaults.
    ///
    /// [`export_to_toml`]: Config::export_to_toml
    pub fn import_from_toml(s: &str) -> Result<Self, ConfigError> {
        Self::from_toml_str(s)
    }

    /// Reset to default configuration.
    pub fn reset_to_defaults() -> Self {
        Self::default()
    }

    // ── Keybinding parsing ─────────────────────────────────────────────

    /// Parse a keybinding string like `"Ctrl+Shift+V"` into modifier flags
    /// and a key name.
    ///
    /// Returns `(ctrl, shift, alt, key)` where the first three elements are
    /// `bool` flags and `key` is the final component (e.g. `"V"`, `"F11"`, `"="`).
    ///
    /// Returns `None` if the string is empty, contains only modifiers with no
    /// key, or has an unrecognized modifier.
    pub fn parse_keybinding(s: &str) -> Option<(bool, bool, bool, &str)> {
        parse_keybinding(s)
    }

    /// Validate all config fields and return the first error encountered, if any.
    ///
    /// Checked ranges:
    /// - `font_size`: 6–32 (inclusive)
    /// - `cell_width` / `cell_height`: 4–32 (inclusive)
    /// - `scrollback_lines`: 100–100_000 (inclusive)
    /// - `theme`: must be a known built-in theme name
    pub fn validate(&self) -> Result<(), ConfigError> {
        let ap = &self.appearance;

        if !(6..=32).contains(&ap.font_size) {
            return Err(ConfigError::Validation(format!(
                "font_size {} is out of range (allowed 6–32)",
                ap.font_size
            )));
        }

        if !(4..=32).contains(&ap.cell_width) {
            return Err(ConfigError::Validation(format!(
                "cell_width {} is out of range (allowed 4–32)",
                ap.cell_width
            )));
        }

        if !(4..=32).contains(&ap.cell_height) {
            return Err(ConfigError::Validation(format!(
                "cell_height {} is out of range (allowed 4–32)",
                ap.cell_height
            )));
        }

        let valid_themes = [
            "dark",
            "light",
            "dracula",
            "solarized-dark",
            "solarized_light",
            "solarized-light",
            "solarized_dark",
            "gruvbox",
            "nord",
            "tokyo-night",
            "tokyo_night",
            "catppuccin-mocha",
            "catppuccin_mocha",
        ];
        if !valid_themes.contains(&ap.theme.as_str()) {
            return Err(ConfigError::Validation(format!(
                "theme '{}' is not a known built-in theme (allowed: dark, light, dracula, solarized-dark, solarized-light, gruvbox, nord, tokyo-night, catppuccin-mocha)",
                ap.theme
            )));
        }

        // Validate background_opacity range.
        if !(0.0..=1.0).contains(&ap.background_opacity) {
            return Err(ConfigError::Validation(format!(
                "background_opacity {} is out of range (allowed 0.0–1.0)",
                ap.background_opacity
            )));
        }

        let sb = self.terminal.scrollback_lines;
        if !(100..=100_000).contains(&sb) {
            return Err(ConfigError::Validation(format!(
                "scrollback_lines {} is out of range (allowed 100–100000)",
                sb
            )));
        }

        // Check for keybinding conflicts — two actions bound to the same
        // key combination silently causes one of them to be dead.
        self.validate_keybinding_conflicts()?;

        Ok(())
    }

    /// Detect keybinding conflicts where two or more actions share
    /// the same key combination.
    ///
    /// Only conflicts within user-configured bindings are checked.
    /// Default bindings are assumed conflict-free.
    fn validate_keybinding_conflicts(&self) -> Result<(), ConfigError> {
        let kb = &self.keybindings;

        // Collect (action_name, parsed_binding) pairs for all configured bindings.
        // Store normalized form as (ctrl, shift, alt, key_lowercase).
        type KeyCombo = (bool, bool, bool, String);
        let mut bindings: Vec<(&str, KeyCombo)> = Vec::new();

        let entries: [(&str, Option<&String>); 13] = [
            ("new_tab", kb.new_tab.as_ref()),
            ("close_tab", kb.close_tab.as_ref()),
            ("switch_tab_1", kb.switch_tab_1.as_ref()),
            ("paste", kb.paste.as_ref()),
            ("copy", kb.copy.as_ref()),
            ("search", kb.search.as_ref()),
            ("zoom_in", kb.zoom_in.as_ref()),
            ("zoom_out", kb.zoom_out.as_ref()),
            ("zoom_reset", kb.zoom_reset.as_ref()),
            ("fullscreen", kb.fullscreen.as_ref()),
            ("clear", kb.clear.as_ref()),
            ("reset", kb.reset.as_ref()),
            ("cycle_theme", kb.cycle_theme.as_ref()),
        ];

        for (action, binding_opt) in &entries {
            if let Some(s) = binding_opt
                && let Some((c, sh, a, k)) = parse_keybinding(s)
            {
                bindings.push((action, (c, sh, a, k.to_lowercase())));
            }
        }

        // Check for duplicates using a HashMap.
        let mut seen: std::collections::HashMap<KeyCombo, &str> = std::collections::HashMap::new();
        for (action, combo) in &bindings {
            if let Some(prev_action) = seen.get(combo) {
                // Format the key combo for the error message.
                let mut label = String::new();
                if combo.0 {
                    label.push_str("Ctrl+");
                }
                if combo.1 {
                    label.push_str("Shift+");
                }
                if combo.2 {
                    label.push_str("Alt+");
                }
                label.push_str(&combo.3.to_uppercase());

                return Err(ConfigError::Validation(format!(
                    "keybinding conflict: '{}' and '{}' are both bound to {}",
                    prev_action, action, label
                )));
            }
            seen.insert(combo.clone(), action);
        }

        Ok(())
    }

    // ── Profile management (P22-C) ──────────────────────────────────────

    /// Returns the sorted list of profile names defined in config.toml.
    pub fn profile_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.profiles.keys().cloned().collect();
        names.sort();
        names
    }

    /// Returns `true` if at least one profile is defined.
    pub fn has_profiles(&self) -> bool {
        !self.profiles.is_empty()
    }

    /// Apply a named profile's overrides to this config in-place.
    ///
    /// Only fields that are `Some` in the profile are applied; `None` fields
    /// retain their current value.  Returns `Err` if the profile name is not
    /// found.
    pub fn apply_profile(&mut self, name: &str) -> Result<(), ConfigError> {
        let profile = self
            .profiles
            .get(name)
            .ok_or_else(|| ConfigError::Validation(format!("profile '{}' not found", name)))?;

        if let Some(fs) = profile.font_size {
            self.appearance.font_size = fs;
        }
        if let Some(ref theme) = profile.theme {
            self.appearance.theme = theme.clone();
        }
        if let Some(sb) = profile.scrollback_lines {
            self.terminal.scrollback_lines = sb;
        }

        Ok(())
    }

    /// Cycle to the next profile (alphabetical order) and apply it.
    ///
    /// Returns the name of the newly-active profile, or `None` if no profiles
    /// are defined.
    ///
    /// `current` is the currently active profile name (empty string = none/base).
    pub fn cycle_profile(&mut self, current: &str) -> Option<String> {
        let names = self.profile_names();
        if names.is_empty() {
            return None;
        }

        let next = if current.is_empty() {
            // No active profile → first one.
            names[0].clone()
        } else {
            // Find current index, advance to next (wraps around).
            let idx = names.iter().position(|n| n == current);
            match idx {
                Some(i) => names[(i + 1) % names.len()].clone(),
                None => names[0].clone(),
            }
        };

        // apply_profile borrows &mut self, so we clone the name first.
        let _ = self.apply_profile(&next);
        Some(next)
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
    /// If GGTERM_CONFIG env var is set, loads from that path instead
    /// (used by `ggterm --config <path>`).
    pub fn load_default() -> Result<Self, ConfigError> {
        // Check for CLI override via env var.
        if let Ok(custom_path) = std::env::var("GGTERM_CONFIG")
            && !custom_path.is_empty()
        {
            let path = PathBuf::from(&custom_path);
            if path.exists() {
                log::info!("Loading config from GGTERM_CONFIG: {custom_path}");
                return Self::load_from(&path);
            } else {
                log::warn!("GGTERM_CONFIG path does not exist: {custom_path}, using default");
            }
        }
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

    /// Get a mutable reference to the config (for profile application).
    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
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
        new_config.validate()?;
        let changed = new_config.appearance.theme != self.config.appearance.theme
            || new_config.appearance.font_size != self.config.appearance.font_size
            || new_config.appearance.cell_width != self.config.appearance.cell_width
            || new_config.appearance.cell_height != self.config.appearance.cell_height
            || new_config.appearance.cursor_style != self.config.appearance.cursor_style
            || new_config.terminal.scrollback_lines != self.config.terminal.scrollback_lines
            || new_config.terminal.shell != self.config.terminal.shell
            || new_config.ai.enabled != self.config.ai.enabled
            || new_config.keybindings.new_tab != self.config.keybindings.new_tab
            || new_config.keybindings.paste != self.config.keybindings.paste
            || new_config.keybindings.search != self.config.keybindings.search
            || new_config.keybindings.fullscreen != self.config.keybindings.fullscreen;

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

    /// Save the current config to disk.
    ///
    /// If no path was previously set, saves to the default location
    /// (`~/.ggterm/config.toml`).
    pub fn save(&mut self) -> Result<(), ConfigError> {
        let path = match &self.config_path {
            Some(p) => p.clone(),
            None => match default_config_path() {
                Some(p) => p,
                None => {
                    return Err(ConfigError::Io(
                        "config path".into(),
                        std::io::Error::new(std::io::ErrorKind::NotFound, "No home directory"),
                    ));
                }
            },
        };
        let toml = self.config.export_to_toml()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ConfigError::Io(parent.display().to_string(), e))?;
        }
        std::fs::write(&path, &toml).map_err(|e| ConfigError::Io(path.display().to_string(), e))?;
        self.config_path = Some(path);
        log::info!("Config saved");
        Ok(())
    }

    // ── File system watching (config-watch feature) ─────────────────────

    /// Start watching the config file for changes.
    ///
    /// When the file is modified, a flag is set.  Call [`poll_reload`]
    /// from your event loop to perform the actual reload.
    ///
    /// Requires the `config-watch` feature.
    #[cfg(feature = "config-watch")]
    /// Start watching the config file for changes.
    ///
    /// Requires the `config-watch` feature.
    #[cfg(feature = "config-watch")]
    pub fn watch(&mut self) -> Result<(), ConfigError> {
        use notify::{RecommendedWatcher, RecursiveMode, Watcher};

        let path = match &self.config_path {
            Some(p) => p.clone(),
            None => {
                return Err(ConfigError::Watch("no config path loaded".to_string()));
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
        #[cfg(feature = "config-watch")]
        {
            if self.reload_pending.swap(false, Ordering::SeqCst) {
                return self.reload();
            }
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

// ─── Keybinding parsing ───────────────────────────────────────────────────

/// Parse a keybinding string like `"Ctrl+Shift+V"` into modifier flags
/// and a key name.
///
/// Returns `(ctrl, shift, alt, key)` where the first three elements are
/// `bool` flags and `key` is the final component (e.g. `"V"`, `"F11"`, `"="`).
///
/// # Examples
/// ```
/// # use ggterm_app::config::parse_keybinding;
/// assert_eq!(parse_keybinding("Ctrl+Shift+V"), Some((true, true, false, "V")));
/// assert_eq!(parse_keybinding("Alt+F4"), Some((false, false, true, "F4")));
/// assert_eq!(parse_keybinding("F11"), Some((false, false, false, "F11")));
/// ```
///
/// Returns `None` if the string is empty, contains only modifiers with no
/// key, or has an unrecognized modifier.
pub fn parse_keybinding(s: &str) -> Option<(bool, bool, bool, &str)> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;

    // Split on '+' and treat all parts except the last as modifiers.
    let parts: Vec<&str> = trimmed.split('+').collect();

    // If there is only one part, it must be the key (no modifiers).
    if parts.len() == 1 {
        let key = parts[0].trim();
        if key.is_empty() {
            return None;
        }
        // Reject bare modifier names (e.g. "Ctrl", "Shift") — they need a key.
        let lower = key.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "ctrl"
                | "control"
                | "shift"
                | "alt"
                | "opt"
                | "option"
                | "super"
                | "cmd"
                | "meta"
                | "win"
        ) {
            return None;
        }
        return Some((false, false, false, key));
    }

    // Process modifier parts (all but the last).
    for &part in &parts[..parts.len() - 1] {
        match part.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" => ctrl = true,
            "shift" => shift = true,
            "alt" | "opt" | "option" => alt = true,
            // "super" / "cmd" are accepted but not tracked separately.
            "super" | "cmd" | "meta" | "win" => {}
            _ => return None,
        }
    }

    let key = parts.last().unwrap_or(&"").trim();
    if key.is_empty() {
        return None;
    }
    // Reject if the "key" is actually a bare modifier name.
    let lower_key = key.to_ascii_lowercase();
    if matches!(
        lower_key.as_str(),
        "ctrl" | "control" | "shift" | "alt" | "opt" | "option" | "super" | "cmd" | "meta" | "win"
    ) {
        return None;
    }

    // Reject modifier names used as the final key (e.g. "Ctrl+Shift" → key "Shift").
    let lower = key.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "ctrl" | "control" | "shift" | "alt" | "opt" | "option" | "super" | "cmd" | "meta" | "win"
    ) {
        return None;
    }

    Some((ctrl, shift, alt, key))
}

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
    /// Configuration validation error (field, message).
    #[error("config validation error: {0}")]
    Validation(String),
    /// TOML serialization error during export.
    #[error("config export error: {0}")]
    Export(toml::ser::Error),
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
    fn test_background_opacity_default() {
        let config = Config::default();
        assert_eq!(config.appearance.background_opacity, 1.0);
    }

    #[test]
    fn test_background_opacity_parse() {
        let toml = r#"
[appearance]
background_opacity = 0.75
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert!((config.appearance.background_opacity - 0.75).abs() < 0.001);
    }

    #[test]
    fn test_background_opacity_clamped() {
        let toml = r#"
[appearance]
background_opacity = -0.5
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert!((config.appearance.background_opacity - 0.0).abs() < 0.001);

        let toml = r#"
[appearance]
background_opacity = 2.0
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert!((config.appearance.background_opacity - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_background_opacity_export() {
        let mut config = Config::default();
        config.appearance.background_opacity = 0.85;
        let toml_str = config.export_to_toml().unwrap();
        assert!(toml_str.contains("background_opacity"));
        // Round-trip
        let config2 = Config::from_toml_str(&toml_str).unwrap();
        assert!((config2.appearance.background_opacity - 0.85).abs() < 0.001);
    }

    #[test]
    fn test_padding_default() {
        let config = Config::default();
        assert_eq!(config.appearance.padding, 8);
    }

    #[test]
    fn test_padding_parse() {
        let toml = r#"
[appearance]
padding = 16
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert_eq!(config.appearance.padding, 16);
    }

    #[test]
    fn test_notify_on_complete_default() {
        let config = Config::default();
        assert!(config.terminal.notify_on_complete);
        assert_eq!(config.terminal.min_notify_duration_secs, 10);
    }

    #[test]
    fn test_notify_on_complete_parse() {
        let toml = r#"
[terminal]
notify_on_complete = false
min_notify_duration_secs = 30
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert!(!config.terminal.notify_on_complete);
        assert_eq!(config.terminal.min_notify_duration_secs, 30);
    }

    #[test]
    fn test_notify_on_complete_clamp() {
        let toml = r#"
[terminal]
min_notify_duration_secs = 0
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert_eq!(config.terminal.min_notify_duration_secs, 1);
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
        // Use a temp directory as HOME so the test doesn't depend on
        // the developer's actual ~/.ggterm/config.toml.
        let temp = std::env::temp_dir().join("ggterm_test_default_home");
        let _ = std::fs::create_dir_all(&temp);
        let old_home = std::env::var_os("HOME");
        // SAFETY: this is a single-threaded test; no other code reads HOME concurrently.
        unsafe {
            std::env::set_var("HOME", &temp);
        }
        let config = Config::load_default().unwrap_or_default();
        // Restore HOME.
        if let Some(h) = old_home {
            unsafe {
                std::env::set_var("HOME", h);
            }
        }
        // Should be valid defaults.
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
    fn test_parse_keybinding_ctrl_shift_v() {
        let (ctrl, shift, alt, key) = parse_keybinding("Ctrl+Shift+V").unwrap();
        assert!(ctrl);
        assert!(shift);
        assert!(!alt);
        assert_eq!(key, "V");
    }

    #[test]
    fn test_parse_keybinding_various_combos() {
        // Ctrl+T
        assert_eq!(parse_keybinding("Ctrl+T"), Some((true, false, false, "T")));
        // Alt+1
        assert_eq!(parse_keybinding("Alt+1"), Some((false, false, true, "1")));
        // F11 (no modifiers)
        assert_eq!(parse_keybinding("F11"), Some((false, false, false, "F11")));
        // Ctrl+Shift+Alt+A (all modifiers)
        assert_eq!(
            parse_keybinding("Ctrl+Shift+Alt+A"),
            Some((true, true, true, "A"))
        );
        // Ctrl+= (special char key)
        assert_eq!(parse_keybinding("Ctrl+="), Some((true, false, false, "=")));
        // Cmd+K (super modifier is accepted but not tracked)
        assert_eq!(parse_keybinding("Cmd+K"), Some((false, false, false, "K")));
        // Control is alias for Ctrl
        assert_eq!(
            parse_keybinding("Control+C"),
            Some((true, false, false, "C"))
        );
        // Whitespace is trimmed
        assert_eq!(
            parse_keybinding("  Ctrl + Shift + F  "),
            Some((true, true, false, "F"))
        );
    }

    #[test]
    fn test_parse_keybinding_empty_returns_none() {
        assert_eq!(parse_keybinding(""), None);
        assert_eq!(parse_keybinding("   "), None);
    }

    #[test]
    fn test_parse_keybinding_only_modifiers_returns_none() {
        // No key after modifiers
        assert_eq!(parse_keybinding("Ctrl"), None);
        assert_eq!(parse_keybinding("Ctrl+Shift"), None);
        assert_eq!(parse_keybinding("Ctrl+"), None);
    }

    #[test]
    fn test_parse_keybinding_unknown_modifier_returns_none() {
        assert_eq!(parse_keybinding("Foo+T"), None);
        assert_eq!(parse_keybinding("Ctrl+Bar+V"), None);
    }

    #[test]
    fn test_default_keybindings_all_none() {
        let config = Config::default();
        assert!(config.keybindings.new_tab.is_none());
        assert!(config.keybindings.paste.is_none());
        assert!(config.keybindings.search.is_none());
        assert!(config.keybindings.fullscreen.is_none());
        assert!(config.keybindings.zoom_in.is_none());
        assert!(config.keybindings.cycle_theme.is_none());
    }

    #[test]
    fn test_toml_with_keybindings() {
        let toml = r#"[keybindings]
new_tab = "Ctrl+T"
close_tab = "Ctrl+W"
paste = "Ctrl+Shift+V"
search = "Ctrl+Shift+F"
fullscreen = "F11"
cycle_theme = "Ctrl+Shift+Alt+T"
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert_eq!(config.keybindings.new_tab.as_deref(), Some("Ctrl+T"));
        assert_eq!(config.keybindings.close_tab.as_deref(), Some("Ctrl+W"));
        assert_eq!(config.keybindings.paste.as_deref(), Some("Ctrl+Shift+V"));
        assert_eq!(config.keybindings.search.as_deref(), Some("Ctrl+Shift+F"));
        assert_eq!(config.keybindings.fullscreen.as_deref(), Some("F11"));
        assert_eq!(
            config.keybindings.cycle_theme.as_deref(),
            Some("Ctrl+Shift+Alt+T")
        );
        // Unspecified fields stay None
        assert!(config.keybindings.copy.is_none());
        assert!(config.keybindings.zoom_in.is_none());
    }

    #[test]
    fn test_toml_without_keybindings_uses_default() {
        let toml = r#"[appearance]
theme = "light"
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert!(config.keybindings.new_tab.is_none());
        assert!(config.keybindings.paste.is_none());
        // Other config still works
        assert_eq!(config.appearance.theme, "light");
    }

    #[test]
    fn test_keybindings_partial_config() {
        let toml = r#"[keybindings]
new_tab = "Ctrl+N"
paste = "Ctrl+V"
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert_eq!(config.keybindings.new_tab.as_deref(), Some("Ctrl+N"));
        assert_eq!(config.keybindings.paste.as_deref(), Some("Ctrl+V"));
        assert!(config.keybindings.search.is_none());
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

    #[allow(dead_code)]
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
        assert!(
            changed,
            "poll_reload should report a change after file modification"
        );
        assert_eq!(mgr.config().appearance.theme, "light");
        assert!(
            called.load(Ordering::SeqCst),
            "on_change callback should have been fired"
        );

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

    // ── P21-C: Validation tests ──────────────────────────────────────────

    #[test]
    fn test_validate_default_ok() {
        let config = Config::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_font_size_too_small() {
        let mut config = Config::default();
        config.appearance.font_size = 3;
        let err = config.validate().unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
        assert!(err.to_string().contains("font_size"));
    }

    #[test]
    fn test_validate_font_size_too_large() {
        let mut config = Config::default();
        config.appearance.font_size = 64;
        let err = config.validate().unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
        assert!(err.to_string().contains("font_size"));
    }

    #[test]
    fn test_validate_font_size_boundaries() {
        let mut config = Config::default();
        config.appearance.font_size = 6;
        assert!(config.validate().is_ok());
        config.appearance.font_size = 32;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_cell_width_out_of_range() {
        let mut config = Config::default();
        config.appearance.cell_width = 2;
        assert!(config.validate().is_err());

        config.appearance.cell_width = 48;
        assert!(config.validate().is_err());

        config.appearance.cell_width = 4;
        assert!(config.validate().is_ok());
        config.appearance.cell_width = 32;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_cell_height_out_of_range() {
        let mut config = Config::default();
        config.appearance.cell_height = 1;
        assert!(config.validate().is_err());

        config.appearance.cell_height = 100;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_scrollback_out_of_range() {
        let mut config = Config::default();
        config.terminal.scrollback_lines = 10;
        assert!(config.validate().is_err());

        config.terminal.scrollback_lines = 200_000;
        assert!(config.validate().is_err());

        config.terminal.scrollback_lines = 100;
        assert!(config.validate().is_ok());
        config.terminal.scrollback_lines = 100_000;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_unknown_theme() {
        let mut config = Config::default();
        config.appearance.theme = "nonexistent".to_string();
        let err = config.validate().unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
        assert!(err.to_string().contains("theme"));
    }

    #[test]
    fn test_validate_known_themes() {
        let mut config = Config::default();
        for theme in &[
            "dark",
            "light",
            "dracula",
            "solarized-dark",
            "solarized-light",
            "gruvbox",
            "nord",
            "tokyo-night",
            "catppuccin-mocha",
        ] {
            config.appearance.theme = theme.to_string();
            assert!(config.validate().is_ok(), "theme {} should be valid", theme);
        }
    }

    #[test]
    fn test_validate_solarized_underscore_variant() {
        let mut config = Config::default();
        config.appearance.theme = "solarized_dark".to_string();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_opacity_range() {
        let mut config = Config::default();
        config.appearance.background_opacity = 0.0;
        assert!(config.validate().is_ok(), "opacity 0.0 should be valid");
        config.appearance.background_opacity = 1.0;
        assert!(config.validate().is_ok(), "opacity 1.0 should be valid");
        config.appearance.background_opacity = 0.85;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_opacity_out_of_range() {
        let mut config = Config::default();
        config.appearance.background_opacity = 1.5;
        assert!(config.validate().is_err());
        config.appearance.background_opacity = -0.1;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validation_error_is_display() {
        let mut config = Config::default();
        config.appearance.font_size = 0;
        let err = config.validate().unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("font_size"));
        assert!(msg.contains("validation"));
    }

    // ── P22-C: Profile tests ─────────────────────────────────────────────

    #[test]
    fn test_profiles_parse_from_toml() {
        let toml = r#"
[profiles.presentation]
font_size = 22
theme = "light"

[profiles.compact]
font_size = 10
scrollback_lines = 50000
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert_eq!(config.profiles.len(), 2);

        let pres = config.profiles.get("presentation").unwrap();
        assert_eq!(pres.font_size, Some(22));
        assert_eq!(pres.theme.as_deref(), Some("light"));
        assert_eq!(pres.scrollback_lines, None);

        let compact = config.profiles.get("compact").unwrap();
        assert_eq!(compact.font_size, Some(10));
        assert_eq!(compact.theme, None);
        assert_eq!(compact.scrollback_lines, Some(50_000));
    }

    #[test]
    fn test_profiles_empty_by_default() {
        let config = Config::default();
        assert!(config.profiles.is_empty());
        assert!(!config.has_profiles());
    }

    #[test]
    fn test_profile_names_sorted() {
        let toml = r#"
[profiles.zebra]
font_size = 12

[profiles.alpha]
font_size = 10

[profiles.mid]
font_size = 14
"#;
        let config = Config::from_toml_str(toml).unwrap();
        let names = config.profile_names();
        assert_eq!(names, vec!["alpha", "mid", "zebra"]);
    }

    #[test]
    fn test_apply_profile_overrides() {
        let toml = r#"
[profiles.big]
font_size = 24
theme = "light"
scrollback_lines = 50000
"#;
        let mut config = Config::from_toml_str(toml).unwrap();
        // Defaults: font 14, dark, scrollback 10000.
        assert_eq!(config.appearance.font_size, 14);
        assert_eq!(config.appearance.theme, "dark");
        assert_eq!(config.terminal.scrollback_lines, 10_000);

        config.apply_profile("big").unwrap();
        assert_eq!(config.appearance.font_size, 24);
        assert_eq!(config.appearance.theme, "light");
        assert_eq!(config.terminal.scrollback_lines, 50_000);
    }

    #[test]
    fn test_apply_profile_partial_override() {
        let toml = r#"
[profiles.partial]
font_size = 20
"#;
        let mut config = Config::from_toml_str(toml).unwrap();
        // Theme and scrollback should NOT change.
        let original_theme = config.appearance.theme.clone();
        let original_sb = config.terminal.scrollback_lines;

        config.apply_profile("partial").unwrap();
        assert_eq!(config.appearance.font_size, 20);
        assert_eq!(config.appearance.theme, original_theme);
        assert_eq!(config.terminal.scrollback_lines, original_sb);
    }

    #[test]
    fn test_apply_nonexistent_profile_errors() {
        let mut config = Config::default();
        let err = config.apply_profile("nonexistent").unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn test_cycle_profile_from_none() {
        let toml = r#"
[profiles.alpha]
font_size = 10

[profiles.beta]
font_size = 20
"#;
        let mut config = Config::from_toml_str(toml).unwrap();
        let next = config.cycle_profile("");
        assert_eq!(next.as_deref(), Some("alpha"));
        assert_eq!(config.appearance.font_size, 10);
    }

    #[test]
    fn test_cycle_profile_wraps_around() {
        let toml = r#"
[profiles.alpha]
font_size = 10

[profiles.beta]
font_size = 20
"#;
        let mut config = Config::from_toml_str(toml).unwrap();

        let p1 = config.cycle_profile("").unwrap();
        assert_eq!(p1, "alpha");

        let p2 = config.cycle_profile(&p1).unwrap();
        assert_eq!(p2, "beta");
        assert_eq!(config.appearance.font_size, 20);

        // Wraps back to alpha.
        let p3 = config.cycle_profile(&p2).unwrap();
        assert_eq!(p3, "alpha");
        assert_eq!(config.appearance.font_size, 10);
    }

    #[test]
    fn test_cycle_profile_no_profiles() {
        let mut config = Config::default();
        assert!(config.cycle_profile("").is_none());
    }

    #[test]
    fn test_cycle_profile_unknown_current_starts_first() {
        let toml = r#"
[profiles.alpha]
font_size = 10
"#;
        let mut config = Config::from_toml_str(toml).unwrap();
        let next = config.cycle_profile("nonexistent");
        assert_eq!(next.as_deref(), Some("alpha"));
    }

    #[test]
    fn test_profiles_with_other_config_sections() {
        let toml = r#"
[appearance]
font_size = 16
theme = "dracula"

[terminal]
scrollback_lines = 8000

[profiles.override_me]
font_size = 12
theme = "light"
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert_eq!(config.appearance.font_size, 16);
        assert_eq!(config.appearance.theme, "dracula");
        assert_eq!(config.terminal.scrollback_lines, 8_000);
        assert!(config.has_profiles());
        assert_eq!(config.profile_names(), vec!["override_me"]);
    }

    // ── P23-D: Plugin config tests ──────────────────────────────────────

    #[test]
    fn test_plugin_config_default() {
        let config = Config::default();
        assert!(!config.plugins.enabled);
        assert_eq!(config.plugins.directory, "~/.ggterm/plugins");
    }

    #[test]
    fn test_plugin_config_parse_enabled() {
        let toml = r#"
[plugins]
enabled = true
directory = "/custom/plugins"
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert!(config.plugins.enabled);
        assert_eq!(config.plugins.directory, "/custom/plugins");
    }

    #[test]
    fn test_plugin_config_parse_disabled() {
        let toml = r#"
[plugins]
enabled = false
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert!(!config.plugins.enabled);
        // Default directory since not specified.
        assert_eq!(config.plugins.directory, "~/.ggterm/plugins");
    }

    #[test]
    fn test_plugin_config_absent_uses_defaults() {
        let toml = "[appearance]\ntheme = \"dark\"\n";
        let config = Config::from_toml_str(toml).unwrap();
        assert!(!config.plugins.enabled);
        assert_eq!(config.plugins.directory, "~/.ggterm/plugins");
    }

    #[test]
    fn test_plugin_config_partial_fields() {
        let toml = r#"
[plugins]
directory = "/opt/ggterm/lua"
"#;
        let config = Config::from_toml_str(toml).unwrap();
        // enabled not specified → default (false).
        assert!(!config.plugins.enabled);
        assert_eq!(config.plugins.directory, "/opt/ggterm/lua");
    }

    // ── P23-B: Export / Import / Reset tests ──────────────────────────

    #[test]
    fn test_export_to_toml_default() {
        let config = Config::default();
        let toml_str = config.export_to_toml().unwrap();
        // Should contain all sections.
        assert!(toml_str.contains("[appearance]"));
        assert!(toml_str.contains("[terminal]"));
        assert!(toml_str.contains("[ai]"));
        assert!(toml_str.contains("[plugins]"));
        // Default config has no keybindings overrides — section omitted.
        assert!(!toml_str.contains("[keybindings]"));
    }

    #[test]
    fn test_export_import_roundtrip() {
        let mut config = Config::default();
        config.appearance.theme = "gruvbox".to_string();
        config.appearance.font_size = 18;
        config.terminal.scrollback_lines = 50000;
        config.terminal.shell = "/bin/fish".to_string();
        config.ai.enabled = true;
        config.ai.model = "gpt-4o".to_string();
        config.plugins.enabled = true;
        config.plugins.directory = "/custom/plugins".to_string();

        let exported = config.export_to_toml().unwrap();
        let imported = Config::import_from_toml(&exported).unwrap();

        assert_eq!(imported.appearance.theme, "gruvbox");
        assert_eq!(imported.appearance.font_size, 18);
        assert_eq!(imported.terminal.scrollback_lines, 50000);
        assert_eq!(imported.terminal.shell, "/bin/fish");
        assert!(imported.ai.enabled);
        assert_eq!(imported.ai.model, "gpt-4o");
        assert!(imported.plugins.enabled);
        assert_eq!(imported.plugins.directory, "/custom/plugins");
    }

    #[test]
    fn test_export_import_with_keybindings() {
        let mut config = Config::default();
        config.keybindings.new_tab = Some("Ctrl+T".to_string());
        config.keybindings.search = Some("Ctrl+Shift+F".to_string());

        let exported = config.export_to_toml().unwrap();
        assert!(exported.contains("new_tab = \"Ctrl+T\""));
        assert!(exported.contains("search = \"Ctrl+Shift+F\""));

        let imported = Config::import_from_toml(&exported).unwrap();
        assert_eq!(imported.keybindings.new_tab, Some("Ctrl+T".to_string()));
        assert_eq!(
            imported.keybindings.search,
            Some("Ctrl+Shift+F".to_string())
        );
    }

    #[test]
    fn test_export_import_with_profiles() {
        let mut config = Config::default();
        config.profiles.insert(
            "presentation".to_string(),
            Profile {
                font_size: Some(24),
                theme: Some("light".to_string()),
                scrollback_lines: None,
            },
        );

        let exported = config.export_to_toml().unwrap();
        assert!(exported.contains("[profiles.presentation]"));

        let imported = Config::import_from_toml(&exported).unwrap();
        let profile = imported.profiles.get("presentation").unwrap();
        assert_eq!(profile.font_size, Some(24));
        assert_eq!(profile.theme, Some("light".to_string()));
    }

    #[test]
    fn test_reset_to_defaults() {
        let mut config = Config::default();
        config.appearance.font_size = 99;
        config.terminal.shell = "/bin/custom".to_string();
        assert_ne!(
            config.appearance.font_size,
            Config::default().appearance.font_size
        );

        let reset = Config::reset_to_defaults();
        assert_eq!(
            reset.appearance.font_size,
            Config::default().appearance.font_size
        );
        assert_eq!(reset.terminal.shell, Config::default().terminal.shell);
    }

    #[test]
    fn test_import_invalid_toml() {
        let result = Config::import_from_toml("not valid toml {{{");
        assert!(result.is_err());
    }

    #[test]
    fn test_save_and_reload() {
        let dir = std::env::temp_dir();
        let path = dir.join("ggterm_test_save.toml");
        let _ = std::fs::remove_file(&path);

        let mut mgr = ConfigManager::new();
        mgr.config_mut().terminal.scrollback_lines = 5000;
        mgr.config_path = Some(path.clone());
        mgr.save().unwrap();
        assert!(path.exists());

        // Reload from disk.
        let mut mgr2 = ConfigManager::new();
        mgr2.config_path = Some(path.clone());
        mgr2.reload().unwrap();
        assert_eq!(mgr2.config().terminal.scrollback_lines, 5000);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_cursor_style_config() {
        // Valid cursor styles
        let toml = r#"
[appearance]
cursor_style = "underline"
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert_eq!(config.appearance.cursor_style, "underline");

        let toml_bar = r#"
[appearance]
cursor_style = "bar"
"#;
        let config_bar = Config::from_toml_str(toml_bar).unwrap();
        assert_eq!(config_bar.appearance.cursor_style, "bar");

        // Invalid cursor style falls back to default (block)
        let toml_bad = r#"
[appearance]
cursor_style = "fancy"
"#;
        let config_bad = Config::from_toml_str(toml_bad).unwrap();
        assert_eq!(config_bad.appearance.cursor_style, "block");
    }

    #[test]
    fn test_cursor_blink_parse() {
        let toml = r#"
[appearance]
cursor_blink = false
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert!(!config.appearance.cursor_blink);

        // Default is true
        assert!(Config::default().appearance.cursor_blink);
    }

    #[test]
    fn test_documented_template_is_valid() {
        let config = Config::default();
        let template = config.generate_documented_template();
        // Template should have comments.
        assert!(template.starts_with("# GGTerm"));
        // Template should be valid TOML when comments are stripped.
        // (strip lines starting with # but keep inline values)
        let clean: String = template
            .lines()
            .map(|l| {
                if l.trim_start().starts_with('#') {
                    ""
                } else {
                    // Remove inline comments
                    if let Some(pos) = l.find("  #") {
                        &l[..pos]
                    } else {
                        l
                    }
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let parsed = Config::from_toml_str(&clean);
        assert!(
            parsed.is_ok(),
            "documented template should parse as valid config"
        );
    }

    #[test]
    fn test_cursor_line_highlight_parse() {
        let toml = r#"
[appearance]
cursor_line_highlight = true
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert!(
            config.appearance.cursor_line_highlight,
            "should parse cursor_line_highlight = true"
        );

        // Default is false
        assert!(
            !Config::default().appearance.cursor_line_highlight,
            "cursor_line_highlight should default to false"
        );
    }

    #[test]
    fn test_copy_on_select_parse() {
        let toml = r#"
[terminal]
copy_on_select = false
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert!(!config.terminal.copy_on_select);

        // Default is true
        assert!(Config::default().terminal.copy_on_select);
    }

    #[test]
    fn test_shell_integration_parse() {
        let toml = r#"
[terminal]
shell_integration = false
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert!(!config.terminal.shell_integration);

        // Default is true
        assert!(Config::default().terminal.shell_integration);
    }

    #[test]
    fn test_word_chars_parse() {
        let toml = r#"
[terminal]
word_chars = "-._/"
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert_eq!(config.terminal.word_chars, "-._/");

        // Default includes path/URL chars
        let default = Config::default();
        assert!(default.terminal.word_chars.contains('/'));
        assert!(default.terminal.word_chars.contains('.'));
        assert!(default.terminal.word_chars.contains('@'));
    }

    #[test]
    fn test_export_includes_new_fields() {
        let config = Config::default();
        let toml = config.export_to_toml().unwrap();
        assert!(toml.contains("cursor_blink"));
        assert!(toml.contains("copy_on_select"));
        assert!(toml.contains("word_chars"));
    }

    #[test]
    fn test_custom_color_palette_parse() {
        let toml = "[appearance.colors]\nansi = [\"#000000\", \"#cc0404\", \"#19cb00\", \"#cecb00\", \"#0d73cc\", \"#cb1ed1\", \"#0dcdcd\", \"#dddddd\", \"#767676\", \"#f2201f\", \"#23fd00\", \"#fffd00\", \"#1a8fff\", \"#fd28ff\", \"#14ffff\", \"#ffffff\"]\nforeground = \"#c0caf5\"\nbackground = \"#1a1b26\"\ncursor = \"#c0caf5\"\nselection = \"#33467c\"\n";
        let config = Config::from_toml_str(toml).unwrap();
        assert!(config.appearance.custom_palette.is_some());
        let palette = config.appearance.custom_palette.as_ref().unwrap();
        assert_eq!(palette.len(), 16);
        assert_eq!(palette[0], "#000000");
        assert_eq!(palette[15], "#ffffff");
        assert_eq!(config.appearance.custom_fg.as_deref(), Some("#c0caf5"));
        assert_eq!(config.appearance.custom_bg.as_deref(), Some("#1a1b26"));
        assert_eq!(config.appearance.custom_cursor.as_deref(), Some("#c0caf5"));
        assert_eq!(
            config.appearance.custom_selection.as_deref(),
            Some("#33467c")
        );
    }

    #[test]
    fn test_custom_color_palette_wrong_count_ignored() {
        let toml = "[appearance.colors]\nansi = [\"#000000\", \"#ff0000\"]\n";
        let config = Config::from_toml_str(toml).unwrap();
        // Only 2 colors (not 16) → palette should be None.
        assert!(config.appearance.custom_palette.is_none());
    }

    #[test]
    fn test_custom_color_palette_partial() {
        let toml = "[appearance.colors]\nforeground = \"#ffffff\"\nbackground = \"#000000\"\n";
        let config = Config::from_toml_str(toml).unwrap();
        assert!(config.appearance.custom_palette.is_none());
        assert_eq!(config.appearance.custom_fg.as_deref(), Some("#ffffff"));
        assert_eq!(config.appearance.custom_bg.as_deref(), Some("#000000"));
        assert!(config.appearance.custom_cursor.is_none());
    }

    #[test]
    fn test_keybinding_conflict_detected() {
        let toml = r#"
[keybindings]
new_tab = "Ctrl+N"
close_tab = "Ctrl+N"
"#;
        let config = Config::from_toml_str(toml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
        assert!(err.to_string().contains("keybinding conflict"));
        assert!(err.to_string().contains("new_tab"));
        assert!(err.to_string().contains("close_tab"));
        assert!(err.to_string().contains("Ctrl+N"));
    }

    #[test]
    fn test_keybinding_no_conflict() {
        // Different keys with same modifier — no conflict.
        let toml = r#"
[keybindings]
new_tab = "Ctrl+T"
close_tab = "Ctrl+W"
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_keybinding_conflict_case_insensitive() {
        // "Ctrl+A" and "Ctrl+a" should be treated as the same binding.
        let toml = r#"
[keybindings]
copy = "Ctrl+A"
paste = "Ctrl+a"
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_keybinding_no_conflict_different_modifiers() {
        // Same key, different modifiers — no conflict.
        let toml = r#"
[keybindings]
copy = "Ctrl+C"
paste = "Ctrl+Shift+C"
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert!(config.validate().is_ok());
    }
}
