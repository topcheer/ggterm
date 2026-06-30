//! Desktop configuration and resize helpers.
//!
//! Extracted from window.rs for clarity (P14-A).
//!
//! `ConfigManager` import removed — tests that need it live in window.rs.

/// Minimum terminal dimensions in cells.
/// Prevents the window from shrinking to an unusable size.
pub const MIN_COLS: u16 = 10;
pub const MIN_ROWS: u16 = 3;

/// Resize debounce interval (milliseconds).
/// During a window drag-resize, winit fires many `Resized` events.
/// We defer the actual Terminal/PTY resize until 100ms after the last event.
pub const RESIZE_DEBOUNCE_MS: u64 = 100;

/// Duration of the visual bell flash in frames (P11-E).
/// At 60 FPS this is about 250ms (15 frames).
pub const VISUAL_BELL_DURATION_FRAMES: u32 = 15;

// ── P26-G: Window layout spacing constants ───────────────────────

/// Tab bar vertical padding (above and below tab items).
pub const TAB_BAR_PADDING_Y: f32 = 6.0;

/// Tab bar horizontal padding (left and right of tab items).
pub const TAB_BAR_PADDING_X: f32 = 8.0;

/// Padding around the terminal content area (all four sides).
pub const CONTENT_PADDING: f32 = 8.0;

/// Height of the bottom status bar in physical pixels.
pub const STATUS_BAR_HEIGHT: f32 = 24.0;

/// Gap between split panes in physical pixels.
pub const PANE_GAP: f32 = 6.0;

/// Compute terminal cell dimensions (cols, rows) from pixel dimensions.
///
/// `width`/`height` are the window inner size in physical pixels.
/// `cell_width`/`cell_height` are the pixel dimensions of a single cell.
/// The result is clamped to at least `MIN_COLS` x `MIN_ROWS`.
pub fn compute_cell_dimensions(
    width: u32,
    height: u32,
    cell_width: f32,
    cell_height: f32,
) -> (u16, u16) {
    let cols = ((width as f32 / cell_width) as u16).max(MIN_COLS);
    let rows = ((height as f32 / cell_height) as u16).max(MIN_ROWS);
    (cols, rows)
}

/// Configuration for the desktop terminal window.
#[derive(Debug, Clone)]
pub struct DesktopConfig {
    /// Window title.
    pub title: String,
    /// Initial column count.
    pub cols: u16,
    /// Initial row count.
    pub rows: u16,
    /// Cell width in pixels.
    pub cell_width: f32,
    /// Cell height in pixels.
    pub cell_height: f32,
    /// Shell program path. `None` = auto-detect.
    pub shell: Option<String>,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            title: "GGTerm".to_string(),
            cols: 80,
            rows: 24,
            cell_width: 8.0,
            cell_height: 16.0,
            shell: None,
        }
    }
}

impl DesktopConfig {
    /// Set the window title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Set the shell program path.
    pub fn with_shell(mut self, shell: impl Into<String>) -> Self {
        self.shell = Some(shell.into());
        self
    }

    /// Set initial terminal dimensions.
    pub fn with_size(mut self, cols: u16, rows: u16) -> Self {
        self.cols = cols;
        self.rows = rows;
        self
    }

    /// Set cell dimensions in pixels.
    pub fn with_cell_size(mut self, w: f32, h: f32) -> Self {
        self.cell_width = w;
        self.cell_height = h;
        self
    }

    /// Window pixel width = cols * cell_width.
    pub fn window_width(&self) -> u32 {
        (self.cols as f32 * self.cell_width).round() as u32
    }

    /// Window pixel height = rows * cell_height.
    pub fn window_height(&self) -> u32 {
        (self.rows as f32 * self.cell_height).round() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = DesktopConfig::default();
        assert_eq!(cfg.title, "GGTerm");
        assert_eq!(cfg.cols, 80);
        assert_eq!(cfg.rows, 24);
        assert_eq!(cfg.cell_width, 8.0);
        assert_eq!(cfg.cell_height, 16.0);
    }

    #[test]
    fn test_config_builder() {
        let cfg = DesktopConfig::default()
            .with_title("My Terminal")
            .with_size(120, 40)
            .with_cell_size(7.5, 15.5);
        assert_eq!(cfg.title, "My Terminal");
        assert_eq!(cfg.cols, 120);
        assert_eq!(cfg.rows, 40);
        assert_eq!(cfg.cell_width, 7.5);
        assert_eq!(cfg.cell_height, 15.5);
    }

    #[test]
    fn test_window_dimensions_default() {
        let cfg = DesktopConfig::default();
        assert_eq!(cfg.window_width(), 640); // 80 * 8
        assert_eq!(cfg.window_height(), 384); // 24 * 16
    }

