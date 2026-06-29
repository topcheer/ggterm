//! Tab bar: visual tab strip data model.
//!
//! Provides [`TabInfo`] for each open tab and [`TabBarState`] which aggregates
//! the display-ready tab list. The actual wgpu rendering happens in
//! `render_frame()` via the renderer overlay; this module owns the data model.

// ── TabInfo ─────────────────────────────────────────────────────────────

/// Display info for a single tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabInfo {
    /// Tab title (from OSC 0/2 or shell name).
    pub title: String,
    /// 1-based tab index.
    pub index: usize,
    /// Whether this is the active tab.
    pub active: bool,
    /// Whether the tab has unseen output (dirty / bell).
    pub dirty: bool,
}

impl TabInfo {
    /// Create a new TabInfo.
    pub fn new(title: impl Into<String>, index: usize, active: bool) -> Self {
        Self {
            title: title.into(),
            index,
            active,
            dirty: false,
        }
    }

    /// Truncate title to `max_chars` characters, appending an ellipsis.
    pub fn truncated_title(&self, max_chars: usize) -> String {
        let count = self.title.chars().count();
        if count > max_chars {
            format!(
                "{}\u{2026}",
                self.title
                    .chars()
                    .take(max_chars.saturating_sub(1))
                    .collect::<String>()
            )
        } else {
            self.title.clone()
        }
    }

    /// Format as a display string: `1:zsh*` (active) or `2:vim` (inactive).
    pub fn format(&self) -> String {
        let title = self.truncated_title(12);
        let suffix = if self.active { "*" } else { "" };
        let dirty = if self.dirty && !self.active { "!" } else { "" };
        format!("{}:{}{}{}", self.index, title, suffix, dirty)
    }
}

// ── TabBarState ─────────────────────────────────────────────────────────

/// Aggregated tab bar state for rendering.
///
/// Updated every frame from the DesktopApp's session list.
#[derive(Debug, Clone, Default)]
pub struct TabBarState {
    /// Display info for each tab.
    pub tabs: Vec<TabInfo>,
    /// Whether the tab bar is visible (more than 1 tab + setting enabled).
    pub visible: bool,
}

impl TabBarState {
    /// Create an empty tab bar state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuild the tab list from session titles and active index.
    pub fn update(&mut self, titles: &[&str], active: usize) {
        self.tabs = titles
            .iter()
            .enumerate()
            .map(|(i, &title)| TabInfo {
                title: title.to_string(),
                index: i + 1,
                active: i == active,
                dirty: false,
            })
            .collect();
        self.visible = self.tabs.len() > 1;
    }

    /// Format the entire tab bar as a single string: `1:zsh* | 2:vim | 3:logs`.
    pub fn format(&self) -> String {
        if self.tabs.is_empty() {
            return String::new();
        }
        self.tabs
            .iter()
            .map(|t| t.format())
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_tab_info_format_active() {
        let tab = TabInfo::new("zsh", 1, true);
        assert_eq!(tab.format(), "1:zsh*");
    }

    #[test]
    fn t_tab_info_format_inactive() {
        let tab = TabInfo::new("vim", 2, false);
        assert_eq!(tab.format(), "2:vim");
    }

    #[test]
    fn t_tab_info_truncated_title() {
        let tab = TabInfo::new("very_long_process_name", 1, true);
        let formatted = tab.format();
        // Should be truncated to 12 chars + ellipsis
        assert!(formatted.starts_with("1:"));
        assert!(formatted.contains('\u{2026}'));
        assert!(formatted.ends_with('*'));
    }

    #[test]
    fn t_tab_info_dirty_marker() {
        let mut tab = TabInfo::new("logs", 3, false);
        tab.dirty = true;
        assert_eq!(tab.format(), "3:logs!");
    }

    #[test]
    fn t_tab_bar_state_single_tab_hidden() {
        let mut state = TabBarState::new();
        state.update(&["zsh"], 0);
        assert!(!state.visible);
    }

    #[test]
    fn t_tab_bar_state_multi_tab_visible() {
        let mut state = TabBarState::new();
        state.update(&["zsh", "vim", "logs"], 1);
        assert!(state.visible);
        assert_eq!(state.format(), "1:zsh | 2:vim* | 3:logs");
    }

    #[test]
    fn t_tab_bar_state_empty() {
        let state = TabBarState::new();
        assert!(!state.visible);
        assert_eq!(state.format(), "");
    }

    #[test]
    fn t_tab_bar_state_update_changes_active() {
        let mut state = TabBarState::new();
        state.update(&["a", "b"], 0);
        assert!(state.tabs[0].active);
        assert!(!state.tabs[1].active);

        state.update(&["a", "b"], 1);
        assert!(!state.tabs[0].active);
        assert!(state.tabs[1].active);
    }

    #[test]
    fn t_tab_info_new_default_not_dirty() {
        let tab = TabInfo::new("test", 1, true);
        assert!(!tab.dirty);
    }

    #[test]
    fn t_truncated_title_short_unchanged() {
        let tab = TabInfo::new("sh", 1, false);
        assert_eq!(tab.truncated_title(10), "sh");
    }

    // ── P19-H: Integration edge cases ─────────────────────────

    #[test]
    fn t_active_dirty_suppresses_marker() {
        // When active=true and dirty=true, dirty marker (!) is suppressed.
        let mut tab = TabInfo::new("logs", 1, true);
        tab.dirty = true;
        // Should be "1:logs*" not "1:logs!*"
        assert_eq!(tab.format(), "1:logs*");
    }

    #[test]
    fn t_truncated_title_exact_boundary() {
        // 12 chars exactly should NOT be truncated.
        let tab = TabInfo::new("twelvechars!", 1, false);
        assert_eq!(tab.truncated_title(12), "twelvechars!");
        // 13 chars should be truncated.
        let tab2 = TabInfo::new("thirteenchars", 1, false);
        assert!(tab2.truncated_title(12).contains('\u{2026}'));
    }

    #[test]
    fn t_tab_bar_many_tabs() {
        let mut state = TabBarState::new();
        let titles: Vec<&str> = (0..10).map(|_| "sh").collect();
        state.update(&titles, 5);
        assert!(state.visible);
        assert_eq!(state.tabs.len(), 10);
        // Tab 6 (index 5) should be active.
        assert!(state.tabs[5].active);
        assert!(!state.tabs[0].active);
    }

    #[test]
    fn t_tab_info_clone_eq() {
        let t1 = TabInfo::new("vim", 2, true);
        let t2 = t1.clone();
        assert_eq!(t1, t2);
    }

    #[test]
    fn t_update_resets_dirty() {
        // Even if a previous tab was dirty, update() resets all dirty flags.
        let mut state = TabBarState::new();
        state.update(&["a", "b"], 0);
        state.tabs[1].dirty = true;
        state.update(&["a", "b"], 0);
        assert!(!state.tabs[1].dirty);
    }

    #[test]
    fn t_format_inactive_last_tab() {
        let mut state = TabBarState::new();
        state.update(&["sh", "vim"], 0);
        // Active is 0, so tab 1 is inactive.
        let formatted = state.format();
        assert!(formatted.contains("1:sh*"));
        assert!(formatted.contains("2:vim"));
    }
}
