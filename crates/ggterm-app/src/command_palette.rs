//! # Command Palette
//!
//! VS Code-style command palette for fuzzy-searching and executing actions.
//! P25-B.

/// A command that can be executed from the palette.
#[derive(Debug, Clone)]
pub struct Command {
    /// Unique identifier (e.g. "tab.new").
    pub id: String,
    /// Display name (e.g. "New Tab").
    pub label: String,
    /// Category for grouping (e.g. "Tab", "Split", "Theme").
    pub category: String,
    /// Optional keyboard shortcut hint.
    pub shortcut: Option<String>,
}

/// Registry of all available commands.
#[derive(Debug, Clone, Default)]
pub struct CommandRegistry {
    commands: Vec<Command>,
}

impl CommandRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a command.
    pub fn register(&mut self, cmd: Command) {
        if !self.commands.iter().any(|c| c.id == cmd.id) {
            self.commands.push(cmd);
        }
    }

    /// Get all registered commands.
    pub fn all(&self) -> &[Command] {
        &self.commands
    }

    /// Create a registry with all default GGTerm commands.
    pub fn defaults() -> Self {
        let mut r = Self::new();
        // Tab commands
        r.register(Command {
            id: "tab.new".into(),
            label: "New Tab".into(),
            category: "Tab".into(),
            shortcut: Some("Ctrl+T".into()),
        });
        r.register(Command {
            id: "tab.close".into(),
            label: "Close Tab".into(),
            category: "Tab".into(),
            shortcut: Some("Ctrl+W".into()),
        });
        r.register(Command {
            id: "tab.next".into(),
            label: "Next Tab".into(),
            category: "Tab".into(),
            shortcut: Some("Ctrl+Tab".into()),
        });
        r.register(Command {
            id: "tab.prev".into(),
            label: "Previous Tab".into(),
            category: "Tab".into(),
            shortcut: Some("Ctrl+Shift+Tab".into()),
        });
        // Split commands
        r.register(Command {
            id: "split.horizontal".into(),
            label: "Split Horizontal".into(),
            category: "Split".into(),
            shortcut: Some("Ctrl+Shift+D".into()),
        });
        r.register(Command {
            id: "split.vertical".into(),
            label: "Split Vertical".into(),
            category: "Split".into(),
            shortcut: Some("Ctrl+Shift+\\".into()),
        });
        r.register(Command {
            id: "split.focus_next".into(),
            label: "Focus Next Pane".into(),
            category: "Split".into(),
            shortcut: Some("Ctrl+Shift+]".into()),
        });
        r.register(Command {
            id: "split.focus_prev".into(),
            label: "Focus Previous Pane".into(),
            category: "Split".into(),
            shortcut: Some("Ctrl+Shift+[".into()),
        });
        // Theme
        r.register(Command {
            id: "theme.cycle".into(),
            label: "Cycle Theme".into(),
            category: "Theme".into(),
            shortcut: Some("Ctrl+Shift+T".into()),
        });
        r.register(Command {
            id: "font.zoom_in".into(),
            label: "Zoom In".into(),
            category: "Font".into(),
            shortcut: Some("Ctrl+=".into()),
        });
        r.register(Command {
            id: "font.zoom_out".into(),
            label: "Zoom Out".into(),
            category: "Font".into(),
            shortcut: Some("Ctrl+-".into()),
        });
        r.register(Command {
            id: "font.reset".into(),
            label: "Reset Font Size".into(),
            category: "Font".into(),
            shortcut: Some("Ctrl+0".into()),
        });
        // Terminal actions
        r.register(Command {
            id: "terminal.clear".into(),
            label: "Clear Screen".into(),
            category: "Terminal".into(),
            shortcut: Some("Ctrl+Shift+K".into()),
        });
        r.register(Command {
            id: "terminal.reset".into(),
            label: "Reset Terminal".into(),
            category: "Terminal".into(),
            shortcut: Some("Ctrl+Shift+R".into()),
        });
        r.register(Command {
            id: "terminal.select_all".into(),
            label: "Select All".into(),
            category: "Terminal".into(),
            shortcut: Some("Ctrl+Shift+A".into()),
        });
        r.register(Command {
            id: "terminal.copy".into(),
            label: "Copy Selection".into(),
            category: "Terminal".into(),
            shortcut: Some("Ctrl+Shift+C".into()),
        });
        r.register(Command {
            id: "terminal.paste".into(),
            label: "Paste".into(),
            category: "Terminal".into(),
            shortcut: Some("Ctrl+Shift+V".into()),
        });
        // View
        r.register(Command {
            id: "view.fullscreen".into(),
            label: "Toggle Fullscreen".into(),
            category: "View".into(),
            shortcut: Some("F11".into()),
        });
        r.register(Command {
            id: "view.maximize".into(),
            label: "Toggle Maximize".into(),
            category: "View".into(),
            shortcut: Some("Ctrl+Shift+Enter".into()),
        });
        r.register(Command {
            id: "view.status_bar".into(),
            label: "Toggle Status Bar".into(),
            category: "View".into(),
            shortcut: Some("Ctrl+Shift+B".into()),
        });
        r.register(Command {
            id: "view.search".into(),
            label: "Search Scrollback".into(),
            category: "View".into(),
            shortcut: Some("Ctrl+Shift+F".into()),
        });
        // AI
        r.register(Command {
            id: "ai.explain".into(),
            label: "AI: Explain".into(),
            category: "AI".into(),
            shortcut: Some("Ctrl+Shift+E".into()),
        });
        r.register(Command {
            id: "ai.suggest".into(),
            label: "AI: Suggest".into(),
            category: "AI".into(),
            shortcut: Some("Ctrl+Shift+S".into()),
        });
        r.register(Command {
            id: "ai.help".into(),
            label: "AI: Help".into(),
            category: "AI".into(),
            shortcut: Some("Ctrl+Shift+H".into()),
        });
        // SSH
        r.register(Command {
            id: "ssh.manager".into(),
            label: "SSH: Connection Manager".into(),
            category: "SSH".into(),
            shortcut: Some("Ctrl+Shift+K".into()),
        });
        // Session
        r.register(Command {
            id: "session.save".into(),
            label: "Save Session".into(),
            category: "Session".into(),
            shortcut: None,
        });
        r.register(Command {
            id: "session.profile".into(),
            label: "Switch Profile".into(),
            category: "Session".into(),
            shortcut: None,
        });
        r
    }
}

