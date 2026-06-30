//! Session persistence — save and restore tab + pane layouts.
//!
//! Serializes the terminal session layout (tabs, panes, split structure) to
//! `~/.ggterm/session.json` on exit and restores it on startup.
//!
//! # Format
//! ```json
//! {
//!   "version": 1,
//!   "tabs": [
//!     {
//!       "title": "zsh",
//!       "active_pane": 0,
//!       "panes": [
//!         { "shell": "/bin/zsh", "cwd": "/home/user" },
//!         { "shell": "/bin/zsh", "cwd": "/home/user/projects" }
//!       ],
//!       "splits": [
//!         { "id": 0, "orientation": "horizontal", "ratio": 0.5, "left": null, "right": null }
//!       ]
//!     }
//!   ],
//!   "active_tab": 0
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::splits::{PaneId, SplitNode, SplitTree};

// ── Serialization types ──────────────────────────────────────

/// Top-level session file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionData {
    /// Format version for forward compatibility.
    pub version: u32,
    /// Saved tabs.
    pub tabs: Vec<TabData>,
    /// Index of the active tab.
    pub active_tab: usize,
    /// Saved window position (x) in logical pixels.
    #[serde(default)]
    pub window_x: Option<i32>,
    /// Saved window position (y) in logical pixels.
    #[serde(default)]
    pub window_y: Option<i32>,
    /// Saved window width in logical pixels.
    #[serde(default)]
    pub window_width: Option<u32>,
    /// Saved window height in logical pixels.
    #[serde(default)]
    pub window_height: Option<u32>,
}

/// One tab's saved state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TabData {
    /// Tab title.
    pub title: String,
    /// Active pane ID within this tab.
    pub active_pane: PaneId,
    /// Per-pane shell + cwd info.
    pub panes: Vec<PaneData>,
    /// Split layout tree (serialized as a flat list of nodes).
    pub splits: SplitNodeData,
}

/// Per-pane metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PaneData {
    /// Shell program path.
    pub shell: String,
    /// Working directory (best-effort; may be empty).
    #[serde(default)]
    pub cwd: String,
}

/// Serializable representation of a [`SplitNode`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum SplitNodeData {
    /// Leaf node — references a pane by ID.
    Pane { id: PaneId },
    /// Horizontal split (left | right).
    Horizontal {
        left: Box<SplitNodeData>,
        right: Box<SplitNodeData>,
        ratio: f32,
    },
    /// Vertical split (top / bottom).
    Vertical {
        top: Box<SplitNodeData>,
        bottom: Box<SplitNodeData>,
        ratio: f32,
    },
}

impl SplitNodeData {
    /// Convert from a runtime [`SplitNode`] to serializable form.
    pub fn from_node(node: &SplitNode) -> Self {
        match node {
            SplitNode::Pane(id) => Self::Pane { id: *id },
            SplitNode::Horizontal { left, right, ratio } => Self::Horizontal {
                left: Box::new(Self::from_node(left)),
                right: Box::new(Self::from_node(right)),
                ratio: *ratio,
            },
            SplitNode::Vertical { top, bottom, ratio } => Self::Vertical {
                top: Box::new(Self::from_node(top)),
                bottom: Box::new(Self::from_node(bottom)),
                ratio: *ratio,
            },
        }
    }

    /// Reconstruct a runtime [`SplitNode`] from serialized form.
    pub fn to_node(&self) -> SplitNode {
        match self {
            Self::Pane { id } => SplitNode::Pane(*id),
            Self::Horizontal { left, right, ratio } => SplitNode::Horizontal {
                left: Box::new(left.to_node()),
                right: Box::new(right.to_node()),
                ratio: *ratio,
            },
            Self::Vertical { top, bottom, ratio } => SplitNode::Vertical {
                top: Box::new(top.to_node()),
                bottom: Box::new(bottom.to_node()),
                ratio: *ratio,
            },
        }
    }

    /// Collect all pane IDs referenced in this subtree.
    pub fn pane_ids(&self) -> Vec<PaneId> {
        match self {
            Self::Pane { id } => vec![*id],
            Self::Horizontal { left, right, .. } => {
                let mut ids = left.pane_ids();
                ids.extend(right.pane_ids());
                ids
            }
            Self::Vertical { top, bottom, .. } => {
                let mut ids = top.pane_ids();
                ids.extend(bottom.pane_ids());
                ids
            }
        }
    }
}

