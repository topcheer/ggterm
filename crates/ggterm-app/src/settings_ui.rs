//! Settings UI: overlay state machine for the settings panel.
//!
//! Provides [`SettingsState`] which manages the editable settings overlay.
//! Opened with `Ctrl+,`, closed with `Esc`. Changes to theme/font are applied
//! immediately; shell/scrollback changes are applied on save.

// ── SettingsField ───────────────────────────────────────────────────────

/// Which settings field is currently selected for editing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    Theme,
    FontSize,
    Scrollback,
    Shell,
    AiEnabled,
    AiEndpoint,
    AiModel,
}

impl SettingsField {
    /// Navigate to the next field (wraps around).
    pub fn next(self) -> Self {
        match self {
            Self::Theme => Self::FontSize,
            Self::FontSize => Self::Scrollback,
            Self::Scrollback => Self::Shell,
            Self::Shell => Self::AiEnabled,
            Self::AiEnabled => Self::AiEndpoint,
            Self::AiEndpoint => Self::AiModel,
            Self::AiModel => Self::Theme,
        }
    }

    /// Navigate to the previous field (wraps around).
    pub fn prev(self) -> Self {
        match self {
            Self::Theme => Self::AiModel,
            Self::FontSize => Self::Theme,
            Self::Scrollback => Self::FontSize,
            Self::Shell => Self::Scrollback,
            Self::AiEnabled => Self::Shell,
            Self::AiEndpoint => Self::AiEnabled,
            Self::AiModel => Self::AiEndpoint,
        }
    }
}

/// Available built-in themes for the settings picker.
pub const THEME_OPTIONS: &[&str] = &[
    "dark",
    "light",
    "dracula",
    "solarized-dark",
    "solarized-light",
    "gruvbox",
];

/// Snapshot of config values for loading into SettingsState.
#[derive(Debug, Clone)]
pub struct SettingsSnapshot {
    pub theme: String,
    pub font_size: u32,
    pub scrollback_lines: usize,
    pub shell: String,
    pub ai_enabled: bool,
    pub ai_endpoint: String,
    pub ai_model: String,
}

// ── SettingsState ───────────────────────────────────────────────────────

/// State machine for the settings overlay.
///
/// Lifecycle: `Hidden → Visible → Hidden`
/// While visible, the user can navigate fields with Up/Down,
/// adjust values with Left/Right or +/-, and save/cancel.
#[derive(Debug, Clone)]
pub struct SettingsState {
    /// Whether the settings overlay is visible.
    pub visible: bool,
    /// Currently selected field.
    pub selected: SettingsField,
    /// Draft theme (applied immediately on change).
    pub theme: String,
    /// Draft font size (applied immediately on change).
    pub font_size: u32,
    /// Draft scrollback lines (applied on save).
    pub scrollback_lines: usize,
    /// Draft shell path (applied on save).
    pub shell: String,
    /// Draft AI enabled flag.
    pub ai_enabled: bool,
    /// Draft AI endpoint.
    pub ai_endpoint: String,
    /// Draft AI model.
    pub ai_model: String,
    /// Whether there are unsaved changes requiring a restart.
    pub dirty: bool,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            visible: false,
            selected: SettingsField::Theme,
            theme: "dark".to_string(),
            font_size: 14,
            scrollback_lines: 10000,
            shell: String::new(),
            ai_enabled: false,
            ai_endpoint: "https://api.openai.com/v1".to_string(),
            ai_model: "gpt-4o-mini".to_string(),
            dirty: false,
        }
    }
}

impl SettingsState {
    /// Create a new hidden settings state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Show the settings overlay.
    pub fn open(&mut self) {
        self.visible = true;
        self.selected = SettingsField::Theme;
        self.dirty = false;
    }

    /// Hide the settings overlay without applying pending changes.
    pub fn close(&mut self) {
        self.visible = false;
    }

    /// Toggle visibility.
    pub fn toggle(&mut self) {
        if self.visible {
            self.close();
        } else {
            self.open();
        }
    }

    /// Navigate selection up (previous field).
    pub fn move_up(&mut self) {
        self.selected = self.selected.prev();
    }

    /// Navigate selection down (next field).
    pub fn move_down(&mut self) {
        self.selected = self.selected.next();
    }

    /// Cycle theme to the next option (applied immediately).
    pub fn cycle_theme(&mut self) {
        let current_idx = THEME_OPTIONS
            .iter()
            .position(|&t| t == self.theme)
            .unwrap_or(0);
        let next_idx = (current_idx + 1) % THEME_OPTIONS.len();
        self.theme = THEME_OPTIONS[next_idx].to_string();
        self.dirty = true;
    }

    /// Increase font size by 1 (applied immediately).
    pub fn font_size_up(&mut self) {
        if self.font_size < 48 {
            self.font_size += 1;
            self.dirty = true;
        }
    }

    /// Decrease font size by 1 (applied immediately).
    pub fn font_size_down(&mut self) {
        if self.font_size > 6 {
            self.font_size -= 1;
            self.dirty = true;
        }
    }

    /// Increase scrollback by 1000.
    pub fn scrollback_up(&mut self) {
        self.scrollback_lines += 1000;
        self.dirty = true;
    }

