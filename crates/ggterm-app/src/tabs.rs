//! Tab management — multi-session terminal support.
//!
//! Each tab owns its own `Terminal` instance. The `TabManager` tracks
//! the active tab and provides navigation (next/prev/switch by index).
//!
//! The actual PTY session management is handled at the app level —
//! `TabManager` is a pure state container that is easy to test.

use std::collections::HashMap;

/// Metadata for a single terminal tab.
#[derive(Debug, Clone)]
pub struct TabInfo {
    /// Unique tab identifier.
    pub id: usize,
    /// Display title (from OSC 0/2 or default).
    pub title: String,
    /// Whether this tab has unsaved output since last view.
    pub dirty: bool,
    /// Column count.
    pub cols: usize,
    /// Row count.
    pub rows: usize,
}

impl TabInfo {
    /// Create a new TabInfo with a default title.
    pub fn new(id: usize, cols: usize, rows: usize) -> Self {
        Self {
            id,
            title: format!("Terminal {}", id + 1),
            dirty: false,
            cols,
            rows,
        }
    }

    /// Create with a specific title.
    pub fn with_title(id: usize, title: impl Into<String>, cols: usize, rows: usize) -> Self {
        Self {
            id,
            title: title.into(),
            dirty: false,
            cols,
            rows,
        }
    }

    /// Mark this tab as dirty (has new output).
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Mark this tab as clean (output was viewed).
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Set the tab title.
    pub fn set_title(&mut self, title: impl Into<String>) {
        let new_title = title.into();
        if new_title != self.title {
            self.title = new_title;
            self.dirty = true;
        }
    }
}

/// Manages multiple terminal tabs.
///
/// TabManager is a pure state container — it tracks metadata about each
/// tab (id, title, dirty flag, dimensions) but does NOT own the actual
/// `Terminal` or `PtySession` instances. Those are managed by the `App`
/// which uses `TabManager` as a bookkeeping layer.
///
/// This design keeps TabManager trivially testable without PTY/spawning.
pub struct TabManager {
    /// Ordered list of tab metadata.
    tabs: Vec<TabInfo>,
    /// Index into `tabs` for the currently active tab.
    active: usize,
    /// Next unique tab ID to assign.
    next_id: usize,
    /// Maximum number of tabs allowed.
    max_tabs: usize,
    /// Default cols for new tabs.
    default_cols: usize,
    /// Default rows for new tabs.
    default_rows: usize,
}

impl TabManager {
    /// Create a new TabManager with a single initial tab.
    pub fn new(cols: usize, rows: usize) -> Self {
        let initial = TabInfo::new(0, cols, rows);
        Self {
            tabs: vec![initial],
            active: 0,
            next_id: 1,
            max_tabs: 10,
            default_cols: cols,
            default_rows: rows,
        }
    }

    /// Create a TabManager with no tabs (for testing).
    pub fn empty(cols: usize, rows: usize) -> Self {
        Self {
            tabs: vec![],
            active: 0,
            next_id: 0,
            max_tabs: 10,
            default_cols: cols,
            default_rows: rows,
        }
    }

    /// Set the maximum number of tabs.
    pub fn set_max_tabs(&mut self, max: usize) {
        self.max_tabs = max.max(1);
    }

    /// Get the maximum number of tabs.
    pub fn max_tabs(&self) -> usize {
        self.max_tabs
    }

    /// Get the number of tabs.
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// Check if any tabs exist.
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// Get the active tab index.
    pub fn active_index(&self) -> usize {
        self.active
    }

    /// Get the active tab ID.
    pub fn active_id(&self) -> Option<usize> {
        self.tabs.get(self.active).map(|t| t.id)
    }

    /// Get the active tab info.
    pub fn active_tab(&self) -> Option<&TabInfo> {
        self.tabs.get(self.active)
    }

    /// Get the active tab info (mutable).
    pub fn active_tab_mut(&mut self) -> Option<&mut TabInfo> {
        self.tabs.get_mut(self.active)
    }

    /// Get all tab infos (in order).
    pub fn tabs(&self) -> &[TabInfo] {
        &self.tabs
    }

