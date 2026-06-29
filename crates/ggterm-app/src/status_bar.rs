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

    /// Format the status bar as a single-line string.
    ///
    /// Example: `"Row:5 Col:10 | Tab 1/3 | bell | search | ai"`
    pub fn format(&self) -> String {
        let mut parts: Vec<String> = Vec::with_capacity(6);

        // Cursor position (always shown).
        parts.push(format!("Row:{} Col:{}", self.cursor_row, self.cursor_col));

        // Tab info (only show "Tab x/y" when more than 1 tab).
        if self.tab_count > 1 {
            parts.push(format!("Tab {}/{}", self.active_tab + 1, self.tab_count));
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
}
