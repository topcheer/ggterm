//! Grid model for terminal cell storage.

mod cell;
mod row;

pub use cell::{Cell, CellFlags, Color};
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
            // Add blank rows at the bottom
            for _ in self.rows.len()..height {
                self.rows.push(Row::new(width));
            }
        } else if height < self.rows.len() {
            // Remove rows from the top (they move to scrollback)
            let excess = self.rows.len() - height;
            for _ in 0..excess {
                let row = self.rows.remove(0);
                self.push_scrollback(row);
            }
        }

        self.width = width;
        self.height = height;
    }

    /// Get a reference to a visible row.
    pub fn row(&self, row: usize) -> Option<&Row> {
        self.rows.get(row)
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

    /// Scroll all rows up by `n` lines. Top rows move to scrollback.
    pub fn scroll_up(&mut self, n: usize) {
 let n = n.min(self.rows.len());
        for _ in 0..n {
            let row = self.rows.remove(0);
            self.push_scrollback(row);
            self.rows.push(Row::new(self.width));
        }
    }

    /// Scroll down by `n` lines (reverse of scroll_up).
    pub fn scroll_down(&mut self, n: usize) {
        let n = n.min(self.rows.len());
        for _ in 0..n {
            if let Some(row) = self.scrollback.pop_back() {
                self.rows.insert(0, row);
            } else {
                self.rows.insert(0, Row::new(self.width));
            }
            self.rows.pop();
        }
    }

    /// Clear all rows to blank.
    pub fn clear(&mut self) {
        for row in &mut self.rows {
            row.clear();
        }
    }

    /// Number of rows in scrollback.
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// Push a row to the scrollback, evicting oldest if over capacity.
    fn push_scrollback(&mut self, row: Row) {
        if self.scrollback.len() >= self.max_scrollback {
            self.scrollback.pop_front();
        }
        self.scrollback.push_back(row);
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
