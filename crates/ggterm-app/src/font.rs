//! Font zoom state for live font size adjustment (P11-A).
//!
//! Tracks the current font size and zoom level relative to the configured
//! base size. Supports increase (Ctrl+=), decrease (Ctrl+-), and reset (Ctrl+0).

/// Default font size if config doesn't specify one.
pub const DEFAULT_FONT_SIZE: f32 = 15.0;

/// Minimum font size (pixels).
pub const MIN_FONT_SIZE: f32 = 6.0;

/// Maximum font size (pixels).
pub const MAX_FONT_SIZE: f32 = 72.0;

/// Font size step per zoom action (pixels).
pub const FONT_ZOOM_STEP: f32 = 1.5;

/// Font zoom state.
#[derive(Debug, Clone)]
pub struct FontZoom {
    /// The base font size from config (before any zoom adjustments).
    base_size: f32,
    /// The current effective font size (after zoom adjustments).
    current_size: f32,
}

impl FontZoom {
    /// Create a new FontZoom with the given base font size.
    pub fn new(base_size: f32) -> Self {
        let size = base_size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        Self {
            base_size: size,
            current_size: size,
        }
    }

    /// Create a FontZoom with the default font size.
    pub fn default_size() -> Self {
        Self::new(DEFAULT_FONT_SIZE)
    }

    /// Get the base (unzoomed) font size.
    pub fn base_size(&self) -> f32 {
        self.base_size
    }

    /// Get the current effective font size.
    pub fn current_size(&self) -> f32 {
        self.current_size
    }

    /// Set a new base font size and reset zoom.
    pub fn set_base_size(&mut self, size: f32) {
        self.base_size = size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        self.current_size = self.base_size;
    }

    /// Increase font size by one step.
    ///
    /// Returns `true` if the size actually changed.
    pub fn zoom_in(&mut self) -> bool {
        let new_size = (self.current_size + FONT_ZOOM_STEP).min(MAX_FONT_SIZE);
        if (new_size - self.current_size).abs() > 0.01 {
            self.current_size = new_size;
            true
        } else {
            false
        }
    }

    /// Decrease font size by one step.
    ///
    /// Returns `true` if the size actually changed.
    pub fn zoom_out(&mut self) -> bool {
        let new_size = (self.current_size - FONT_ZOOM_STEP).max(MIN_FONT_SIZE);
        if (self.current_size - new_size).abs() > 0.01 {
            self.current_size = new_size;
            true
        } else {
            false
        }
    }

    /// Reset font size to the base size.
    ///
    /// Returns `true` if the size actually changed.
    pub fn reset(&mut self) -> bool {
        if (self.current_size - self.base_size).abs() > 0.01 {
            self.current_size = self.base_size;
            true
        } else {
            false
        }
    }

    /// Check if the current size is at the base (no zoom).
    pub fn is_at_base(&self) -> bool {
        (self.current_size - self.base_size).abs() < 0.01
    }

    /// Get the current zoom level relative to base.
    ///
    /// Returns 0.0 at base size, positive when zoomed in, negative when zoomed out.
    pub fn zoom_level(&self) -> f32 {
        (self.current_size - self.base_size) / FONT_ZOOM_STEP
    }
}

impl Default for FontZoom {
    fn default() -> Self {
        Self::default_size()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_clamps_size() {
        let fz = FontZoom::new(3.0);
        assert_eq!(fz.base_size(), MIN_FONT_SIZE);
        assert_eq!(fz.current_size(), MIN_FONT_SIZE);
    }

    #[test]
    fn test_new_clamps_max() {
        let fz = FontZoom::new(100.0);
        assert_eq!(fz.base_size(), MAX_FONT_SIZE);
    }

    #[test]
    fn test_zoom_in() {
        let mut fz = FontZoom::new(15.0);
        assert!(fz.zoom_in());
        assert!((fz.current_size() - 16.5).abs() < 0.01);
    }

    #[test]
    fn test_zoom_out() {
        let mut fz = FontZoom::new(15.0);
        assert!(fz.zoom_out());
        assert!((fz.current_size() - 13.5).abs() < 0.01);
    }

    #[test]
    fn test_reset() {
        let mut fz = FontZoom::new(15.0);
        fz.zoom_in();
        fz.zoom_in();
        assert!(!fz.is_at_base());
        assert!(fz.reset());
        assert!(fz.is_at_base());
        assert!((fz.current_size() - 15.0).abs() < 0.01);
    }

    #[test]
    fn test_zoom_in_at_max() {
        let mut fz = FontZoom::new(MAX_FONT_SIZE);
        assert!(!fz.zoom_in());
        assert_eq!(fz.current_size(), MAX_FONT_SIZE);
    }

    #[test]
    fn test_zoom_out_at_min() {
        let mut fz = FontZoom::new(MIN_FONT_SIZE);
        assert!(!fz.zoom_out());
        assert_eq!(fz.current_size(), MIN_FONT_SIZE);
    }

    #[test]
    fn test_set_base_size_resets() {
        let mut fz = FontZoom::new(15.0);
        fz.zoom_in();
        fz.set_base_size(20.0);
        assert_eq!(fz.base_size(), 20.0);
        assert_eq!(fz.current_size(), 20.0);
        assert!(fz.is_at_base());
    }

    #[test]
    fn test_zoom_level() {
        let mut fz = FontZoom::new(15.0);
        assert_eq!(fz.zoom_level(), 0.0);
        fz.zoom_in();
        assert!((fz.zoom_level() - 1.0).abs() < 0.01);
        fz.zoom_in();
        assert!((fz.zoom_level() - 2.0).abs() < 0.01);
        fz.zoom_out();
        assert!((fz.zoom_level() - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_multiple_zoom_cycles() {
        let mut fz = FontZoom::new(15.0);
        for _ in 0..5 {
            fz.zoom_in();
        }
        for _ in 0..5 {
            fz.zoom_out();
        }
        assert!(fz.is_at_base());
    }

    #[test]
    fn test_default_size() {
        let fz = FontZoom::default_size();
        assert_eq!(fz.base_size(), DEFAULT_FONT_SIZE);
        assert_eq!(fz.current_size(), DEFAULT_FONT_SIZE);
    }

    #[test]
    fn test_reset_no_change() {
        let mut fz = FontZoom::new(15.0);
        assert!(!fz.reset());
    }

    #[test]
    fn test_set_base_size_clamps() {
        let mut fz = FontZoom::new(15.0);
        fz.set_base_size(2.0);
        assert_eq!(fz.base_size(), MIN_FONT_SIZE);
    }

    #[test]
    fn test_set_base_size_clamps_max() {
        let mut fz = FontZoom::new(15.0);
        fz.set_base_size(200.0);
        assert_eq!(fz.base_size(), MAX_FONT_SIZE);
    }
}
