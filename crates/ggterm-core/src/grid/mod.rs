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
    /// Total scrollback rows evicted since Grid creation (for mark adjustment).
    total_evicted: usize,
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
            total_evicted: 0,
            content_dirty: true,
        }
    }

    /// Resize the grid. Existing content is preserved where possible.
    pub fn resize(&mut self, width: usize, height: usize) {
        let width = width.max(1);
        let height = height.max(1);
        // Save the current scroll position — we restore it after resize
        // so the user doesn't lose their place in the scrollback.
        let saved_offset = self.display_offset;

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
            // Shrink: push TOP rows (oldest content) to scrollback,
            // keep BOTTOM rows (where cursor and recent output are).
            // drain(..overflow_count) is O(rows.len()), same as split_off
            // but keeps the correct end.
            let overflow_count = self.rows.len() - height;
            let overflow: Vec<Row> = self.rows.drain(..overflow_count).collect();
            for row in overflow {
                self.push_scrollback(row);
            }
        }

        self.width = width;
        self.height = height;
        self.scroll_top = 0;
        self.scroll_bottom = height;
        // Restore scroll position, clamped to new scrollback size.
        self.display_offset = saved_offset.min(self.scrollback.len());
        self.damage = DamageTracker::new(width);
        self.damage.mark_all(height);
        self.content_dirty = true;
    }

    /// Resize the grid with reflow support (DECSET 2027).
    ///
    /// When reflowing, growing the height pulls rows from scrollback back
    /// into the visible area (if available), so the user sees more history
    /// after making the window taller. Shrinking behaves like normal resize.
    pub fn reflow_resize(&mut self, width: usize, height: usize) {
        let width = width.max(1);
        let height = height.max(1);
        if width == self.width && height == self.height {
            return;
        }

        // Fast path: only height changed, no need to re-wrap lines.
        // Just move rows between scrollback and visible area.
        if width == self.width {
            if height > self.height {
                // Growing: pull rows from scrollback into visible area.
                let extra = height - self.height;
                let take = extra.min(self.scrollback.len());
                // Collect in reverse order (pop_back yields newest-first),
                // then reverse once. O(n) instead of O(n²) with insert(0).
                let mut pulled: Vec<Row> = Vec::with_capacity(take);
                for _ in 0..take {
                    if let Some(row) = self.scrollback.pop_back() {
                        pulled.push(row);
                    }
                }
                pulled.reverse();
                pulled.append(&mut self.rows);
                self.rows = pulled;
                self.scrollback.shrink_to_fit();
                self.rows.resize_with(height, || Row::new(width));
            } else {
                // Shrinking: push TOP excess visible rows to scrollback,
                // keep BOTTOM rows (where cursor and recent output are).
                let excess = self.rows.len().saturating_sub(height);
                if excess > 0 {
                    let overflow: Vec<Row> = self.rows.drain(..excess).collect();
                    for row in overflow {
                        self.scrollback.push_back(row);
                    }
                    let max_sb = self.max_scrollback;
                    while self.scrollback.len() > max_sb {
                        self.scrollback.pop_front();
                    }
                }
            }
            self.height = height;
            // Reset scroll region to full screen (consistent with resize()).
            self.scroll_top = 0;
            self.scroll_bottom = height;
            self.damage.mark_all(height);
            self.content_dirty = true;
            return;
        }

        // Collect ALL rows (scrollback + visible) into a single flat list.
        // We'll re-wrap the entire history, then redistribute into
        // scrollback and visible rows.
        let mut all_rows: Vec<Row> = self.scrollback.drain(..).collect();
        all_rows.append(&mut self.rows);

        // Re-wrap each logical line to the new width.
        // A logical line is a sequence of rows connected by wrap=true.
        let mut reflown: Vec<Row> = Vec::with_capacity(all_rows.len());
        {
            let mut current: Option<Row> = None; // accumulating a logical line
            for row in all_rows {
                if let Some(ref mut acc) = current {
                    // Trim trailing blanks from the accumulator before joining.
                    // A soft-wrapped row may have trailing blank padding (e.g.
                    // from DCH/ICH operations). Without trimming, these blanks
                    // become gaps in the rejoined logical line when widening.
                    trim_trailing_blanks(&mut acc.cells);
                    // Append this row's cells to the accumulator.
                    acc.cells.extend(row.cells.iter().cloned());
                    // Keep the wrap flag from the row being appended.
                    acc.wrap = row.wrap;
                } else {
                    current = Some(row);
                }

                // If this row was NOT soft-wrapped, it's the end of a logical line.
                if current.as_ref().is_some_and(|r| !r.wrap) {
                    // unwrap is safe: is_some_and guaranteed Some.
                    let mut line = current.take().expect("checked Some above");
                    // Trim trailing blank cells from non-wrapped lines.
                    // Without this, a short line like "Hello" at width 80
                    // would be split into 2 rows when shrinking to 40,
                    // because blank padding cells are treated as content.
                    trim_trailing_blanks(&mut line.cells);
                    reflow_line(&mut reflown, line, width);
                }
            }
            // Handle dangling soft-wrapped line at the very end.
            if let Some(line) = current {
                reflow_line(&mut reflown, line, width);
            }
        }

        // Redistribute: scrollback gets excess rows, visible gets the rest.
        if reflown.len() > height {
            let split = reflown.len() - height;
            self.scrollback = reflown[..split].iter().cloned().collect();
            self.rows = reflown[split..].to_vec();
        } else {
            self.scrollback = VecDeque::new();
            self.rows = reflown;
            while self.rows.len() < height {
                self.rows.push(Row::new(width));
            }
        }

        // Enforce scrollback cap.
        while self.scrollback.len() > self.max_scrollback {
            self.scrollback.pop_front();
            self.total_evicted += 1;
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

    /// Set the wrap (soft-wrap) flag on a row.
    pub fn set_row_wrap(&mut self, row: usize, wrap: bool) {
        if let Some(r) = self.rows.get_mut(row) {
            r.wrap = wrap;
        }
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
        // If the user is scrolled up in the scrollback, preserve their position
        // by advancing display_offset by n. Only auto-scroll to bottom when
        // the user is already viewing the latest output (offset == 0).
        let was_scrolled = self.display_offset > 0;

        // Track whether rows were actually pushed to scrollback.
        // Only full-screen scrolls push to scrollback; scroll-region scrolls
        // rotate within the visible area and must NOT advance display_offset.
        let mut pushed_to_scrollback = false;

        if self.scroll_top == 0 && self.scroll_bottom == self.height {
            // Full-screen scroll (most common case):
            // Drain first n rows → push to scrollback, then append n blanks.
            // This is O(n) instead of O(rows × n) with per-line remove(0).
            let drained: Vec<Row> = self.rows.drain(..n).collect();
            for row in drained {
                self.push_scrollback(row);
            }
            self.rows.extend((0..n).map(|_| Row::new(self.width)));
            pushed_to_scrollback = true;
        } else {
            // Scroll region (DECSTBM): rotate region [T..B) left by n,
            // then fill the last n positions of the region with blank rows.
            self.rows[self.scroll_top..self.scroll_bottom].rotate_left(n);
            for i in 0..n {
                self.rows[self.scroll_bottom - 1 - i] = Row::new(self.width);
            }
        }

        if was_scrolled && pushed_to_scrollback {
            // Keep the user at the same scrollback position by advancing offset.
            // Only advance when rows were actually pushed to scrollback (full-screen
            // scroll). Scroll-region scrolls don't add to scrollback, so advancing
            // would incorrectly shift the viewport.
            self.display_offset = self.display_offset.saturating_add(n);
        }
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
        if self.scroll_top == 0 && self.scroll_bottom == self.height {
            // Full-screen scroll: truncate last n rows, restore from scrollback.
            // This is O(n) instead of O(rows × n) with per-line remove/insert.
            let len = self.rows.len();
            self.rows.truncate(len.saturating_sub(n));
            let mut restored = Vec::with_capacity(n);
            for _ in 0..n {
                // Resize restored rows to current grid width — they may have
                // been pushed to scrollback when the grid was a different size.
                let mut row = self
                    .scrollback
                    .pop_back()
                    .unwrap_or_else(|| Row::new(self.width));
                row.resize(self.width);
                restored.push(row);
            }
            // pop_back gives most-recent-first; reverse for chronological order.
            restored.reverse();
            self.rows.splice(0..0, restored);
        } else {
            // Scroll region: rotate [T..B) right by n, fill first n with blanks.
            self.rows[self.scroll_top..self.scroll_bottom].rotate_right(n);
            for i in 0..n {
                self.rows[self.scroll_top + i] = Row::new(self.width);
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
        // Rotate region right by count, then fill the vacated top with blanks.
        // O(region_height) instead of O(count × region_height) with remove/insert.
        self.rows[row..self.scroll_bottom].rotate_right(count);
        for i in 0..count {
            self.rows[row + i] = Row::new(self.width);
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
        // Rotate region left by count, then fill the vacated bottom with blanks.
        // O(region_height) instead of O(count × region_height) with remove/insert.
        self.rows[row..self.scroll_bottom].rotate_left(count);
        for i in 0..count {
            self.rows[self.scroll_bottom - 1 - i] = Row::new(self.width);
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
            self.content_dirty = true;
        }
    }

    /// Delete `count` characters at `(col, row)` (ANSI DCH).
    pub fn delete_char(&mut self, col: usize, row: usize, count: usize) {
        if let Some(r) = self.rows.get_mut(row) {
            r.delete_char(col, count);
            self.damage.mark_row(row);
            self.content_dirty = true;
        }
    }

    /// Erase `count` characters from `(col, row)` (ANSI ECH).
    pub fn erase_char(&mut self, col: usize, row: usize, count: usize) {
        if let Some(r) = self.rows.get_mut(row) {
            r.erase_char(col, count);
            self.damage.mark_rect(col, row, count, 1);
            self.content_dirty = true;
        }
    }

    /// Insert `count` blank columns at `(col, row)` for every row in
    /// the scroll region (DECIC — Insert Column).
    /// Cells to the right of `col` shift right; cells pushed past the
    /// right edge are lost.
    pub fn insert_column(&mut self, col: usize, count: usize) {
        if col >= self.width {
            return;
        }
        let count = count.min(self.width - col);
        for row in 0..self.height {
            if let Some(r) = self.rows.get_mut(row) {
                r.insert_char(col, count);
            }
        }
        self.damage.mark_all(self.height);
        self.content_dirty = true;
    }

    /// Delete `count` columns at `(col, row)` for every row (DECDC — Delete Column).
    /// Cells to the right shift left; blank cells fill the right edge.
    pub fn delete_column(&mut self, col: usize, count: usize) {
        if col >= self.width {
            return;
        }
        let count = count.min(self.width - col);
        for row in 0..self.height {
            if let Some(r) = self.rows.get_mut(row) {
                r.delete_char(col, count);
            }
        }
        self.damage.mark_all(self.height);
        self.content_dirty = true;
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

    /// Clear all scrollback history (ED mode 3 / OSC 1337 ClearScrollback).
    pub fn clear_scrollback(&mut self) {
        self.scrollback.clear();
        self.scrollback.shrink_to_fit();
        self.display_offset = 0;
        self.damage.mark_all(self.height);
        self.content_dirty = true;
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

    /// Export the entire terminal content (scrollback + visible screen) as plain text.
    ///
    /// Lines are joined with `\n`. Trailing whitespace is trimmed per line.
    /// Empty trailing lines are omitted.
    pub fn export_text(&self) -> String {
        // Collect all rows, then merge soft-wrapped lines (wrap=true)
        // so the exported text preserves original line structure.
        let all_rows: Vec<&Row> = self.scrollback.iter().chain(self.rows.iter()).collect();
        let mut lines: Vec<String> = Vec::with_capacity(all_rows.len());

        let mut current = String::new();
        for row in &all_rows {
            current.push_str(&row.text());
            if row.wrap {
                // Soft-wrapped: continue on the same logical line.
                continue;
            }
            lines.push(std::mem::take(&mut current));
        }
        // Handle trailing soft-wrapped content.
        if !current.is_empty() {
            lines.push(current);
        }

        // Trim trailing empty lines
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }

        lines.join("\n")
    }

    /// Export only the visible terminal screen (no scrollback history).
    ///
    /// This is the text currently visible on screen, excluding scrolled-off
    /// scrollback. Useful for quickly copying the current terminal state.
    pub fn export_visible_text(&self) -> String {
        let mut lines: Vec<String> = Vec::with_capacity(self.height);
        for row in &self.rows {
            // row.text() already trims trailing whitespace.
            lines.push(row.text());
        }
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        lines.join("\n")
    }

    /// Set the maximum scrollback capacity.
    /// Truncates existing scrollback if new limit is smaller.
    pub fn set_scrollback(&mut self, max: usize) {
        self.max_scrollback = max;
        while self.scrollback.len() > max {
            self.scrollback.pop_front();
        }
        // Clamp display_offset to valid range after trimming.
        if self.display_offset > self.scrollback.len() {
            self.display_offset = self.scrollback.len();
        }
    }

    /// Export terminal output as an HTML document with ANSI colors preserved.
    ///
    /// Generates a self-contained HTML page with inline CSS that reproduces
    /// the terminal's colors (fg, bg, bold, italic, underline, reverse video).
    /// Useful for sharing terminal output in documentation or bug reports.
    pub fn export_html(&self) -> String {
        use crate::grid::cell::{CellFlags, Color};

        let palette = Color::default_palette();

        let mut html = String::with_capacity(8192);
        html.push_str("<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n");
        html.push_str("<style>\n");
        html.push_str("body { background: #1e1e1e; color: #d4d4d4; ");
        html.push_str("font-family: 'Menlo', 'DejaVu Sans Mono', 'Cascadia Mono', monospace; ");
        html.push_str("font-size: 14px; line-height: 1.4; padding: 16px; margin: 0; }\n");
        html.push_str("pre { margin: 0; white-space: pre-wrap; }\n");
        html.push_str("</style>\n</head>\n<body>\n<pre>\n");

        // Helper: resolve a Color to CSS rgb string.
        let resolve_color = |color: &Color, is_fg: bool| -> String {
            match color {
                Color::Default => {
                    if is_fg {
                        "#d4d4d4".to_string()
                    } else {
                        "#1e1e1e".to_string()
                    }
                }
                Color::Indexed(idx) => {
                    if (*idx as usize) < palette.len() {
                        match &palette[*idx as usize] {
                            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
                            _ => {
                                if is_fg {
                                    "#d4d4d4".to_string()
                                } else {
                                    "#1e1e1e".to_string()
                                }
                            }
                        }
                    } else {
                        // 256-color extensions — approximate.
                        "#d4d4d4".to_string()
                    }
                }
                Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
            }
        };

        // Process all rows (scrollback + visible).
        let all_rows: Vec<&Row> = self.scrollback.iter().chain(self.rows.iter()).collect();

        for row in &all_rows {
            for cell in &row.cells {
                if cell.flags.contains(CellFlags::WIDE_SPACER) {
                    continue;
                }
                if cell.ch == '\0' || cell.ch == ' ' && cell.fg == Color::Default {
                    html.push(' ');
                    continue;
                }

                // Build inline style.
                let mut styles: Vec<String> = Vec::new();
                let fg = if cell.flags.contains(CellFlags::REVERSE) {
                    &cell.bg
                } else {
                    &cell.fg
                };
                let bg = if cell.flags.contains(CellFlags::REVERSE) {
                    &cell.fg
                } else {
                    &cell.bg
                };

                if *fg != Color::Default {
                    styles.push(format!("color: {}", resolve_color(fg, true)));
                }
                if *bg != Color::Default {
                    styles.push(format!("background-color: {}", resolve_color(bg, false)));
                }
                if cell.flags.contains(CellFlags::BOLD) {
                    styles.push("font-weight: bold".to_string());
                }
                if cell.flags.contains(CellFlags::ITALIC) {
                    styles.push("font-style: italic".to_string());
                }
                if cell.flags.contains(CellFlags::UNDERLINE) {
                    styles.push("text-decoration: underline".to_string());
                }
                if cell.flags.contains(CellFlags::STRIKETHROUGH) {
                    styles.push("text-decoration: line-through".to_string());
                }
                if cell.flags.contains(CellFlags::HIDDEN) {
                    styles.push("visibility: hidden".to_string());
                }

                if styles.is_empty() {
                    html.push_str(&html_escape_char(cell.ch));
                    for &mc in &cell.combining {
                        html.push_str(&html_escape_char(mc));
                    }
                } else {
                    let mut content = html_escape_char(cell.ch);
                    for &mc in &cell.combining {
                        content.push_str(&html_escape_char(mc));
                    }
                    html.push_str(&format!(
                        "<span style=\"{}\">{}</span>",
                        styles.join("; "),
                        content
                    ));
                }
            }
            html.push('\n');
        }

        html.push_str("</pre>\n</body>\n</html>\n");
        html
    }

    /// Push a row to the scrollback, evicting oldest if over capacity.
    fn push_scrollback(&mut self, row: Row) {
        // If scrollback is disabled (max_scrollback == 0, e.g. alt screen),
        // do not accumulate any rows.
        if self.max_scrollback == 0 {
            return;
        }
        if self.scrollback.len() >= self.max_scrollback {
            self.scrollback.pop_front();
            self.total_evicted += 1;
        }
        self.scrollback.push_back(row);
    }

    /// Total scrollback rows evicted since Grid creation.
    pub fn total_evicted(&self) -> usize {
        self.total_evicted
    }

    /// Update the scrollback capacity, evicting oldest rows if shrinking.
    ///
    /// Tracks evicted rows in `total_evicted` so that command marks
    /// (OSC 133) referencing absolute scrollback rows can be adjusted.
    pub fn set_max_scrollback(&mut self, max: usize) {
        self.max_scrollback = max;
        while self.scrollback.len() > max {
            self.scrollback.pop_front();
            self.total_evicted += 1;
        }
        // Clamp display_offset to the new (possibly smaller) scrollback
        // length. Without this, a scrollback capacity reduction (e.g. config
        // reload) while the user is scrolled up would leave display_offset
        // pointing past the end of scrollback, showing stale/blank content.
        if self.display_offset > self.scrollback.len() {
            self.display_offset = self.scrollback.len();
            self.damage.mark_all(self.height);
            self.content_dirty = true;
        }
    }

    /// Return the current scrollback limit.
    pub fn max_scrollback(&self) -> usize {
        self.max_scrollback
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

    /// Set the display offset directly (clamped to scrollback length).
    pub fn set_display_offset(&mut self, offset: usize) {
        let max = self.scrollback.len();
        let new_offset = offset.min(max);
        if new_offset != self.display_offset {
            self.display_offset = new_offset;
            self.damage.mark_all(self.height);
            self.content_dirty = true;
        }
    }

    /// Scroll viewport so that `grid_row` is centered in the visible area.
    ///
    /// `grid_row` is a Y coordinate in the *current* grid (0 = top visible row).
    /// Since scrollback content shifts the grid up by `display_offset` lines,
    /// we convert the grid row to an absolute position and compute the offset
    /// that centers it.
    pub fn scroll_to_grid_row(&mut self, grid_row: usize) {
        // grid_row indexes from the top of the entire content (scrollback + visible).
        // Rows < scrollback_len are in scrollback; rows >= scrollback_len are visible.
        // If the target is in the visible area and we're at the bottom, stay.
        // Otherwise, center the target in the viewport.
        let abs_bottom = self.scrollback.len() + self.height;
        let abs_target = self.scrollback.len() + grid_row;
        let desired_offset = abs_bottom.saturating_sub(abs_target + self.height / 2);
        self.set_display_offset(desired_offset);
    }

    /// Get a row by absolute position (scrollback + visible).
    ///
    /// `abs_row` 0 is the oldest row in scrollback.
    /// Rows < scrollback_len are in scrollback; rows >= scrollback_len
    /// are in the visible grid.
    pub fn absolute_row(&self, abs_row: usize) -> Option<&Row> {
        let sb_len = self.scrollback.len();
        if abs_row < sb_len {
            self.scrollback.get(abs_row)
        } else {
            self.rows.get(abs_row - sb_len)
        }
    }

    /// Scroll viewport to center an absolute row.
    ///
    /// `abs_row` indexes from the top of all content (0 = oldest scrollback).
    pub fn scroll_to_absolute_row(&mut self, abs_row: usize) {
        let abs_bottom = self.scrollback.len() + self.height;
        let desired_offset = abs_bottom.saturating_sub(abs_row + self.height / 2);
        self.set_display_offset(desired_offset);
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

/// Escape a single character for HTML output.
fn html_escape_char(ch: char) -> String {
    match ch {
        '<' => "&lt;".to_string(),
        '>' => "&gt;".to_string(),
        '&' => "&amp;".to_string(),
        '"' => "&quot;".to_string(),
        '\'' => "&#39;".to_string(),
        _ => ch.to_string(),
    }
}

/// Re-wrap a single logical line (already merged from soft-wrapped rows)
/// into `width`-column rows. Appends each physical row to `out`.
///
/// Each physical row except the last gets `wrap = true`.
/// Wide char boundaries are respected: if a wide char would straddle
/// a line boundary, it's pushed to the next line.
/// Trim trailing blank cells from a cell vector, keeping at least 1 cell.
/// This prevents blank padding from creating unnecessary rows during reflow.
fn trim_trailing_blanks(cells: &mut Vec<Cell>) {
    // Keep cells up to (and including) the last non-blank cell.
    while cells.len() > 1 && cells.last().is_some_and(|c| c.is_blank()) {
        cells.pop();
    }
}

fn reflow_line(out: &mut Vec<Row>, line: Row, width: usize) {
    use crate::grid::cell::CellFlags;

    let cells = line.cells;
    let total = cells.len();
    if width == 0 || total == 0 {
        out.push(Row::new(width.max(1)));
        return;
    }

    // Optimization / correctness fix: if the entire row is blank (all cells
    // are spaces with no attributes), don't split it into multiple rows when
    // shrinking. A blank line should remain a single blank line regardless
    // of width. Without this, shrinking from e.g. 80→40 columns would turn
    // each blank row into 2 blank rows, pushing real content into scrollback.
    if cells.iter().all(|c| c.is_blank()) {
        out.push(Row {
            cells: vec![Cell::blank(); width],
            wrap: false,
        });
        return;
    }

    let mut start = 0;
    while start < total {
        let mut end = (start + width).min(total);
        // If the last cell in this chunk is a wide char lead (not a spacer),
        // the spacer would be orphaned at the next line start.
        // Only include the spacer if the chunk won't exceed width.
        // If including it would overflow, push the wide char to the next row
        // by padding the current row with a blank at the end.
        if end < total
            && end > start
            && cells[end - 1].flags.contains(CellFlags::WIDE_CHAR)
            && end == start + width
        {
            // Wide char at exact boundary — can't fit spacer. Push wide char
            // to next line by decrementing end (leave a blank at the end).
            end -= 1;
        } else if end < total && end > start && cells[end - 1].flags.contains(CellFlags::WIDE_CHAR)
        {
            // There's room (end < start + width) — include the spacer.
            end += 1;
        }
        // Ensure we always make progress: if the above adjustments pushed
        // end back to start (e.g. width=1 with a wide char), force advance
        // by at least one cell. The wide char lead will be blanked by the
        // truncate logic below since it can't fit in 1 column.
        if end <= start {
            end = start + 1;
        }
        // If the first cell of the next chunk is a wide spacer (orphaned),
        // consume it here — it was part of the current row's wide char.
        if end < total && cells[end].flags.contains(CellFlags::WIDE_SPACER) {
            end += 1;
        }

        let mut chunk: Vec<Cell> = cells[start..end].to_vec();
        let is_last = end >= total;
        let is_continued = !is_last; // this row continues on the next line

        // Pad to width if short.
        if chunk.len() < width {
            chunk.resize(width, Cell::blank());
        }
        // Truncate if over. A wide lead at the truncation boundary gets
        // replaced with a blank so it's not split from its spacer.
        if chunk.len() > width {
            if chunk[width - 1].flags.contains(CellFlags::WIDE_CHAR) {
                chunk[width - 1] = Cell::blank();
            }
            chunk.truncate(width);
        }
        // If the last cell is a wide char lead without its spacer (can happen
        // when width=1 or the force-advance fallback kicks in), blank it
        // to avoid a dangling WIDE_CHAR flag.
        if chunk
            .last()
            .is_some_and(|c| c.flags.contains(CellFlags::WIDE_CHAR))
            && let Some(last) = chunk.last_mut()
        {
            *last = Cell::blank();
        }

        out.push(Row {
            cells: chunk,
            wrap: is_continued,
        });
        start = end;
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
    fn row_delete_char_on_wide_spacer_includes_lead() {
        // Place a wide char at col 1 (lead=1, spacer=2), then 'X' at col 3
        let mut r = Row::new(10);
        r.put_char(0, 'A');
        r.put_char(1, '\u{4E00}'); // CJK wide char: col 1=lead, col 2=spacer
        r.put_char(3, 'X');
        assert!(r[2].is_wide_spacer(), "col 2 should be spacer");
        // Delete 1 cell starting at col 2 (spacer).
        // Wide spacer detection adjusts start to col 1 (lead).
        // Since col 1 is a wide lead with spacer at col 2, and count=1
        // doesn't cover the spacer, deletion extends to include both cells.
        // After delete: A(0) X(1) blank(2)...
        r.delete_char(2, 1);
        assert_eq!(r[0].ch, 'A');
        // Col 1 now has X (shifted left after full wide char deletion)
        assert_eq!(
            r[1].ch, 'X',
            "X should shift to col 1 after wide pair deleted"
        );
        // No orphaned wide spacer
        assert!(!r[1].is_wide_spacer(), "no orphaned spacer at col 1");
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
    fn resize_shrink_clears_dangling_wide_lead() {
        // Place a wide char at col 8 (lead=8, spacer=9) in a 10-wide grid.
        let mut g = Grid::new(10, 1);
        g.put_char(8, 0, '\u{4E00}'); // CJK wide char
        assert!(g[(8, 0)].is_wide());
        assert!(g[(9, 0)].is_wide_spacer());
        // Shrink to 9 columns — spacer at col 9 is truncated.
        g.resize(9, 1);
        // The lead at col 8 should be cleared (no spacer to pair with).
        assert!(!g[(8, 0)].is_wide(), "dangling wide lead should be cleared");
        assert_eq!(g[(8, 0)].ch, ' ');
    }

    #[test]
    fn resize_shrink_to_scrollback() {
        let mut g = Grid::new(10, 5);
        g.resize(10, 3);
        assert_eq!(g.height(), 3);
        assert_eq!(g.scrollback_len(), 2);
    }

    #[test]
    fn resize_preserves_scroll_position() {
        // When scrolling up in scrollback and then resizing, the scroll
        // position should be preserved (not reset to bottom).
        let mut g = Grid::with_scrollback(10, 3, 100);
        // Fill content to create scrollback
        for i in 0..10 {
            g[(0, 0)] = Cell::with_char((b'A' + i as u8) as char);
            g.scroll_up(1);
        }
        assert!(g.scrollback_len() > 0);
        // Scroll up 3 lines
        g.scroll_up_viewport(3);
        assert_eq!(g.display_offset(), 3);
        // Resize — scroll position should be preserved
        g.resize(10, 5);
        assert_eq!(
            g.display_offset(),
            3,
            "scroll position should survive resize"
        );
    }

    #[test]
    fn resize_clamps_scroll_position() {
        // If scrollback shrinks during resize, offset should be clamped.
        let mut g = Grid::with_scrollback(10, 3, 100);
        for i in 0..10 {
            g[(0, 0)] = Cell::with_char((b'A' + i as u8) as char);
            g.scroll_up(1);
        }
        let scrollback_len = g.scrollback_len();
        g.scroll_up_viewport(scrollback_len);
        assert_eq!(g.display_offset(), scrollback_len);
        // Clear scrollback and resize — offset should clamp to 0
        g.clear_scrollback();
        g.resize(10, 5);
        assert_eq!(g.display_offset(), 0);
    }

    #[test]
    fn scroll_up_preserves_user_scrollback_position() {
        // When the user is scrolled up in scrollback and new output arrives
        // (scroll_up), the display_offset should advance to keep the user
        // viewing the same scrollback content. Only auto-scroll to bottom
        // when the user is already at the bottom (offset == 0).
        let mut g = Grid::with_scrollback(10, 3, 100);
        // Fill some content to create scrollback
        for i in 0..5 {
            g[(0, 0)] = Cell::with_char((b'A' + i as u8) as char);
            g.scroll_up(1);
        }
        assert!(g.scrollback_len() >= 5);
        // User scrolls up 3 lines to read history
        g.scroll_up_viewport(3);
        let offset_before = g.display_offset();
        assert_eq!(offset_before, 3);
        // New output arrives — scroll_up should preserve position
        g[(0, 0)] = Cell::with_char('Z');
        g.scroll_up(1);
        // Offset should have advanced by 1, not reset to 0
        assert_eq!(
            g.display_offset(),
            offset_before + 1,
            "scroll position should advance when user is scrolled up"
        );
    }

    #[test]
    fn scroll_up_auto_scrolls_when_at_bottom() {
        // When user is at the bottom (offset == 0), new output should NOT
        // change the offset — it stays at 0 (showing latest content).
        let mut g = Grid::with_scrollback(10, 3, 100);
        for i in 0..5 {
            g[(0, 0)] = Cell::with_char((b'A' + i as u8) as char);
            g.scroll_up(1);
        }
        assert_eq!(g.display_offset(), 0, "should be at bottom");
        // New output arrives — should stay at bottom
        g[(0, 0)] = Cell::with_char('Z');
        g.scroll_up(1);
        assert_eq!(g.display_offset(), 0, "should auto-scroll to bottom");
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

    #[test]
    fn set_max_scrollback_tracks_eviction() {
        // Reducing scrollback capacity should evict rows and update total_evicted.
        let mut g = Grid::with_scrollback(3, 2, 5);
        for i in 0..5u8 {
            g[(0, 0)] = Cell::with_char((b'A' + i) as char);
            g.scroll_up(1);
        }
        assert_eq!(g.scrollback_len(), 5);
        assert_eq!(g.total_evicted(), 0);

        // Reduce capacity to 3 — should evict 2 rows.
        g.set_max_scrollback(3);
        assert_eq!(g.scrollback_len(), 3);
        assert_eq!(
            g.total_evicted(),
            2,
            "set_max_scrollback should track evicted rows for mark adjustment"
        );
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
    fn viewport_preserves_position_on_new_scroll() {
        // When the user is scrolled up, new terminal output should NOT
        // reset the viewport — it should advance the offset to keep the
        // user at the same scrollback position.
        let mut g = Grid::with_scrollback(3, 2, 100);
        g.scroll_up(1);
        g.scroll_up_viewport(1);
        assert_eq!(g.display_offset(), 1);
        // New content scrolls — viewport advances (preserving user position).
        g.scroll_up(1);
        assert_eq!(g.display_offset(), 2);
        // When at bottom (offset 0), new output stays at bottom.
        g.scroll_down_viewport(2);
        assert_eq!(g.display_offset(), 0);
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

    // ================================================================
    //  Export text (3 tests)
    // ================================================================

    #[test]
    fn test_export_text_visible_only() {
        let mut g = Grid::new(10, 3);
        g.put_char(0, 0, 'H');
        g.put_char(1, 0, 'i');
        g.put_char(0, 1, 'W');
        g.put_char(1, 1, 'o');
        g.put_char(2, 1, 'r');
        g.put_char(3, 1, 'l');
        g.put_char(4, 1, 'd');

        let text = g.export_text();
        assert_eq!(text, "Hi\nWorld");
    }

    #[test]
    fn test_export_text_with_scrollback() {
        let mut g = Grid::with_scrollback(10, 2, 100);
        // Fill first row, then scroll it to scrollback
        g.put_char(0, 0, 'O');
        g.put_char(1, 0, 'l');
        g.put_char(2, 0, 'd');
        g.scroll_up(1);
        // Row 0 is now blank (new), write on row 0
        g.put_char(0, 0, 'N');
        g.put_char(1, 0, 'e');
        g.put_char(2, 0, 'w');

        let text = g.export_text();
        assert_eq!(text, "Old\nNew");
    }

    #[test]
    fn test_export_text_trims_trailing_empty() {
        let g = Grid::new(10, 5);
        // Only row 0 has content
        let mut g = g;
        g.put_char(0, 0, 'X');

        let text = g.export_text();
        assert_eq!(text, "X");
    }

    // ================================================================
    //  Reflow resize (DECSET 2027)
    // ================================================================

    #[test]
    fn reflow_grow_pulls_from_scrollback() {
        let mut g = Grid::with_scrollback(10, 3, 100);
        // Fill content to create scrollback
        for i in 0..5 {
            g[(0, 0)] = Cell::with_char((b'A' + i as u8) as char);
            g.scroll_up(1);
        }
        assert!(g.scrollback_len() >= 5);

        let sb_before = g.scrollback_len();
        // Reflow resize: grow height from 3 to 6
        g.reflow_resize(10, 6);
        assert_eq!(g.height(), 6);
        // Scrollback should be smaller (rows pulled back)
        assert!(g.scrollback_len() < sb_before);
    }

    #[test]
    fn reflow_grow_empty_scrollback_adds_blank() {
        let mut g = Grid::new(10, 3);
        g[(0, 0)] = Cell::with_char('X');
        // No scrollback — reflow grow should just add blank rows
        g.reflow_resize(10, 6);
        assert_eq!(g.height(), 6);
        assert_eq!(g[(0, 0)].ch, 'X');
        assert_eq!(g.scrollback_len(), 0);
    }

    #[test]
    fn reflow_shrink_pushes_to_scrollback() {
        let mut g = Grid::new(10, 5);
        g[(0, 0)] = Cell::with_char('A');
        g[(0, 1)] = Cell::with_char('B');
        // Shrink
        g.reflow_resize(10, 3);
        assert_eq!(g.height(), 3);
        assert_eq!(g.scrollback_len(), 2);
    }

    #[test]
    fn reflow_width_change_resizes_rows() {
        let mut g = Grid::new(10, 3);
        g[(0, 0)] = Cell::with_char('X');
        g.reflow_resize(15, 3);
        assert_eq!(g.width(), 15);
        assert_eq!(g[(0, 0)].ch, 'X');
    }

    #[test]
    fn reflow_height_only_preserves_content() {
        // Fast path: width unchanged, height grows — content must be preserved.
        let mut g = Grid::with_scrollback(10, 3, 100);
        g[(0, 0)] = Cell::with_char('A');
        g[(1, 0)] = Cell::with_char('B');
        g[(2, 0)] = Cell::with_char('C');
        g.reflow_resize(10, 5);
        assert_eq!(g.width(), 10);
        assert_eq!(g.height(), 5);
        assert_eq!(g[(0, 0)].ch, 'A');
        assert_eq!(g[(1, 0)].ch, 'B');
        assert_eq!(g[(2, 0)].ch, 'C');
    }

    #[test]
    fn reflow_height_only_shrink_pushes_bottom() {
        // Fast path: width unchanged, height shrinks — TOP rows go to
        // scrollback, BOTTOM rows (cursor/recent output) stay visible.
        let mut g = Grid::new(10, 5);
        g[(0, 0)] = Cell::with_char('T'); // top
        g[(0, 4)] = Cell::with_char('B'); // bottom
        g.reflow_resize(10, 3);
        assert_eq!(g.height(), 3);
        assert_eq!(g.scrollback_len(), 2);
        // 'T' (old row 0) is pushed to scrollback, 'B' (old row 4) stays visible
        // at new row 2 (old row 4 - 2 pushed rows = row 2).
        assert_eq!(
            g[(0, 2)].ch,
            'B',
            "bottom content stays visible after shrink"
        );
    }

    #[test]
    fn set_display_offset_clamps() {
        let mut g = Grid::with_scrollback(5, 4, 100);
        g.scroll_up(4); // push 4 rows to scrollback (height = 4)
        g.set_display_offset(10); // over-scroll
        assert_eq!(g.display_offset(), 4); // clamped to scrollback_len
        g.set_display_offset(1);
        assert_eq!(g.display_offset(), 1);
        g.set_display_offset(0);
        assert_eq!(g.display_offset(), 0);
    }

    #[test]
    fn set_display_offset_marks_dirty() {
        let mut g = Grid::with_scrollback(5, 4, 100);
        g.scroll_up(4);
        g.clear_damage();
        assert!(!g.is_dirty());
        g.set_display_offset(2);
        assert!(g.is_dirty());
    }

    #[test]
    fn scroll_to_grid_row_centers() {
        let mut g = Grid::with_scrollback(5, 10, 100);
        g.scroll_up(5); // 5 rows in scrollback
        g.scroll_up_viewport(5); // fully scrolled back
        assert_eq!(g.display_offset(), 5);
        // Scroll to grid row 5 (middle of viewport).
        g.scroll_to_grid_row(5);
        // Should center: abs_bottom = 5 + 10 = 15, abs_target = 5 + 5 = 10
        // desired = 15 - 10 - 5 = 0 → shows bottom
        assert_eq!(g.display_offset(), 0);
    }

    #[test]
    fn scroll_to_grid_row_works_from_bottom() {
        let mut g = Grid::with_scrollback(5, 10, 100);
        g.scroll_up(5);
        // From bottom, scroll_to_grid_row(0) centers row 0 in viewport.
        // abs_bottom = 5+10=15, abs_target = 5+0=5, offset = 15-(5+5) = 5.
        g.scroll_to_grid_row(0);
        assert_eq!(g.display_offset(), 5);
    }

    #[test]
    fn export_html_basic() {
        let mut g = Grid::new(10, 3);
        g[(0, 0)] = Cell::with_char('H');
        g[(1, 0)] = Cell::with_char('i');
        let html = g.export_html();
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("</html>"));
        assert!(html.contains("<pre>"));
        assert!(html.contains("Hi"));
    }

    #[test]
    fn export_html_escapes_special_chars() {
        let mut g = Grid::new(10, 1);
        g[(0, 0)] = Cell::with_char('<');
        g[(1, 0)] = Cell::with_char('>');
        g[(2, 0)] = Cell::with_char('&');
        let html = g.export_html();
        assert!(html.contains("&lt;"));
        assert!(html.contains("&gt;"));
        assert!(html.contains("&amp;"));
    }

    #[test]
    fn export_html_preserves_colors() {
        use crate::grid::cell::Color;
        let mut g = Grid::new(10, 1);
        let mut cell = Cell::with_char('R');
        cell.fg = Color::Indexed(1); // red
        g[(0, 0)] = cell;
        let html = g.export_html();
        assert!(html.contains("color:"));
        assert!(html.contains("#cc0000")); // palette red
    }

    #[test]
    fn export_html_preserves_bold() {
        use crate::grid::cell::CellFlags;
        let mut g = Grid::new(10, 1);
        let mut cell = Cell::with_char('B');
        cell.flags.insert(CellFlags::BOLD);
        g[(0, 0)] = cell;
        let html = g.export_html();
        assert!(html.contains("font-weight: bold"));
    }

    #[test]
    fn export_html_preserves_reverse_video() {
        use crate::grid::cell::{CellFlags, Color};
        let mut g = Grid::new(10, 1);
        let mut cell = Cell::with_char('X');
        cell.fg = Color::Rgb(255, 0, 0);
        cell.bg = Color::Rgb(0, 0, 255);
        cell.flags.insert(CellFlags::REVERSE);
        g[(0, 0)] = cell;
        let html = g.export_html();
        // In reverse, fg becomes bg and vice versa
        assert!(html.contains("background-color: #ff0000")); // fg was red
        assert!(html.contains("color: #0000ff")); // bg was blue
    }

    #[test]
    fn export_visible_text_basic() {
        let g = Grid::with_scrollback(4, 3, 100);
        let visible = g.export_visible_text();
        // Empty grid should produce empty string (all rows blank → trimmed)
        assert!(
            visible.is_empty(),
            "empty grid should produce empty: {visible:?}"
        );

        // With content
        let mut g2 = Grid::with_scrollback(4, 3, 100);
        g2[(1, 0)] = Cell::with_char('H');
        g2[(1, 1)] = Cell::with_char('i');
        let v2 = g2.export_visible_text();
        assert!(
            v2.contains('H') && v2.contains('i'),
            "should contain Hi: {v2:?}"
        );
    }

    // ── Reflow tests ──────────────────────────────────────────────

    #[test]
    fn reflow_shrink_splits_long_line() {
        // One 80-col line with no wrap → after shrink to 40, should be two rows.
        let mut g = Grid::new(80, 2);
        for c in 0..80 {
            g[(c, 0)] = Cell::with_char('X');
        }
        g.reflow_resize(40, 2);
        assert_eq!(g.width(), 40);
        // The content should be split across visible rows or scrollback.
        let text = g.export_text();
        let x_count = text.matches('X').count();
        assert_eq!(x_count, 80, "all 80 X chars should survive shrink: {text}");
    }

    #[test]
    fn reflow_grow_merges_soft_wrapped() {
        // Two 40-col rows connected by wrap=true → grow to 80 should merge.
        let mut g = Grid::new(40, 3);
        for c in 0..40 {
            g[(c, 0)] = Cell::with_char('A');
        }
        g.row_mut(0).unwrap().wrap = true; // soft-wrapped
        for c in 0..40 {
            g[(c, 1)] = Cell::with_char('B');
        }
        // Row 1 has wrap=false (hard newline)

        g.reflow_resize(80, 3);
        let text = g.export_text();
        // Row 0+1 merged into one 80-col line: AAAA...BBBB...
        assert!(
            text.contains('A') && text.contains('B'),
            "merged content should survive: {text}"
        );
        // There should be no gap between A and B (they're on the same line now)
        let first_line = text.lines().next().unwrap_or("");
        assert!(
            first_line.contains('A') && first_line.contains('B'),
            "A and B should be on the same line after merge: {first_line}"
        );
    }

    #[test]
    fn reflow_hard_newline_not_merged() {
        // Two rows, both wrap=false → grow should NOT merge them.
        let mut g = Grid::new(40, 3);
        for c in 0..40 {
            g[(c, 0)] = Cell::with_char('A');
        }
        // wrap defaults to false
        for c in 0..40 {
            g[(c, 1)] = Cell::with_char('B');
        }

        g.reflow_resize(80, 3);
        let text = g.export_text();
        let lines: Vec<&str> = text.lines().collect();
        // Each line stays separate
        let has_a = lines.iter().any(|l| l.contains('A'));
        let has_b = lines.iter().any(|l| l.contains('B'));
        assert!(has_a && has_b, "both lines should survive");
        // Verify they're on different lines
        let first_line = lines[0];
        assert!(
            !(first_line.contains('A') && first_line.contains('B')),
            "A and B should NOT be merged when wrap=false"
        );
    }

    #[test]
    fn reflow_short_nonwrapped_line_no_blank_split() {
        // "Hello World" at width 80 → shrink to 40.
        let mut g = Grid::new(80, 3);
        for (i, ch) in "Hello World".chars().enumerate() {
            g[(i, 0)] = Cell::with_char(ch);
        }
        // Verify content before reflow
        assert_eq!(g[(0, 0)].ch, 'H', "content should be set before reflow");

        g.reflow_resize(40, 3);

        // After reflow: row 0 should have content, row 1 should be fresh blank
        let r0 = g.row(0).unwrap();
        let r1 = g.row(1).unwrap();
        let r0_has_content = r0.cells.iter().any(|c| c.ch != ' ');
        let r1_has_content = r1.cells.iter().any(|c| c.ch != ' ');
        assert!(r0_has_content, "row 0 should have content after reflow");
        // The bug: r1 has content (blank continuation cells from old row)
        // Correct: r0 has content and wrap=false (fits in 40 cols)
        // Bug: r0 has content with wrap=true, r1 is all-blank continuation
        if r0.wrap {
            // This is the bug — short line was unnecessarily split
            panic!(
                "short non-wrapped line was split: row0.wrap={}, row1_has_content={}",
                r0.wrap, r1_has_content
            );
        }
    }

    #[test]
    fn reflow_wide_char_boundary() {
        // A wide char (CJK) at the boundary should move to the next line
        // rather than being split.
        use crate::grid::cell::CellFlags;
        let mut g = Grid::new(10, 2);
        // Fill cols 0-7 with 'A', put a wide char at col 8 (cells 8+9)
        for c in 0..8 {
            g[(c, 0)] = Cell::with_char('A');
        }
        let mut wide = Cell::with_char('\u{4E2D}'); // 中
        wide.flags.insert(CellFlags::WIDE_CHAR);
        g[(8, 0)] = wide;
        let mut spacer = Cell::blank();
        spacer.flags.insert(CellFlags::WIDE_SPACER);
        g[(9, 0)] = spacer;

        // Shrink to 9 cols — the wide char at col 8 would be split
        // (only 1 col remaining). Reflow should push it to the next line.
        g.reflow_resize(9, 2);
        let text = g.export_text();
        // The wide char should still exist
        assert!(
            text.contains('\u{4E2D}'),
            "wide char should survive reflow: {text}"
        );
    }

    #[test]
    fn reflow_scrollback_preserved() {
        // Content in scrollback should also be reflowed.
        let mut g = Grid::new(10, 1);
        // Fill the row and scroll it into scrollback by adding more lines.
        for c in 0..10 {
            g[(c, 0)] = Cell::with_char('S');
        }
        // Trigger scroll-up by line-feeding past the bottom.
        // Simulate: push rows to scrollback manually via resize shrink.
        g.reflow_resize(10, 1); // no-op but sets up state
        // Directly test: shrink width, content should survive in scrollback.
        g.reflow_resize(5, 1);
        let text = g.export_text();
        let s_count = text.matches('S').count();
        assert_eq!(
            s_count, 10,
            "all S chars should survive in scrollback after shrink: {text}"
        );
    }

    #[test]
    fn reflow_then_grow_restores_content() {
        // Shrink then grow should restore content (not lose chars).
        let mut g = Grid::new(80, 5);
        for c in 0..80 {
            g[(c, 0)] = Cell::with_char('Z');
        }
        g.reflow_resize(40, 5);
        let mid_text = g.export_text();
        let mid_z = mid_text.matches('Z').count();
        assert_eq!(mid_z, 80, "content preserved after shrink");

        g.reflow_resize(80, 5);
        let final_text = g.export_text();
        let final_z = final_text.matches('Z').count();
        assert_eq!(
            final_z, 80,
            "content preserved after grow-back: {final_text}"
        );
    }

    #[test]
    fn resize_non_reflow_preserves_wrap_field() {
        // The non-reflow resize path should not panic with the new wrap field.
        let mut g = Grid::new(20, 3);
        for c in 0..20 {
            g[(c, 0)] = Cell::with_char('X');
        }
        g.resize(10, 2);
        assert_eq!(g.width(), 10);
        // Wrap should default to false on all rows.
        for r in 0..g.height() {
            if let Some(row) = g.row(r) {
                assert!(!row.wrap, "wrap should be false after non-reflow resize");
            }
        }
    }

    #[test]
    fn scroll_up_preserves_display_offset() {
        // When user is scrolled up in history and new content scrolls,
        // the display_offset should be advanced to keep the same view.
        let mut g = Grid::with_scrollback(10, 3, 100);
        // Fill 3 rows then scroll to create scrollback
        g.put_char(0, 0, 'A');
        g.put_char(0, 1, 'B');
        g.put_char(0, 2, 'C');
        // Simulate scroll up (push row 0 to scrollback)
        g.scroll_up(1);
        assert_eq!(g.scrollback_len(), 1);
        // Scroll viewport up by 1 to view history
        g.scroll_up_viewport(1);
        assert_eq!(g.display_offset, 1);
        // Now scroll up again — display_offset should advance
        g.scroll_up(1);
        assert_eq!(
            g.display_offset, 2,
            "display_offset should advance when scrolled up"
        );
    }

    #[test]
    fn reflow_wide_char_at_boundary_no_corrupt() {
        // Wide char at exact column boundary during reflow.
        let mut g = Grid::with_scrollback(4, 2, 100);
        g.put_char(0, 0, 'A');
        g.put_char(1, 0, 'B');
        // Wide char at cols 2-3
        g.put_char(2, 0, '\u{4E00}');
        g.set_row_wrap(0, true);
        g.put_char(0, 1, 'C');
        // Reflow to width 3 — wide char doesn't fit at col 2 (needs 2 cols)
        g.reflow_resize(3, 2);
        // Check no orphaned wide char anywhere
        for r in 0..g.height() {
            if let Some(row) = g.row(r) {
                for (c, cell) in row.cells.iter().enumerate() {
                    if cell.is_wide() {
                        // Wide lead must have spacer at next col
                        assert!(
                            c + 1 < row.cells.len(),
                            "WIDE_CHAR at col {} row {} has no spacer (end of row)",
                            c,
                            r
                        );
                        assert!(
                            row.cells[c + 1].is_wide_spacer(),
                            "WIDE_CHAR at col {} row {} missing spacer at col {}",
                            c,
                            r,
                            c + 1
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn resize_grow_pulls_from_scrollback() {
        // Height increase with reflow should pull rows from scrollback into visible area.
        let mut g = Grid::with_scrollback(5, 2, 100);
        // Fill visible area
        for r in 0..2 {
            g.put_char(0, r, (b'A' + r as u8) as char);
        }
        // Create scrollback by scrolling up
        g.scroll_up(1);
        assert_eq!(g.scrollback_len(), 1);
        // Grow from height 2 to 3 using reflow (same width)
        g.reflow_resize(5, 3);
        assert_eq!(g.height(), 3);
        assert_eq!(
            g.scrollback_len(),
            0,
            "scrollback should be drained by reflow"
        );
    }

    #[test]
    fn scroll_down_viewport_clamps_at_zero() {
        // Scrolling viewport down past the bottom should clamp at display_offset = 0.
        let mut g = Grid::with_scrollback(10, 3, 100);
        // Create scrollback
        for _ in 0..5 {
            g.scroll_up(1);
        }
        assert!(g.scrollback_len() > 0);
        // Scroll viewport up
        g.scroll_up_viewport(3);
        assert!(g.display_offset > 0);
        // Scroll viewport down past bottom
        g.scroll_down_viewport(100);
        assert_eq!(g.display_offset, 0, "viewport should clamp at bottom");
    }

    #[test]
    fn reflow_shrink_wide_char_at_boundary() {
        // Width=6: "AB你CD" — A(0) B(1) 你(2-3) C(4) D(5)
        // Shrink to width=3: wide char straddles boundary, should push to next row.
        // Verify no split wide char pairs after reflow.
        let mut g = Grid::new(6, 4);
        g.cell_mut(0, 0).unwrap().ch = 'A';
        g.cell_mut(1, 0).unwrap().ch = 'B';
        assert_eq!(g.put_char(2, 0, '\u{4E00}'), 2); // wide char 你
        g.cell_mut(4, 0).unwrap().ch = 'C';
        g.cell_mut(5, 0).unwrap().ch = 'D';

        g.reflow_resize(3, 4);

        // Check scrollback rows: wide char pair must be intact.
        // Expected: row0="AB " row1="你~C" row2="D  " (~ = spacer)
        let sb_text: Vec<String> = (0..g.scrollback_len())
            .map(|i| {
                g.scrollback_row(i)
                    .map(|r| {
                        r.cells
                            .iter()
                            .map(|c| if c.is_wide_spacer() { '~' } else { c.ch })
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect();

        // Find the row containing the wide char lead — its spacer must follow.
        let mut wide_pair_intact = false;
        for row_text in &sb_text {
            // Wide lead should be followed by '~' (spacer).
            for pair in row_text.as_bytes().windows(2) {
                if pair[0] != b' ' && pair[0] != b'~' && pair[1] == b'~' {
                    wide_pair_intact = true;
                }
            }
        }
        assert!(
            wide_pair_intact,
            "wide char pair must stay intact after reflow, scrollback: {:?}",
            sb_text
        );
        // No orphaned spacer at start of any row
        for row_text in &sb_text {
            assert!(
                !row_text.starts_with('~'),
                "no orphaned wide spacer at row start: {:?}",
                sb_text
            );
        }
    }

    #[test]
    fn reflow_grow_merges_wide_chars() {
        // Width=4: "你好" occupies 2 rows when width=4 (each wide = 2 cols).
        // Row 0: 你(0-1) 好(2-3)
        // Grow to width=8: should merge into one row.
        let mut g = Grid::new(4, 2);
        assert_eq!(g.put_char(0, 0, '\u{4F60}'), 2); // 你
        assert_eq!(g.put_char(2, 0, '\u{597D}'), 2); // 好
        g.set_row_wrap(0, true); // soft-wrapped
        g.reflow_resize(8, 2);
        // Both wide chars should be on row 0
        assert_eq!(g.cell(0, 0).unwrap().ch, '\u{4F60}');
        assert_eq!(g.cell(2, 0).unwrap().ch, '\u{597D}');
        assert!(g.cell(1, 0).unwrap().is_wide_spacer());
        assert!(g.cell(3, 0).unwrap().is_wide_spacer());
    }

    #[test]
    fn reflow_width_one_with_wide_char() {
        // Width=1 with a wide char that needs 2 columns.
        // The wide char can't fit in width=1, so the row should not loop.
        // Previously this caused an infinite loop (end decremented to start).
        let mut g = Grid::new(4, 4);
        assert_eq!(g.put_char(0, 0, '\u{4E00}'), 2); // wide char
        g.reflow_resize(1, 4);
        // Just reaching here means the bug is fixed.
        assert_eq!(g.width(), 1);
    }

    #[test]
    fn scroll_region_up_does_not_advance_display_offset() {
        // When the user has scrolled up in scrollback (display_offset > 0)
        // and content scrolls within a scroll REGION (not full screen),
        // display_offset should NOT advance because no rows are pushed to
        // scrollback. Advancing it would incorrectly shift the viewport.
        let mut grid = Grid::with_scrollback(10, 5, 100);

        // Fill rows with identifiable content.
        for row in 0..5 {
            for col in 0..10 {
                grid[(col, row)] =
                    Cell::with_char(char::from_u32(b'A' as u32 + row as u32).unwrap());
            }
        }

        // Push some content to scrollback so we can scroll up.
        grid.scroll_up(2); // rows A,B pushed to scrollback
        assert_eq!(grid.scrollback_len(), 2);

        // User scrolls up to view scrollback.
        grid.scroll_up_viewport(2);
        assert_eq!(grid.display_offset(), 2);

        // Now set a scroll region (rows 0-3, leaving row 4 outside).
        grid.set_scroll_region(0, 4);

        // Scroll up within the region. This rotates [0..4) — no scrollback change.
        grid.scroll_up(1);

        // display_offset should NOT have changed — no rows were pushed to scrollback.
        assert_eq!(
            grid.display_offset(),
            2,
            "display_offset should not advance for scroll-region scroll"
        );
    }

    #[test]
    fn reflow_no_dangling_wide_char_lead() {
        // After reflow to width=1, the wide char lead that couldn't fit
        // should be blanked, not left with a dangling WIDE_CHAR flag.
        let mut g = Grid::new(4, 4);
        g.put_char(0, 0, 'A');
        g.put_char(1, 0, '\u{4E00}'); // wide char at cols 1-2
        g.put_char(3, 0, 'B');
        g.reflow_resize(1, 8);
        // Check every cell in the first row — none should have WIDE_CHAR
        // without a following WIDE_SPACER.
        for col in 0..g.width() {
            if let Some(c) = g.cell(col, 0)
                && c.flags.contains(crate::grid::cell::CellFlags::WIDE_CHAR)
            {
                // If it's a wide lead, the next cell must be a spacer.
                let next = g.cell(col + 1, 0);
                assert!(
                    next.is_some_and(|n| n.is_wide_spacer()),
                    "dangling WIDE_CHAR at col {col} after reflow to width 1"
                );
            }
        }
    }

    #[test]
    fn t_p131_reflow_preserves_sgr_colors() {
        // SGR colors should be preserved across reflow.
        use crate::grid::cell::CellFlags;
        let mut g = Grid::with_scrollback(10, 2, 100);
        // Write red text then wrap to next line
        let mut cell = Cell::with_char('R');
        cell.fg = crate::Color::Indexed(1); // Red
        g.cell_mut(0, 0).unwrap();
        for col in 0..10 {
            let c = g.cell_mut(col, 0).unwrap();
            c.ch = 'R';
            c.fg = crate::Color::Indexed(1);
        }
        g.set_row_wrap(0, true);
        // Write more on second line
        for col in 0..5 {
            let c = g.cell_mut(col, 1).unwrap();
            c.ch = 'R';
            c.fg = crate::Color::Indexed(1);
        }
        // Shrink to 5 columns — should reflow
        g.reflow_resize(5, 4);
        // All non-blank cells should still have red foreground
        for row in 0..g.height() {
            for col in 0..g.width() {
                if let Some(c) = g.cell(col, row) {
                    if c.ch == 'R' {
                        assert_eq!(
                            c.fg,
                            crate::Color::Indexed(1),
                            "SGR color lost at ({},{}) after reflow",
                            col,
                            row
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn t_p131_reflow_wide_char_at_shrink_boundary() {
        // When shrinking, a wide char pair should not be split across rows.
        // Place content: "AB你CD" at cols 0-5 (6 cols), shrink to 4.
        // 你 is at cols 2-3 (lead + spacer). Width 4 boundary is at col 4.
        // The wide pair fits within the first row.
        let mut g = Grid::with_scrollback(6, 2, 100);
        g.cell_mut(0, 0).unwrap().ch = 'A';
        g.cell_mut(1, 0).unwrap().ch = 'B';
        // Place wide char at cols 2-3
        let lead = g.cell_mut(2, 0).unwrap();
        lead.ch = '你';
        lead.flags.insert(CellFlags::WIDE_CHAR);
        let spacer = g.cell_mut(3, 0).unwrap();
        spacer.ch = '\0';
        spacer.flags.insert(CellFlags::WIDE_SPACER);
        g.cell_mut(4, 0).unwrap().ch = 'C';
        g.cell_mut(5, 0).unwrap().ch = 'D';
        // Shrink to 4 cols
        g.reflow_resize(4, 3);
        // Row 0 should have: A B 你(spacer)
        assert_eq!(g.cell(0, 0).unwrap().ch, 'A');
        assert_eq!(g.cell(1, 0).unwrap().ch, 'B');
        assert_eq!(g.cell(2, 0).unwrap().ch, '你');
        assert!(g.cell(3, 0).unwrap().is_wide_spacer());
        // C D should be on the next row
        assert_eq!(g.cell(0, 1).unwrap().ch, 'C');
        assert_eq!(g.cell(1, 1).unwrap().ch, 'D');
    }

    #[test]
    fn t_p131_reflow_wide_char_pushed_to_next_row() {
        // When shrinking so a wide char pair straddles the boundary,
        // the wide char should be pushed to the next row (not split).
        // Content: "你BC" at cols 0-2 (3 cols), shrink to 2.
        // 你 takes cols 0-1. Shrinking to width 2 means the boundary is col 2.
        // The wide pair (cols 0-1) fits in the first row, B goes to row 2.
        let mut g = Grid::with_scrollback(4, 2, 100);
        let lead = g.cell_mut(0, 0).unwrap();
        lead.ch = '你';
        lead.flags.insert(CellFlags::WIDE_CHAR);
        let spacer = g.cell_mut(1, 0).unwrap();
        spacer.ch = '\0';
        spacer.flags.insert(CellFlags::WIDE_SPACER);
        g.cell_mut(2, 0).unwrap().ch = 'B';
        g.cell_mut(3, 0).unwrap().ch = 'C';
        // Shrink to 2 cols
        g.reflow_resize(2, 4);
        // Row 0: 你(spacer)
        assert_eq!(g.cell(0, 0).unwrap().ch, '你');
        assert!(g.cell(1, 0).unwrap().is_wide_spacer());
        // B C on next row
        assert_eq!(g.cell(0, 1).unwrap().ch, 'B');
        assert_eq!(g.cell(1, 1).unwrap().ch, 'C');
    }

    #[test]
    fn t_p131_reflow_blank_line_not_doubled() {
        // Blank lines should not be doubled when shrinking.
        let mut g = Grid::with_scrollback(10, 4, 100);
        // Row 0-3 are all blank (default)
        // Shrink from 10 to 5 columns
        g.reflow_resize(5, 4);
        // Should still have exactly 4 rows (no doubling)
        assert_eq!(g.height(), 4);
        // All rows should still be blank
        for row in 0..4 {
            for col in 0..5 {
                assert!(
                    g.cell(col, row).unwrap().is_blank(),
                    "expected blank at ({},{})",
                    col,
                    row
                );
            }
        }
    }

    #[test]
    fn t_p131_set_max_scrollback_clamps_display_offset() {
        // When scrollback capacity is reduced while the user is scrolled up,
        // display_offset must be clamped to the new scrollback length.
        let mut g = Grid::with_scrollback(10, 3, 100);
        // Fill content to create scrollback
        for _ in 0..20 {
            for col in 0..10 {
                let c = g.cell_mut(col, 0).unwrap();
                c.ch = 'X';
            }
            g.scroll_up(1);
        }
        assert!(g.scrollback_len() > 0, "should have scrollback");
        let sb_len = g.scrollback_len();
        // Scroll up to view scrollback
        g.scroll_up_viewport(sb_len);
        assert_eq!(g.display_offset(), sb_len);
        // Reduce scrollback capacity to less than current display_offset
        let new_max = sb_len / 2;
        g.set_max_scrollback(new_max);
        // display_offset should be clamped to new scrollback length
        assert!(
            g.display_offset() <= g.scrollback_len(),
            "display_offset {} should be <= scrollback_len {} after capacity reduction",
            g.display_offset(),
            g.scrollback_len()
        );
        assert_eq!(g.scrollback_len(), new_max);
    }

    #[test]
    fn t_p146_reflow_trailing_blank_in_wrapped_row() {
        // When a soft-wrapped row has trailing blanks (e.g. from DCH),
        // reflow should NOT insert a gap when rejoining on widen.
        let mut g = Grid::with_scrollback(5, 2, 100);
        // Fill row 0 completely: ABCDE, then wrap to row 1: FG
        for (i, ch) in ['A', 'B', 'C', 'D', 'E'].iter().enumerate() {
            g.cell_mut(i, 0).unwrap().ch = *ch;
        }
        g.set_row_wrap(0, true);
        for (i, ch) in ['F', 'G'].iter().enumerate() {
            g.cell_mut(i, 1).unwrap().ch = *ch;
        }
        // Simulate DCH: delete char at col 3, creating trailing blanks
        // in row 0 while wrap stays true.
        g.cell_mut(3, 0).unwrap().clear();
        g.cell_mut(4, 0).unwrap().clear();
        // Row 0: A B C _ _ (wrap=true), Row 1: F G _ _ _ (wrap=false)
        // Widen to 10: logical line should be "ABCFG" with no gap.
        g.reflow_resize(10, 2);
        // Row 0 should have A, B, C, F, G in sequence — no gap at col 3.
        assert_eq!(g.cell(0, 0).unwrap().ch, 'A');
        assert_eq!(g.cell(1, 0).unwrap().ch, 'B');
        assert_eq!(g.cell(2, 0).unwrap().ch, 'C');
        assert_eq!(
            g.cell(3, 0).unwrap().ch,
            'F',
            "no gap from trailing blanks in wrapped row"
        );
        assert_eq!(g.cell(4, 0).unwrap().ch, 'G');
    }
}
