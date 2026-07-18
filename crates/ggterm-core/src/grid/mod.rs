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
        let old_height = self.rows.len();
        let saved_offset = self.display_offset;

        // Resize each existing row to new width
        for row in &mut self.rows {
            row.resize(width);
        }

        if height > old_height {
            // Growing: pull rows back from scrollback to fill new space
            let needed = height - old_height;
            let pulled = self.scrollback.len().min(needed);
            for _ in 0..pulled {
                if let Some(sr) = self.scrollback.pop_front() {
                    // Resize scrollback row to new width
                    let mut row = sr;
                    row.resize(width);
                    self.rows.insert(0, row);
                }
            }
            // Add blank rows if still short
            while self.rows.len() < height {
                self.rows.push(Row::new(width));
            }
        } else if height < old_height {
            // Shrinking: push excess rows to scrollback
            let excess = old_height - height;
            for _ in 0..excess {
                let row = self.rows.remove(0);
                self.push_scrollback(row);
            }
        }

        self.width = width;
        self.height = height;
        self.scroll_top = 0;
        self.scroll_bottom = height;
        self.display_offset = saved_offset.min(self.scrollback.len());
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
        // If the user is scrolled up in the scrollback, preserve their position
        // by advancing display_offset by n. Only auto-scroll to bottom when
        // the user is already viewing the latest output (offset == 0).
        let was_scrolled = self.display_offset > 0;

        if self.scroll_top == 0 {
            // Full-screen scroll (most common case):
            // Drain first n rows → push to scrollback, then append n blanks.
            // This is O(n) instead of O(rows × n) with per-line remove(0).
            let drained: Vec<Row> = self.rows.drain(..n).collect();
            for row in drained {
                self.push_scrollback(row);
            }
            self.rows.extend((0..n).map(|_| Row::new(self.width)));
        } else {
            // Scroll region (DECSTBM): rotate region [T..B) left by n,
            // then fill the last n positions of the region with blank rows.
            self.rows[self.scroll_top..self.scroll_bottom].rotate_left(n);
            for i in 0..n {
                self.rows[self.scroll_bottom - 1 - i] = Row::new(self.width);
            }
        }

        if was_scrolled {
            // Keep the user at the same scrollback position by advancing offset.
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
        if self.scroll_top == 0 {
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

    /// Clear all scrollback history (ED mode 3 / OSC 1337 ClearScrollback).
    pub fn clear_scrollback(&mut self) {
        self.scrollback.clear();
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
        let mut lines: Vec<String> = Vec::with_capacity(self.scrollback.len() + self.height);

        // Scrollback (oldest first)
        for row in &self.scrollback {
            // row.text() already trims trailing whitespace.
            lines.push(row.text());
        }

        // Visible screen
        for row in &self.rows {
            lines.push(row.text());
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
        // Current absolute bottom row.
        let abs_bottom = self.scrollback.len() + self.height;
        // The grid_row maps to absolute row (scrollback.len() + grid_row).
        let abs_target = self.scrollback.len() + grid_row;
        // Center target in viewport.
        let desired_offset = abs_bottom.saturating_sub(abs_target + self.height / 2);
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
        // After delete of 1 cell from col 1: spacer shifts to col 1, X to col 2
        r.delete_char(2, 1);
        // The lead char at col 1 is gone; spacer content moved.
        assert_eq!(r[0].ch, 'A');
        // Col 1 now has the old spacer content (shifted left by 1)
        assert!(r[1].is_wide_spacer(), "spacer should have shifted to col 1");
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
        // display_offset is 0 (at bottom). scroll_to_grid_row now scrolls
        // to center the target row instead of being a no-op.
        g.scroll_to_grid_row(0); // target top row
        // Should scroll back to show row 0 centered.
        assert!(g.display_offset() > 0, "should scroll up to center row 0");
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
}