/// Fuzzy match score: higher = better match, 0 = no match.
/// Simple subsequence scoring.
fn fuzzy_score(query: &str, text: &str) -> i32 {
    if query.is_empty() {
        return 1;
    }
    let q = query.to_lowercase();
    let t = text.to_lowercase();

    // Exact substring match gets highest score.
    if t.contains(&q) {
        // Earlier match = higher score.
        return 100 - t.find(&q).unwrap_or(0) as i32;
    }

    // Subsequence match.
    let mut qi = q.chars().peekable();
    let mut score = 0;
    let mut consecutive = 0;
    let mut last_match = false;

    for tc in t.chars() {
        if let Some(&qc) = qi.peek() {
            if tc == qc {
                qi.next();
                consecutive += 1;
                score += 10 + consecutive * 5;
                last_match = true;
            } else {
                consecutive = 0;
                last_match = false;
            }
        }
    }

    if qi.peek().is_none() {
        // All query chars matched.
        if last_match {
            score += 10; // Bonus for ending with a match.
        }
        score
    } else {
        0 // Not all query chars found.
    }
}

/// Command palette overlay UI state.
#[derive(Debug, Clone, Default)]
pub struct CommandPaletteState {
    /// Whether the palette overlay is visible.
    pub visible: bool,
    /// Current search query.
    pub query: String,
    /// Selected index in the results.
    pub selected: usize,
    /// The selected command ID when Enter is pressed (consumed by caller).
    pub pending_action: Option<String>,
}

