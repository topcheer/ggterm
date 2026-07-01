//! Grid model for terminal cell storage.
//!
//! Provides a 2D cell array with scrollback history, damage tracking,
//! scroll region support, and ANSI editing operations (IL, DL, ICH, DCH, ECH).

mod cell;
mod damage;
mod row;

pub use cell::{Cell, CellFlags, Color, char_width, str_width};
pub use damage::{DamageTracker, DirtyRect};
pub use row::Row;

use std::collections::VecDeque;

/// The terminal grid: a 2D array of [`Row`]s with scrollback history.
///
/// The grid uses a `VecDeque` for scrollback and a `Vec` for the visible
/// portion. When the terminal scrolls, rows that fall off the top are
/// moved into the scrollback, and new blank rows appear at the bottom.
///
/// # Layout
///
/// ```text
/// ┌──────────────┐
/// │  scrollback  │  ← history (capped at max_scrollback rows)
/// │    ...       │
/// ├──────────────┤
/// │  row 0       │  ← visible screen (height rows)
/// │  row 1       │
/// │  ...         │
/// │  row N-1     │  ← bottom (most recent)
/// └──────────────┘
///       width →
/// ```
#[derive(Clone)]
pub struct Grid {
    /// Visible screen rows.
    rows: Vec<Row>,
    /// Scrollback history (older rows that scrolled off the top).
    scrollback: VecDeque<Row>,
    /// Maximum scrollback rows to retain.
    max_scrollback: usize,
    /// Grid width (number of columns).
    width: usize,
    /// Grid height (number of visible rows).
    height: usize,
    /// Scroll region top (inclusive). Defaults to 0.
    scroll_top: usize,
    /// Scroll region bottom (exclusive). Defaults to `height`.
    scroll_bottom: usize,
    /// How many scrollback lines are shown above the visible grid (0 = bottom).
    display_offset: usize,
    /// Damage tracker for efficient partial rendering.
    damage: DamageTracker,
    /// P23-C: Coarse dirty flag — set true on any content change.
    /// Used for conditional redraw (skip frames with no PTY data or interaction).
    content_dirty: bool,
}

impl Grid {
    /// Create a new grid with the given dimensions.
    pub fn new(width: usize, height: usize) -> Self {
        Self::with_scrollback(width, height, 10_000)
    }

    /// Create a grid with a custom scrollback limit.
    pub fn with_scrollback(width: usize, height: usize, max_scrollback: usize) -> Self {
        let rows = (0..height).map(|_| Row::new(width)).collect();
        Self {
            rows,
            scrollback: VecDeque::with_capacity(max_scrollback.min(1024)),
            max_scrollback,
            width,
            height,
            scroll_top: 0,
            scroll_bottom: height,
            display_offset: 0,
            damage: DamageTracker::new(width),
            content_dirty: true,
        }
    }

    /// Resize the grid. Existing content is preserved where possible.
    pub fn resize(&mut self, width: usize, height: usize) {
        // Resize each existing row
        for row in &mut self.rows {
            row.resize(width);
        }

        // Adjust row count
        if height > self.rows.len() {
            for _ in self.rows.len()..height {
                self.rows.push(Row::new(width));
            }
        } else if height < self.rows.len() {
            let excess = self.rows.len() - height;
            for _ in 0..excess {
                let row = self.rows.remove(0);
                self.push_scrollback(row);
            }
        }

        self.width = width;
        self.height = height;
        self.scroll_top = 0;
        self.scroll_bottom = height;
        self.display_offset = 0;
        self.damage = DamageTracker::new(width);
        self.damage.mark_all(height);
        self.content_dirty = true;
    }

    // ------------------------------------------------------------------
    //  Cell & row access
    // ------------------------------------------------------------------

    /// Get a reference to a visible row.
    pub fn row(&self, row: usize) -> Option<&Row> {
        self.rows.get(row)
    }

    /// Get the text content of a visible row.
    pub fn row_text(&self, row: usize) -> Option<String> {
        self.rows.get(row).map(|r| r.text())
    }

    /// Get a mutable reference to a visible row.
    pub fn row_mut(&mut self, row: usize) -> Option<&mut Row> {
        self.rows.get_mut(row)
    }

    /// Get a cell at (col, row).
    pub fn cell(&self, col: usize, row: usize) -> Option<&Cell> {
        self.rows.get(row).and_then(|r| r.cell(col))
    }

    /// Get a mutable cell at (col, row).
    pub fn cell_mut(&mut self, col: usize, row: usize) -> Option<&mut Cell> {
        self.rows.get_mut(row).and_then(|r| r.cell_mut(col))
    }

    /// Grid width (columns).
    pub fn width(&self) -> usize {
        self.width
    }

    /// Grid height (visible rows).
    pub fn height(&self) -> usize {
        self.height
    }

    // ------------------------------------------------------------------
    //  Scroll region
    // ------------------------------------------------------------------

    /// Set the scroll region (DECSTBM).
    /// `top` is inclusive, `bottom` is exclusive.
    /// Resets to full screen if `top >= bottom` or `bottom > height`.
    pub fn set_scroll_region(&mut self, top: usize, bottom: usize) {
        if top >= bottom || bottom > self.height {
            self.scroll_top = 0;
            self.scroll_bottom = self.height;
        } else {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
        }
    }