// ── File I/O ─────────────────────────────────────────────────

/// Default session file path: `~/.ggterm/session.json`.
pub fn session_file_path() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| Path::new(&h).join(".ggterm").join("session.json"))
}

/// Save session data to disk as JSON.
///
/// Creates the parent directory if it doesn't exist.
pub fn save_session(data: &SessionData) -> Result<(), SessionError> {
    let path = session_file_path().ok_or(SessionError::NoHomeDir)?;
    save_to_path(data, &path)
}

/// Save session data to a specific path (used by tests).
pub fn save_to_path(data: &SessionData, path: &Path) -> Result<(), SessionError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(data)?;
    std::fs::write(path, json + "\n")?;
    log::info!("Session saved to {}", path.display());
    Ok(())
}

/// Load session data from the default path.
///
/// Returns `Ok(None)` if the file doesn't exist (clean start).
pub fn load_session() -> Result<Option<SessionData>, SessionError> {
    let path = match session_file_path() {
        Some(p) => p,
        None => return Ok(None),
    };
    load_from_path(&path)
}

/// Load session data from a specific path (used by tests).
///
/// Returns `Ok(None)` if the file doesn't exist or is empty.
pub fn load_from_path(path: &Path) -> Result<Option<SessionData>, SessionError> {
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(path)?;
    if contents.trim().is_empty() {
        return Ok(None);
    }
    let data: SessionData = serde_json::from_str(&contents)?;
    log::info!("Session loaded from {}", path.display());
    Ok(Some(data))
}

/// Delete the session file (e.g., after a successful restore).
pub fn clear_session() -> Result<(), SessionError> {
    let path = session_file_path().ok_or(SessionError::NoHomeDir)?;
    clear_at_path(&path)
}

/// Delete session file at a specific path (used by tests).
pub fn clear_at_path(path: &Path) -> Result<(), SessionError> {
    if path.exists() {
        std::fs::remove_file(path)?;
        log::info!("Session file cleared");
    }
    Ok(())
}

// ── Error type ───────────────────────────────────────────────

/// Errors that can occur during session save/load.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// `$HOME` not set.
    #[error("HOME environment variable not set")]
    NoHomeDir,
    /// File I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ── Builders for DesktopApp integration ──────────────────────

/// Description of a single pane that needs to be created.
///
/// This is what `DesktopApp` uses when restoring a session — it tells
/// the caller which shell to spawn and in what order to build splits.
#[derive(Debug, Clone, PartialEq)]
pub struct PaneSpec {
    /// Shell program path for this pane.
    pub shell: String,
    /// Working directory (best-effort).
    pub cwd: String,
}

/// Description of a tab that needs to be created during session restore.
#[derive(Debug, Clone, PartialEq)]
pub struct TabSpec {
    /// Tab title.
    pub title: String,
    /// Pane specs indexed by PaneId.
    pub panes: Vec<PaneSpec>,
    /// Split layout tree.
    pub splits: SplitNodeData,
    /// Which pane is active.
    pub active_pane: PaneId,
}

/// A complete session restore plan.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionPlan {
    /// Tabs to create.
    pub tabs: Vec<TabSpec>,
    /// Which tab is active.
    pub active_tab: usize,
}

impl SessionPlan {
    /// Build a restore plan from loaded session data.
    ///
    /// The plan describes what panes to create and how to arrange them.
    /// The caller (`DesktopApp`) is responsible for actually spawning PTYs
    /// and constructing `TabSession` objects.
    pub fn from_data(data: &SessionData) -> Self {
        let tabs = data
            .tabs
            .iter()
            .map(|tab| TabSpec {
                title: tab.title.clone(),
                panes: tab
                    .panes
                    .iter()
                    .map(|p| PaneSpec {
                        shell: p.shell.clone(),
                        cwd: p.cwd.clone(),
                    })
                    .collect(),
                splits: tab.splits.clone(),
                active_pane: tab.active_pane,
            })
            .collect();
        Self {
            tabs,
            active_tab: data.active_tab.min(data.tabs.len().saturating_sub(1)),
        }
    }
}

/// Capture a `SplitTree` into serializable form.
///
/// This is a convenience function for `DesktopApp` to call during save.
pub fn capture_split_tree(tree: &SplitTree) -> SplitNodeData {
    SplitNodeData::from_node(tree.root())
}

