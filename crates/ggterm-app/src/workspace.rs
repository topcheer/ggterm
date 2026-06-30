//! P28-D: Workspace manager — named, switchable collections of tabs.
//!
//! Each workspace is a named session group. Users can create custom
//! workspaces (e.g. "project-a", "project-b") and switch between them
//! quickly. Each workspace preserves its own set of tabs.

use std::collections::HashMap;

/// Maximum number of workspaces.
const MAX_WORKSPACES: usize = 20;

/// Default workspace name.
pub const DEFAULT_WORKSPACE: &str = "default";

/// A workspace holds tab metadata for session save/restore.
#[derive(Debug, Clone)]
pub struct Workspace {
    /// Unique name of this workspace.
    pub name: String,
    /// Tab titles in this workspace (for display).
    pub tab_titles: Vec<String>,
    /// Whether this workspace is currently active.
    pub active: bool,
}

impl Workspace {
    /// Create a new workspace with the given name.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            tab_titles: Vec::new(),
            active: false,
        }
    }
}

/// Central workspace manager.
#[derive(Debug)]
pub struct WorkspaceManager {
    /// All workspaces by name.
    workspaces: HashMap<String, Workspace>,
    /// Order of workspace names (for display/cycling).
    order: Vec<String>,
    /// Currently active workspace name.
    active: String,
}

impl Default for WorkspaceManager {
    fn default() -> Self {
        let mut mgr = Self {
            workspaces: HashMap::new(),
            order: Vec::new(),
            active: DEFAULT_WORKSPACE.to_string(),
        };
        mgr.add_workspace(DEFAULT_WORKSPACE);
        mgr.set_active(DEFAULT_WORKSPACE);
        mgr
    }
}

impl WorkspaceManager {
    /// Create new workspace manager with a default workspace.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a new workspace. Returns false if name already exists or limit reached.
    pub fn add_workspace(&mut self, name: &str) -> bool {
        if self.workspaces.contains_key(name) {
            return false;
        }
        if self.workspaces.len() >= MAX_WORKSPACES {
            return false;
        }
        let ws = Workspace::new(name);
        self.workspaces.insert(name.to_string(), ws);
        self.order.push(name.to_string());
        true
    }

    /// Remove a workspace. Cannot remove if it's the only one or the default.
    pub fn remove_workspace(&mut self, name: &str) -> bool {
        if name == DEFAULT_WORKSPACE {
            return false;
        }
        if self.workspaces.len() <= 1 {
            return false;
        }
        if self.workspaces.remove(name).is_none() {
            return false;
        }
        self.order.retain(|n| n != name);
        if self.active == name {
            self.active = DEFAULT_WORKSPACE.to_string();
            if let Some(ws) = self.workspaces.get_mut(&self.active) {
                ws.active = true;
            }
        }
        true
    }

    /// Set the active workspace. Returns false if not found.
    pub fn set_active(&mut self, name: &str) -> bool {
        if !self.workspaces.contains_key(name) {
            return false;
        }
        // Deactivate all
        for ws in self.workspaces.values_mut() {
            ws.active = false;
        }
        // Activate the target
        if let Some(ws) = self.workspaces.get_mut(name) {
            ws.active = true;
        }
        self.active = name.to_string();
        true
    }

    /// Get the active workspace name.
    pub fn active_name(&self) -> &str {
        &self.active
    }

    /// Get the active workspace.
    pub fn active_workspace(&self) -> Option<&Workspace> {
        self.workspaces.get(&self.active)
    }

    /// Get the active workspace mutably.
    pub fn active_workspace_mut(&mut self) -> Option<&mut Workspace> {
        self.workspaces.get_mut(&self.active)
    }

    /// List all workspace names in order.
    pub fn names(&self) -> &[String] {
        &self.order
    }

    /// Get all workspaces.
    pub fn workspaces(&self) -> impl Iterator<Item = &Workspace> {
        self.order.iter().filter_map(|n| self.workspaces.get(n))
    }

    /// Number of workspaces.
    pub fn len(&self) -> usize {
        self.workspaces.len()
    }