    /// Get the scroll region as (top, bottom).
    pub fn scroll_region(&self) -> (usize, usize) {
        (self.scroll_top, self.scroll_bottom)
    }

    /// Reset the scroll region to the full screen.
    pub fn reset_scroll_region(&mut self) {
        self.scroll_top = 0;
        self.scroll_bottom = self.height;
    }

    // ------------------------------------------------------------------
    //  Scrolling
    // ------------------------------------------------------------------

    /// Scroll the content within the scroll region up by `n` lines.
    ///
    /// Rows that fall off the top move to scrollback (only if scroll_top == 0).
    /// New blank rows appear at the bottom of the scroll region.
    pub fn scroll_up(&mut self, n: usize) {
        let region_height = self.scroll_bottom.saturating_sub(self.scroll_top);
        let n = n.min(region_height);
        if n == 0 {
            return;
        }
        for _ in 0..n {
            let removed = self.rows.remove(self.scroll_top);
            if self.scroll_top == 0 {
                self.push_scrollback(removed);
            }
            self.rows
                .insert(self.scroll_bottom - 1, Row::new(self.width));
        }
        // Reset viewport when new content causes scrolling.
        self.display_offset = 0;
        self.damage.mark_rows(self.scroll_top, region_height);
        self.content_dirty = true;
    }

    /// Scroll the content within the scroll region down by `n` lines.
    ///
    /// Rows that fall off the bottom are lost.
    /// New blank rows appear at the top of the scroll region (or restored
    /// from scrollback if available and scroll_top == 0).
    pub fn scroll_down(&mut self, n: usize) {
        let region_height = self.scroll_bottom.saturating_sub(self.scroll_top);
        let n = n.min(region_height);
        if n == 0 {
            return;
        }
        for _ in 0..n {
            let _removed = self.rows.remove(self.scroll_bottom - 1);
            // When scroll region starts at row 0, try to restore from scrollback
            if self.scroll_top == 0 {
                if let Some(row) = self.scrollback.pop_back() {
                    self.rows.insert(0, row);
                } else {
                    self.rows.insert(0, Row::new(self.width));
                }
            } else {
                self.rows.insert(self.scroll_top, Row::new(self.width));
            }
        }
        self.damage.mark_rows(self.scroll_top, region_height);
        self.content_dirty = true;
    }

    /// Scroll up within the scroll region. Alias for [`scroll_up`](Self::scroll_up).
    pub fn scroll_region_up(&mut self, n: usize) {
        self.scroll_up(n);
    }

    /// Scroll down within the scroll region. Alias for [`scroll_down`](Self::scroll_down).
    pub fn scroll_region_down(&mut self, n: usize) {
        self.scroll_down(n);
    }

    // ------------------------------------------------------------------
    //  Line editing (IL / DL)
    // ------------------------------------------------------------------

    /// Insert `count` blank lines at `row` (ANSI IL — Insert Line).
    ///
    /// Lines from `row` to the bottom of the scroll region shift down.
    /// Lines pushed past the bottom of the scroll region are lost.
    /// No-op if `row` is outside the scroll region.
    pub fn insert_line(&mut self, row: usize, count: usize) {
        if count == 0 || row < self.scroll_top || row >= self.scroll_bottom {
            return;
        }
        let count = count.min(self.scroll_bottom - row);
        for _ in 0..count {
            let _removed = self.rows.remove(self.scroll_bottom - 1);
            self.rows.insert(row, Row::new(self.width));
        }
        self.damage.mark_rows(row, self.scroll_bottom - row);
    }

    /// Delete `count` lines starting at `row` (ANSI DL — Delete Line).
    ///
    /// Lines from `row` to the bottom of the scroll region shift up.
    /// Blank lines appear at the bottom of the scroll region.
    /// No-op if `row` is outside the scroll region.
    pub fn delete_line(&mut self, row: usize, count: usize) {
        if count == 0 || row < self.scroll_top || row >= self.scroll_bottom {
            return;
        }
        let count = count.min(self.scroll_bottom - row);
        for _ in 0..count {
            self.rows.remove(row);
            self.rows
                .insert(self.scroll_bottom - 1, Row::new(self.width));
        }
        self.damage.mark_rows(row, self.scroll_bottom - row);
    }

    // ------------------------------------------------------------------
    //  Character editing (ICH / DCH / ECH)
    // ------------------------------------------------------------------

    /// Insert `count` blank characters at `(col, row)` (ANSI ICH).
    pub fn insert_char(&mut self, col: usize, row: usize, count: usize) {
        if let Some(r) = self.rows.get_mut(row) {
            r.insert_char(col, count);
            self.damage.mark_row(row);
        }
    }

    /// Delete `count` characters at `(col, row)` (ANSI DCH).
    pub fn delete_char(&mut self, col: usize, row: usize, count: usize) {
        if let Some(r) = self.rows.get_mut(row) {
            r.delete_char(col, count);
            self.damage.mark_row(row);
        }
    }

