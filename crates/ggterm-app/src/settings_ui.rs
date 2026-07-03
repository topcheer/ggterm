//! Settings UI: overlay state machine for the settings panel.
//!
//! Provides [`SettingsState`] which manages the editable settings overlay.
//! Opened with `Ctrl+,`, closed with `Esc`. Changes to theme/font/cursor are
//! applied immediately; shell/scrollback/restore changes are applied on close.

// ── SettingsField ───────────────────────────────────────────────────────

/// Which settings field is currently selected for editing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    // Appearance section
    Theme,
    FontSize,
    CursorStyle,
    FontFamily,
    // Terminal section
    Scrollback,
    Shell,
    RestoreSession,
    // AI section
    AiEnabled,
    AiEndpoint,
    AiModel,
}

impl SettingsField {
    /// Navigate to the next field (wraps around).
    pub fn next(self) -> Self {
        match self {
            Self::Theme => Self::FontSize,
            Self::FontSize => Self::CursorStyle,
            Self::CursorStyle => Self::FontFamily,
            Self::FontFamily => Self::Scrollback,
            Self::Scrollback => Self::Shell,
            Self::Shell => Self::RestoreSession,
            Self::RestoreSession => Self::AiEnabled,
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
            Self::CursorStyle => Self::FontSize,
            Self::FontFamily => Self::CursorStyle,
            Self::Scrollback => Self::FontFamily,
            Self::Shell => Self::Scrollback,
            Self::RestoreSession => Self::Shell,
            Self::AiEnabled => Self::RestoreSession,
            Self::AiEndpoint => Self::AiEnabled,
            Self::AiModel => Self::AiEndpoint,
        }
    }

    /// Which section this field belongs to.
    pub fn section(self) -> SettingsSection {
        match self {
            Self::Theme | Self::FontSize | Self::CursorStyle | Self::FontFamily => {
                SettingsSection::Appearance
            }
            Self::Scrollback | Self::Shell | Self::RestoreSession => SettingsSection::Terminal,
            Self::AiEnabled | Self::AiEndpoint | Self::AiModel => SettingsSection::Ai,
        }
    }
}

/// Settings panel sections for visual grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    Appearance,
    Terminal,
    Ai,
}

impl SettingsSection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Terminal => "Terminal",
            Self::Ai => "AI",
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
    "nord",
    "tokyo-night",
    "catppuccin-mocha",
];

/// Available cursor styles.
pub const CURSOR_STYLE_OPTIONS: &[&str] = &["block", "underline", "bar"];

/// Snapshot of config values for loading into SettingsState.
#[derive(Debug, Clone)]
pub struct SettingsSnapshot {
    pub theme: String,
    pub font_size: u32,
    pub font_family: String,
    pub cursor_style: String,
    pub scrollback_lines: usize,
    pub shell: String,
    pub restore_session: bool,
    pub ai_enabled: bool,
    pub ai_endpoint: String,
    pub ai_model: String,
}

// ── SettingsState ───────────────────────────────────────────────────────

