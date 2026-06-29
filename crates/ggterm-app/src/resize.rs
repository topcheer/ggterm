//! Terminal resize calculation and debounce logic.
//!
//! This module provides the pure computation needed during a window resize:
//! - Pixel dimensions → cell dimensions (cols/rows)
//! - Minimum size clamping (10 cols × 3 rows)
//! - Debounce timestamp tracking (100ms)
//!
//! The actual winit/wgpu/PTY wiring lives in [`DesktopApp`](crate::window::DesktopApp).

use std::time::{Duration, Instant};

/// Minimum terminal dimensions in cells.
pub const MIN_COLS: u16 = 10;
pub const MIN_ROWS: u16 = 3;

/// Debounce interval: resize events arriving faster than this are coalesced.
pub const DEBOUNCE: Duration = Duration::from_millis(100);

/// Result of a resize calculation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellDims {
    /// Number of columns (width in cells).
    pub cols: u16,
    /// Number of rows (height in cells).
    pub rows: u16,
}

impl CellDims {
    /// Compute cell dimensions from pixel size and cell pixel dimensions.
    ///
    /// Returns `None` if cell_width or cell_height is zero/negative.
    pub fn from_pixels(
        pixel_width: u32,
        pixel_height: u32,
        cell_width: f32,
        cell_height: f32,
    ) -> Option<Self> {
        if cell_width <= 0.0 || cell_height <= 0.0 {
            return None;
        }
        let cols = (pixel_width as f32 / cell_width).floor() as u16;
        let rows = (pixel_height as f32 / cell_height).floor() as u16;
        Some(Self {
            cols: cols.max(MIN_COLS),
            rows: rows.max(MIN_ROWS),
        })
    }
}

/// Track whether a resize should be applied now or deferred (debounced).
#[derive(Debug, Clone)]
pub struct ResizeDebouncer {
    /// Last time a resize was actually applied.
    last_applied: Option<Instant>,
}

impl Default for ResizeDebouncer {
    fn default() -> Self {
        Self::new()
    }
}

impl ResizeDebouncer {
    /// Create a new debouncer with no prior resize.
    pub fn new() -> Self {
        Self { last_applied: None }
    }

    /// Returns `true` if enough time has elapsed since the last applied resize
    /// (i.e. the debounce window has expired). Returns `true` on the very first
    /// call.
    pub fn should_apply(&self, now: Instant) -> bool {
        match self.last_applied {
            None => true,
            Some(t) => now.duration_since(t) >= DEBOUNCE,
        }
    }

    /// Record that a resize was applied at time `now`.
    pub fn mark_applied(&mut self, now: Instant) {
        self.last_applied = Some(now);
    }