    /// Erase `count` characters from `(col, row)` (ANSI ECH).
    pub fn erase_char(&mut self, col: usize, row: usize, count: usize) {
        if let Some(r) = self.rows.get_mut(row) {
            r.erase_char(col, count);
            self.damage.mark_rect(col, row, count, 1);
        }
    }

    /// Place a character at `(col, row)` with wide-char handling.
    ///
    /// Returns the number of cells consumed (0, 1, or 2).
    pub fn put_char(&mut self, col: usize, row: usize, ch: char) -> usize {
        let w = if let Some(r) = self.rows.get_mut(row) {
            r.put_char(col, ch)
        } else {
            return 0;
        };
        self.damage.mark_rect(col, row, w.max(1), 1);
        self.content_dirty = true;
        w
    }

    // ------------------------------------------------------------------
    //  Clearing
    // ------------------------------------------------------------------

    /// Clear all visible rows to blank.
    pub fn clear(&mut self) {
        for row in &mut self.rows {
            row.clear();
        }
        self.damage.mark_all(self.height);
        self.content_dirty = true;
    }

    /// Clear the scrollback history (ED mode 3).
    pub fn clear_scrollback(&mut self) {
        self.scrollback.clear();
    }

    /// Clear from (col, row) to end of line.
    pub fn clear_line_from(&mut self, col: usize, row: usize) {
        if let Some(r) = self.rows.get_mut(row) {
            r.clear_from(col);
            let w = self.width.saturating_sub(col);
            self.damage.mark_rect(col, row, w, 1);
            self.content_dirty = true;
        }
    }

    /// Clear from start of line to (col, row) inclusive.
    pub fn clear_line_to(&mut self, col: usize, row: usize) {
        if let Some(r) = self.rows.get_mut(row) {
            r.clear_to(col + 1);
            self.damage.mark_rect(0, row, col + 1, 1);
            self.content_dirty = true;
        }
    }

    /// Clear an entire row.
    pub fn clear_line(&mut self, row: usize) {
        if let Some(r) = self.rows.get_mut(row) {
            r.clear();
            self.damage.mark_row(row);
            self.content_dirty = true;
        }
    }

    // ------------------------------------------------------------------
    //  Scrollback
    // ------------------------------------------------------------------

    /// Number of rows in scrollback.
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// Get a scrollback row by index (0 = oldest).
    pub fn scrollback_row(&self, index: usize) -> Option<&Row> {
        self.scrollback.get(index)
    }

    /// Get the text content of a scrollback row.
    pub fn scrollback_row_text(&self, index: usize) -> Option<String> {
        self.scrollback.get(index).map(|r| r.text())
    }

    /// Set the maximum scrollback capacity.
    /// Truncates existing scrollback if new limit is smaller.
    pub fn set_scrollback(&mut self, max: usize) {
        self.max_scrollback = max;
        while self.scrollback.len() > max {
            self.scrollback.pop_front();
        }
    }

    /// Push a row to the scrollback, evicting oldest if over capacity.
    fn push_scrollback(&mut self, row: Row) {
        if self.scrollback.len() >= self.max_scrollback {
            self.scrollback.pop_front();
        }
        self.scrollback.push_back(row);
    }

    // ------------------------------------------------------------------
    //  Viewport scrolling (mouse wheel scrollback)
    // ------------------------------------------------------------------

    /// Scroll the viewport up by `n` lines (towards older scrollback).
    /// This does NOT modify the grid content — it just changes which
    /// scrollback rows are visible above the active grid.
    pub fn scroll_up_viewport(&mut self, n: usize) {
        let max = self.scrollback.len();
        self.display_offset = (self.display_offset + n).min(max);
        self.damage.mark_all(self.height);
        self.content_dirty = true;
    }

    /// Scroll the viewport down by `n` lines (towards the active bottom).
    pub fn scroll_down_viewport(&mut self, n: usize) {
        self.display_offset = self.display_offset.saturating_sub(n);
        self.damage.mark_all(self.height);
        self.content_dirty = true;
    }

    /// Reset the viewport to the bottom (show active content).
    pub fn reset_viewport(&mut self) {
        if self.display_offset > 0 {
            self.display_offset = 0;
            self.damage.mark_all(self.height);
            self.content_dirty = true;
        }
    }

    /// Return the current display offset (0 = at the active bottom).
    pub fn display_offset(&self) -> usize {
        self.display_offset
    }

    /// Return true if the viewport is scrolled into scrollback history.
    pub fn is_scrolled(&self) -> bool {
        self.display_offset > 0
    }

    /// Get a row considering the display offset.
    ///
    /// If `row` is within the visible area but `display_offset > 0`,
    /// returns rows from scrollback instead.
    pub fn display_row(&self, row: usize) -> Option<&Row> {
        if self.display_offset == 0 {
            return self.rows.get(row);
        }
        let scrollback_visible = self.display_offset.min(self.scrollback.len());
        let scrollback_start = self.scrollback.len() - scrollback_visible;
        if row < scrollback_visible {
            // Row comes from scrollback.
            self.scrollback.get(scrollback_start + row)
        } else {
            // Row comes from the active grid, offset.
            self.rows.get(row - scrollback_visible)
        }
    }