    #[test]
    fn test_window_dimensions_custom() {
        let cfg = DesktopConfig::default()
            .with_size(100, 30)
            .with_cell_size(7.5, 15.5);
        assert_eq!(cfg.window_width(), 750); // 100 * 7.5
        assert_eq!(cfg.window_height(), 465); // 30 * 15.5
    }

    #[test]
    fn test_desktop_config_with_shell() {
        let cfg = DesktopConfig::default().with_shell("/bin/bash");
        assert_eq!(cfg.shell.as_deref(), Some("/bin/bash"));
    }

    #[test]
    fn test_desktop_config_shell_default_none() {
        let cfg = DesktopConfig::default();
        assert!(cfg.shell.is_none(), "shell should default to None");
    }

    #[test]
    fn test_cell_size_from_config_applied() {
        let mut desktop_config = DesktopConfig::default();
        assert_eq!(desktop_config.cell_width, 8.0);
        let config_cell_width: u32 = 10;
        if desktop_config.cell_width == 8.0 {
            desktop_config.cell_width = config_cell_width as f32;
        }
        assert_eq!(desktop_config.cell_width, 10.0);
    }

    #[test]
    fn test_cell_size_cli_overrides_config() {
        let mut desktop_config = DesktopConfig::default().with_cell_size(9.5, 19.0);
        let config_cell_width: u32 = 10;
        if desktop_config.cell_width == 8.0 {
            desktop_config.cell_width = config_cell_width as f32;
        }
        assert_eq!(
            desktop_config.cell_width, 9.5,
            "CLI cell_width should be preserved"
        );
    }

    // ── P9-H: Resize computation tests ────────────────────────────────

    #[test]
    fn test_compute_cell_dimensions_basic() {
        let (cols, rows) = compute_cell_dimensions(640, 384, 8.0, 16.0);
        assert_eq!(cols, 80);
        assert_eq!(rows, 24);
    }

    #[test]
    fn test_compute_cell_dimensions_minimum() {
        let (cols, rows) = compute_cell_dimensions(0, 0, 8.0, 16.0);
        assert_eq!(cols, MIN_COLS);
        assert_eq!(rows, MIN_ROWS);
    }

    #[test]
    fn test_compute_cell_dimensions_small_window() {
        let (cols, rows) = compute_cell_dimensions(40, 32, 8.0, 16.0);
        assert_eq!(cols, MIN_COLS);
        assert_eq!(rows, MIN_ROWS);
    }

    #[test]
    fn test_compute_cell_dimensions_just_at_minimum() {
        let (cols, rows) = compute_cell_dimensions(80, 48, 8.0, 16.0);
        assert_eq!(cols, 10);
        assert_eq!(rows, 3);
    }

    #[test]
    fn test_compute_cell_dimensions_large_window() {
        let (cols, rows) = compute_cell_dimensions(3840, 2160, 8.0, 16.0);
        assert_eq!(cols, 480);
        assert_eq!(rows, 135);
    }

    #[test]
    fn test_compute_cell_dimensions_custom_cell_size() {
        let (cols, rows) = compute_cell_dimensions(1200, 720, 12.0, 24.0);
        assert_eq!(cols, 100);
        assert_eq!(rows, 30);
    }

    #[test]
    fn test_compute_cell_dimensions_subpixel_floor() {
        let (cols, _) = compute_cell_dimensions(644, 384, 8.0, 16.0);
        assert_eq!(cols, 80);
    }

    #[test]
    fn test_min_cols_constant() {
        assert_eq!(MIN_COLS, 10);
    }

    #[test]
    fn test_min_rows_constant() {
        assert_eq!(MIN_ROWS, 3);
    }

    #[test]
    fn test_debounce_ms_constant() {
        assert_eq!(RESIZE_DEBOUNCE_MS, 100);
    }

    // ── P26-G: Layout spacing constant tests ────────────────────────

    #[test]
    fn test_tab_bar_padding_y() {
        assert_eq!(TAB_BAR_PADDING_Y, 6.0);
    }

    #[test]
    fn test_tab_bar_padding_x() {
        assert_eq!(TAB_BAR_PADDING_X, 8.0);
    }

    #[test]
    fn test_content_padding() {
        assert_eq!(CONTENT_PADDING, 8.0);
    }

    #[test]
    fn test_status_bar_height() {
        assert_eq!(STATUS_BAR_HEIGHT, 24.0);
    }

    #[test]
    fn test_pane_gap() {
        assert_eq!(PANE_GAP, 6.0);
    }
}
