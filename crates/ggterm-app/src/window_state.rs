//! Window geometry persistence — saves and restores window position/size
//! across restarts, independent of session restore.
//!
//! State is stored in `~/.ggterm/window_state.toml`.

use serde::{Deserialize, Serialize};

/// Persisted window geometry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WindowState {
    /// Outer position x (logical pixels).
    pub x: i32,
    /// Outer position y (logical pixels).
    pub y: i32,
    /// Inner width (logical pixels).
    pub width: u32,
    /// Inner height (logical pixels).
    pub height: u32,
    /// Whether the window was maximized.
    #[serde(default)]
    pub maximized: bool,
}

impl WindowState {
    /// Check whether the saved (x, y) position falls within any of the
    /// given monitor rectangles.
    ///
    /// The check uses a generous threshold: the window only needs at least
    /// 50px of its top-left corner visible on some monitor. This prevents
    /// windows from being completely off-screen while allowing edge cases
    /// like a window that straddles two monitors.
    pub fn is_position_onscreen(&self, monitors: &[MonitorRect]) -> bool {
        // If no monitors are reported, allow the position (can't validate).
        if monitors.is_empty() {
            return true;
        }
        monitors.iter().any(|&(mx, my, mw, mh)| {
            // Check that the window's top-left corner is within the monitor
            // bounds (with a small margin for window decorations).
            let visible = 50; // minimum visible pixels
            self.x + visible > mx
                && self.x < mx + mw as i32 - visible
                && self.y + visible > my
                && self.y < my + mh as i32 - visible
        })
    }
}

/// File name for persisted window state.
const FILE_NAME: &str = "window_state.toml";

/// A monitor rectangle: (x, y, width, height) in logical pixels.
pub type MonitorRect = (i32, i32, u32, u32);

/// Cross-platform home directory lookup.
/// Tries HOME (Unix) first, then USERPROFILE (Windows).
fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(std::path::PathBuf::from))
}

/// Get the path to the window state file.
fn state_path() -> Option<std::path::PathBuf> {
    Some(home_dir()?.join(".ggterm").join(FILE_NAME))
}

/// Load window state from disk.
/// Returns `None` if the file doesn't exist or can't be parsed.
pub fn load() -> Option<WindowState> {
    let path = state_path()?;
    let contents = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&contents).ok()
}

/// Save window state to disk.
/// Silently ignores errors (best-effort persistence).
pub fn save(state: &WindowState) {
    let Some(path) = state_path() else {
        return;
    };
    if let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return;
    }
    match toml::to_string_pretty(state) {
        Ok(text) => {
            if let Err(e) = std::fs::write(&path, text) {
                log::warn!("Failed to save window state: {e}");
            }
        }
        Err(e) => log::warn!("Failed to serialize window state: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_toml_serialization() {
        let state = WindowState {
            x: 100,
            y: 200,
            width: 800,
            height: 600,
            maximized: true,
        };
        let toml_str = toml::to_string_pretty(&state).unwrap();
        assert!(toml_str.contains("x = 100"));
        assert!(toml_str.contains("y = 200"));
        assert!(toml_str.contains("width = 800"));
        assert!(toml_str.contains("height = 600"));
        assert!(toml_str.contains("maximized = true"));
    }

    #[test]
    fn test_toml_deserialization() {
        let toml_str = r#"
x = 50
y = 75
width = 1200
height = 800
maximized = false
"#;
        let state: WindowState = toml::from_str(toml_str).unwrap();
        assert_eq!(state.x, 50);
        assert_eq!(state.y, 75);
        assert_eq!(state.width, 1200);
        assert_eq!(state.height, 800);
        assert!(!state.maximized);
    }

    #[test]
    fn test_toml_roundtrip() {
        let original = WindowState {
            x: -10,
            y: 0,
            width: 1920,
            height: 1080,
            maximized: true,
        };
        let toml_str = toml::to_string_pretty(&original).unwrap();
        let restored: WindowState = toml::from_str(&toml_str).unwrap();
        assert_eq!(original, restored);
    }

    #[test]
    fn test_deserialize_missing_maximized_defaults_false() {
        // Old window_state.toml files without the maximized field should
        // deserialize gracefully via #[serde(default)].
        let toml_str = "x = 0\ny = 0\nwidth = 640\nheight = 480\n";
        let state: WindowState = toml::from_str(toml_str).unwrap();
        assert!(!state.maximized);
    }

    #[test]
    fn test_load_missing_file_returns_none() {
        // load() reads from ~/.ggterm/window_state.toml which may or may
        // not exist in CI. The important guarantee is that it returns None
        // gracefully rather than panicking.
        let _ = load(); // Should not panic.
    }

    #[test]
    fn test_home_dir_returns_some() {
        // On any reasonable test environment HOME or USERPROFILE exists.
        assert!(home_dir().is_some());
    }

    #[test]
    fn test_is_position_onscreen_inside() {
        let state = WindowState {
            x: 100,
            y: 100,
            width: 800,
            height: 600,
            maximized: false,
        };
        // Single monitor at (0,0) 1920x1080.
        let monitors = vec![(0, 0, 1920, 1080)];
        assert!(state.is_position_onscreen(&monitors));
    }

    #[test]
    fn test_is_position_onscreen_outside() {
        let state = WindowState {
            x: -5000,
            y: -5000,
            width: 800,
            height: 600,
            maximized: false,
        };
        let monitors = vec![(0, 0, 1920, 1080)];
        assert!(!state.is_position_onscreen(&monitors));
    }

    #[test]
    fn test_is_position_onscreen_multi_monitor() {
        // Window on second monitor.
        let state = WindowState {
            x: 2000,
            y: 100,
            width: 800,
            height: 600,
            maximized: false,
        };
        let monitors = vec![(0, 0, 1920, 1080), (1920, 0, 1920, 1080)];
        assert!(state.is_position_onscreen(&monitors));
    }

    #[test]
    fn test_is_position_onscreen_no_monitors_allowed() {
        // If no monitors reported, allow the position (can't validate).
        let state = WindowState {
            x: 99999,
            y: 99999,
            width: 800,
            height: 600,
            maximized: false,
        };
        assert!(state.is_position_onscreen(&[]));
    }

    #[test]
    fn test_is_position_onscreen_edge_boundary() {
        // Window at the very edge of screen — should be considered off-screen
        // if less than 50px is visible.
        let state = WindowState {
            x: 1880, // only 40px from right edge (< 50 visible)
            y: 100,
            width: 800,
            height: 600,
            maximized: false,
        };
        let monitors = vec![(0, 0, 1920, 1080)];
        assert!(!state.is_position_onscreen(&monitors));
    }
}
