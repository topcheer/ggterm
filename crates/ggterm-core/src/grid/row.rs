use super::cell::Cell;

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
        self.cells.iter().map(|c| c.ch).collect::<String>().trim_end().to_string()
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