    /// Get a cell considering the display offset.
    pub fn display_cell(&self, col: usize, row: usize) -> Option<&Cell> {
        self.display_row(row).and_then(|r| r.cell(col))
    }

    // ------------------------------------------------------------------
    //  Damage tracking
    // ------------------------------------------------------------------

    /// Returns `true` if any cells have been modified since last render.
    pub fn is_dirty(&self) -> bool {
        self.damage.is_dirty()
    }

    /// Get the current dirty region without clearing.
    pub fn dirty(&self) -> Option<DirtyRect> {
        self.damage.dirty()
    }

    /// Get a reference to the damage tracker.
    pub fn damage(&self) -> &DamageTracker {
        &self.damage
    }

    /// Mark a specific cell as dirty.
    pub fn mark_dirty(&mut self, col: usize, row: usize) {
        self.damage.mark_cell(col, row);
    }

    /// Mark an entire row as dirty.
    pub fn mark_row_dirty(&mut self, row: usize) {
        self.damage.mark_row(row);
    }

    /// Mark the entire grid as dirty (full repaint needed).
    pub fn mark_all_dirty(&mut self) {
        self.damage.mark_all(self.height);
    }

    /// Take ownership of the dirty region, clearing the tracker.
    pub fn take_damage(&mut self) -> Option<DirtyRect> {
        self.damage.take_dirty()
    }

    /// Clear all dirty marks without processing them.
    pub fn clear_damage(&mut self) {
        self.damage.clear();
    }

    /// P23-C: Returns true if any content changed since last `clear_dirty()`.
    pub fn content_dirty(&self) -> bool {
        self.content_dirty
    }

    /// P23-C: Clear the coarse content-dirty flag.
    /// Called by the render loop after a frame is produced.
    pub fn clear_dirty(&mut self) {
        self.content_dirty = false;
    }
}

impl std::ops::Index<(usize, usize)> for Grid {
    type Output = Cell;

    fn index(&self, (col, row): (usize, usize)) -> &Self::Output {
        &self.rows[row].cells[col]
    }
}