    /// Whether there are no workspaces (should never happen — always has default).
    pub fn is_empty(&self) -> bool {
        self.workspaces.is_empty()
    }

    /// Cycle to the next workspace. Returns the new active name.
    pub fn cycle_next(&mut self) -> &str {
        let current_idx = self
            .order
            .iter()
            .position(|n| n == &self.active)
            .unwrap_or(0);
        let next_idx = (current_idx + 1) % self.order.len();
        let next_name = self.order[next_idx].clone();
        self.set_active(&next_name);
        &self.order[next_idx]
    }

    /// Cycle to the previous workspace.
    pub fn cycle_prev(&mut self) -> &str {
        let current_idx = self
            .order
            .iter()
            .position(|n| n == &self.active)
            .unwrap_or(0);
        let prev_idx = if current_idx == 0 {
            self.order.len() - 1
        } else {
            current_idx - 1
        };
        let prev_name = self.order[prev_idx].clone();
        self.set_active(&prev_name);
        &self.order[prev_idx]
    }

    /// Rename a workspace. Returns false if not found or new name exists.
    pub fn rename(&mut self, old_name: &str, new_name: &str) -> bool {
        if old_name == DEFAULT_WORKSPACE {
            return false; // Can't rename default
        }
        if self.workspaces.contains_key(new_name) {
            return false;
        }
        if let Some(mut ws) = self.workspaces.remove(old_name) {
            ws.name = new_name.to_string();
            self.workspaces.insert(new_name.to_string(), ws);
            // Update order
            if let Some(pos) = self.order.iter().position(|n| n == old_name) {
                self.order[pos] = new_name.to_string();
            }
            if self.active == old_name {
                self.active = new_name.to_string();
            }
            true
        } else {
            false
        }
    }

    /// Update tab titles for the active workspace.
    pub fn update_tab_titles(&mut self, titles: Vec<String>) {
        if let Some(ws) = self.workspaces.get_mut(&self.active) {
            ws.tab_titles = titles;
        }
    }

    /// Get the index of the active workspace (for display).
    pub fn active_index(&self) -> usize {
        self.order
            .iter()
            .position(|n| n == &self.active)
            .unwrap_or(0)
    }

    /// Get a workspace by name.
    pub fn get(&self, name: &str) -> Option<&Workspace> {
        self.workspaces.get(name)
    }
}

