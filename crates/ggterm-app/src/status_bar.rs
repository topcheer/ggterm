//! Status bar: cursor position, tab count, and active mode indicators.
//!
//! Renders a single-line summary string suitable for display in the window
//! title or a dedicated status line.
//!
//! Example output: `"Row:5 Col:10 | Tab 1/3 | bell | search"`

// ── StatusBar ───────────────────────────────────────────────────────────

/// Aggregated terminal status for display in the window title or status line.
///
/// Updated every redraw from the active session's terminal state and the
/// DesktopApp's mode flags (bell, search, AI overlay).
pub struct StatusBar {
    /// Cursor row (0-based terminal row).
    pub cursor_row: usize,
    /// Cursor column (0-based terminal column).
    pub cursor_col: usize,
    /// Total number of open tabs.
    pub tab_count: usize,
    /// Index of the active tab (0-based).
    pub active_tab: usize,
    /// Whether the terminal bell was recently triggered.
    pub bell_active: bool,
    /// Whether the scrollback search bar is open.
    pub search_active: bool,
    /// Whether the AI overlay is visible.
    pub ai_active: bool,
    /// Last command exit code (None = no command completed yet).
    pub exit_code: Option<i32>,
    /// Configuration validation error message (P21-G).
    ///
    /// When set, a `!ERROR!` indicator is prepended to the status bar format.
    /// The renderer can use `has_config_error()` / `config_error_text()` to
    /// draw a red indicator.
    pub config_error: Option<String>,
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusBar {
    /// Create a new status bar with default (empty) state.
    pub fn new() -> Self {
        Self {
            cursor_row: 0,
            cursor_col: 0,
            tab_count: 1,
            active_tab: 0,
            bell_active: false,
            search_active: false,
            ai_active: false,
            exit_code: None,
            config_error: None,
        }
    }

    /// Update the cursor position.
    pub fn update_cursor(&mut self, row: usize, col: usize) {
        self.cursor_row = row;
        self.cursor_col = col;
    }

    /// Update tab information.
    pub fn update_tabs(&mut self, count: usize, active: usize) {
        self.tab_count = count;
        self.active_tab = active;
    }

    /// Set the bell indicator.
    pub fn set_bell(&mut self, active: bool) {
        self.bell_active = active;
    }

    /// Set the search indicator.
    pub fn set_search(&mut self, active: bool) {
        self.search_active = active;
    }

    /// Set the AI overlay indicator.
    pub fn set_ai(&mut self, active: bool) {
        self.ai_active = active;
    }

    /// Set the last command exit code (P17-E).
    pub fn set_exit_code(&mut self, code: Option<i32>) {
        self.exit_code = code;
    }

    /// Set a configuration validation error message (P21-G).
    ///
    /// Pass `None` or an empty string to clear.
    pub fn set_config_error(&mut self, msg: Option<String>) {
        match msg {
            Some(m) if !m.is_empty() => self.config_error = Some(m),
            _ => self.config_error = None,
        }
    }

    /// Clear the configuration error indicator (P21-G).
    pub fn clear_config_error(&mut self) {
        self.config_error = None;
    }

    /// Returns `true` if a config error is currently displayed (P21-G).
    pub fn has_config_error(&self) -> bool {
        self.config_error.is_some()
    }

    /// Returns the config error message for renderer use (P21-G).
    pub fn config_error_text(&self) -> Option<&str> {
        self.config_error.as_deref()
    }

    /// Format the status bar as a single-line string.
    ///
    /// Example: `"!ERROR! | Row:5 Col:10 | Tab 1/3 | exit:0 | bell | search | ai"`
    ///
    /// When a config error is set, `!ERROR!` is prepended so the renderer
    /// can highlight it in red.
    pub fn format(&self) -> String {
        let mut parts: Vec<String> = Vec::with_capacity(8);

        // Config error indicator — shown first for visibility (P21-G).
        if self.config_error.is_some() {
            parts.push("!ERROR!".to_string());
        }

        // Cursor position (always shown).
        parts.push(format!("Row:{} Col:{}", self.cursor_row, self.cursor_col));

        // Tab info (only show "Tab x/y" when more than 1 tab).
        if self.tab_count > 1 {
            parts.push(format!("Tab {}/{}", self.active_tab + 1, self.tab_count));
        }

        // Command exit code (P17-E).
        if let Some(code) = self.exit_code {
            if code == 0 {
                parts.push("exit:0".to_string());
            } else {
                parts.push(format!("exit:{}", code));
            }
        }

        // Mode indicators.
        if self.bell_active {
            parts.push("bell".to_string());
        }
        if self.search_active {
            parts.push("search".to_string());
        }
        if self.ai_active {
            parts.push("ai".to_string());
        }

        parts.join(" | ")
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_default_format() {
        let sb = StatusBar::new();
        let formatted = sb.format();
        // Default: cursor (0,0), single tab (not shown), no flags.
        assert_eq!(formatted, "Row:0 Col:0");
    }

    #[test]
    fn t_update_cursor() {
        let mut sb = StatusBar::new();
        sb.update_cursor(5, 10);
        assert_eq!(sb.format(), "Row:5 Col:10");
    }

    #[test]
    fn t_bell_search_ai_flags() {
        let mut sb = StatusBar::new();
        sb.update_cursor(3, 7);
        sb.set_bell(true);
        sb.set_search(true);
        sb.set_ai(true);
        assert_eq!(sb.format(), "Row:3 Col:7 | bell | search | ai");
    }

    #[test]
    fn t_multi_tab_display() {
        let mut sb = StatusBar::new();
        sb.update_cursor(0, 0);
        sb.update_tabs(3, 1); // 3 tabs, active = index 1 (Tab 2/3)
        assert_eq!(sb.format(), "Row:0 Col:0 | Tab 2/3");
    }

    #[test]
    fn t_all_flags_cleared() {
        let mut sb = StatusBar::new();
        sb.update_cursor(10, 20);
        sb.update_tabs(2, 0);
        sb.set_bell(true);
        sb.set_search(true);
        sb.set_ai(true);

        // Now clear everything.
        sb.set_bell(false);
        sb.set_search(false);
        sb.set_ai(false);

        // Should show cursor + tabs only.
        assert_eq!(sb.format(), "Row:10 Col:20 | Tab 1/2");
    }

    #[test]
    fn t_single_tab_not_shown() {
        let mut sb = StatusBar::new();
        sb.update_cursor(0, 0);
        sb.update_tabs(1, 0);
        // With a single tab, "Tab 1/1" should NOT appear.
        assert_eq!(sb.format(), "Row:0 Col:0");
    }

    #[test]
    fn t_bell_only() {
        let mut sb = StatusBar::new();
        sb.set_bell(true);
        assert_eq!(sb.format(), "Row:0 Col:0 | bell");
    }

    #[test]
    fn t_search_only() {
        let mut sb = StatusBar::new();
        sb.set_search(true);
        assert_eq!(sb.format(), "Row:0 Col:0 | search");
    }

    // ── P17-E: Exit code tests ──────────────────────────────────────

    #[test]
    fn t_exit_code_zero_shows_ok() {
        let mut sb = StatusBar::new();
        sb.set_exit_code(Some(0));
        assert_eq!(sb.format(), "Row:0 Col:0 | exit:0");
    }

    #[test]
    fn t_exit_code_nonzero_shows_code() {
        let mut sb = StatusBar::new();
        sb.set_exit_code(Some(127));
        assert_eq!(sb.format(), "Row:0 Col:0 | exit:127");
    }

    #[test]
    fn t_exit_code_none_not_shown() {
        let mut sb = StatusBar::new();
        sb.set_exit_code(None);
        assert_eq!(sb.format(), "Row:0 Col:0");
    }

    // ── P21-G: Config error indicator tests ──────────────────────────

    #[test]
    fn t_config_error_default_none() {
        let sb = StatusBar::new();
        assert!(!sb.has_config_error());
        assert!(sb.config_error_text().is_none());
    }

    #[test]
    fn t_config_error_shown_in_format() {
        let mut sb = StatusBar::new();
        sb.set_config_error(Some("font_size out of range".to_string()));
        assert!(sb.has_config_error());
        assert_eq!(sb.config_error_text(), Some("font_size out of range"));
        assert_eq!(sb.format(), "!ERROR! | Row:0 Col:0");
    }

    #[test]
    fn t_config_error_with_other_flags() {
        let mut sb = StatusBar::new();
        sb.update_cursor(5, 10);
        sb.update_tabs(2, 0);
        sb.set_bell(true);
        sb.set_config_error(Some("bad theme".to_string()));
        assert_eq!(sb.format(), "!ERROR! | Row:5 Col:10 | Tab 1/2 | bell");
    }

    #[test]
    fn t_config_error_cleared() {
        let mut sb = StatusBar::new();
        sb.set_config_error(Some("oops".to_string()));
        assert!(sb.has_config_error());

        sb.clear_config_error();
        assert!(!sb.has_config_error());
        assert_eq!(sb.format(), "Row:0 Col:0");
    }

    #[test]
    fn t_config_error_set_none_clears() {
        let mut sb = StatusBar::new();
        sb.set_config_error(Some("oops".to_string()));
        sb.set_config_error(None);
        assert!(!sb.has_config_error());
    }

    #[test]
    fn t_config_error_empty_string_clears() {
        let mut sb = StatusBar::new();
        sb.set_config_error(Some("oops".to_string()));
        sb.set_config_error(Some(String::new()));
        assert!(!sb.has_config_error());
    }

    #[test]
    fn t_config_error_precedes_cursor() {
        let mut sb = StatusBar::new();
        sb.update_cursor(3, 7);
        sb.set_config_error(Some("e".to_string()));
        let formatted = sb.format();
        // ERROR must appear before cursor position.
        let err_pos = formatted.find("!ERROR!").unwrap();
        let cursor_pos = formatted.find("Row:3").unwrap();
        assert!(err_pos < cursor_pos);
    }
}
