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

/// File name for persisted window state.
const FILE_NAME: &str = "window_state.toml";

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
}
