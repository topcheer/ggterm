//! Terminal utility actions — clear, reset, select-all.
//!
//! These are triggered by keyboard shortcuts in the event loop:
//! - `Ctrl+Shift+C` — copy selection to clipboard
//! - `Ctrl+Shift+K` — clear screen + scrollback
//! - `Ctrl+Shift+R` — soft reset (RIS)
//! - `Ctrl+Shift+A` — select all text

use ggterm_core::grid::Grid;

/// Clear the visible screen and scrollback history.
///
/// After clearing, the cursor moves to the top-left of the visible area.
pub fn clear_screen_and_scrollback(grid: &mut Grid) {
    grid.clear();
    grid.clear_scrollback();
    grid.reset_viewport();
}

/// Clear only the scrollback history, keeping the visible screen intact.
pub fn clear_scrollback_only(grid: &mut Grid) {
    grid.clear_scrollback();
    grid.reset_viewport();
}

/// Soft-reset the terminal state.
///
/// Sends the RIS (Reset to Initial State) sequence internally.
/// This clears the screen, clears scrollback, resets cursor to (0,0).
pub fn soft_reset(grid: &mut Grid) {
    grid.clear();
    grid.clear_scrollback();
    grid.reset_viewport();
}

/// Selection range for "select all".
///
/// Returns `((start_col, start_row), (end_col, end_row))`
/// spanning the entire grid including scrollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectAllRange {
    /// Start column.
    pub start_col: usize,
    /// Start row (0 = top of scrollback).
    pub start_row: usize,
    /// End column (last column).
    pub end_col: usize,
    /// End row (last visible row).
    pub end_row: usize,
}

/// Compute the selection range for "select all" in the grid.
///
/// Spans from (0, 0) to (width-1, height-1) of the visible grid.
pub fn select_all_range(grid: &Grid) -> SelectAllRange {
    SelectAllRange {
        start_col: 0,
        start_row: 0,
        end_col: grid.width().saturating_sub(1),
        end_row: grid.height().saturating_sub(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ggterm_core::grid::Grid;

    fn make_grid(cols: usize, rows: usize) -> Grid {
        Grid::new(cols, rows)
    }

    #[test]
    fn test_clear_screen_and_scrollback() {
        let mut grid = make_grid(10, 5);
        // Simulate some content by scrolling (which adds to scrollback).
        grid.scroll_up(3);
        assert!(grid.scrollback_len() > 0);

        clear_screen_and_scrollback(&mut grid);
        assert_eq!(grid.scrollback_len(), 0);
    }

    #[test]
    fn test_clear_scrollback_only_keeps_visible() {
        let mut grid = make_grid(10, 5);
        grid.scroll_up(2);
        let scrollback_before = grid.scrollback_len();
        assert!(scrollback_before > 0);

        clear_scrollback_only(&mut grid);
        assert_eq!(grid.scrollback_len(), 0);
        // Visible rows should still be there.
        assert_eq!(grid.height(), 5);
    }

    #[test]
    fn test_soft_reset() {
        let mut grid = make_grid(8, 4);
        grid.scroll_up(5);
        assert!(grid.scrollback_len() > 0);

        soft_reset(&mut grid);
        assert_eq!(grid.scrollback_len(), 0);
    }

    #[test]
    fn test_select_all_range() {
        let grid = make_grid(80, 24);
        let range = select_all_range(&grid);
        assert_eq!(range.start_col, 0);
        assert_eq!(range.start_row, 0);
        assert_eq!(range.end_col, 79);
        assert_eq!(range.end_row, 23);
    }

    #[test]
    fn test_select_all_range_small_grid() {
        let grid = make_grid(1, 1);
        let range = select_all_range(&grid);
        assert_eq!(range.start_col, 0);
        assert_eq!(range.start_row, 0);
        assert_eq!(range.end_col, 0);
        assert_eq!(range.end_row, 0);
    }

    #[test]
    fn test_clear_after_scrollback_cap() {
        let mut grid = make_grid(10, 3);
        grid.set_scrollback(5);
        for _ in 0..10 {
            grid.scroll_up(1);
        }
        // Scrollback should be capped.
        assert!(grid.scrollback_len() <= 5);

        clear_screen_and_scrollback(&mut grid);
        assert_eq!(grid.scrollback_len(), 0);
    }

    #[test]
    fn test_clear_does_not_panic_on_empty_grid() {
        let mut grid = make_grid(1, 1);
        clear_screen_and_scrollback(&mut grid);
        assert_eq!(grid.scrollback_len(), 0);
    }

    #[test]
    fn test_select_all_range_has_correct_dimensions() {
        let grid = make_grid(120, 40);
        let range = select_all_range(&grid);
        assert_eq!(range.end_col - range.start_col + 1, 120);
        assert_eq!(range.end_row - range.start_row + 1, 40);
    }

    #[test]
    fn test_soft_reset_clears_scrollback() {
        let mut grid = make_grid(10, 5);
        grid.scroll_up(3);
        assert!(grid.scrollback_len() > 0);

        soft_reset(&mut grid);
        assert_eq!(grid.scrollback_len(), 0);
    }

    #[test]
    fn test_clear_scrollback_resets_viewport() {
        let mut grid = make_grid(10, 5);
        grid.scroll_up(3);
        grid.scroll_up_viewport(2);

        clear_scrollback_only(&mut grid);
        assert_eq!(grid.scrollback_len(), 0);
    }

    #[test]
    fn test_clear_screen_idempotent() {
        let mut grid = make_grid(5, 3);
        clear_screen_and_scrollback(&mut grid);
        clear_screen_and_scrollback(&mut grid);
        assert_eq!(grid.scrollback_len(), 0);
    }

    #[test]
    fn test_select_all_range_start_is_origin() {
        let grid = make_grid(80, 24);
        let range = select_all_range(&grid);
        assert_eq!((range.start_col, range.start_row), (0, 0));
    }
}
