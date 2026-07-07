//! P29-A: Keyboard shortcut reference overlay.
//!
//! A searchable, categorized list of all keyboard shortcuts.
//! Toggle with Ctrl+Shift+/ (or Ctrl+?).

/// A single keyboard shortcut entry.
#[derive(Debug, Clone)]
pub struct ShortcutEntry {
    /// The key combination (e.g., "Ctrl+T").
    pub keys: String,
    /// Human-readable description.
    pub description: String,
    /// Category for grouping.
    pub category: ShortcutCategory,
    /// Whether this shortcut is configurable.
    pub configurable: bool,
}

/// Shortcut category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShortcutCategory {
    Tab,
    Split,
    Edit,
    View,
    Search,
    Ai,
    Shell,
    Terminal,
    System,
    Effects,
}

impl ShortcutCategory {
    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Tab => "Tabs",
            Self::Split => "Splits",
            Self::Edit => "Editing",
            Self::View => "View",
            Self::Search => "Search",
            Self::Ai => "AI",
            Self::Shell => "Shell",
            Self::Terminal => "Terminal",
            Self::System => "System",
            Self::Effects => "Effects",
        }
    }

    /// Accent color (R, G, B) for the category badge.
    pub fn color(self) -> (u8, u8, u8) {
        match self {
            Self::Tab => (100, 180, 255),
            Self::Split => (255, 150, 100),
            Self::Edit => (100, 255, 150),
            Self::View => (200, 150, 255),
            Self::Search => (255, 200, 100),
            Self::Ai => (100, 255, 255),
            Self::Shell => (255, 100, 200),
            Self::Terminal => (150, 200, 255),
            Self::System => (200, 200, 200),
            Self::Effects => (255, 180, 80),
        }
    }
}

/// State for the shortcut help overlay.
#[derive(Debug)]
pub struct ShortcutHelpState {
    /// Whether the overlay is visible.
    pub visible: bool,
    /// Search query (empty = show all).
    pub query: String,
    /// Scroll offset.
    pub scroll: usize,
    /// All shortcuts.
    shortcuts: Vec<ShortcutEntry>,
}

impl Default for ShortcutHelpState {
    fn default() -> Self {
        Self {
            visible: false,
            query: String::new(),
            scroll: 0,
            shortcuts: all_shortcuts(),
        }
    }
}

impl ShortcutHelpState {
    /// Create new shortcut help state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Toggle visibility.
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        if self.visible {
            self.query.clear();
            self.scroll = 0;
        }
    }

    /// Close the overlay.
    pub fn close(&mut self) {
        self.visible = false;
    }

    /// Type a character into the search query.
    pub fn type_char(&mut self, c: char) {
        if c.is_ascii_graphic() || c == ' ' {
            self.query.push(c);
            self.scroll = 0;
        }
    }

    /// Backspace in the search query.
    pub fn backspace(&mut self) {
        self.query.pop();
    }

    /// Scroll up.
    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(3);
    }

    /// Scroll down.
    pub fn scroll_down(&mut self) {
        let max_scroll = self.filtered().len().saturating_sub(15);
        self.scroll = (self.scroll + 3).min(max_scroll);
    }

    /// Get filtered shortcuts matching the current query.
    pub fn filtered(&self) -> Vec<&ShortcutEntry> {
        if self.query.is_empty() {
            self.shortcuts.iter().collect()
        } else {
            let q = self.query.to_lowercase();
            self.shortcuts
                .iter()
                .filter(|s| {
                    s.keys.to_lowercase().contains(&q)
                        || s.description.to_lowercase().contains(&q)
                        || s.category.label().to_lowercase().contains(&q)
                })
                .collect()
        }
    }

    /// Number of total shortcuts.
    pub fn len(&self) -> usize {
        self.shortcuts.len()
    }

    /// Whether there are any shortcuts at all.
    pub fn is_empty(&self) -> bool {
        self.shortcuts.is_empty()
    }

    /// Whether any shortcuts match the current filter.
    pub fn has_results(&self) -> bool {
        !self.filtered().is_empty()
    }
}