impl CommandPaletteState {
    /// Toggle visibility.
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        if !self.visible {
            self.query.clear();
            self.selected = 0;
        }
    }

    /// Type a character into the query.
    pub fn type_char(&mut self, c: char) {
        self.query.push(c);
        self.selected = 0;
    }

    /// Remove the last character.
    pub fn backspace(&mut self) {
        self.query.pop();
        self.selected = 0;
    }

    /// Move selection up.
    pub fn move_up(&mut self, max: usize) {
        if max == 0 {
            return;
        }
        self.selected = if self.selected == 0 {
            max - 1
        } else {
            self.selected - 1
        };
    }

    /// Move selection down.
    pub fn move_down(&mut self, max: usize) {
        if max == 0 {
            return;
        }
        self.selected = (self.selected + 1) % max;
    }

    /// Confirm selection. Sets pending_action to the selected command's ID.
    pub fn confirm(&mut self, results: &[(&Command, i32)]) {
        if let Some(&(cmd, _)) = results.get(self.selected) {
            self.pending_action = Some(cmd.id.clone());
        }
    }

    /// Take the pending action (consumes it).
    pub fn take_action(&mut self) -> Option<String> {
        self.pending_action.take()
    }

    /// Get filtered + sorted results from the registry.
    pub fn results<'a>(&self, registry: &'a CommandRegistry) -> Vec<(&'a Command, i32)> {
        let mut results: Vec<(&Command, i32)> = registry
            .all()
            .iter()
            .map(|cmd| {
                let score =
                    fuzzy_score(&self.query, &cmd.label).max(fuzzy_score(&self.query, &cmd.id));
                (cmd, score)
            })
            .filter(|(_, score)| *score > 0)
            .collect();
        results.sort_by(|a, b| b.1.cmp(&a.1));
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_registry_defaults_count() {
        let r = CommandRegistry::defaults();
        assert!(r.all().len() >= 20);
    }

    #[test]
    fn t_registry_dedup() {
        let mut r = CommandRegistry::new();
        r.register(Command {
            id: "a".into(),
            label: "A".into(),
            category: "Test".into(),
            shortcut: None,
        });
        r.register(Command {
            id: "a".into(),
            label: "A2".into(),
            category: "Test".into(),
            shortcut: None,
        });
        assert_eq!(r.all().len(), 1);
    }

    #[test]
    fn t_fuzzy_exact_match() {
        assert!(fuzzy_score("new", "New Tab") > 0);
    }

    #[test]
    fn t_fuzzy_subsequence() {
        assert!(fuzzy_score("nt", "New Tab") > 0);
    }

    #[test]
    fn t_fuzzy_no_match() {
        assert_eq!(fuzzy_score("xyz", "New Tab"), 0);
    }

    #[test]
    fn t_fuzzy_empty_query() {
        assert!(fuzzy_score("", "anything") > 0);
    }

    #[test]
    fn t_fuzzy_case_insensitive() {
        assert!(fuzzy_score("NEW", "new tab") > 0);
    }

    #[test]
    fn t_state_results_sorted_by_score() {
        let registry = CommandRegistry::defaults();
        let mut st = CommandPaletteState::default();
        st.query = "tab".to_string();
        let results = st.results(&registry);
        assert!(!results.is_empty());
        // Results should be sorted descending by score.
        for i in 1..results.len() {
            assert!(results[i - 1].1 >= results[i].1);
        }
    }

    #[test]
    fn t_state_confirm_sets_action() {
        let cmd = Command {
            id: "test.action".into(),
            label: "Test".into(),
            category: "Test".into(),
            shortcut: None,
        };
        let results = vec![(&cmd, 50)];
        let mut st = CommandPaletteState::default();
        st.selected = 0;
        st.confirm(&results);
        assert_eq!(st.take_action(), Some("test.action".to_string()));
    }

    #[test]
    fn t_state_navigation() {
        let mut st = CommandPaletteState::default();
        st.visible = true;
        st.move_down(5);
        assert_eq!(st.selected, 1);
        st.move_down(5);
        assert_eq!(st.selected, 2);
        st.move_up(5);
        assert_eq!(st.selected, 1);
        st.selected = 0;
        st.move_up(5);
        assert_eq!(st.selected, 4); // wrap to last
    }

    #[test]
    fn t_state_toggle_resets() {
        let mut st = CommandPaletteState::default();
        st.visible = true; // start visible so toggle() turns it off
        st.query = "abc".to_string();
        st.selected = 3;
        st.toggle();
        assert!(!st.visible);
        assert!(st.query.is_empty());
        assert_eq!(st.selected, 0);
    }
}
