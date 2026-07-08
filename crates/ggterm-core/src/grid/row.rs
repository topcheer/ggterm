use super::cell::{Cell, CellFlags, char_width};

/// A row of terminal cells.
///
/// Each row has a fixed width and stores one [`Cell`] per column.
/// Rows can be cleared to blank, resized, and have their cells
/// individually accessed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The cells in this row, left to right.
    pub cells: Vec<Cell>,
}

impl Row {
    /// Create a new blank row of the given width.
    pub fn new(width: usize) -> Self {
        Self {
            cells: vec![Cell::blank(); width],
        }
    }

    /// Row width (number of columns).
    pub fn width(&self) -> usize {
        self.cells.len()
    }

    /// Get a cell reference by column index.
    pub fn cell(&self, col: usize) -> Option<&Cell> {
        self.cells.get(col)
    }

    /// Get a mutable cell reference by column index.
    pub fn cell_mut(&mut self, col: usize) -> Option<&mut Cell> {
        self.cells.get_mut(col)
    }

    /// Clear all cells to blank.
    pub fn clear(&mut self) {
        for cell in &mut self.cells {
            cell.clear();
        }
    }

    /// Clear cells from `start` to end of row.
    pub fn clear_from(&mut self, start: usize) {
        for cell in &mut self.cells[start..] {
            cell.clear();
        }
    }

    /// Clear cells from start to `end` (exclusive).
    pub fn clear_to(&mut self, end: usize) {
        let end = end.min(self.cells.len());
        for cell in &mut self.cells[..end] {
            cell.clear();
        }
    }

    /// Resize the row. New cells are blank.
    pub fn resize(&mut self, new_width: usize) {
        self.cells.resize(new_width, Cell::blank());
    }

    /// Get the text content of this row as a String (trailing spaces trimmed).
    pub fn text(&self) -> String {
        let mut s = String::new();
        for c in &self.cells {
            if c.is_wide_spacer() {
                continue;
            }
            s.push(c.ch);
            for &mc in &c.combining {
                s.push(mc);
            }
        }
        s.trim_end().to_string()
    }

    // --------------------------------------------------------------------
    //  Character-level edits (ICH / DCH / ECH)
    // --------------------------------------------------------------------

    /// Insert `count` blank cells at `col`, shifting cells right.
    ///
    /// Cells pushed beyond the row width are lost.
    /// Simulates ANSI **ICH** (Insert Character, `ESC [ @`).
    ///
    /// Wide-character aware: if a insertion point lands on a wide spacer,
    /// the lead cell is cleared first.
    pub fn insert_char(&mut self, col: usize, count: usize) {
        let len = self.cells.len();
        if col >= len || count == 0 {
            return;
        }
        let count = count.min(len - col);
        // If inserting on a wide spacer, clear the lead cell to its left
        if col > 0 && self.cells[col].is_wide_spacer() {
            self.cells[col - 1].clear();
            self.cells[col].clear();
        }
        // Shift right (Cell is Clone, not Copy, so use clone)
        let src_end = len - count;
        for i in (col..src_end).rev() {
            self.cells[i + count] = self.cells[i].clone();
        }
        // Fill the gap with blanks
        for cell in &mut self.cells[col..col + count] {
            *cell = Cell::blank();
        }
    }

    /// Delete `count` cells starting at `col`, shifting cells left.
    ///
    /// Blank cells are appended at the right.
    /// Simulates ANSI **DCH** (Delete Character, `ESC [ P`).
    ///
    /// Wide-character aware: if deletion starts on a wide spacer, the
    /// lead cell is also removed.
    pub fn delete_char(&mut self, col: usize, count: usize) {
        let len = self.cells.len();
        if col >= len || count == 0 {
            return;
        }
        // If starting on a wide spacer, include the lead cell in deletion
        let actual_col = if col > 0 && self.cells[col].is_wide_spacer() {
            col - 1
        } else {
            col
        };
        let count = count.min(len - actual_col);
        // Shift left
        // Shift left (Cell is Clone, not Copy, so use clone)
        let len = self.cells.len();
        let actual_col = col.min(len);
        let actual_count = count.min(len - actual_col);
        for i in actual_col + actual_count..len {
            self.cells[i - actual_count] = self.cells[i].clone();
        }
        // Fill the vacated tail with blanks
        for cell in &mut self.cells[len - count..] {
            *cell = Cell::blank();
        }
    }

    /// Erase (blank) `count` cells starting at `col`.
    ///
    /// Unlike [`delete_char`](Self::delete_char), cells are NOT shifted.
    /// Simulates ANSI **ECH** (Erase Character, `ESC [ X`).
    pub fn erase_char(&mut self, col: usize, count: usize) {
        let len = self.cells.len();
        if col >= len || count == 0 {
            return;
        }
        let end = (col + count).min(len);
        for cell in &mut self.cells[col..end] {
            cell.clear();
        }
    }

    /// Place a character at `col`, handling wide characters.
    ///
    /// Returns the number of cells consumed (1 for normal, 2 for wide,
    /// 0 for zero-width combining).
    /// Automatically marks the trailing cell as `WIDE_SPACER` for
    /// double-width characters, and clears any existing wide/spacer
    /// cells that are overwritten.
    pub fn put_char(&mut self, col: usize, ch: char) -> usize {
        let len = self.cells.len();
        if col >= len {
            return 0;
        }
        let w = char_width(ch);

        // Clear existing wide char lead or spacer at col
        if self.cells[col].is_wide() && col + 1 < len {
            self.cells[col + 1].clear();
        }
        if self.cells[col].is_wide_spacer() && col > 0 {
            self.cells[col - 1].clear();
        }

        self.cells[col].clear();
        self.cells[col].ch = ch;

        if w == 2 {
            self.cells[col].flags |= CellFlags::WIDE_CHAR;
            if col + 1 < len {
                self.cells[col + 1].set_wide_spacer();
            }
            return 2;
        }

        1
    }

    /// Return an iterator of (col, &Cell) pairs, skipping wide spacers.
    pub fn visible_cells(&self) -> impl Iterator<Item = (usize, &Cell)> {
        self.cells
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.is_wide_spacer())
    }
}

impl std::ops::Index<usize> for Row {
    type Output = Cell;
    fn index(&self, col: usize) -> &Self::Output {
        &self.cells[col]
    }
}

impl std::ops::IndexMut<usize> for Row {
    fn index_mut(&mut self, col: usize) -> &mut Self::Output {
        &mut self.cells[col]
    }
}