impl std::ops::IndexMut<(usize, usize)> for Grid {
    fn index_mut(&mut self, (col, row): (usize, usize)) -> &mut Self::Output {
        &mut self.rows[row].cells[col]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: fill a grid with sequential characters, then clear damage.
    fn fill_grid(grid: &mut Grid) {
        for row in 0..grid.height() {
            for col in 0..grid.width() {
                let ch =
                    char::from_u32(b'A' as u32 + (row * grid.width() + col) as u32).unwrap_or(' ');
                grid[(col, row)] = Cell::with_char(ch);
            }
        }
        grid.clear_damage();
    }

    // ================================================================
    //  Construction & basic access (3 tests)
    // ================================================================

    #[test]
    fn grid_new_defaults() {
        let g = Grid::new(80, 24);
        assert_eq!(g.width(), 80);
        assert_eq!(g.height(), 24);
        assert_eq!(g.scrollback_len(), 0);
        assert!(!g.is_dirty());
    }

    #[test]
    fn grid_cell_access() {
        let mut g = Grid::new(10, 5);
        g[(3, 2)] = Cell::with_char('X');
        assert_eq!(g[(3, 2)].ch, 'X');
        assert_eq!(g.cell(3, 2).unwrap().ch, 'X');
    }

    #[test]
    fn grid_row_access() {
        let mut g = Grid::new(10, 3);
        g.row_mut(1).unwrap().put_char(5, 'Z');
        assert_eq!(g.row(1).unwrap()[5].ch, 'Z');
    }

    // ================================================================
    //  Unicode width (8 tests)
    // ================================================================

    #[test]
    fn unicode_width_ascii() {
        assert_eq!(char_width('A'), 1);
        assert_eq!(char_width(' '), 1);
        assert_eq!(char_width('~'), 1);
    }

    #[test]
    fn unicode_width_cjk() {
        assert_eq!(char_width('中'), 2);
        assert_eq!(char_width('文'), 2);
        assert_eq!(char_width('ー'), 2);
    }

    #[test]
    fn unicode_width_emoji() {
        assert_eq!(char_width('😀'), 2);
    }

    #[test]
    fn unicode_width_combining() {
        assert_eq!(char_width('\u{0301}'), 0); // combining acute accent
    }

    #[test]
    fn unicode_str_width() {
        assert_eq!(str_width("AB"), 2);
        assert_eq!(str_width("中文"), 4);
        assert_eq!(str_width("A中"), 3);
    }

    #[test]
    fn grid_put_wide_char_sets_flags() {
        let mut g = Grid::new(10, 1);
        let w = g.put_char(2, 0, '中');
        assert_eq!(w, 2);
        assert!(g[(2, 0)].is_wide());
        assert!(g[(3, 0)].is_wide_spacer());
    }

    #[test]
    fn grid_put_normal_char_no_flags() {
        let mut g = Grid::new(10, 1);
        let w = g.put_char(0, 0, 'A');
        assert_eq!(w, 1);
        assert!(!g[(0, 0)].is_wide());
    }

    #[test]
    fn cell_set_char_wide_clears_on_normal() {
        let mut c = Cell::blank();
        c.set_char('中');
        assert!(c.is_wide());
        c.set_char('A');
        assert!(!c.is_wide());
    }

    // ================================================================
    //  Row-level character ops (8 tests)
    // ================================================================

    #[test]
    fn row_insert_char_basic() {
        let mut r = Row::new(10);
        r.put_char(0, 'A');
        r.put_char(1, 'B');
        r.put_char(2, 'C');
        r.insert_char(1, 2);
        assert_eq!(r[0].ch, 'A');
        assert_eq!(r[1].ch, ' ');
        assert_eq!(r[2].ch, ' ');
        assert_eq!(r[3].ch, 'B');
        assert_eq!(r[4].ch, 'C');
    }

    #[test]
    fn row_insert_char_at_end() {
        let mut r = Row::new(5);
        r.put_char(0, 'X');
        r.insert_char(4, 1);
        assert_eq!(r[0].ch, 'X');
        assert_eq!(r[4].ch, ' ');
    }

    #[test]
    fn row_insert_char_past_end_noop() {
        let mut r = Row::new(5);
        r.insert_char(10, 1);
        assert!(r.cells.iter().all(|c| c.is_blank()));
    }

    #[test]
    fn row_delete_char_basic() {
        let mut r = Row::new(10);
        r.put_char(0, 'A');
        r.put_char(1, 'B');
        r.put_char(2, 'C');
        r.put_char(3, 'D');
        r.delete_char(1, 1);
        assert_eq!(r[0].ch, 'A');
        assert_eq!(r[1].ch, 'C');
        assert_eq!(r[2].ch, 'D');
        assert_eq!(r[3].ch, ' ');
    }

    #[test]
    fn row_delete_char_multiple() {
        let mut r = Row::new(10);
        r.put_char(0, 'A');
        r.put_char(1, 'B');
        r.put_char(2, 'C');
        r.put_char(3, 'D');
        r.delete_char(0, 2);
        assert_eq!(r[0].ch, 'C');
        assert_eq!(r[1].ch, 'D');
    }

    #[test]
    fn row_erase_char_basic() {
        let mut r = Row::new(10);
        r.put_char(0, 'A');
        r.put_char(1, 'B');
        r.put_char(2, 'C');
        r.erase_char(1, 1);
        assert_eq!(r[0].ch, 'A');
        assert_eq!(r[1].ch, ' '); // erased, NOT shifted
        assert_eq!(r[2].ch, 'C');
    }

    #[test]
    fn row_put_wide_then_delete() {
        let mut r = Row::new(10);
        let w = r.put_char(0, '中');
        assert_eq!(w, 2);
        assert!(r[0].is_wide());
        assert!(r[1].is_wide_spacer());
        r.delete_char(0, 1);
        assert!(!r[0].is_wide());
    }

    #[test]
    fn row_put_wide_overwrites_existing() {
        let mut r = Row::new(10);
        r.put_char(0, '中'); // wide at 0-1
        r.put_char(0, 'A'); // overwrite with normal
        assert_eq!(r[0].ch, 'A');
        assert!(!r[0].is_wide());
        assert!(!r[1].is_wide_spacer());
    }

    // ================================================================
    //  Grid insert_line / delete_line (6 tests)
    // ================================================================

    #[test]
    fn grid_insert_line_basic() {
        let mut g = Grid::new(5, 5);
        fill_grid(&mut g);
        g.insert_line(1, 1);
        assert_eq!(g[(0, 0)].ch, 'A'); // row 0 unchanged
        assert!(g.row(1).unwrap().cells.iter().all(|c| c.is_blank()));
        assert_eq!(g[(0, 2)].ch, 'F'); // was row 1
    }

    #[test]
    fn grid_insert_line_multiple() {
        let mut g = Grid::new(5, 5);
        fill_grid(&mut g);
        g.insert_line(0, 2);
        assert!(g.row(0).unwrap().cells.iter().all(|c| c.is_blank()));
        assert!(g.row(1).unwrap().cells.iter().all(|c| c.is_blank()));
        assert_eq!(g[(0, 2)].ch, 'A'); // was row 0
    }

    #[test]
    fn grid_insert_line_outside_scroll_region_noop() {
        let mut g = Grid::new(5, 10);
        g.set_scroll_region(3, 8);
        fill_grid(&mut g);
        g.insert_line(1, 1); // row 1 outside scroll region → no-op
        assert!(!g.is_dirty());
    }

    #[test]
    fn grid_delete_line_basic() {
        let mut g = Grid::new(5, 5);
        fill_grid(&mut g);
        g.delete_line(0, 1);
        assert_eq!(g[(0, 0)].ch, 'F'); // was row 1
        assert!(g.row(4).unwrap().cells.iter().all(|c| c.is_blank()));
    }

    #[test]
    fn grid_delete_line_multiple() {
        let mut g = Grid::new(5, 5);
        fill_grid(&mut g);
        g.delete_line(0, 2);
        assert_eq!(g[(0, 0)].ch, 'K'); // was row 2
        assert!(g.row(3).unwrap().cells.iter().all(|c| c.is_blank()));
        assert!(g.row(4).unwrap().cells.iter().all(|c| c.is_blank()));
    }

    #[test]
    fn grid_delete_line_in_scroll_region() {
        let mut g = Grid::new(3, 6);
        g.set_scroll_region(1, 5);
        fill_grid(&mut g);
        g.delete_line(2, 1);
        assert_eq!(g[(0, 0)].ch, 'A'); // outside scroll region, unchanged
        assert_eq!(g[(0, 2)].ch, 'J'); // was row 3: 'A' + 3*3 = 'J'
    }

    // ================================================================
    //  Grid character ops: ICH / DCH / ECH (3 tests)
    // ================================================================

    #[test]
    fn grid_insert_char() {
        let mut g = Grid::new(10, 1);
        g.put_char(0, 0, 'A');
        g.put_char(1, 0, 'B');
        g.put_char(2, 0, 'C');
        g.clear_damage();
        g.insert_char(1, 0, 2);
        assert_eq!(g[(0, 0)].ch, 'A');
        assert_eq!(g[(1, 0)].ch, ' ');
        assert_eq!(g[(2, 0)].ch, ' ');
        assert_eq!(g[(3, 0)].ch, 'B');
        assert_eq!(g[(4, 0)].ch, 'C');
        assert!(g.is_dirty());
    }

    #[test]
    fn grid_delete_char() {
        let mut g = Grid::new(10, 1);
        g.put_char(0, 0, 'A');
        g.put_char(1, 0, 'B');
        g.put_char(2, 0, 'C');
        g.clear_damage();
        g.delete_char(0, 0, 1);
        assert_eq!(g[(0, 0)].ch, 'B');
        assert_eq!(g[(1, 0)].ch, 'C');
        assert!(g.is_dirty());
    }

    #[test]
    fn grid_erase_char() {
        let mut g = Grid::new(10, 1);
        g.put_char(0, 0, 'A');
        g.put_char(1, 0, 'B');
        g.put_char(2, 0, 'C');
        g.clear_damage();
        g.erase_char(1, 0, 1);
        assert_eq!(g[(0, 0)].ch, 'A');
        assert_eq!(g[(1, 0)].ch, ' '); // erased, not shifted
        assert_eq!(g[(2, 0)].ch, 'C');
        assert!(g.is_dirty());
    }

    // ================================================================
    //  Scroll region (7 tests)
    // ================================================================

    #[test]
    fn scroll_region_default() {
        let g = Grid::new(80, 24);
        let (top, bottom) = g.scroll_region();
        assert_eq!(top, 0);
        assert_eq!(bottom, 24);
    }

    #[test]
    fn scroll_region_set() {
        let mut g = Grid::new(80, 24);
        g.set_scroll_region(5, 15);
        let (top, bottom) = g.scroll_region();
        assert_eq!(top, 5);
        assert_eq!(bottom, 15);
    }

    #[test]
    fn scroll_region_reset() {
        let mut g = Grid::new(80, 24);
        g.set_scroll_region(5, 15);
        g.reset_scroll_region();
        let (top, bottom) = g.scroll_region();
        assert_eq!(top, 0);
        assert_eq!(bottom, 24);
    }

    #[test]
    fn scroll_region_invalid_resets() {
        let mut g = Grid::new(80, 24);
        g.set_scroll_region(10, 5); // top > bottom
        let (top, bottom) = g.scroll_region();
        assert_eq!(top, 0);
        assert_eq!(bottom, 24);
    }

    #[test]
    fn scroll_region_up_moves_to_scrollback() {
        let mut g = Grid::new(5, 5);
        fill_grid(&mut g);
        g.scroll_up(1);
        assert_eq!(g.scrollback_len(), 1);
        assert_eq!(g[(0, 0)].ch, 'F'); // was row 1
    }

    #[test]
    fn scroll_region_down_restores_from_scrollback() {
        let mut g = Grid::new(5, 5);
        fill_grid(&mut g);
        g.scroll_up(1); // row 0 ('A'..'E') moves to scrollback
        g.clear_damage();
        g.scroll_down(1); // row 0 restored from scrollback
        // Row 0 should have original content restored
        assert_eq!(g[(0, 0)].ch, 'A');
        assert_eq!(g.scrollback_len(), 0);
    }

    #[test]
    fn scroll_region_down_no_scrollback_inserts_blank() {
        let mut g = Grid::new(5, 5);
        fill_grid(&mut g);
        g.clear_damage();
        // scroll_down without prior scroll_up → no scrollback to restore
        g.scroll_down(1);
        // Row 0 should be blank (inserted), row 4 lost
        assert!(g.row(0).unwrap().cells.iter().all(|c| c.is_blank()));
    }

    #[test]
    fn scroll_region_partial_does_not_affect_outside() {
        let mut g = Grid::new(3, 6);
        g.set_scroll_region(1, 5);
        fill_grid(&mut g);
        g.clear_damage();
        g.scroll_up(1);
        // Row 0 outside scroll region, unchanged
        assert_eq!(g[(0, 0)].ch, 'A');
        // Row 1 should have what was in row 2: 'A' + 2*3 = 'G'
        assert_eq!(g[(0, 1)].ch, 'G');
    }

    // ================================================================
    //  Damage tracking (7 tests)
    // ================================================================

    #[test]
    fn damage_initially_clean() {
        let g = Grid::new(80, 24);
        assert!(!g.is_dirty());
        assert!(g.dirty().is_none());
    }

    #[test]
    fn damage_from_put_char() {
        let mut g = Grid::new(80, 24);
        g.put_char(10, 5, 'X');
        assert!(g.is_dirty());
        let d = g.take_damage().unwrap();
        assert_eq!(d.x, 10);
        assert_eq!(d.y, 5);
        assert!(!g.is_dirty());
    }

    #[test]
    fn damage_from_clear() {
        let mut g = Grid::new(80, 24);
        g.clear_damage();
        g.clear();
        assert!(g.is_dirty());
        let d = g.take_damage().unwrap();
        assert_eq!(d.x, 0);
        assert_eq!(d.y, 0);
        assert_eq!(d.width, 80);
        assert_eq!(d.height, 24);
    }

    #[test]
    fn damage_from_insert_line() {
        let mut g = Grid::new(10, 10);
        g.clear_damage();
        g.insert_line(3, 2);
        assert!(g.is_dirty());
        let d = g.take_damage().unwrap();
        assert!(d.y <= 3);
        assert!(d.bottom() >= 10);
    }

    #[test]
    fn damage_from_resize() {
        let mut g = Grid::new(80, 24);
        g.clear_damage();
        g.resize(100, 30);
        assert!(g.is_dirty());
        let d = g.take_damage().unwrap();
        assert!(d.width >= 80);
        assert!(d.height >= 24);
    }

    #[test]
    fn damage_take_clears() {
        let mut g = Grid::new(80, 24);
        g.put_char(0, 0, 'X');
        assert!(g.is_dirty());
        let _ = g.take_damage();
        assert!(!g.is_dirty());
    }

    #[test]
    fn damage_mark_all() {
        let mut g = Grid::new(80, 24);
        g.mark_all_dirty();
        assert!(g.is_dirty());
        let d = g.take_damage().unwrap();
        assert_eq!(d, DirtyRect::new(0, 0, 80, 24));
    }

    // ================================================================
    //  Clearing (3 tests)
    // ================================================================

    #[test]
    fn clear_line_from() {
        let mut g = Grid::new(10, 1);
        g.put_char(0, 0, 'A');
        g.put_char(1, 0, 'B');
        g.put_char(2, 0, 'C');
        g.clear_damage();
        g.clear_line_from(1, 0);
        assert_eq!(g[(0, 0)].ch, 'A');
        assert_eq!(g[(1, 0)].ch, ' ');
        assert_eq!(g[(2, 0)].ch, ' ');
    }

    #[test]
    fn clear_line_to() {
        let mut g = Grid::new(10, 1);
        g.put_char(0, 0, 'A');
        g.put_char(1, 0, 'B');
        g.put_char(2, 0, 'C');
        g.clear_damage();
        g.clear_line_to(1, 0);
        assert_eq!(g[(0, 0)].ch, ' ');
        assert_eq!(g[(1, 0)].ch, ' ');
        assert_eq!(g[(2, 0)].ch, 'C');
    }

    #[test]
    fn clear_line_full() {
        let mut g = Grid::new(10, 2);
        g.put_char(0, 0, 'A');
        g.put_char(0, 1, 'B');
        g.clear_damage();
        g.clear_line(0);
        assert!(g.row(0).unwrap().cells.iter().all(|c| c.is_blank()));
        assert_eq!(g[(0, 1)].ch, 'B'); // row 1 unchanged
    }

    // ================================================================
    //  Resize (2 tests)
    // ================================================================

    #[test]
    fn resize_grow() {
        let mut g = Grid::new(10, 5);
        g.put_char(0, 0, 'X');
        g.resize(15, 8);
        assert_eq!(g.width(), 15);
        assert_eq!(g.height(), 8);
        assert_eq!(g[(0, 0)].ch, 'X');
    }

    #[test]
    fn resize_shrink_to_scrollback() {
        let mut g = Grid::new(10, 5);
        g.resize(10, 3);
        assert_eq!(g.height(), 3);
        assert_eq!(g.scrollback_len(), 2);
    }

    // ================================================================
    //  Scrollback (2 tests)
    // ================================================================

    #[test]
    fn scrollback_access() {
        let mut g = Grid::with_scrollback(3, 2, 100);
        g[(0, 0)] = Cell::with_char('A');
        g.scroll_up(1);
        assert_eq!(g.scrollback_len(), 1);
        assert_eq!(g.scrollback_row(0).unwrap()[0].ch, 'A');
    }

    #[test]
    fn scrollback_cap() {
        let mut g = Grid::with_scrollback(3, 2, 3);
        for i in 0..10u8 {
            g[(0, 0)] = Cell::with_char((b'0' + i) as char);
            g.scroll_up(1);
        }
        assert_eq!(g.scrollback_len(), 3); // capped
    }

    // ── Viewport scrolling ───────────────────────────────────────────

    #[test]
    fn viewport_scroll_up_down() {
        let mut g = Grid::with_scrollback(3, 2, 100);
        // Fill some content and scroll it into history.
        g[(0, 0)] = Cell::with_char('A');
        g.scroll_up(1);
        g[(0, 0)] = Cell::with_char('B');
        g.scroll_up(1);

        assert_eq!(g.scrollback_len(), 2);
        assert_eq!(g.display_offset(), 0);
        assert!(!g.is_scrolled());

        // Scroll viewport up.
        g.scroll_up_viewport(1);
        assert_eq!(g.display_offset(), 1);
        assert!(g.is_scrolled());

        // Scroll viewport up again.
        g.scroll_up_viewport(5); // over-scroll clamps
        assert_eq!(g.display_offset(), 2); // clamped to scrollback_len

        // Scroll back down.
        g.scroll_down_viewport(1);
        assert_eq!(g.display_offset(), 1);
        g.scroll_down_viewport(5);
        assert_eq!(g.display_offset(), 0);
    }

    #[test]
    fn viewport_reset() {
        let mut g = Grid::with_scrollback(3, 2, 100);
        g.scroll_up(1); // push to scrollback
        g.scroll_up_viewport(1);
        assert!(g.is_scrolled());
        g.reset_viewport();
        assert!(!g.is_scrolled());
        assert_eq!(g.display_offset(), 0);
    }

    #[test]
    fn viewport_resets_on_new_scroll() {
        let mut g = Grid::with_scrollback(3, 2, 100);
        g.scroll_up(1);
        g.scroll_up_viewport(1);
        assert_eq!(g.display_offset(), 1);
        // New content scrolls — viewport resets.
        g.scroll_up(1);
        assert_eq!(g.display_offset(), 0);
    }

    #[test]
    fn display_row_with_offset() {
        let mut g = Grid::with_scrollback(3, 2, 100);
        g[(0, 0)] = Cell::with_char('A');
        g.scroll_up(1); // 'A' goes to scrollback[0]
        g[(0, 0)] = Cell::with_char('B');
        g.scroll_up(1); // 'B' goes to scrollback[1]
        // scrollback = ['A', 'B'], active = [' ', ' ']

        g.scroll_up_viewport(2);
        // With offset=2, we show 2 rows from scrollback:
        // display_row(0) → scrollback_start = 2-2=0, row 0 = scrollback[0] = 'A'
        // display_row(1) → scrollback[1] = 'B'
        assert_eq!(g.display_row(0).unwrap()[0].ch, 'A');
        assert_eq!(g.display_row(1).unwrap()[0].ch, 'B');
    }

    #[test]
    fn display_cell_no_offset() {
        let mut g = Grid::with_scrollback(3, 2, 100);
        g[(0, 0)] = Cell::with_char('X');
        // No offset — display_cell == regular cell.
        assert_eq!(g.display_cell(0, 0).unwrap().ch, 'X');
    }

    // ── P23-C: content_dirty tests ────────────────────────────

    #[test]
    fn test_content_dirty_default_true() {
        let g = Grid::new(10, 5);
        assert!(g.content_dirty(), "new grid should be dirty");
    }

    #[test]
    fn test_clear_dirty() {
        let mut g = Grid::new(10, 5);
        assert!(g.content_dirty());

        g.clear_dirty();
        assert!(!g.content_dirty(), "should be clean after clear_dirty");
    }

    #[test]
    fn test_put_char_sets_dirty() {
        let mut g = Grid::new(10, 5);
        g.clear_dirty();

        g.put_char(0, 0, 'X');
        assert!(g.content_dirty(), "put_char should mark dirty");
    }

    #[test]
    fn test_scroll_up_sets_dirty() {
        let mut g = Grid::with_scrollback(10, 5, 100);
        g.clear_dirty();

        g.scroll_up(1);
        assert!(g.content_dirty(), "scroll_up should mark dirty");
    }

    #[test]
    fn test_scroll_down_sets_dirty() {
        let mut g = Grid::with_scrollback(10, 5, 100);
        g.clear_dirty();

        g.scroll_down(1);
        assert!(g.content_dirty(), "scroll_down should mark dirty");
    }

    #[test]
    fn test_clear_sets_dirty() {
        let mut g = Grid::new(10, 5);
        g.clear_dirty();

        g.clear();
        assert!(g.content_dirty(), "clear should mark dirty");
    }

    #[test]
    fn test_clear_line_sets_dirty() {
        let mut g = Grid::new(10, 5);
        g.clear_dirty();

        g.clear_line(0);
        assert!(g.content_dirty(), "clear_line should mark dirty");
    }

    #[test]
    fn test_resize_sets_dirty() {
        let mut g = Grid::new(10, 5);
        g.clear_dirty();

        g.resize(20, 10);
        assert!(g.content_dirty(), "resize should mark dirty");
    }

    #[test]
    fn test_no_change_stays_clean() {
        let mut g = Grid::new(10, 5);
        g.clear_dirty();
        g.clear_dirty();

        // Reading doesn't set dirty.
        let _ = g.row(0);
        assert!(!g.content_dirty(), "read-only ops should not mark dirty");
    }
}