    /// Decrease scrollback by 1000 (min 100).
    pub fn scrollback_down(&mut self) {
        if self.scrollback_lines > 1000 {
            self.scrollback_lines -= 1000;
        } else {
            self.scrollback_lines = 100;
        }
        self.dirty = true;
    }

    /// Toggle AI enabled flag.
    pub fn toggle_ai(&mut self) {
        self.ai_enabled = !self.ai_enabled;
        self.dirty = true;
    }

    /// Load values from a Config snapshot.
    pub fn load_from_config(&mut self, cfg: &SettingsSnapshot) {
        self.theme = cfg.theme.clone();
        self.font_size = cfg.font_size;
        self.scrollback_lines = cfg.scrollback_lines;
        self.shell = cfg.shell.clone();
        self.ai_enabled = cfg.ai_enabled;
        self.ai_endpoint = cfg.ai_endpoint.clone();
        self.ai_model = cfg.ai_model.clone();
        self.dirty = false;
    }

    /// Format the settings overlay as a display string (for logging / title bar).
    pub fn format_summary(&self) -> String {
        format!(
            "Settings: theme={}, font={}, scrollback={}, ai={} | {} ({})",
            self.theme,
            self.font_size,
            self.scrollback_lines,
            if self.ai_enabled { "on" } else { "off" },
            self.ai_endpoint,
            self.ai_model,
        )
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_default_hidden() {
        let s = SettingsState::new();
        assert!(!s.visible);
    }

    #[test]
    fn t_open_close_toggle() {
        let mut s = SettingsState::new();
        assert!(!s.visible);

        s.open();
        assert!(s.visible);

        s.close();
        assert!(!s.visible);

        s.toggle();
        assert!(s.visible);

        s.toggle();
        assert!(!s.visible);
    }

    #[test]
    fn t_navigate_fields() {
        let mut s = SettingsState::new();
        s.open();
        assert_eq!(s.selected, SettingsField::Theme);

        s.move_down();
        assert_eq!(s.selected, SettingsField::FontSize);

        s.move_down();
        assert_eq!(s.selected, SettingsField::Scrollback);

        s.move_up();
        assert_eq!(s.selected, SettingsField::FontSize);
    }

    #[test]
    fn t_navigate_wraps_around() {
        let mut s = SettingsState::new();
        s.selected = SettingsField::AiModel;
        s.move_down();
        assert_eq!(s.selected, SettingsField::Theme);

        s.selected = SettingsField::Theme;
        s.move_up();
        assert_eq!(s.selected, SettingsField::AiModel);
    }

    #[test]
    fn t_cycle_theme() {
        let mut s = SettingsState::new();
        s.theme = "dark".to_string();
        s.cycle_theme();
        assert_eq!(s.theme, "light");

        // Cycle through all themes
        s.theme = "gruvbox".to_string();
        s.cycle_theme();
        assert_eq!(s.theme, "dark"); // wraps around
    }

    #[test]
    fn t_font_size_up_down() {
        let mut s = SettingsState::new();
        let initial = s.font_size;
        s.font_size_up();
        assert_eq!(s.font_size, initial + 1);
        assert!(s.dirty);

        s.font_size_down();
        assert_eq!(s.font_size, initial);

        // Test clamping
        s.font_size = 48;
        s.font_size_up();
        assert_eq!(s.font_size, 48); // max
    }

    #[test]
    fn t_font_size_down_clamp() {
        let mut s = SettingsState::new();
        s.font_size = 6;
        s.font_size_down();
        assert_eq!(s.font_size, 6); // min
    }

    #[test]
    fn t_scrollback_adjust() {
        let mut s = SettingsState::new();
        s.scrollback_lines = 10000;
        s.scrollback_up();
        assert_eq!(s.scrollback_lines, 11000);

        s.scrollback_down();
        assert_eq!(s.scrollback_lines, 10000);
    }

    #[test]
    fn t_scrollback_min_clamp() {
        let mut s = SettingsState::new();
        s.scrollback_lines = 500;
        s.scrollback_down();
        assert_eq!(s.scrollback_lines, 100);
    }

    #[test]
    fn t_toggle_ai() {
        let mut s = SettingsState::new();
        assert!(!s.ai_enabled);
        s.toggle_ai();
        assert!(s.ai_enabled);
        s.toggle_ai();
        assert!(!s.ai_enabled);
    }

    #[test]
    fn t_load_from_config() {
        let mut s = SettingsState::new();
        let cfg = SettingsSnapshot {
            theme: "dracula".to_string(),
            font_size: 16,
            scrollback_lines: 5000,
            shell: "/bin/fish".to_string(),
            ai_enabled: true,
            ai_endpoint: "http://api".to_string(),
            ai_model: "llama".to_string(),
        };
        s.load_from_config(&cfg);
        assert_eq!(s.theme, "dracula");
        assert_eq!(s.font_size, 16);
        assert_eq!(s.scrollback_lines, 5000);
        assert_eq!(s.shell, "/bin/fish");
        assert!(s.ai_enabled);
        assert_eq!(s.ai_endpoint, "http://api");
        assert_eq!(s.ai_model, "llama");
        assert!(!s.dirty);
    }

    #[test]
    fn t_format_summary() {
        let s = SettingsState::new();
        let summary = s.format_summary();
        assert!(summary.contains("theme=dark"));
        assert!(summary.contains("font=14"));
        assert!(summary.contains("ai=off"));
    }
}