    /// Open a new tab. Returns the new tab's index, or `None` if at max capacity.
    pub fn open_tab(&mut self) -> Option<usize> {
        if self.tabs.len() >= self.max_tabs {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        let info = TabInfo::new(id, self.default_cols, self.default_rows);
        self.tabs.push(info);
        let new_index = self.tabs.len() - 1;
        self.active = new_index;
        Some(new_index)
    }

    /// Open a new tab with a specific title.
    pub fn open_tab_titled(&mut self, title: impl Into<String>) -> Option<usize> {
        if self.tabs.len() >= self.max_tabs {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        let info = TabInfo::with_title(id, title, self.default_cols, self.default_rows);
        self.tabs.push(info);
        let new_index = self.tabs.len() - 1;
        self.active = new_index;
        Some(new_index)
    }

    /// Close the tab at the given index. The active tab shifts to the
    /// previous tab (or next if closing the first tab).
    ///
    /// Returns the ID of the closed tab, or `None` if the index is invalid
    /// or if this is the last tab (cannot close the final tab).
    pub fn close_tab(&mut self, index: usize) -> Option<usize> {
        if self.tabs.len() <= 1 || index >= self.tabs.len() {
            return None;
        }
        let closed_id = self.tabs[index].id;
        self.tabs.remove(index);
        // Adjust active index
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if index < self.active {
            self.active -= 1;
        }
        Some(closed_id)
    }

    /// Close the currently active tab.
    pub fn close_active(&mut self) -> Option<usize> {
        let idx = self.active;
        self.close_tab(idx)
    }

    /// Switch to the tab at the given index. Returns `true` if successful.
    pub fn switch_tab(&mut self, index: usize) -> bool {
        if index < self.tabs.len() {
            self.active = index;
            // Mark the newly active tab as clean
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.mark_clean();
            }
            true
        } else {
            false
        }
    }

    /// Switch to the next tab (wraps around).
    pub fn next_tab(&mut self) {
        if self.tabs.len() > 1 {
            self.active = (self.active + 1) % self.tabs.len();
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.mark_clean();
            }
        }
    }

    /// Switch to the previous tab (wraps around).
    pub fn prev_tab(&mut self) {
        if self.tabs.len() > 1 {
            self.active = if self.active == 0 {
                self.tabs.len() - 1
            } else {
                self.active - 1
            };
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.mark_clean();
            }
        }
    }

    /// Switch to the tab with the given ID. Returns `true` if found.
    pub fn switch_to_id(&mut self, id: usize) -> bool {
        if let Some(index) = self.tabs.iter().position(|t| t.id == id) {
            self.active = index;
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.mark_clean();
            }
            true
        } else {
            false
        }
    }

    /// Mark the active tab as dirty (new output arrived).
    pub fn mark_active_dirty(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.mark_dirty();
        }
    }

    /// Set the title of the active tab.
    pub fn set_active_title(&mut self, title: impl Into<String>) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.set_title(title);
        }
    }

    /// Set the title of a specific tab by ID.
    pub fn set_title(&mut self, id: usize, title: impl Into<String>) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
            tab.set_title(title);
        }
    }

    /// Update dimensions for all tabs (e.g., on window resize).
    pub fn resize_all(&mut self, cols: usize, rows: usize) {
        self.default_cols = cols;
        self.default_rows = rows;
        for tab in &mut self.tabs {
            tab.cols = cols;
            tab.rows = rows;
        }
    }

    /// Get a mapping of tab_id → title for quick lookup.
    pub fn titles_map(&self) -> HashMap<usize, String> {
        self.tabs.iter().map(|t| (t.id, t.title.clone())).collect()
    }

    /// Find a tab by ID.
    pub fn find(&self, id: usize) -> Option<&TabInfo> {
        self.tabs.iter().find(|t| t.id == id)
    }

    /// Check if any non-active tab is dirty.
    pub fn has_dirty_background_tabs(&self) -> bool {
        self.tabs
            .iter()
            .enumerate()
            .any(|(i, t)| i != self.active && t.dirty)
    }
}