/// Get all known keyboard shortcuts.
fn all_shortcuts() -> Vec<ShortcutEntry> {
    vec![
        // Tabs
        ShortcutEntry {
            keys: "Ctrl+T".into(),
            description: "Open new tab".into(),
            category: ShortcutCategory::Tab,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+W".into(),
            description: "Close current tab".into(),
            category: ShortcutCategory::Tab,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Alt+1-9".into(),
            description: "Switch to tab N".into(),
            category: ShortcutCategory::Tab,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Tab".into(),
            description: "Next tab".into(),
            category: ShortcutCategory::Tab,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Tab".into(),
            description: "Previous tab".into(),
            category: ShortcutCategory::Tab,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+`".into(),
            description: "Toggle last tab".into(),
            category: ShortcutCategory::Tab,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+T".into(),
            description: "Reopen last closed tab".into(),
            category: ShortcutCategory::Tab,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+N".into(),
            description: "Open new window".into(),
            category: ShortcutCategory::Tab,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Command Palette".into(),
            description: "Pin/Unpin tab (prevents accidental close)".into(),
            category: ShortcutCategory::Tab,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+I".into(),
            description: "Rename current tab".into(),
            category: ShortcutCategory::Tab,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+PageUp/Down".into(),
            description: "Move tab left/right".into(),
            category: ShortcutCategory::Tab,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+D".into(),
            description: "Duplicate current tab".into(),
            category: ShortcutCategory::Tab,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+W".into(),
            description: "Close all other tabs".into(),
            category: ShortcutCategory::Tab,
            configurable: false,
        },
        // Splits
        ShortcutEntry {
            keys: "Ctrl+Shift+D".into(),
            description: "Split horizontal".into(),
            category: ShortcutCategory::Split,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+\\".into(),
            description: "Split vertical".into(),
            category: ShortcutCategory::Split,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+[/]".into(),
            description: "Focus prev/next pane".into(),
            category: ShortcutCategory::Split,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+Arrows".into(),
            description: "Adjust split ratio".into(),
            category: ShortcutCategory::Split,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Z".into(),
            description: "Toggle pane zoom (maximize/restore)".into(),
            category: ShortcutCategory::Split,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+B".into(),
            description: "Balance split panes (even spacing)".into(),
            category: ShortcutCategory::Split,
            configurable: false,
        },
        // View
        ShortcutEntry {
            keys: "Ctrl+Shift+U".into(),
            description: "Open URL at cursor".into(),
            category: ShortcutCategory::View,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+O".into(),
            description: "Open config file in editor".into(),
            category: ShortcutCategory::View,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Space".into(),
            description: "Toggle scrollback browse mode (j/k/G/g/d/u/q)".into(),
            category: ShortcutCategory::View,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+X".into(),
            description: "Swap active pane with next".into(),
            category: ShortcutCategory::View,
            configurable: false,
        },
        // Edit
        ShortcutEntry {
            keys: "Ctrl+Shift+V".into(),
            description: "Paste from clipboard".into(),
            category: ShortcutCategory::Edit,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+C".into(),
            description: "Copy selection".into(),
            category: ShortcutCategory::Edit,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+O".into(),
            description: "Copy last command output".into(),
            category: ShortcutCategory::Edit,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+L".into(),
            description: "Toggle terminal lock (read-only)".into(),
            category: ShortcutCategory::Terminal,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+P".into(),
            description: "Copy current directory path".into(),
            category: ShortcutCategory::Edit,
            configurable: true,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+A".into(),
            description: "Select all text".into(),
            category: ShortcutCategory::Edit,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+S".into(),
            description: "Save scrollback to text file".into(),
            category: ShortcutCategory::Edit,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+E".into(),
            description: "Export terminal as HTML (with colors)".into(),
            category: ShortcutCategory::Edit,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+H".into(),
            description: "Import SSH hosts from ~/.ssh/config".into(),
            category: ShortcutCategory::Edit,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Alt+H/J/K/L".into(),
            description: "Vim-style pane navigation".into(),
            category: ShortcutCategory::Terminal,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Insert".into(),
            description: "Copy selection (Linux/Windows)".into(),
            category: ShortcutCategory::Edit,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Shift+Insert".into(),
            description: "Paste from clipboard (Linux/Windows)".into(),
            category: ShortcutCategory::Edit,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Shift+Arrows".into(),
            description: "Extend selection".into(),
            category: ShortcutCategory::Edit,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Left/Right".into(),
            description: "Extend selection by word".into(),
            category: ShortcutCategory::Edit,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Alt+Drag".into(),
            description: "Block (rectangular) selection".into(),
            category: ShortcutCategory::Edit,
            configurable: false,
        },
        // View
        ShortcutEntry {
            keys: "Ctrl+=".into(),
            description: "Zoom in (font size+)".into(),
            category: ShortcutCategory::View,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+-".into(),
            description: "Zoom out (font size-)".into(),
            category: ShortcutCategory::View,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+0".into(),
            description: "Reset font size".into(),
            category: ShortcutCategory::View,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+]".into(),
            description: "Increase background opacity (+5%)".into(),
            category: ShortcutCategory::View,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+[".into(),
            description: "Decrease background opacity (-5%)".into(),
            category: ShortcutCategory::View,
            configurable: false,
        },
        ShortcutEntry {
            keys: "F11".into(),
            description: "Toggle fullscreen".into(),
            category: ShortcutCategory::View,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+A".into(),
            description: "Toggle always-on-top".into(),
            category: ShortcutCategory::View,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Enter".into(),
            description: "Toggle maximized".into(),
            category: ShortcutCategory::View,
            configurable: false,
        },
        // Mouse
        ShortcutEntry {
            keys: "Cmd+Click (Ctrl+Click)".into(),
            description: "Open URL under cursor".into(),
            category: ShortcutCategory::Edit,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Double-click".into(),
            description: "Select word".into(),
            category: ShortcutCategory::Edit,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Triple-click".into(),
            description: "Select line".into(),
            category: ShortcutCategory::Edit,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Middle-click".into(),
            description: "Paste from clipboard".into(),
            category: ShortcutCategory::Edit,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+T".into(),
            description: "Cycle theme".into(),
            category: ShortcutCategory::View,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+B".into(),
            description: "Toggle status bar".into(),
            category: ShortcutCategory::View,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+G".into(),
            description: "Toggle perf monitor".into(),
            category: ShortcutCategory::View,
            configurable: false,
        },
        // Search
        ShortcutEntry {
            keys: "Ctrl+Shift+F".into(),
            description: "Search scrollback".into(),
            category: ShortcutCategory::Search,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Tab (in search)".into(),
            description: "Toggle case sensitivity".into(),
            category: ShortcutCategory::Search,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Shift+Tab (in search)".into(),
            description: "Toggle regex mode".into(),
            category: ShortcutCategory::Search,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Up/Down (in search)".into(),
            description: "Search history navigation".into(),
            category: ShortcutCategory::Search,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+End".into(),
            description: "Scroll to bottom".into(),
            category: ShortcutCategory::Search,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+Up".into(),
            description: "Scroll to mark (OSC 1337)".into(),
            category: ShortcutCategory::Search,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Shift+PageUp".into(),
            description: "Scroll up one page".into(),
            category: ShortcutCategory::Search,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Shift+PageDown".into(),
            description: "Scroll down one page".into(),
            category: ShortcutCategory::Search,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Shift+Home".into(),
            description: "Scroll to top of scrollback".into(),
            category: ShortcutCategory::Search,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Shift+End".into(),
            description: "Scroll to bottom".into(),
            category: ShortcutCategory::Search,
            configurable: false,
        },
        // AI
        ShortcutEntry {
            keys: "Ctrl+Shift+E".into(),
            description: "AI: Explain output".into(),
            category: ShortcutCategory::Ai,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+S".into(),
            description: "AI: Suggest command".into(),
            category: ShortcutCategory::Ai,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+H".into(),
            description: "AI: Help".into(),
            category: ShortcutCategory::Ai,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+N".into(),
            description: "AI: Natural language to command".into(),
            category: ShortcutCategory::Ai,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Tab (in AI overlay)".into(),
            description: "Insert AI suggested command into terminal".into(),
            category: ShortcutCategory::Ai,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Enter (in AI overlay)".into(),
            description: "Execute AI suggested command immediately".into(),
            category: ShortcutCategory::Ai,
            configurable: false,
        },
        // Shell
        ShortcutEntry {
            keys: "Ctrl+Shift+L".into(),
            description: "Quick shell switcher".into(),
            category: ShortcutCategory::Shell,
            configurable: false,
        },
        // Terminal
        ShortcutEntry {
            keys: "Ctrl+Shift+K".into(),
            description: "Clear screen + scrollback".into(),
            category: ShortcutCategory::Terminal,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+R".into(),
            description: "Reset terminal (RIS)".into(),
            category: ShortcutCategory::Terminal,
            configurable: false,
        },
        // System
        ShortcutEntry {
            keys: "Ctrl+Shift+P".into(),
            description: "Command palette".into(),
            category: ShortcutCategory::System,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+M".into(),
            description: "Toggle sound".into(),
            category: ShortcutCategory::System,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+B".into(),
            description: "Cycle broadcast mode".into(),
            category: ShortcutCategory::System,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+W".into(),
            description: "Cycle workspace".into(),
            category: ShortcutCategory::System,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+/".into(),
            description: "This help screen".into(),
            category: ShortcutCategory::System,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+P".into(),
            description: "Cycle config profile".into(),
            category: ShortcutCategory::System,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+, (Cmd+,)".into(),
            description: "Open config file in editor".into(),
            category: ShortcutCategory::System,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+E".into(),
            description: "Export config to clipboard".into(),
            category: ShortcutCategory::System,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+I".into(),
            description: "Import config from clipboard".into(),
            category: ShortcutCategory::System,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+R".into(),
            description: "Reset config to defaults".into(),
            category: ShortcutCategory::System,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+L".into(),
            description: "Reload config from file".into(),
            category: ShortcutCategory::System,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+N".into(),
            description: "Reset layout to single pane".into(),
            category: ShortcutCategory::System,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+,".into(),
            description: "Open Settings panel".into(),
            category: ShortcutCategory::System,
            configurable: false,
        },
        // Effects / History
        ShortcutEntry {
            keys: "Ctrl+Shift+Y".into(),
            description: "Toggle command history".into(),
            category: ShortcutCategory::Effects,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+X".into(),
            description: "Swap active pane with next".into(),
            category: ShortcutCategory::Split,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+S".into(),
            description: "Export scrollback to file".into(),
            category: ShortcutCategory::Effects,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+End".into(),
            description: "Scroll to bottom".into(),
            category: ShortcutCategory::View,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Alt+Drag".into(),
            description: "Block/rectangular selection".into(),
            category: ShortcutCategory::Edit,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+P".into(),
            description: "Cycle profiles".into(),
            category: ShortcutCategory::View,
            configurable: false,
        },
        ShortcutEntry {
            keys: "Ctrl+Shift+Alt+E".into(),
            description: "Export config to clipboard (TOML)".into(),
            category: ShortcutCategory::View,
            configurable: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_default_hidden() {
        let state = ShortcutHelpState::new();
        assert!(!state.visible);
        assert!(state.query.is_empty());
    }

    #[test]
    fn t_toggle() {
        let mut state = ShortcutHelpState::new();
        assert!(!state.visible);
        state.toggle();
        assert!(state.visible);
        state.toggle();
        assert!(!state.visible);
    }

    #[test]
    fn t_close() {
        let mut state = ShortcutHelpState::new();
        state.visible = true;
        state.close();
        assert!(!state.visible);
    }

    #[test]
    fn t_has_shortcuts() {
        let state = ShortcutHelpState::new();
        assert!(state.len() > 20, "should have many shortcuts");
    }

    #[test]
    fn t_all_categories_present() {
        let state = ShortcutHelpState::new();
        let entries = state.filtered();
        let categories: std::collections::HashSet<_> = entries.iter().map(|e| e.category).collect();
        assert!(categories.contains(&ShortcutCategory::Tab));
        assert!(categories.contains(&ShortcutCategory::Split));
        assert!(categories.contains(&ShortcutCategory::View));
        assert!(categories.contains(&ShortcutCategory::Ai));
    }

    #[test]
    fn t_filter_by_keys() {
        let mut state = ShortcutHelpState::new();
        state.query = "ctrl+t".to_string();
        let results = state.filtered();
        assert!(results.iter().any(|e| e.keys.contains("Ctrl+T")));
    }

    #[test]
    fn t_filter_by_description() {
        let mut state = ShortcutHelpState::new();
        state.query = "paste".to_string();
        let results = state.filtered();
        assert!(results.iter().any(|e| e.description.contains("Paste")));
    }

    #[test]
    fn t_filter_by_category() {
        let mut state = ShortcutHelpState::new();
        state.query = "split".to_string();
        let results = state.filtered();
        assert!(
            results
                .iter()
                .any(|e| e.category == ShortcutCategory::Split)
        );
    }

    #[test]
    fn t_filter_case_insensitive() {
        let mut state = ShortcutHelpState::new();
        state.query = "AI".to_string();
        let results = state.filtered();
        assert!(results.iter().any(|e| e.category == ShortcutCategory::Ai));
    }

    #[test]
    fn t_no_results() {
        let mut state = ShortcutHelpState::new();
        state.query = "zzzznonexistent".to_string();
        assert!(!state.has_results());
    }

    #[test]
    fn t_type_char() {
        let mut state = ShortcutHelpState::new();
        state.type_char('a');
        state.type_char('i');
        assert_eq!(state.query, "ai");
    }

    #[test]
    fn t_backspace() {
        let mut state = ShortcutHelpState::new();
        state.query = "test".to_string();
        state.backspace();
        assert_eq!(state.query, "tes");
    }

    #[test]
    fn t_scroll_up_clamps() {
        let mut state = ShortcutHelpState::new();
        state.scroll_up();
        assert_eq!(state.scroll, 0);
    }

    #[test]
    fn t_scroll_down_advances() {
        let mut state = ShortcutHelpState::new();
        state.scroll_down();
        assert_eq!(state.scroll, 3);
        state.scroll_down();
        assert_eq!(state.scroll, 6);
    }

    #[test]
    fn t_toggle_clears_query() {
        let mut state = ShortcutHelpState::new();
        state.query = "old query".to_string();
        state.visible = false;
        state.toggle();
        assert!(state.query.is_empty());
    }

    #[test]
    fn t_category_color() {
        let c = ShortcutCategory::Tab.color();
        // Color should not be all-zero (black) — each category has a distinct color.
        assert!(
            !(c.0 == 0 && c.1 == 0 && c.2 == 0),
            "color should be non-black"
        );
    }

    #[test]
    fn t_category_label() {
        assert_eq!(ShortcutCategory::Tab.label(), "Tabs");
        assert_eq!(ShortcutCategory::Split.label(), "Splits");
        assert_eq!(ShortcutCategory::Ai.label(), "AI");
    }
}