/// Reconstruct a `SplitTree` from serialized form.
///
/// Returns a new tree matching the saved structure. The caller must
/// ensure that pane IDs in the tree correspond to valid pane indices.
pub fn restore_split_tree(data: &SplitNodeData, active: PaneId) -> SplitTree {
    let root = data.to_node();
    SplitTree::from_parts(root, active)
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── SplitNodeData round-trip tests ───────────────────────

    #[test]
    fn t_split_data_single_pane_roundtrip() {
        let node = SplitNode::Pane(0);
        let data = SplitNodeData::from_node(&node);
        assert_eq!(data.to_node(), node);
    }

    #[test]
    fn t_split_data_horizontal_roundtrip() {
        let node = SplitNode::Horizontal {
            left: Box::new(SplitNode::Pane(0)),
            right: Box::new(SplitNode::Pane(1)),
            ratio: 0.5,
        };
        let data = SplitNodeData::from_node(&node);
        assert_eq!(data.to_node(), node);
    }

    #[test]
    fn t_split_data_vertical_roundtrip() {
        let node = SplitNode::Vertical {
            top: Box::new(SplitNode::Pane(0)),
            bottom: Box::new(SplitNode::Pane(1)),
            ratio: 0.3,
        };
        let data = SplitNodeData::from_node(&node);
        assert_eq!(data.to_node(), node);
    }

    #[test]
    fn t_split_data_nested_roundtrip() {
        // [0 | [1 / 2]]
        let node = SplitNode::Horizontal {
            left: Box::new(SplitNode::Pane(0)),
            right: Box::new(SplitNode::Vertical {
                top: Box::new(SplitNode::Pane(1)),
                bottom: Box::new(SplitNode::Pane(2)),
                ratio: 0.5,
            }),
            ratio: 0.5,
        };
        let data = SplitNodeData::from_node(&node);
        assert_eq!(data.to_node(), node);
    }

    #[test]
    fn t_split_data_deep_nested_roundtrip() {
        // [[0 | 1] / [2 | 3]]
        let node = SplitNode::Vertical {
            top: Box::new(SplitNode::Horizontal {
                left: Box::new(SplitNode::Pane(0)),
                right: Box::new(SplitNode::Pane(1)),
                ratio: 0.5,
            }),
            bottom: Box::new(SplitNode::Horizontal {
                left: Box::new(SplitNode::Pane(2)),
                right: Box::new(SplitNode::Pane(3)),
                ratio: 0.5,
            }),
            ratio: 0.5,
        };
        let data = SplitNodeData::from_node(&node);
        assert_eq!(data.to_node(), node);
    }

    // ── pane_ids tests ───────────────────────────────────────

    #[test]
    fn t_split_data_pane_ids_single() {
        let data = SplitNodeData::Pane { id: 5 };
        assert_eq!(data.pane_ids(), vec![5]);
    }

    #[test]
    fn t_split_data_pane_ids_nested() {
        let data = SplitNodeData::Horizontal {
            left: Box::new(SplitNodeData::Pane { id: 0 }),
            right: Box::new(SplitNodeData::Vertical {
                top: Box::new(SplitNodeData::Pane { id: 1 }),
                bottom: Box::new(SplitNodeData::Pane { id: 2 }),
                ratio: 0.5,
            }),
            ratio: 0.5,
        };
        assert_eq!(data.pane_ids(), vec![0, 1, 2]);
    }

    // ── JSON round-trip tests ────────────────────────────────

    #[test]
    fn t_json_single_tab_single_pane() {
        let data = SessionData {
            version: 1,
            tabs: vec![TabData {
                title: "zsh".into(),
                active_pane: 0,
                panes: vec![PaneData {
                    shell: "/bin/zsh".into(),
                    cwd: "/home/user".into(),
                }],
                splits: SplitNodeData::Pane { id: 0 },
            }],
            active_tab: 0,
            window_x: None,
            window_y: None,
            window_width: None,
            window_height: None,
        };
        let json = serde_json::to_string(&data).unwrap();
        let restored: SessionData = serde_json::from_str(&json).unwrap();
        assert_eq!(data, restored);
    }

    #[test]
    fn t_json_multi_tab_multi_pane() {
        let data = SessionData {
            version: 1,
            tabs: vec![
                TabData {
                    title: "tab1".into(),
                    active_pane: 0,
                    panes: vec![
                        PaneData {
                            shell: "/bin/zsh".into(),
                            cwd: "/home".into(),
                        },
                        PaneData {
                            shell: "/bin/bash".into(),
                            cwd: "/tmp".into(),
                        },
                    ],
                    splits: SplitNodeData::Horizontal {
                        left: Box::new(SplitNodeData::Pane { id: 0 }),
                        right: Box::new(SplitNodeData::Pane { id: 1 }),
                        ratio: 0.5,
                    },
                },
                TabData {
                    title: "tab2".into(),
                    active_pane: 0,
                    panes: vec![PaneData {
                        shell: "/bin/fish".into(),
                        cwd: "".into(),
                    }],
                    splits: SplitNodeData::Pane { id: 0 },
                },
            ],
            active_tab: 1,
            window_x: None,
            window_y: None,
            window_width: None,
            window_height: None,
        };
        let json = serde_json::to_string_pretty(&data).unwrap();
        let restored: SessionData = serde_json::from_str(&json).unwrap();
        assert_eq!(data, restored);
    }

    #[test]
    fn t_json_nested_splits() {
        let data = SessionData {
            version: 1,
            tabs: vec![TabData {
                title: "dev".into(),
                active_pane: 2,
                panes: vec![
                    PaneData {
                        shell: "/bin/zsh".into(),
                        cwd: "/proj".into(),
                    },
                    PaneData {
                        shell: "/bin/zsh".into(),
                        cwd: "/proj".into(),
                    },
                    PaneData {
                        shell: "/bin/zsh".into(),
                        cwd: "/proj/logs".into(),
                    },
                ],
                splits: SplitNodeData::Horizontal {
                    left: Box::new(SplitNodeData::Pane { id: 0 }),
                    right: Box::new(SplitNodeData::Vertical {
                        top: Box::new(SplitNodeData::Pane { id: 1 }),
                        bottom: Box::new(SplitNodeData::Pane { id: 2 }),
                        ratio: 0.6,
                    }),
                    ratio: 0.4,
                },
            }],
            active_tab: 0,
            window_x: None,
            window_y: None,
            window_width: None,
            window_height: None,
        };
        let json = serde_json::to_string(&data).unwrap();
        let restored: SessionData = serde_json::from_str(&json).unwrap();
        assert_eq!(data, restored);
    }

    #[test]
    fn t_json_empty_cwd_default() {
        // Missing "cwd" field should default to empty string.
        let json = r#"{"shell": "/bin/zsh"}"#;
        let pane: PaneData = serde_json::from_str(json).unwrap();
        assert_eq!(pane.cwd, "");
    }

    // ── SessionPlan tests ────────────────────────────────────

    #[test]
    fn t_session_plan_from_data() {
        let data = SessionData {
            version: 1,
            tabs: vec![TabData {
                title: "test".into(),
                active_pane: 1,
                panes: vec![
                    PaneData {
                        shell: "/bin/zsh".into(),
                        cwd: "/a".into(),
                    },
                    PaneData {
                        shell: "/bin/zsh".into(),
                        cwd: "/b".into(),
                    },
                ],
                splits: SplitNodeData::Horizontal {
                    left: Box::new(SplitNodeData::Pane { id: 0 }),
                    right: Box::new(SplitNodeData::Pane { id: 1 }),
                    ratio: 0.5,
                },
            }],
            active_tab: 0,
            window_x: None,
            window_y: None,
            window_width: None,
            window_height: None,
        };
        let plan = SessionPlan::from_data(&data);
        assert_eq!(plan.tabs.len(), 1);
        assert_eq!(plan.active_tab, 0);
        assert_eq!(plan.tabs[0].panes.len(), 2);
        assert_eq!(plan.tabs[0].panes[0].shell, "/bin/zsh");
        assert_eq!(plan.tabs[0].panes[1].cwd, "/b");
        assert_eq!(plan.tabs[0].active_pane, 1);
    }

    #[test]
    fn t_session_plan_active_tab_clamped() {
        let data = SessionData {
            version: 1,
            tabs: vec![TabData {
                title: "only".into(),
                active_pane: 0,
                panes: vec![PaneData {
                    shell: "/bin/sh".into(),
                    cwd: "".into(),
                }],
                splits: SplitNodeData::Pane { id: 0 },
            }],
            active_tab: 99, // out of bounds
            window_x: None,
            window_y: None,
            window_width: None,
            window_height: None,
        };
        let plan = SessionPlan::from_data(&data);
        assert_eq!(plan.active_tab, 0); // clamped
    }

    #[test]
    fn t_session_plan_empty_tabs() {
        let data = SessionData {
            version: 1,
            tabs: vec![],
            active_tab: 0,
            window_x: None,
            window_y: None,
            window_width: None,
            window_height: None,
        };
        let plan = SessionPlan::from_data(&data);
        assert!(plan.tabs.is_empty());
    }

    // ── SplitTree capture/restore tests ──────────────────────

    #[test]
    fn t_capture_split_tree_single() {
        let tree = SplitTree::new(0);
        let data = capture_split_tree(&tree);
        assert_eq!(data, SplitNodeData::Pane { id: 0 });
    }

    #[test]
    fn t_restore_split_tree_single() {
        let data = SplitNodeData::Pane { id: 0 };
        let tree = restore_split_tree(&data, 0);
        assert_eq!(tree.active(), 0);
        assert_eq!(tree.pane_ids(), vec![0]);
    }

    #[test]
    fn t_restore_split_tree_multi() {
        let data = SplitNodeData::Horizontal {
            left: Box::new(SplitNodeData::Pane { id: 0 }),
            right: Box::new(SplitNodeData::Pane { id: 1 }),
            ratio: 0.5,
        };
        let tree = restore_split_tree(&data, 1);
        assert_eq!(tree.active(), 1);
        assert_eq!(tree.pane_ids(), vec![0, 1]);
        assert_eq!(tree.pane_count(), 2);
    }

    #[test]
    fn t_restore_split_tree_next_id() {
        let data = SplitNodeData::Horizontal {
            left: Box::new(SplitNodeData::Pane { id: 0 }),
            right: Box::new(SplitNodeData::Vertical {
                top: Box::new(SplitNodeData::Pane { id: 1 }),
                bottom: Box::new(SplitNodeData::Pane { id: 4 }),
                ratio: 0.5,
            }),
            ratio: 0.5,
        };
        let mut tree = restore_split_tree(&data, 0);
        // next_id should be max_id + 1 = 5
        let new_id = tree.alloc_id();
        assert_eq!(new_id, 5);
    }

    // ── File I/O tests (use path-based functions, no env) ────

    #[test]
    fn t_load_nonexistent_returns_none() {
        let path = tempdir().join("session.json");
        let result = load_from_path(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn t_save_and_load_roundtrip() {
        let path = tempdir().join("session.json");
        let data = SessionData {
            version: 1,
            tabs: vec![TabData {
                title: "test".into(),
                active_pane: 0,
                panes: vec![PaneData {
                    shell: "/bin/zsh".into(),
                    cwd: "/tmp".into(),
                }],
                splits: SplitNodeData::Pane { id: 0 },
            }],
            active_tab: 0,
            window_x: None,
            window_y: None,
            window_width: None,
            window_height: None,
        };

        save_to_path(&data, &path).unwrap();
        let loaded = load_from_path(&path).unwrap().unwrap();
        assert_eq!(data, loaded);

        clear_at_path(&path).unwrap();
        assert!(load_from_path(&path).unwrap().is_none());
    }

    #[test]
    fn t_save_creates_parent_dir() {
        let dir = tempdir();
        let path = dir.join("sub").join("session.json");
        assert!(!path.exists());

        let data = SessionData {
            version: 1,
            tabs: vec![],
            active_tab: 0,
            window_x: None,
            window_y: None,
            window_width: None,
            window_height: None,
        };
        save_to_path(&data, &path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn t_load_empty_file_returns_none() {
        let path = tempdir().join("session.json");
        std::fs::write(&path, "").unwrap();
        let result = load_from_path(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn t_load_corrupt_json_errors() {
        let path = tempdir().join("session.json");
        std::fs::write(&path, "{ broken").unwrap();
        let result = load_from_path(&path);
        assert!(result.is_err());
    }

    #[test]
    fn t_clear_nonexistent_is_ok() {
        let path = tempdir().join("session.json");
        assert!(clear_at_path(&path).is_ok());
    }

    // ── Helpers ──────────────────────────────────────────────

    fn tempdir() -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir =
            std::env::temp_dir().join(format!("ggterm-test-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