impl Default for TabManager {
    fn default() -> Self {
        Self::new(80, 24)
    }
}

#[cfg(test)]
impl TabManager {
    /// Test-only helper to get mutable access to all tabs.
    fn tabs_mut_for_test(&mut self) -> &mut [TabInfo] {
        &mut self.tabs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── TabInfo tests ──

    #[test]
    fn t_tabinfo_new() {
        let info = TabInfo::new(0, 80, 24);
        assert_eq!(info.id, 0);
        assert_eq!(info.title, "Terminal 1");
        assert!(!info.dirty);
        assert_eq!(info.cols, 80);
        assert_eq!(info.rows, 24);
    }

    #[test]
    fn t_tabinfo_with_title() {
        let info = TabInfo::with_title(2, "vim", 80, 24);
        assert_eq!(info.id, 2);
        assert_eq!(info.title, "vim");
    }

    #[test]
    fn t_tabinfo_mark_dirty() {
        let mut info = TabInfo::new(0, 80, 24);
        info.mark_dirty();
        assert!(info.dirty);
    }

    #[test]
    fn t_tabinfo_mark_clean() {
        let mut info = TabInfo::new(0, 80, 24);
        info.mark_dirty();
        info.mark_clean();
        assert!(!info.dirty);
    }

    #[test]
    fn t_tabinfo_set_title() {
        let mut info = TabInfo::new(0, 80, 24);
        info.set_title("new title");
        assert_eq!(info.title, "new title");
        assert!(info.dirty, "set_title should mark dirty if changed");
    }

    #[test]
    fn t_tabinfo_set_same_title_no_dirty() {
        let mut info = TabInfo::with_title(0, "same", 80, 24);
        info.dirty = false;
        info.set_title("same");
        assert!(!info.dirty, "set_title should NOT mark dirty if unchanged");
    }

    // ── TabManager construction ──

    #[test]
    fn t_manager_default_has_one_tab() {
        let mgr = TabManager::default();
        assert_eq!(mgr.tab_count(), 1);
        assert_eq!(mgr.active_index(), 0);
    }

    #[test]
    fn t_manager_new_custom_size() {
        let mgr = TabManager::new(120, 40);
        assert_eq!(mgr.tab_count(), 1);
        let tab = mgr.active_tab().unwrap();
        assert_eq!(tab.cols, 120);
        assert_eq!(tab.rows, 40);
    }

    #[test]
    fn t_manager_empty() {
        let mgr = TabManager::empty(80, 24);
        assert!(mgr.is_empty());
        assert_eq!(mgr.tab_count(), 0);
        assert!(mgr.active_tab().is_none());
    }

    // ── open_tab ──

    #[test]
    fn t_open_tab_basic() {
        let mut mgr = TabManager::new(80, 24);
        let idx = mgr.open_tab().unwrap();
        assert_eq!(idx, 1);
        assert_eq!(mgr.tab_count(), 2);
        assert_eq!(mgr.active_index(), 1);
    }

    #[test]
    fn t_open_tab_titled() {
        let mut mgr = TabManager::new(80, 24);
        let idx = mgr.open_tab_titled("server logs").unwrap();
        assert_eq!(idx, 1);
        assert_eq!(mgr.active_tab().unwrap().title, "server logs");
    }

    #[test]
    fn t_open_tab_multiple() {
        let mut mgr = TabManager::new(80, 24);
        mgr.open_tab();
        mgr.open_tab();
        mgr.open_tab();
        assert_eq!(mgr.tab_count(), 4);
        assert_eq!(mgr.active_index(), 3);
    }

    #[test]
    fn t_open_tab_max_limit() {
        let mut mgr = TabManager::new(80, 24);
        mgr.set_max_tabs(3);
        assert!(mgr.open_tab().is_some()); // tab 2
        assert!(mgr.open_tab().is_some()); // tab 3
        assert!(mgr.open_tab().is_none()); // max reached
        assert_eq!(mgr.tab_count(), 3);
    }

    #[test]
    fn t_set_max_tabs() {
        let mut mgr = TabManager::new(80, 24);
        mgr.set_max_tabs(5);
        assert_eq!(mgr.max_tabs(), 5);
    }

    #[test]
    fn t_set_max_tabs_minimum_1() {
        let mut mgr = TabManager::new(80, 24);
        mgr.set_max_tabs(0);
        assert_eq!(mgr.max_tabs(), 1);
    }

    // ── close_tab ──

    #[test]
    fn t_close_tab_basic() {
        let mut mgr = TabManager::new(80, 24);
        mgr.open_tab();
        mgr.open_tab();
        // Now 3 tabs, active = 2
        let closed_id = mgr.close_tab(2);
        assert_eq!(closed_id, Some(2));
        assert_eq!(mgr.tab_count(), 2);
        assert_eq!(mgr.active_index(), 1);
    }

    #[test]
    fn t_close_tab_middle() {
        let mut mgr = TabManager::new(80, 24);
        mgr.open_tab();
        mgr.open_tab();
        // 3 tabs: [0, 1, 2], active = 2
        mgr.switch_tab(0);
        // Close tab at index 1 — active should shift down
        let closed_id = mgr.close_tab(1);
        assert_eq!(closed_id, Some(1));
        assert_eq!(mgr.tab_count(), 2);
        assert_eq!(mgr.active_index(), 0);
    }

    #[test]
    fn t_close_tab_cannot_close_last() {
        let mut mgr = TabManager::new(80, 24);
        assert_eq!(mgr.tab_count(), 1);
        assert!(mgr.close_tab(0).is_none());
        assert_eq!(mgr.tab_count(), 1);
    }

    #[test]
    fn t_close_tab_invalid_index() {
        let mut mgr = TabManager::new(80, 24);
        mgr.open_tab();
        assert!(mgr.close_tab(99).is_none());
        assert_eq!(mgr.tab_count(), 2);
    }

    #[test]
    fn t_close_active() {
        let mut mgr = TabManager::new(80, 24);
        mgr.open_tab();
        let closed_id = mgr.close_active();
        assert_eq!(closed_id, Some(1));
        assert_eq!(mgr.tab_count(), 1);
    }

    // ── switch_tab ──

    #[test]
    fn t_switch_tab_basic() {
        let mut mgr = TabManager::new(80, 24);
        mgr.open_tab();
        mgr.open_tab();
        assert!(mgr.switch_tab(0));
        assert_eq!(mgr.active_index(), 0);
        assert!(mgr.switch_tab(2));
        assert_eq!(mgr.active_index(), 2);
    }

    #[test]
    fn t_switch_tab_invalid() {
        let mut mgr = TabManager::new(80, 24);
        assert!(!mgr.switch_tab(99));
    }

    #[test]
    fn t_switch_tab_marks_clean() {
        let mut mgr = TabManager::new(80, 24);
        mgr.open_tab(); // tab 1, active
        mgr.switch_tab(0); // back to tab 0
        // Tab 1 should be dirty when we switch away and something arrives
        mgr.switch_tab(1);
        mgr.mark_active_dirty();
        assert!(mgr.active_tab().unwrap().dirty);
        mgr.switch_tab(0);
        mgr.switch_tab(1);
        assert!(
            !mgr.active_tab().unwrap().dirty,
            "switching to tab marks it clean"
        );
    }

    // ── next/prev ──

    #[test]
    fn t_next_tab() {
        let mut mgr = TabManager::new(80, 24);
        mgr.open_tab();
        mgr.open_tab();
        // 3 tabs, active=2
        mgr.next_tab();
        assert_eq!(mgr.active_index(), 0); // wraps
    }

    #[test]
    fn t_prev_tab() {
        let mut mgr = TabManager::new(80, 24);
        mgr.open_tab();
        mgr.open_tab();
        mgr.switch_tab(1);
        mgr.prev_tab();
        assert_eq!(mgr.active_index(), 0);
        mgr.prev_tab();
        assert_eq!(mgr.active_index(), 2); // wraps
    }

    #[test]
    fn t_next_tab_single() {
        let mut mgr = TabManager::new(80, 24);
        mgr.next_tab();
        assert_eq!(mgr.active_index(), 0); // no-op with 1 tab
    }

    #[test]
    fn t_prev_tab_single() {
        let mut mgr = TabManager::new(80, 24);
        mgr.prev_tab();
        assert_eq!(mgr.active_index(), 0); // no-op with 1 tab
    }

    // ── switch_to_id ──

    #[test]
    fn t_switch_to_id() {
        let mut mgr = TabManager::new(80, 24);
        mgr.open_tab();
        mgr.open_tab();
        assert!(mgr.switch_to_id(0));
        assert_eq!(mgr.active_index(), 0);
        assert!(mgr.switch_to_id(2));
        assert_eq!(mgr.active_index(), 2);
    }

    #[test]
    fn t_switch_to_id_not_found() {
        let mut mgr = TabManager::new(80, 24);
        assert!(!mgr.switch_to_id(99));
    }

    // ── dirty/title ──

    #[test]
    fn t_mark_active_dirty() {
        let mut mgr = TabManager::new(80, 24);
        assert!(!mgr.active_tab().unwrap().dirty);
        mgr.mark_active_dirty();
        assert!(mgr.active_tab().unwrap().dirty);
    }

    #[test]
    fn t_set_active_title() {
        let mut mgr = TabManager::new(80, 24);
        mgr.set_active_title("my shell");
        assert_eq!(mgr.active_tab().unwrap().title, "my shell");
    }

    #[test]
    fn t_set_title_by_id() {
        let mut mgr = TabManager::new(80, 24);
        mgr.open_tab();
        mgr.switch_tab(0);
        mgr.set_title(1, "tab 1 title");
        assert_eq!(mgr.tabs()[1].title, "tab 1 title");
    }

    #[test]
    fn t_has_dirty_background_tabs() {
        let mut mgr = TabManager::new(80, 24);
        mgr.open_tab();
        mgr.switch_tab(0); // active = 0
        // Tab 1 is in background — mark it dirty
        mgr.tabs_mut_for_test()[1].mark_dirty();
        assert!(mgr.has_dirty_background_tabs());
    }

    #[test]
    fn t_no_dirty_background_tabs() {
        let mgr = TabManager::new(80, 24);
        assert!(!mgr.has_dirty_background_tabs());
    }

    // ── resize ──

    #[test]
    fn t_resize_all() {
        let mut mgr = TabManager::new(80, 24);
        mgr.open_tab();
        mgr.resize_all(120, 40);
        assert_eq!(mgr.default_cols, 120);
        assert_eq!(mgr.default_rows, 40);
        for tab in mgr.tabs() {
            assert_eq!(tab.cols, 120);
            assert_eq!(tab.rows, 40);
        }
    }

    // ── misc ──

    #[test]
    fn t_titles_map() {
        let mut mgr = TabManager::new(80, 24);
        mgr.open_tab_titled("logs");
        mgr.open_tab_titled("vim");
        let map = mgr.titles_map();
        assert_eq!(map.len(), 3);
        assert!(map.contains_key(&0));
        assert!(map.contains_key(&1));
        assert!(map.contains_key(&2));
    }

    #[test]
    fn t_find_by_id() {
        let mgr = TabManager::new(80, 24);
        assert!(mgr.find(0).is_some());
        assert!(mgr.find(99).is_none());
    }

    #[test]
    fn t_active_id() {
        let mut mgr = TabManager::new(80, 24);
        assert_eq!(mgr.active_id(), Some(0));
        mgr.open_tab();
        assert_eq!(mgr.active_id(), Some(1));
    }

    #[test]
    fn t_open_close_open_id_increments() {
        let mut mgr = TabManager::new(80, 24);
        // Tab 0 exists
        mgr.open_tab(); // Tab 1
        mgr.open_tab(); // Tab 2
        mgr.close_tab(2); // Close tab with id=2
        let idx = mgr.open_tab().unwrap(); // Should get id=3 (not reused)
        let new_tab = &mgr.tabs()[idx];
        assert_eq!(new_tab.id, 3);
    }
}