/// Generate a default workspace name based on index.
pub fn default_workspace_name(index: usize) -> String {
    if index == 0 {
        DEFAULT_WORKSPACE.to_string()
    } else {
        format!("workspace-{}", index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_default_workspace_exists() {
        let mgr = WorkspaceManager::new();
        assert_eq!(mgr.len(), 1);
        assert_eq!(mgr.active_name(), DEFAULT_WORKSPACE);
    }

    #[test]
    fn t_add_workspace() {
        let mut mgr = WorkspaceManager::new();
        assert!(mgr.add_workspace("project-a"));
        assert_eq!(mgr.len(), 2);
        assert!(mgr.names().contains(&"project-a".to_string()));
    }

    #[test]
    fn t_add_duplicate_fails() {
        let mut mgr = WorkspaceManager::new();
        assert!(!mgr.add_workspace(DEFAULT_WORKSPACE)); // already exists
    }

    #[test]
    fn t_add_max_workspaces() {
        let mut mgr = WorkspaceManager::new();
        for i in 0..MAX_WORKSPACES - 1 {
            assert!(mgr.add_workspace(&format!("ws{}", i)));
        }
        assert_eq!(mgr.len(), MAX_WORKSPACES);
        assert!(!mgr.add_workspace("overflow")); // should fail
    }

    #[test]
    fn t_remove_workspace() {
        let mut mgr = WorkspaceManager::new();
        mgr.add_workspace("temp");
        assert!(mgr.remove_workspace("temp"));
        assert_eq!(mgr.len(), 1);
    }

    #[test]
    fn t_remove_default_fails() {
        let mut mgr = WorkspaceManager::new();
        assert!(!mgr.remove_workspace(DEFAULT_WORKSPACE));
    }

    #[test]
    fn t_remove_only_workspace_fails() {
        let mut mgr = WorkspaceManager::new();
        assert!(!mgr.remove_workspace(DEFAULT_WORKSPACE));
    }

    #[test]
    fn t_remove_active_switches_to_default() {
        let mut mgr = WorkspaceManager::new();
        mgr.add_workspace("temp");
        mgr.set_active("temp");
        assert!(mgr.remove_workspace("temp"));
        assert_eq!(mgr.active_name(), DEFAULT_WORKSPACE);
    }

    #[test]
    fn t_set_active() {
        let mut mgr = WorkspaceManager::new();
        mgr.add_workspace("test");
        assert!(mgr.set_active("test"));
        assert_eq!(mgr.active_name(), "test");
        assert!(mgr.active_workspace().unwrap().active);
    }

    #[test]
    fn t_set_active_nonexistent_fails() {
        let mut mgr = WorkspaceManager::new();
        assert!(!mgr.set_active("nonexistent"));
    }

    #[test]
    fn t_cycle_next() {
        let mut mgr = WorkspaceManager::new();
        mgr.add_workspace("ws1");
        mgr.add_workspace("ws2");
        assert_eq!(mgr.active_name(), DEFAULT_WORKSPACE);
        mgr.cycle_next();
        assert_eq!(mgr.active_name(), "ws1");
        mgr.cycle_next();
        assert_eq!(mgr.active_name(), "ws2");
        mgr.cycle_next();
        assert_eq!(mgr.active_name(), DEFAULT_WORKSPACE); // wraps
    }

    #[test]
    fn t_cycle_prev() {
        let mut mgr = WorkspaceManager::new();
        mgr.add_workspace("ws1");
        mgr.cycle_prev(); // wraps to last
        assert_eq!(mgr.active_name(), "ws1");
    }

    #[test]
    fn t_rename() {
        let mut mgr = WorkspaceManager::new();
        mgr.add_workspace("temp");
        assert!(mgr.rename("temp", "renamed"));
        assert!(mgr.get("temp").is_none());
        assert!(mgr.get("renamed").is_some());
    }

    #[test]
    fn t_rename_default_fails() {
        let mut mgr = WorkspaceManager::new();
        assert!(!mgr.rename(DEFAULT_WORKSPACE, "new-default"));
    }

    #[test]
    fn t_rename_to_existing_fails() {
        let mut mgr = WorkspaceManager::new();
        mgr.add_workspace("a");
        mgr.add_workspace("b");
        assert!(!mgr.rename("a", "b"));
    }

    #[test]
    fn t_rename_active() {
        let mut mgr = WorkspaceManager::new();
        mgr.add_workspace("active-ws");
        mgr.set_active("active-ws");
        mgr.rename("active-ws", "renamed-active");
        assert_eq!(mgr.active_name(), "renamed-active");
    }

    #[test]
    fn t_update_tab_titles() {
        let mut mgr = WorkspaceManager::new();
        mgr.update_tab_titles(vec!["tab1".to_string(), "tab2".to_string()]);
        let ws = mgr.active_workspace().unwrap();
        assert_eq!(ws.tab_titles.len(), 2);
    }

    #[test]
    fn t_active_index() {
        let mut mgr = WorkspaceManager::new();
        mgr.add_workspace("ws1");
        mgr.add_workspace("ws2");
        assert_eq!(mgr.active_index(), 0); // default
        mgr.set_active("ws2");
        assert_eq!(mgr.active_index(), 2);
    }

    #[test]
    fn t_default_workspace_name() {
        assert_eq!(default_workspace_name(0), DEFAULT_WORKSPACE);
        assert_eq!(default_workspace_name(1), "workspace-1");
        assert_eq!(default_workspace_name(5), "workspace-5");
    }

    #[test]
    fn t_workspaces_iterator() {
        let mut mgr = WorkspaceManager::new();
        mgr.add_workspace("a");
        mgr.add_workspace("b");
        let names: Vec<_> = mgr.workspaces().map(|w| w.name.clone()).collect();
        assert_eq!(names, vec!["default", "a", "b"]);
    }
}