/// State machine for the settings overlay.
///
/// Lifecycle: `Hidden -> Visible -> Hidden`
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
    /// Draft font family.
    pub font_family: String,
    /// Draft cursor style.
    pub cursor_style: String,
    /// Draft scrollback lines (applied on save).
    pub scrollback_lines: usize,
    /// Draft shell path (applied on save).
    pub shell: String,
    /// Draft restore session flag.
    pub restore_session: bool,
    /// Draft AI enabled flag.
    pub ai_enabled: bool,
    /// Draft AI endpoint.
    pub ai_endpoint: String,
    /// Draft AI model.
    pub ai_model: String,
    /// Whether there are unsaved changes.
    pub dirty: bool,
    /// Error message to display in the settings overlay.
    pub error_message: Option<String>,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            visible: false,
            selected: SettingsField::Theme,
            theme: "dark".to_string(),
            font_size: 14,
            font_family: "monospace".to_string(),
            cursor_style: "block".to_string(),
            scrollback_lines: 10000,
            shell: String::new(),
            restore_session: false,
            ai_enabled: false,
            ai_endpoint: "https://api.openai.com/v1".to_string(),
            ai_model: "gpt-4o-mini".to_string(),
            dirty: false,
            error_message: None,
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
        self.error_message = None;
    }

    /// Hide the settings overlay.
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

    /// Cycle theme backward.
    pub fn cycle_theme_prev(&mut self) {
        let current_idx = THEME_OPTIONS
            .iter()
            .position(|&t| t == self.theme)
            .unwrap_or(0);
        let prev_idx = if current_idx == 0 {
            THEME_OPTIONS.len() - 1
        } else {
            current_idx - 1
        };
        self.theme = THEME_OPTIONS[prev_idx].to_string();
        self.dirty = true;
    }

    /// Cycle cursor style to the next option.
    pub fn cycle_cursor_style(&mut self) {
        let current_idx = CURSOR_STYLE_OPTIONS
            .iter()
            .position(|&t| t == self.cursor_style)
            .unwrap_or(0);
        let next_idx = (current_idx + 1) % CURSOR_STYLE_OPTIONS.len();
        self.cursor_style = CURSOR_STYLE_OPTIONS[next_idx].to_string();
        self.dirty = true;
    }

    /// Cycle cursor style backward.
    pub fn cycle_cursor_style_prev(&mut self) {
        let current_idx = CURSOR_STYLE_OPTIONS
            .iter()
            .position(|&t| t == self.cursor_style)
            .unwrap_or(0);
        let prev_idx = if current_idx == 0 {
            CURSOR_STYLE_OPTIONS.len() - 1
        } else {
            current_idx - 1
        };
        self.cursor_style = CURSOR_STYLE_OPTIONS[prev_idx].to_string();
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

    /// Toggle restore session flag.
    pub fn toggle_restore_session(&mut self) {
        self.restore_session = !self.restore_session;
        self.dirty = true;
    }

    /// Load values from a Config snapshot.
    pub fn load_from_config(&mut self, cfg: &SettingsSnapshot) {
        self.theme = cfg.theme.clone();
        self.font_size = cfg.font_size;
        self.font_family = cfg.font_family.clone();
        self.cursor_style = cfg.cursor_style.clone();
        self.scrollback_lines = cfg.scrollback_lines;
        self.shell = cfg.shell.clone();
        self.restore_session = cfg.restore_session;
        self.ai_enabled = cfg.ai_enabled;
        self.ai_endpoint = cfg.ai_endpoint.clone();
        self.ai_model = cfg.ai_model.clone();
        self.dirty = false;
    }

    /// Set an error message to display in the overlay.
    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.error_message = Some(msg.into());
    }

    /// Clear any displayed error message.
    pub fn clear_error(&mut self) {
        self.error_message = None;
    }

    /// Returns the error message if one is present, for overlay rendering.
    pub fn error_text(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    /// Format the settings overlay as a display string (for logging).
    pub fn format_summary(&self) -> String {
        format!(
            "Settings: theme={}, font={}, cursor={}, scrollback={}, restore={}, ai={} | {} ({})",
            self.theme,
            self.font_size,
            self.cursor_style,
            self.scrollback_lines,
            if self.restore_session { "on" } else { "off" },
            if self.ai_enabled { "on" } else { "off" },
            self.ai_endpoint,
            self.ai_model,
        )
    }

    /// Returns all field labels and values for rendering, grouped by section.
    /// Each item is (section, label, value).
    pub fn field_rows(&self) -> Vec<(SettingsSection, &'static str, String)> {
        vec![
            (SettingsSection::Appearance, "Theme", self.theme.clone()),
            (
                SettingsSection::Appearance,
                "Font Size",
                format!("{}px", self.font_size),
            ),
            (
                SettingsSection::Appearance,
                "Cursor Style",
                self.cursor_style.clone(),
            ),
            (
                SettingsSection::Appearance,
                "Font Family",
                self.font_family.clone(),
            ),
            (
                SettingsSection::Terminal,
                "Scrollback",
                format!("{} lines", self.scrollback_lines),
            ),
            (
                SettingsSection::Terminal,
                "Shell",
                if self.shell.is_empty() {
                    "(default)".to_string()
                } else {
                    self.shell.clone()
                },
            ),
            (
                SettingsSection::Terminal,
                "Restore Session",
                if self.restore_session { "on" } else { "off" }.to_string(),
            ),
            (
                SettingsSection::Ai,
                "AI Enabled",
                if self.ai_enabled { "on" } else { "off" }.to_string(),
            ),
            (SettingsSection::Ai, "AI Endpoint", self.ai_endpoint.clone()),
            (SettingsSection::Ai, "AI Model", self.ai_model.clone()),
        ]
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
    fn t_toggle() {
        let mut s = SettingsState::new();
        s.toggle();
        assert!(s.visible);
        s.toggle();
        assert!(!s.visible);
    }

    #[test]
    fn t_navigation_wraps() {
        let mut f = SettingsField::Theme;
        for _ in 0..20 {
            f = f.next();
        }
        // After 20 next() from Theme (10 fields), should be back at Theme
        assert_eq!(f, SettingsField::Theme);
    }

    #[test]
    fn t_prev_next_inverse() {
        for f in [
            SettingsField::Theme,
            SettingsField::FontSize,
            SettingsField::CursorStyle,
            SettingsField::FontFamily,
            SettingsField::Scrollback,
            SettingsField::Shell,
            SettingsField::RestoreSession,
            SettingsField::AiEnabled,
            SettingsField::AiEndpoint,
            SettingsField::AiModel,
        ] {
            assert_eq!(f.next().prev(), f);
        }
    }

    #[test]
    fn t_theme_cycle() {
        let mut s = SettingsState::new();
        s.theme = "dark".to_string();
        s.cycle_theme();
        assert_eq!(s.theme, "light");
        s.cycle_theme_prev();
        assert_eq!(s.theme, "dark");
    }

    #[test]
    fn t_cursor_style_cycle() {
        let mut s = SettingsState::new();
        assert_eq!(s.cursor_style, "block");
        s.cycle_cursor_style();
        assert_eq!(s.cursor_style, "underline");
        s.cycle_cursor_style();
        assert_eq!(s.cursor_style, "bar");
        s.cycle_cursor_style();
        assert_eq!(s.cursor_style, "block");
    }

    #[test]
    fn t_cursor_style_prev() {
        let mut s = SettingsState::new();
        s.cursor_style = "block".to_string();
        s.cycle_cursor_style_prev();
        assert_eq!(s.cursor_style, "bar");
    }

    #[test]
    fn t_font_size_bounds() {
        let mut s = SettingsState::new();
        s.font_size = 48;
        s.font_size_up();
        assert_eq!(s.font_size, 48); // capped
        s.font_size = 6;
        s.font_size_down();
        assert_eq!(s.font_size, 6); // capped
    }

    #[test]
    fn t_scrollback_bounds() {
        let mut s = SettingsState::new();
        s.scrollback_lines = 500;
        s.scrollback_down();
        assert_eq!(s.scrollback_lines, 100); // min 100
    }

    #[test]
    fn t_toggle_restore_session() {
        let mut s = SettingsState::new();
        assert!(!s.restore_session);
        s.toggle_restore_session();
        assert!(s.restore_session);
        assert!(s.dirty);
    }

    #[test]
    fn t_section_grouping() {
        assert_eq!(SettingsField::Theme.section(), SettingsSection::Appearance);
        assert_eq!(
            SettingsField::Scrollback.section(),
            SettingsSection::Terminal
        );
        assert_eq!(SettingsField::AiModel.section(), SettingsSection::Ai);
    }

    #[test]
    fn t_field_rows_count() {
        let s = SettingsState::new();
        let rows = s.field_rows();
        assert_eq!(rows.len(), 10); // 4 + 3 + 3 fields
    }

    #[test]
    fn t_load_from_config() {
        let mut s = SettingsState::new();
        let snap = SettingsSnapshot {
            theme: "nord".to_string(),
            font_size: 20,
            font_family: "Fira Code".to_string(),
            cursor_style: "bar".to_string(),
            scrollback_lines: 5000,
            shell: "/bin/fish".to_string(),
            restore_session: true,
            ai_enabled: true,
            ai_endpoint: "http://localhost:8080".to_string(),
            ai_model: "llama3".to_string(),
        };
        s.load_from_config(&snap);
        assert_eq!(s.theme, "nord");
        assert_eq!(s.font_size, 20);
        assert_eq!(s.cursor_style, "bar");
        assert_eq!(s.font_family, "Fira Code");
        assert!(s.restore_session);
        assert!(!s.dirty); // loading clears dirty
    }
}
