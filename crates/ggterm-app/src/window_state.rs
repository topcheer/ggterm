//! Window geometry persistence — saves and restores window position/size
//! across restarts, independent of session restore.
//!
//! State is stored in `~/.ggterm/window_state.toml`.

use serde::{Deserialize, Serialize};

/// Persisted window geometry.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Get the path to the window state file.
fn state_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(
        std::path::PathBuf::from(home)
            .join(".ggterm")
            .join(FILE_NAME),
    )
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
