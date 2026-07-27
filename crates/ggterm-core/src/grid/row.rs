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
    /// True when the line was soft-wrapped (content continues on next row).
    /// Used by reflow_resize to merge/split lines when the terminal width changes.
    pub wrap: bool,
}

impl Row {
    /// Create a new blank row of the given width.
    pub fn new(width: usize) -> Self {
        Self {
            cells: vec![Cell::blank(); width],
            wrap: false,
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
        // If starting on a wide spacer, the lead cell at start-1
        // would be left dangling with WIDE_CHAR flag. Clear it too.
        let actual_start = if start > 0 && self.cells[start].is_wide_spacer() {
            start - 1
        } else {
            start
        };
        for cell in &mut self.cells[actual_start..] {
            cell.clear();
        }
    }

    /// Clear cells from start to `end` (exclusive).
    pub fn clear_to(&mut self, end: usize) {
        let end = end.min(self.cells.len());
        // If ending exactly on a wide spacer, its lead cell at end-1
        // would be left dangling. Include the spacer.
        let actual_end = if end < self.cells.len() && self.cells[end].is_wide_spacer() {
            end + 1
        } else {
            end
        };
        let actual_end = actual_end.min(self.cells.len());
        for cell in &mut self.cells[..actual_end] {
            cell.clear();
        }
    }

    /// Resize the row. New cells are blank.
    pub fn resize(&mut self, new_width: usize) {
        // When shrinking: handle wide char pairs at the new boundary.
        if new_width < self.cells.len() && new_width > 0 {
            // Case 1: the new last cell is a wide-char lead (its spacer
            // was at new_width, which will be truncated). Clear the lead
            // to avoid a dangling WIDE_CHAR without its spacer.
            if self.cells[new_width - 1].is_wide() {
                self.cells[new_width - 1] = Cell::blank();
            }
            // Note: if new_width-1 is a wide spacer and new_width-2 is
            // its lead, the pair fits entirely within the new width and
            // should be preserved. No cleanup needed.
        }
        self.cells.resize(new_width, Cell::blank());
    }

    /// Get the text content of this row as a String (trailing spaces trimmed).
    pub fn text(&self) -> String {
        let mut s = String::with_capacity(self.cells.len());
        for c in &self.cells {
            if c.is_wide_spacer() {
                continue;
            }
            // Skip null chars from uninitialized cells.
            if c.ch != '\0' {
                s.push(c.ch);
            }
            for &mc in &c.combining {
                s.push(mc);
            }
        }
        // Trim trailing whitespace in-place to avoid trim_end().to_string() allocation.
        while s.ends_with(|ch: char| ch.is_whitespace()) {
            s.pop();
        }
        s
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
        // If inserting on a wide char lead, also clear its spacer so the
        // pair isn't split by the insertion (lead shifts but spacer stays).
        if self.cells[col].is_wide() && col + 1 < len && self.cells[col + 1].is_wide_spacer() {
            self.cells[col].clear();
            self.cells[col + 1].clear();
        }
        // Shift right (Cell is Clone, not Copy, so use clone)
        let src_end = len - count;
        // Check if the last cell being shifted (at src_end-1) is a wide
        // char lead. If so, it would be moved to src_end-1+count = len-1,
        // but its spacer (at src_end) stays behind and gets blanked.
        // Clear the orphaned lead before shifting to avoid a dangling
        // WIDE_CHAR flag at the shifted position.
        if src_end > 0
            && src_end < len
            && self.cells[src_end - 1].is_wide()
            && self.cells[src_end].is_wide_spacer()
        {
            self.cells[src_end - 1].clear();
        }
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
        // If starting on a wide char lead and the deletion count doesn't
        // already cover the spacer, extend by 1 to avoid orphaning the spacer.
        let extra = if self.cells[actual_col].is_wide()
            && actual_col + 1 < len
            && self.cells[actual_col + 1].is_wide_spacer()
            && actual_col + count <= actual_col + 1
        {
            1
        } else {
            0
        };
        let actual_count = (count + extra).min(len - actual_col);
        // Shift left
        for i in actual_col + actual_count..len {
            self.cells[i - actual_count] = self.cells[i].clone();
        }
        // Fill the vacated tail with blanks
        for cell in &mut self.cells[len - actual_count..] {
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
        // If starting on a wide spacer, include the lead cell.
        let actual_col = if col > 0 && self.cells[col].is_wide_spacer() {
            col - 1
        } else {
            col
        };
        // End is col + count (original, not adjusted for lead).
        let end = (col + count).min(len);
        // If ending right after a wide lead (on its spacer), include the spacer.
        let end = if end < len && self.cells[end].is_wide_spacer() {
            end + 1
        } else {
            end
        };
        for cell in &mut self.cells[actual_col..end] {
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

        // When writing a WIDE char at col, the new spacer will be placed
        // at col+1. If col+1 is currently a wide lead (its own spacer is
        // at col+2), the old spacer at col+2 would be orphaned. Clear it.
        if w == 2 && col + 2 < len && self.cells[col + 1].is_wide() {
            self.cells[col + 2].clear();
        }

        self.cells[col].clear();
        self.cells[col].ch = ch;

        if w == 2 {
            // Set WIDE_CHAR flag and spacer if there's room for both cells.
            if col + 1 < len {
                self.cells[col + 1].set_wide_spacer();
                self.cells[col].flags |= CellFlags::WIDE_CHAR;
                return 2;
            }
            // Not enough room for the spacer — don't set WIDE_CHAR flag.
            // A wide char lead without its spacer would render incorrectly
            // and confuse cursor positioning. Fall through to return 1.
            return 1;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_row_text_basic() {
        let mut row = Row::new(10);
        row.cells[0].ch = 'H';
        row.cells[1].ch = 'i';
        row.cells[2].ch = '!';
        assert_eq!(row.text(), "Hi!");
    }

    #[test]
    fn t_row_text_empty() {
        let row = Row::new(10);
        assert_eq!(row.text(), "");
    }

    #[test]
    fn t_row_text_combining_char() {
        let mut row = Row::new(10);
        row.cells[0].ch = 'e';
        row.cells[0].combining.push('\u{0301}'); // combining acute → é
        row.cells[1].ch = 'x';
        assert_eq!(row.text(), "e\u{0301}x");
    }

    #[test]
    fn t_row_text_skips_wide_spacer() {
        let mut row = Row::new(10);
        row.cells[0].ch = 'あ'; // wide CJK char
        row.cells[0].flags = CellFlags::WIDE_CHAR;
        row.cells[1].set_wide_spacer(); // spacer cell
        row.cells[2].ch = 'B';
        assert_eq!(row.text(), "あB");
    }

    #[test]
    fn t_row_text_multiple_combining() {
        let mut row = Row::new(10);
        row.cells[0].ch = 'a';
        row.cells[0].combining.push('\u{0308}'); // diaeresis
        row.cells[0].combining.push('\u{0304}'); // macron
        assert_eq!(row.text(), "a\u{0308}\u{0304}");
    }

    #[test]
    fn t_put_wide_char_at_last_column_no_dangling() {
        // A wide char at the last column can't have a spacer.
        // It should NOT be marked as WIDE_CHAR (no dangling lead) and
        // should return width 1 (consumed 1 cell, placed as narrow fallback).
        // The terminal layer wraps before this happens, but Row::put_char
        // must not create a dangling WIDE_CHAR lead without its spacer.
        let mut row = Row::new(3);
        row.put_char(0, 'A');
        row.put_char(1, 'B');
        let consumed = row.put_char(2, 'あ');
        assert_eq!(consumed, 1, "wide char at last col returns 1 (no spacer)");
        assert!(
            !row.cells[2].is_wide(),
            "no WIDE_CHAR flag without spacer (no dangling lead)"
        );
        assert_eq!(row.cells[2].ch, 'あ', "char still placed in cell");
    }

    #[test]
    fn t_clear_from_wide_spacer_clears_lead() {
        let mut row = Row::new(6);
        row.cells[0].ch = 'A';
        row.cells[1].ch = 'あ'; // wide CJK
        row.cells[1].flags = CellFlags::WIDE_CHAR;
        row.cells[2].set_wide_spacer();
        row.cells[3].ch = 'B';
        // Clear from col 2 (the spacer) — should also clear col 1 (lead).
        row.clear_from(2);
        assert!(!row.cells[1].is_wide(), "wide lead should be cleared");
        assert!(!row.cells[2].is_wide_spacer(), "spacer should be cleared");
        assert_eq!(row.cells[0].ch, 'A', "col 0 should be untouched");
    }

    #[test]
    fn t_clear_to_wide_lead_includes_spacer() {
        let mut row = Row::new(6);
        row.cells[0].ch = 'A';
        row.cells[1].ch = 'あ'; // wide CJK
        row.cells[1].flags = CellFlags::WIDE_CHAR;
        row.cells[2].set_wide_spacer();
        // Clear to col 2 — the wide lead at col 1 should be cleared,
        // and since col 2 is its spacer, it should also be cleared.
        row.clear_to(2);
        assert!(!row.cells[1].is_wide(), "wide lead should be cleared");
        assert!(
            !row.cells[2].is_wide_spacer(),
            "spacer should also be cleared"
        );
    }

    #[test]
    fn t_erase_char_on_wide_spacer_clears_lead() {
        let mut row = Row::new(6);
        row.cells[0].ch = 'A';
        row.cells[1].ch = 'あ'; // wide CJK
        row.cells[1].flags = CellFlags::WIDE_CHAR;
        row.cells[2].set_wide_spacer();
        row.cells[3].ch = 'B';
        // Erase starting from col 2 (the spacer) — should clear col 1 too.
        row.erase_char(2, 1);
        assert!(!row.cells[1].is_wide(), "wide lead should be cleared");
        assert!(!row.cells[2].is_wide_spacer(), "spacer should be cleared");
        assert_eq!(row.cells[3].ch, 'B', "col 3 should be untouched");
    }

    #[test]
    fn t_put_wide_over_wide_lead_no_orphan_spacer() {
        // Overwriting a wide char lead with another wide char must not
        // orphan the old char's spacer at col+2.
        let mut row = Row::new(5);
        row.put_char(0, 'A');
        row.put_char(1, 'あ'); // wide at 1-2
        row.put_char(3, 'B');
        // Now overwrite col 1 with another wide char '大'.
        // The old spacer at col 2 must be reclaimed (replaced by the new
        // spacer), and col 3 must NOT retain an orphaned WIDE_SPACER.
        row.put_char(1, '大');
        assert!(row.cells[1].is_wide(), "new wide lead should be at col 1");
        assert!(
            row.cells[2].is_wide_spacer(),
            "new spacer should be at col 2"
        );
        assert!(
            !row.cells[3].is_wide_spacer(),
            "col 3 should not be an orphaned spacer"
        );
        assert_eq!(row.cells[3].ch, 'B', "col 3 should still be 'B'");
    }

    // ── insert_char (ICH) tests ─────────────────────────────────

    #[test]
    fn t_insert_char_basic() {
        // "ABCDEF" insert 2 at col 1 → "A  BCDE" (blanks shift in, D,F lost)
        let mut row = Row::new(7);
        for (i, ch) in ['A', 'B', 'C', 'D', 'E', 'F'].iter().enumerate() {
            row.cells[i].ch = *ch;
        }
        row.insert_char(1, 2);
        assert_eq!(row.cells[0].ch, 'A');
        assert_eq!(row.cells[1].ch, ' ', "col 1 should be blank");
        assert_eq!(row.cells[2].ch, ' ', "col 2 should be blank");
        assert_eq!(row.cells[3].ch, 'B');
        assert_eq!(row.cells[4].ch, 'C');
    }

    #[test]
    fn t_insert_char_at_end() {
        let mut row = Row::new(5);
        row.cells[0].ch = 'X';
        row.insert_char(4, 3); // at last col — no effect
        assert_eq!(row.cells[0].ch, 'X');
    }

    #[test]
    fn t_insert_char_orphans_wide_lead_at_boundary() {
        // ICH should not leave a dangling WIDE_CHAR flag when the
        // shift boundary splits a wide char pair.
        // Setup: [A] [中 lead] [spacer] [B] [ ] (5 cols)
        let mut row = Row::new(5);
        row.cells[0].ch = 'A';
        row.put_char(1, '中'); // wide at cols 1-2
        row.cells[3].ch = 'B';
        // Insert 1 at col 0 — shifts everything right by 1.
        // The wide lead at col 3 (was col 2, shifted) would be at
        // the last position without its spacer (spacer was blanked).
        // After fix: the orphaned lead should be cleared.
        row.insert_char(0, 1);
        // The cell that was the wide lead (now shifted) should not
        // have a dangling WIDE_CHAR flag.
        for cell in &row.cells {
            if cell.is_wide() {
                // If there's a wide lead, it must have a spacer after it.
                let idx = row.cells.iter().position(|c| c.is_wide()).unwrap();
                assert!(
                    idx + 1 < row.cells.len() && row.cells[idx + 1].is_wide_spacer(),
                    "wide char lead at col {idx} must have a spacer"
                );
            }
        }
    }

    #[test]
    fn t_insert_char_zero_count() {
        let mut row = Row::new(5);
        row.cells[0].ch = 'X';
        row.insert_char(0, 0);
        assert_eq!(row.cells[0].ch, 'X');
    }

    #[test]
    fn t_insert_char_beyond_width() {
        let mut row = Row::new(5);
        row.cells[0].ch = 'X';
        row.insert_char(10, 2); // beyond width — no effect
        assert_eq!(row.cells[0].ch, 'X');
    }

    // ── delete_char (DCH) tests ─────────────────────────────────

    #[test]
    fn t_delete_char_basic() {
        // "ABCDEF" delete 2 at col 1 → "CDEF  " (cells shift left)
        let mut row = Row::new(6);
        for (i, ch) in ['A', 'B', 'C', 'D', 'E', 'F'].iter().enumerate() {
            row.cells[i].ch = *ch;
        }
        row.delete_char(1, 2);
        assert_eq!(row.cells[0].ch, 'A');
        assert_eq!(row.cells[1].ch, 'D', "D should shift to col 1");
        assert_eq!(row.cells[2].ch, 'E');
        assert_eq!(row.cells[3].ch, 'F');
        assert_eq!(row.cells[4].ch, ' ', "tail should be blank");
        assert_eq!(row.cells[5].ch, ' ');
    }

    #[test]
    fn t_delete_char_entire_row() {
        let mut row = Row::new(4);
        for (i, ch) in ['A', 'B', 'C', 'D'].iter().enumerate() {
            row.cells[i].ch = *ch;
        }
        row.delete_char(0, 4);
        for cell in &row.cells {
            assert_eq!(cell.ch, ' ', "all cells should be blank");
        }
    }

    #[test]
    fn t_delete_char_beyond_width() {
        let mut row = Row::new(3);
        row.cells[0].ch = 'X';
        row.delete_char(5, 1); // no-op
        assert_eq!(row.cells[0].ch, 'X');
    }

    // ── erase_char (ECH) tests ──────────────────────────────────

    #[test]
    fn t_erase_char_basic() {
        // "ABCDEF" erase 2 at col 1 → "A  DEF" (no shift)
        let mut row = Row::new(6);
        for (i, ch) in ['A', 'B', 'C', 'D', 'E', 'F'].iter().enumerate() {
            row.cells[i].ch = *ch;
        }
        row.erase_char(1, 2);
        assert_eq!(row.cells[0].ch, 'A');
        assert_eq!(row.cells[1].ch, ' ', "B erased");
        assert_eq!(row.cells[2].ch, ' ', "C erased");
        assert_eq!(row.cells[3].ch, 'D', "D untouched");
        assert_eq!(row.cells[4].ch, 'E');
    }

    #[test]
    fn t_erase_char_beyond_end() {
        // Erase beyond row width should clamp
        let mut row = Row::new(4);
        for (i, ch) in ['A', 'B', 'C', 'D'].iter().enumerate() {
            row.cells[i].ch = *ch;
        }
        row.erase_char(2, 100);
        assert_eq!(row.cells[0].ch, 'A');
        assert_eq!(row.cells[1].ch, 'B');
        assert_eq!(row.cells[2].ch, ' ');
        assert_eq!(row.cells[3].ch, ' ');
    }

    #[test]
    fn t_erase_char_zero_count() {
        let mut row = Row::new(4);
        row.cells[0].ch = 'X';
        row.erase_char(0, 0);
        assert_eq!(row.cells[0].ch, 'X');
    }

    // ── resize tests ─────────────────────────────────────────────

    #[test]
    fn t_row_resize_grow() {
        let mut row = Row::new(3);
        row.cells[0].ch = 'A';
        row.resize(5);
        assert_eq!(row.cells.len(), 5);
        assert_eq!(row.cells[0].ch, 'A');
        assert_eq!(row.cells[3].ch, ' ', "new cells blank");
    }

    #[test]
    fn t_row_resize_shrink() {
        let mut row = Row::new(5);
        row.cells[0].ch = 'A';
        row.cells[3].ch = 'D';
        row.resize(2);
        assert_eq!(row.cells.len(), 2);
        assert_eq!(row.cells[0].ch, 'A');
    }

    // ── Round 37: Row text extraction + set_cell edge cases ────────────

    #[test]
    fn t_r37_row_text_trailing_nulls_trimmed() {
        // Cells with null char should not appear in text().
        let mut row = Row::new(5);
        row.cells[0].ch = 'A';
        row.cells[1].ch = 'B';
        // cells 2-4 have ch = '\0' (uninitialized)
        let text = row.text();
        assert_eq!(text, "AB", "null chars not in text, trailing trimmed");
    }

    #[test]
    fn t_r37_row_text_mixed_wide_and_combining() {
        // Wide char + combining + narrow chars.
        let mut row = Row::new(6);
        row.put_char(0, '中'); // cols 0-1
        row.cells[0].combining.push('\u{0301}');
        row.put_char(2, 'e'); // col 2
        row.cells[2].combining.push('\u{0301}');
        row.put_char(3, 'X'); // col 3
        let text = row.text();
        assert!(text.contains("中"), "wide char in text");
        assert!(text.contains("e\u{0301}"), "combining after narrow");
        assert!(text.contains("X"), "narrow char in text");
    }

    #[test]
    fn t_r37_row_set_cell_wide_at_last_col() {
        // Wide char at last column — spacer can't fit.
        // Should NOT set WIDE_CHAR flag (no dangling lead) and return 1.
        let mut row = Row::new(3);
        row.put_char(0, 'A');
        row.put_char(1, 'B');
        let w = row.put_char(2, '中'); // wide at last col
        assert_eq!(w, 1, "wide char at last col returns 1 (no spacer room)");
        assert!(!row.cells[2].is_wide(), "no WIDE_CHAR flag without spacer");
        assert!(row.text().contains('中'));
    }

    #[test]
    fn t_r37_row_clear_from_with_combining() {
        // clear_from should reset combining chars too.
        let mut row = Row::new(5);
        row.put_char(0, 'A');
        row.cells[0].combining.push('\u{0301}');
        row.put_char(1, 'B');
        row.clear_from(0);
        assert!(row.cells[0].is_blank(), "cell cleared");
        assert!(row.cells[0].combining.is_empty(), "combining cleared");
    }

    #[test]
    fn t_r37_row_clear_to_with_combining() {
        // clear_to should reset combining chars.
        let mut row = Row::new(5);
        row.put_char(0, 'A');
        row.put_char(1, 'B');
        row.cells[1].combining.push('\u{0301}');
        row.clear_to(2); // clear cols 0-1
        assert!(row.cells[0].is_blank(), "cell 0 cleared");
        assert!(row.cells[1].is_blank(), "cell 1 cleared");
        assert!(row.cells[1].combining.is_empty(), "combining cleared");
    }

    #[test]
    fn t_r37_row_resize_grow_preserves_content() {
        // Resize wider should preserve existing content.
        let mut row = Row::new(3);
        row.put_char(0, 'A');
        row.put_char(1, 'B');
        row.put_char(2, 'C');
        row.resize(5);
        assert_eq!(row.cells[0].ch, 'A', "content preserved after grow");
        assert_eq!(row.cells[2].ch, 'C', "content preserved");
        assert!(row.cells[3].is_blank(), "new cell blank");
        assert!(row.cells[4].is_blank(), "new cell blank");
    }

    #[test]
    fn t_r37_row_resize_shrink_truncates() {
        // Resize narrower should truncate content.
        let mut row = Row::new(5);
        row.put_char(0, 'A');
        row.put_char(1, 'B');
        row.put_char(4, 'E');
        row.resize(3);
        assert_eq!(row.cells.len(), 3, "row shrunk to 3");
        assert_eq!(row.cells[0].ch, 'A', "A preserved");
        assert_eq!(row.cells[2].ch, ' ', "E truncated");
    }

    #[test]
    fn t_r37_row_text_all_blank() {
        // Empty row text should be empty string.
        let row = Row::new(5);
        assert_eq!(row.text(), "", "empty row text is empty");
    }

    #[test]
    fn t_r37_row_visible_cells_skips_spacer() {
        // visible_cells() should skip WIDE_SPACER cells.
        let mut row = Row::new(4);
        row.put_char(0, '中'); // cols 0-1
        row.put_char(2, 'A'); // col 2
        let visible: Vec<_> = row.visible_cells().collect();
        // Should be 3 visible cells: 中(0), A(2), blank(3) — NOT the spacer at col 1
        assert_eq!(visible.len(), 3, "3 visible cells (spacer skipped)");
        assert_eq!(visible[0].0, 0, "first visible at col 0");
        assert_eq!(visible[1].0, 2, "second visible at col 2");
    }

    #[test]
    fn t_r37_row_erase_char_at_boundary() {
        // Erase char at the boundary of the row.
        let mut row = Row::new(5);
        row.put_char(0, 'A');
        row.put_char(1, 'B');
        row.put_char(2, 'C');
        row.erase_char(4, 2); // erase 2 chars at col 4 (only 1 exists in range)
        // Erasing at col 4 when row is 5 wide — clears cols 4 to end
        assert_eq!(row.cells[4].ch, ' ', "last cell erased");
        assert_eq!(row.cells[0].ch, 'A', "content before erased preserved");
    }

    #[test]
    fn t_resize_shrink_truncates_wide_char_spacer_clears_lead() {
        // When shrinking truncates a wide char's spacer, the lead cell
        // should be cleared (no dangling WIDE_CHAR flag).
        let mut row = Row::new(5);
        row.put_char(2, '你'); // wide char at cols 2-3
        // Row: [blank, blank, 你(WIDE), spacer, blank]
        assert!(row.cells[2].is_wide());
        assert!(row.cells[3].is_wide_spacer());
        // Shrink to 3 — col 3 (spacer) is truncated, lead at col 2 should be cleared
        row.resize(3);
        assert_eq!(row.cells.len(), 3);
        // Without fix: cells[2] would still have WIDE_CHAR flag (orphaned lead).
        // With fix: cells[2] is cleared because its spacer was truncated.
        assert!(
            !row.cells[2].is_wide(),
            "lead should be cleared when spacer is truncated by resize"
        );
        assert_eq!(row.cells[2].ch, ' ', "cleared to blank");
    }

    #[test]
    fn t_resize_shrink_truncates_wide_char_lead_clears_it() {
        // When shrinking truncates a wide char's lead (lead at last position),
        // the lead should be cleared.
        let mut row = Row::new(5);
        row.put_char(3, '你'); // wide char at cols 3-4
        // Row: [blank, blank, blank, 你(WIDE), spacer]
        // Shrink to 4 — col 4 (spacer) truncated, lead at col 3 is now last
        // but without its spacer → should be cleared.
        row.resize(4);
        assert!(!row.cells[3].is_wide(), "orphaned lead should be cleared");
    }

    #[test]
    fn put_char_wide_at_last_column_no_dangling_lead() {
        // Writing a wide char at the last column (col == len-1) should not
        // create a dangling WIDE_CHAR lead without a spacer. The terminal's
        // put_printable_char wraps before calling put_char, but Row::put_char
        // may be called from other paths (e.g., reflow, restore). It must not
        // leave a WIDE_CHAR flag without a spacer.
        let mut row = Row::new(4);
        // Place a wide char at col 3 (last column) — only 1 cell available.
        let _consumed = row.put_char(3, '\u{4e00}'); // CJK wide char
        // The char should NOT have been placed as a wide char lead without
        // its spacer. It should either be rejected (consumed=0) or placed
        // as a narrow fallback.
        assert!(
            !row.cells[3].is_wide(),
            "wide char at last column should not create dangling WIDE_CHAR lead"
        );
    }

    #[test]
    fn resize_preserves_wide_char_pair_at_boundary() {
        // A wide char pair at cols 3-4 (lead+spacer) should survive a resize
        // to width 5, since both cells fit within the new width.
        // Case 2 in Row::resize was incorrectly blanking the lead, AND
        // leaving an orphaned spacer.
        let mut row = Row::new(10);
        row.put_char(0, 'A');
        row.put_char(1, 'B');
        row.put_char(2, 'C');
        row.put_char(3, '\u{4e00}'); // CJK wide char: lead at 3, spacer at 4

        // Verify wide char is intact before resize
        assert!(row.cells[3].is_wide(), "lead at col 3 before resize");
        assert!(
            row.cells[4].is_wide_spacer(),
            "spacer at col 4 before resize"
        );

        // Resize to width 5 — the pair at cols 3-4 fits entirely
        row.resize(5);

        // The wide char pair should be preserved — both cells are in-bounds.
        assert!(
            row.cells[3].is_wide(),
            "wide char lead at col 3 should survive resize to width 5"
        );
        assert!(
            row.cells[4].is_wide_spacer(),
            "wide char spacer at col 4 should survive resize to width 5"
        );
        assert_eq!(
            row.cells[3].ch, '\u{4e00}',
            "wide char content should be preserved"
        );
    }
}
