//! P28-C: Command history sidebar.
//!
//! Uses OSC 133 shell integration marks to build a navigable command
//! history sidebar. Users can click on a past command to scroll to it.

use std::collections::VecDeque;

/// Maximum number of command history entries to keep.
const MAX_ENTRIES: usize = 500;

/// A single command history entry derived from OSC 133 marks.
#[derive(Debug, Clone)]
pub struct CommandHistoryEntry {
    /// The command text (from CommandStart to OutputStart).
    pub command: String,
    /// Grid row where the command was entered.
    pub row: usize,
    /// Exit code (if known from OSC 133;D).
    pub exit_code: Option<i32>,
    /// Timestamp (relative to app start, in milliseconds).
    pub timestamp_ms: u64,
    /// Whether this command is still running.
    pub running: bool,
}

/// State for the command history sidebar.
#[derive(Debug)]
pub struct CommandHistoryState {
    /// Whether the sidebar is visible.
    pub visible: bool,
    /// Scroll offset in the history list.
    pub scroll_offset: usize,
    /// History entries.
    entries: VecDeque<CommandHistoryEntry>,
    /// Selected entry index (for keyboard navigation).
    pub selected: Option<usize>,
    /// Current timestamp counter (incremented on each add).
    tick_counter: u64,
    /// Search filter query (empty = show all).
    pub search_query: String,
    /// Whether the search input is active (typing filters history).
    pub search_active: bool,
}

impl Default for CommandHistoryState {
    fn default() -> Self {
        Self {
            visible: false,
            scroll_offset: 0,
            entries: VecDeque::with_capacity(MAX_ENTRIES),
            selected: None,
            tick_counter: 0,
            search_query: String::new(),
            search_active: false,
        }
    }
}

impl CommandHistoryState {
    /// Create new command history state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Toggle sidebar visibility.
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    /// Add a new command entry.
    pub fn add(&mut self, command: String, row: usize) {
        self.tick_counter += 1;
        let entry = CommandHistoryEntry {
            command,
            row,
            exit_code: None,
            timestamp_ms: self.tick_counter,
            running: true,
        };

        // Check if the last entry is still running — update it
        if let Some(last) = self.entries.back_mut()
            && last.running
        {
            // The previous command finished; mark it as done
            last.running = false;
        }

        self.entries.push_back(entry);

        // Trim if over capacity
        while self.entries.len() > MAX_ENTRIES {
            self.entries.pop_front();
        }
    }

    /// Mark the last running command as completed with the given exit code.
    pub fn complete_last(&mut self, exit_code: i32) {
        if let Some(last) = self.entries.back_mut()
            && last.running
        {
            last.running = false;
            last.exit_code = Some(exit_code);
        }
    }

    /// Get all entries (newest first for display).
    pub fn entries_rev(&self) -> impl Iterator<Item = &CommandHistoryEntry> {
        self.entries.iter().rev()
    }

    /// Get filtered entries (newest first), matching search query.
    pub fn filtered_entries_rev(&self) -> Vec<&CommandHistoryEntry> {
        if self.search_query.is_empty() {
            return self.entries.iter().rev().collect();
        }
        let q = self.search_query.to_lowercase();
        self.entries
            .iter()
            .rev()
            .filter(|e| e.command.to_lowercase().contains(&q))
            .collect()
    }

    /// Append a character to the search query.
    pub fn search_push(&mut self, c: char) {
        self.search_query.push(c);
        self.selected = None;
        self.scroll_offset = 0;
    }

    /// Remove the last character from the search query.
    pub fn search_backspace(&mut self) {
        self.search_query.pop();
        self.selected = None;
        self.scroll_offset = 0;
    }

    /// Clear the search query.
    pub fn search_clear(&mut self) {
        self.search_query.clear();
        self.selected = None;
        self.scroll_offset = 0;
    }

    /// Toggle search input mode.
    pub fn toggle_search(&mut self) {
        self.search_active = !self.search_active;
        if !self.search_active {
            self.search_query.clear();
            self.selected = None;
            self.scroll_offset = 0;
        }
    }

    /// Get all entries (oldest first).
    pub fn entries(&self) -> impl Iterator<Item = &CommandHistoryEntry> {
        self.entries.iter()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the history is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get a specific entry by index (from oldest).
    pub fn get(&self, index: usize) -> Option<&CommandHistoryEntry> {
        self.entries.get(index)
    }

    /// Clear all history.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.selected = None;
        self.scroll_offset = 0;
        self.search_query.clear();
    }

    /// Move selection up (newer entries).
    pub fn select_up(&mut self) {
        let total = self.entries.len();
        if total == 0 {
            return;
        }
        self.selected = Some(match self.selected {
            None => 0,
            Some(i) => {
                if i + 1 < total {
                    i + 1
                } else {
                    i
                }
            }
        });
    }

    /// Move selection down (older entries).
    pub fn select_down(&mut self) {
        self.selected = Some(match self.selected {
            None => 0,
            Some(0) => 0,
            Some(i) => i - 1,
        });
    }