    /// Returns the remaining debounce duration (zero if ready now).
    pub fn remaining(&self, now: Instant) -> Duration {
        match self.last_applied {
            None => Duration::ZERO,
            Some(t) => {
                let elapsed = now.duration_since(t);
                if elapsed >= DEBOUNCE {
                    Duration::ZERO
                } else {
                    DEBOUNCE - elapsed
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── CellDims::from_pixels ──

    #[test]
    fn test_basic_calculation() {
        let dims = CellDims::from_pixels(800, 600, 8.0, 16.0).unwrap();
        assert_eq!(dims.cols, 100);
        assert_eq!(dims.rows, 37); // 600 / 16 = 37.5 → floor → 37
    }

    #[test]
    fn test_exact_fit() {
        // 80 * 8 = 640, 24 * 16 = 384
        let dims = CellDims::from_pixels(640, 384, 8.0, 16.0).unwrap();
        assert_eq!(dims.cols, 80);
        assert_eq!(dims.rows, 24);
    }

    #[test]
    fn test_subpixel_floor() {
        // 80.5 * 8 = 644 → floor(644/8) = 80
        let dims = CellDims::from_pixels(644, 384, 8.0, 16.0).unwrap();
        assert_eq!(dims.cols, 80);
    }

    #[test]
    fn test_minimum_cols_enforced() {
        // 1 pixel wide → 0 cols → clamped to MIN_COLS
        let dims = CellDims::from_pixels(1, 100, 8.0, 16.0).unwrap();
        assert_eq!(dims.cols, MIN_COLS);
    }

    #[test]
    fn test_minimum_rows_enforced() {
        let dims = CellDims::from_pixels(200, 1, 8.0, 16.0).unwrap();
        assert_eq!(dims.rows, MIN_ROWS);
    }

    #[test]
    fn test_both_minimum_enforced() {
        let dims = CellDims::from_pixels(0, 0, 8.0, 16.0).unwrap();
        assert_eq!(dims.cols, MIN_COLS);
        assert_eq!(dims.rows, MIN_ROWS);
    }

    #[test]
    fn test_just_above_minimum() {
        // MIN_COLS * 8 = 80 pixels → exactly 10 cols
        let dims = CellDims::from_pixels(80, 48, 8.0, 16.0).unwrap();
        assert_eq!(dims.cols, 10);
        assert_eq!(dims.rows, 3);
    }

    #[test]
    fn test_just_below_minimum() {
        // 79 px / 8 = 9.875 → floor → 9 → clamped to 10
        let dims = CellDims::from_pixels(79, 47, 8.0, 16.0).unwrap();
        assert_eq!(dims.cols, MIN_COLS);
        assert_eq!(dims.rows, MIN_ROWS);
    }

    #[test]
    fn test_large_window() {
        let dims = CellDims::from_pixels(3840, 2160, 8.0, 16.0).unwrap();
        assert_eq!(dims.cols, 480);
        assert_eq!(dims.rows, 135);
    }

    #[test]
    fn test_different_cell_size() {
        let dims = CellDims::from_pixels(1200, 800, 12.0, 24.0).unwrap();
        assert_eq!(dims.cols, 100);
        assert_eq!(dims.rows, 33); // 800 / 24 = 33.33 → floor
    }

    #[test]
    fn test_zero_cell_width_returns_none() {
        assert!(CellDims::from_pixels(800, 600, 0.0, 16.0).is_none());
    }

    #[test]
    fn test_zero_cell_height_returns_none() {
        assert!(CellDims::from_pixels(800, 600, 8.0, 0.0).is_none());
    }

    #[test]
    fn test_negative_cell_width_returns_none() {
        assert!(CellDims::from_pixels(800, 600, -1.0, 16.0).is_none());
    }

    #[test]
    fn test_very_small_cell() {
        let dims = CellDims::from_pixels(100, 100, 0.5, 0.5).unwrap();
        assert_eq!(dims.cols, 200);
        assert_eq!(dims.rows, 200);
    }

    // ── ResizeDebouncer ──

    #[test]
    fn test_debouncer_first_call_always_applies() {
        let db = ResizeDebouncer::new();
        let now = Instant::now();
        assert!(db.should_apply(now));
    }

    #[test]
    fn test_debouncer_blocks_within_window() {
        let mut db = ResizeDebouncer::new();
        let t0 = Instant::now();
        db.mark_applied(t0);

        // Within the debounce window
        let t1 = t0 + Duration::from_millis(50);
        assert!(!db.should_apply(t1));
    }

    #[test]
    fn test_debouncer_allows_after_window() {
        let mut db = ResizeDebouncer::new();
        let t0 = Instant::now();
        db.mark_applied(t0);

        // After the debounce window
        let t1 = t0 + Duration::from_millis(101);
        assert!(db.should_apply(t1));
    }

    #[test]
    fn test_debouncer_boundary_exactly_100ms() {
        let mut db = ResizeDebouncer::new();
        let t0 = Instant::now();
        db.mark_applied(t0);

        // Exactly at the boundary → should be allowed (>=)
        let t1 = t0 + Duration::from_millis(100);
        assert!(db.should_apply(t1));
    }

    #[test]
    fn test_debouncer_remaining_zero_on_first_call() {
        let db = ResizeDebouncer::new();
        let now = Instant::now();
        assert_eq!(db.remaining(now), Duration::ZERO);
    }

    #[test]
    fn test_debouncer_remaining_after_apply() {
        let mut db = ResizeDebouncer::new();
        let t0 = Instant::now();
        db.mark_applied(t0);

        let t1 = t0 + Duration::from_millis(30);
        let remaining = db.remaining(t1);
        assert!(remaining > Duration::from_millis(60));
        assert!(remaining <= Duration::from_millis(70));
    }

    #[test]
    fn test_debouncer_remaining_zero_after_window() {
        let mut db = ResizeDebouncer::new();
        let t0 = Instant::now();
        db.mark_applied(t0);

        let t1 = t0 + Duration::from_millis(200);
        assert_eq!(db.remaining(t1), Duration::ZERO);
    }

    #[test]
    fn test_debouncer_reset_after_new_apply() {
        let mut db = ResizeDebouncer::new();
        let t0 = Instant::now();
        db.mark_applied(t0);

        // After first window
        let t1 = t0 + Duration::from_millis(150);
        assert!(db.should_apply(t1));
        db.mark_applied(t1);

        // Within second window
        let t2 = t1 + Duration::from_millis(50);
        assert!(!db.should_apply(t2));
    }

    #[test]
    fn test_debouncer_rapid_fire_only_first_passes() {
        let mut db = ResizeDebouncer::new();
        let t0 = Instant::now();

        // First call passes
        assert!(db.should_apply(t0));
        db.mark_applied(t0);

        // Rapid-fire calls within debounce window all blocked
        for ms in [10, 20, 30, 40, 50, 60, 70, 80, 90] {
            let t = t0 + Duration::from_millis(ms);
            assert!(!db.should_apply(t), "should block at {}ms", ms);
        }

        // After window, allowed again
        let t1 = t0 + Duration::from_millis(100);
        assert!(db.should_apply(t1));
    }

    // ── Constants ──

    #[test]
    fn test_min_cols_is_10() {
        assert_eq!(MIN_COLS, 10);
    }

    #[test]
    fn test_min_rows_is_3() {
        assert_eq!(MIN_ROWS, 3);
    }

    #[test]
    fn test_debounce_is_100ms() {
        assert_eq!(DEBOUNCE, Duration::from_millis(100));
    }
}