    /// Get the row of the selected entry (for scrolling).
    pub fn selected_row(&self) -> Option<usize> {
        self.selected
            .and_then(|i| self.entries.get(i).map(|e| e.row))
    }

    /// Filter entries by a search query.
    pub fn filter<'a>(
        &'a self,
        query: &'a str,
    ) -> impl Iterator<Item = &'a CommandHistoryEntry> + 'a {
        self.entries.iter().filter(move |e| {
            query.is_empty() || e.command.to_lowercase().contains(&query.to_lowercase())
        })
    }

    /// Get running command count.
    pub fn running_count(&self) -> usize {
        self.entries.iter().filter(|e| e.running).count()
    }

    /// Get failed command count (exit code != 0).
    pub fn failed_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.exit_code.is_some_and(|c| c != 0))
            .count()
    }

    /// Get the most recent N entries for display.
    pub fn recent(&self, n: usize) -> Vec<&CommandHistoryEntry> {
        self.entries.iter().rev().take(n).collect()
    }
}

/// Format an entry for sidebar display.
pub fn format_entry_short(entry: &CommandHistoryEntry, max_width: usize) -> String {
    let status = match (entry.running, entry.exit_code) {
        (true, _) => "...".to_string(),
        (false, Some(0)) => "OK".to_string(),
        (false, Some(c)) => format!("E{}", c),
        (false, None) => "??".to_string(),
    };

    let cmd = if entry.command.len() > max_width.saturating_sub(status.len() + 3) {
        let trim = max_width.saturating_sub(status.len() + 6);
        format!("{}...", &entry.command[..trim])
    } else {
        entry.command.clone()
    };

    format!("[{}] {}", status, cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_default_hidden() {
        let state = CommandHistoryState::new();
        assert!(!state.visible);
        assert!(state.is_empty());
    }

    #[test]
    fn t_toggle() {
        let mut state = CommandHistoryState::new();
        assert!(!state.visible);
        state.toggle();
        assert!(state.visible);
        state.toggle();
        assert!(!state.visible);
    }

    #[test]
    fn t_add_entries() {
        let mut state = CommandHistoryState::new();
        state.add("ls -la".to_string(), 10);
        state.add("git status".to_string(), 20);
        state.add("make test".to_string(), 30);
        assert_eq!(state.len(), 3);
    }

    #[test]
    fn t_complete_last() {
        let mut state = CommandHistoryState::new();
        state.add("ls".to_string(), 5);
        state.complete_last(0);
        assert_eq!(state.entries().next().unwrap().exit_code, Some(0));
        assert!(!state.entries().next().unwrap().running);
    }

    #[test]
    fn t_complete_last_failed() {
        let mut state = CommandHistoryState::new();
        state.add("false".to_string(), 5);
        state.complete_last(1);
        assert_eq!(state.entries().next().unwrap().exit_code, Some(1));
    }

    #[test]
    fn t_multiple_commands() {
        let mut state = CommandHistoryState::new();
        state.add("ls".to_string(), 5);
        state.complete_last(0);
        state.add("grep foo".to_string(), 10);
        state.complete_last(1);
        state.add("make".to_string(), 15);

        let entries: Vec<_> = state.entries().collect();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].command, "ls");
        assert_eq!(entries[0].exit_code, Some(0));
        assert_eq!(entries[1].command, "grep foo");
        assert_eq!(entries[1].exit_code, Some(1));
        assert_eq!(entries[2].command, "make");
        assert!(entries[2].running); // still running
    }

    #[test]
    fn t_capacity_limit() {
        let mut state = CommandHistoryState::new();
        for i in 0..600 {
            state.add(format!("cmd{}", i), i);
        }
        assert_eq!(state.len(), MAX_ENTRIES);
    }

    #[test]
    fn t_clear() {
        let mut state = CommandHistoryState::new();
        state.add("test".to_string(), 0);
        state.clear();
        assert!(state.is_empty());
    }

    #[test]
    fn t_filter() {
        let mut state = CommandHistoryState::new();
        state.add("ls -la".to_string(), 0);
        state.add("git status".to_string(), 5);
        state.add("git log".to_string(), 10);

        let git_entries: Vec<_> = state.filter("git").collect();
        assert_eq!(git_entries.len(), 2);
    }

    #[test]
    fn t_filter_case_insensitive() {
        let mut state = CommandHistoryState::new();
        state.add("LS -la".to_string(), 0);
        let results: Vec<_> = state.filter("ls").collect();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn t_recent() {
        let mut state = CommandHistoryState::new();
        state.add("cmd1".to_string(), 0);
        state.add("cmd2".to_string(), 1);
        state.add("cmd3".to_string(), 2);

        let recent = state.recent(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].command, "cmd3");
        assert_eq!(recent[1].command, "cmd2");
    }

    #[test]
    fn t_select_navigation() {
        let mut state = CommandHistoryState::new();
        state.add("a".to_string(), 0);
        state.add("b".to_string(), 1);
        state.add("c".to_string(), 2);

        assert_eq!(state.selected, None);
        state.select_up();
        assert_eq!(state.selected, Some(0));
        state.select_up();
        assert_eq!(state.selected, Some(1));
        state.select_up();
        assert_eq!(state.selected, Some(2));
        state.select_up();
        assert_eq!(state.selected, Some(2)); // clamp at top
        state.select_down();
        assert_eq!(state.selected, Some(1));
    }

    #[test]
    fn t_selected_row() {
        let mut state = CommandHistoryState::new();
        state.add("a".to_string(), 10);
        state.add("b".to_string(), 20);
        state.selected = Some(1);
        assert_eq!(state.selected_row(), Some(20));
    }

    #[test]
    fn t_running_count() {
        let mut state = CommandHistoryState::new();
        state.add("a".to_string(), 0);
        state.add("b".to_string(), 1);
        state.complete_last(0);
        state.add("c".to_string(), 2);
        assert_eq!(state.running_count(), 1);
    }

    #[test]
    fn t_failed_count() {
        let mut state = CommandHistoryState::new();
        state.add("a".to_string(), 0);
        state.complete_last(0);
        state.add("b".to_string(), 1);
        state.complete_last(1);
        state.add("c".to_string(), 2);
        state.complete_last(0);
        assert_eq!(state.failed_count(), 1);
    }

    #[test]
    fn t_format_entry_short_ok() {
        let entry = CommandHistoryEntry {
            command: "ls -la".to_string(),
            row: 0,
            exit_code: Some(0),
            timestamp_ms: 0,
            running: false,
        };
        let s = format_entry_short(&entry, 30);
        assert!(s.contains("[OK]"));
        assert!(s.contains("ls -la"));
    }

    #[test]
    fn t_format_entry_short_error() {
        let entry = CommandHistoryEntry {
            command: "false".to_string(),
            row: 0,
            exit_code: Some(1),
            timestamp_ms: 0,
            running: false,
        };
        let s = format_entry_short(&entry, 30);
        assert!(s.contains("[E1]"));
    }

    #[test]
    fn t_format_entry_short_running() {
        let entry = CommandHistoryEntry {
            command: "sleep 100".to_string(),
            row: 0,
            exit_code: None,
            timestamp_ms: 0,
            running: true,
        };
        let s = format_entry_short(&entry, 30);
        assert!(s.contains("[...]"));
    }

    #[test]
    fn t_format_entry_short_truncates() {
        let entry = CommandHistoryEntry {
            command: "very_long_command_name_that_exceeds_the_display_width".to_string(),
            row: 0,
            exit_code: Some(0),
            timestamp_ms: 0,
            running: false,
        };
        let s = format_entry_short(&entry, 20);
        assert!(
            s.len() <= 20,
            "formatted string too long: {} ({})",
            s,
            s.len()
        );
    }

    #[test]
    fn t_entries_rev_order() {
        let mut state = CommandHistoryState::new();
        state.add("first".to_string(), 0);
        state.add("second".to_string(), 1);
        state.add("third".to_string(), 2);

        let rev: Vec<_> = state.entries_rev().collect();
        assert_eq!(rev[0].command, "third");
        assert_eq!(rev[1].command, "second");
        assert_eq!(rev[2].command, "first");
    }

    #[test]
    fn t_search_filter_basic() {
        let mut state = CommandHistoryState::new();
        state.add("ls -la".to_string(), 0);
        state.add("grep foo".to_string(), 1);
        state.add("ls -lh".to_string(), 2);
        state.search_query = "ls".to_string();
        let filtered = state.filtered_entries_rev();
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].command, "ls -lh");
        assert_eq!(filtered[1].command, "ls -la");
    }

    #[test]
    fn t_search_filter_case_insensitive() {
        let mut state = CommandHistoryState::new();
        state.add("LS -la".to_string(), 0);
        state.add("grep FOO".to_string(), 1);
        state.search_query = "foo".to_string();
        let filtered = state.filtered_entries_rev();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].command, "grep FOO");
    }

    #[test]
    fn t_search_filter_empty_shows_all() {
        let mut state = CommandHistoryState::new();
        state.add("a".to_string(), 0);
        state.add("b".to_string(), 1);
        let filtered = state.filtered_entries_rev();
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn t_search_push_backspace_clear() {
        let mut state = CommandHistoryState::new();
        state.search_push('l');
        state.search_push('s');
        assert_eq!(state.search_query, "ls");
        state.search_backspace();
        assert_eq!(state.search_query, "l");
        state.search_clear();
        assert_eq!(state.search_query, "");
    }

    #[test]
    fn t_toggle_search() {
        let mut state = CommandHistoryState::new();
        assert!(!state.search_active);
        state.toggle_search();
        assert!(state.search_active);
        state.search_push('x');
        state.toggle_search();
        assert!(!state.search_active);
        assert_eq!(state.search_query, "");
    }
}
