//! P1-C3: Grid editing operations — IL/DL/ICH/DCH/ECH, scroll regions,
//! damage tracking, clear_line*, scrollback_row, visible_cells, wide char cells.

use ggterm_core::{Cell, DamageTracker, DirtyRect, Grid, Row};

// ===========================================================================
// Scroll Region tests (DECSTBM)
// ===========================================================================

#[test]
fn test_scroll_region_default_is_full_screen() {
    let grid = Grid::new(10, 8);
    let (top, bottom) = grid.scroll_region();
    assert_eq!(top, 0);
    assert_eq!(bottom, 8);
}

#[test]
fn test_set_scroll_region() {
    let mut grid = Grid::new(10, 8);
    grid.set_scroll_region(2, 6); // rows 2..6 (exclusive)
    let (top, bottom) = grid.scroll_region();
    assert_eq!(top, 2);
    assert_eq!(bottom, 6);
}

#[test]
fn test_scroll_region_reset() {
    let mut grid = Grid::new(10, 8);
    grid.set_scroll_region(2, 6);
    grid.reset_scroll_region();
    let (top, bottom) = grid.scroll_region();
    assert_eq!(top, 0);
    assert_eq!(bottom, 8);
}

#[test]
fn test_scroll_region_invalid_resets_to_full() {
    let mut grid = Grid::new(10, 8);
    // top >= bottom → reset
    grid.set_scroll_region(5, 5);
    assert_eq!(grid.scroll_region(), (0, 8));
    // top > bottom → reset
    grid.set_scroll_region(6, 3);
    assert_eq!(grid.scroll_region(), (0, 8));
    // bottom > height → reset
    grid.set_scroll_region(0, 100);
    assert_eq!(grid.scroll_region(), (0, 8));
}

#[test]
fn test_scroll_up_in_region_only() {
    let mut grid = Grid::new(3, 6);
    grid.set_scroll_region(2, 5); // rows 2, 3, 4

    // Fill identifying content
    for row in 0..6 {
        for col in 0..3 {
            grid[(col, row)] = Cell::with_char((b'A' + row as u8) as char);
        }
    }
    // Rows: A B C D E F

    grid.scroll_up(1);
    // Only rows 2-4 affected: D shifts to row2, E to row3, blank at row4
    // Rows 0,1,5 untouched
    assert_eq!(grid[(0, 0)].ch, 'A', "row 0 untouched");
    assert_eq!(grid[(0, 1)].ch, 'B', "row 1 untouched");
    assert_eq!(grid[(0, 2)].ch, 'D', "row 2 = old row 3 (D)");
    assert_eq!(grid[(0, 3)].ch, 'E', "row 3 = old row 4 (E)");
    assert!(grid.cell(0, 4).unwrap().is_blank(), "row 4 = blank (new)");
    assert_eq!(grid[(0, 5)].ch, 'F', "row 5 untouched");
    // Nothing scrolled to scrollback because scroll_top != 0
    assert_eq!(grid.scrollback_len(), 0);
}

#[test]
fn test_scroll_down_in_region_only() {
    let mut grid = Grid::new(3, 6);
    grid.set_scroll_region(2, 5); // rows 2, 3, 4

    for row in 0..6 {
        for col in 0..3 {
            grid[(col, row)] = Cell::with_char((b'A' + row as u8) as char);
        }
    }

    grid.scroll_down(1);
    // Row C (row2) pushed out; blank inserted at row2, D→row3, E→row4
    // Wait: scroll_down shifts content down within region
    // Row2=blank, Row3=old row2 (C), Row4=old row3 (D)
    assert_eq!(grid[(0, 0)].ch, 'A', "row 0 untouched");
    assert_eq!(grid[(0, 1)].ch, 'B', "row 1 untouched");
    assert!(grid.cell(0, 2).unwrap().is_blank(), "row 2 = blank (new)");
    assert_eq!(grid[(0, 3)].ch, 'C', "row 3 = old row 2 (C)");
    assert_eq!(grid[(0, 4)].ch, 'D', "row 4 = old row 3 (D)");
    assert_eq!(grid[(0, 5)].ch, 'F', "row 5 untouched");
}

// ===========================================================================
// insert_line / delete_line (IL / DL)
// ===========================================================================

#[test]
fn test_insert_line_basic() {
    let mut grid = Grid::new(3, 5);
    for row in 0..5 {
        for col in 0..3 {
            grid[(col, row)] = Cell::with_char((b'A' + row as u8) as char);
        }
    }
    // Rows: A B C D E

    grid.insert_line(1, 2); // insert 2 blank lines at row 1
    // Row 0 = A (untouched)
    // Row 1 = blank (inserted)
    // Row 2 = blank (inserted)
    // Row 3 = B (shifted from row 1)
    // Row 4 = C (shifted from row 2)
    // D, E pushed off bottom
    assert_eq!(grid[(0, 0)].ch, 'A');
    assert!(grid.cell(0, 1).unwrap().is_blank());
    assert!(grid.cell(0, 2).unwrap().is_blank());
    assert_eq!(grid[(0, 3)].ch, 'B');
    assert_eq!(grid[(0, 4)].ch, 'C');
}

#[test]
fn test_insert_line_at_row_zero() {
    let mut grid = Grid::new(2, 3);
    for row in 0..3 {
        for col in 0..2 {
            grid[(col, row)] = Cell::with_char((b'A' + row as u8) as char);
        }
    }

    grid.insert_line(0, 1);
    // Row 0 = blank, Row 1 = A, Row 2 = B; C lost
    assert!(grid.cell(0, 0).unwrap().is_blank());
    assert_eq!(grid[(0, 1)].ch, 'A');
    assert_eq!(grid[(0, 2)].ch, 'B');
}

#[test]
fn test_insert_line_count_zero_is_noop() {
    let mut grid = Grid::new(3, 4);
    grid[(0, 0)] = Cell::with_char('X');
    grid.insert_line(0, 0);
    assert_eq!(
        grid[(0, 0)].ch,
        'X',
        "insert_line with count=0 should be no-op"
    );
}

#[test]
fn test_insert_line_outside_region_is_noop() {
    let mut grid = Grid::new(3, 5);
    grid.set_scroll_region(0, 3); // only rows 0-2
    grid[(0, 4)] = Cell::with_char('X');
    grid.insert_line(4, 1); // row 4 is outside region
    assert_eq!(
        grid[(0, 4)].ch,
        'X',
        "outside-region insert should be no-op"
    );
}

#[test]
fn test_delete_line_basic() {
    let mut grid = Grid::new(3, 5);
    for row in 0..5 {
        for col in 0..3 {
            grid[(col, row)] = Cell::with_char((b'A' + row as u8) as char);
        }
    }
    // Rows: A B C D E

    grid.delete_line(1, 2); // delete 2 lines at row 1
    // Row 0 = A (untouched)
    // Row 1 = D (shifted from row 3)
    // Row 2 = E (shifted from row 4)
    // Row 3 = blank
    // Row 4 = blank
    assert_eq!(grid[(0, 0)].ch, 'A');
    assert_eq!(grid[(0, 1)].ch, 'D');
    assert_eq!(grid[(0, 2)].ch, 'E');
    assert!(grid.cell(0, 3).unwrap().is_blank());
    assert!(grid.cell(0, 4).unwrap().is_blank());
}

#[test]
fn test_delete_line_count_zero_is_noop() {
    let mut grid = Grid::new(3, 3);
    grid[(0, 1)] = Cell::with_char('Z');
    grid.delete_line(1, 0);
    assert_eq!(grid[(0, 1)].ch, 'Z');
}

// ===========================================================================
// insert_char / delete_char / erase_char (ICH / DCH / ECH)
// ===========================================================================

#[test]
fn test_grid_insert_char_shifts_right() {
    let mut grid = Grid::new(6, 1);
    // "ABCDEF"
    for i in 0..6 {
        grid[(i, 0)] = Cell::with_char((b'A' + i as u8) as char);
    }

    grid.insert_char(1, 0, 2); // insert 2 blanks at col 1
    // A _ _ B C D (E and F lost)
    assert_eq!(grid[(0, 0)].ch, 'A');
    assert!(grid.cell(1, 0).unwrap().is_blank());
    assert!(grid.cell(2, 0).unwrap().is_blank());
    assert_eq!(grid[(3, 0)].ch, 'B');
    assert_eq!(grid[(4, 0)].ch, 'C');
    assert_eq!(grid[(5, 0)].ch, 'D');
}

#[test]
fn test_grid_insert_char_at_end() {
    let mut grid = Grid::new(5, 1);
    grid[(0, 0)] = Cell::with_char('X');
    grid.insert_char(3, 0, 2); // insert at col 3 in mostly-blank row
    assert_eq!(grid[(0, 0)].ch, 'X', "col 0 preserved");
    assert!(grid.cell(3, 0).unwrap().is_blank());
    assert!(grid.cell(4, 0).unwrap().is_blank());
}

#[test]
fn test_grid_delete_char_shifts_left() {
    let mut grid = Grid::new(6, 1);
    for i in 0..6 {
        grid[(i, 0)] = Cell::with_char((b'A' + i as u8) as char);
    }
    // "ABCDEF"

    grid.delete_char(1, 0, 2); // delete 2 chars at col 1
    // A D E F _ _
    assert_eq!(grid[(0, 0)].ch, 'A');
    assert_eq!(grid[(1, 0)].ch, 'D');
    assert_eq!(grid[(2, 0)].ch, 'E');
    assert_eq!(grid[(3, 0)].ch, 'F');
    assert!(grid.cell(4, 0).unwrap().is_blank());
    assert!(grid.cell(5, 0).unwrap().is_blank());
}

#[test]
fn test_grid_delete_char_count_exceeds_width() {
    let mut grid = Grid::new(4, 1);
    for i in 0..4 {
        grid[(i, 0)] = Cell::with_char((b'A' + i as u8) as char);
    }
    grid.delete_char(2, 0, 100); // delete more than available
    assert_eq!(grid[(0, 0)].ch, 'A');
    assert_eq!(grid[(1, 0)].ch, 'B');
    assert!(grid.cell(2, 0).unwrap().is_blank());
    assert!(grid.cell(3, 0).unwrap().is_blank());
}

#[test]
fn test_grid_erase_char_does_not_shift() {
    let mut grid = Grid::new(5, 1);
    for i in 0..5 {
        grid[(i, 0)] = Cell::with_char((b'A' + i as u8) as char);
    }
    // "ABCDE"

    grid.erase_char(1, 0, 2); // erase cols 1-2 (no shift!)
    assert_eq!(grid[(0, 0)].ch, 'A');
    assert!(grid.cell(1, 0).unwrap().is_blank());
    assert!(grid.cell(2, 0).unwrap().is_blank());
    assert_eq!(
        grid[(3, 0)].ch,
        'D',
        "col 3 unchanged (erase doesn't shift)"
    );
    assert_eq!(grid[(4, 0)].ch, 'E', "col 4 unchanged");
}

#[test]
fn test_grid_erase_char_count_exceeds_width() {
    let mut grid = Grid::new(4, 1);
    for i in 0..4 {
        grid[(i, 0)] = Cell::with_char((b'A' + i as u8) as char);
    }
    grid.erase_char(2, 0, 100);
    assert_eq!(grid[(0, 0)].ch, 'A');
    assert_eq!(grid[(1, 0)].ch, 'B');
    assert!(grid.cell(2, 0).unwrap().is_blank());
    assert!(grid.cell(3, 0).unwrap().is_blank());
}

// ===========================================================================
// Row-level insert_char / delete_char / erase_char
// ===========================================================================

#[test]
fn test_row_insert_char() {
    let mut row = Row::new(5);
    for i in 0..5 {
        row[i] = Cell::with_char((b'A' + i as u8) as char);
    }
    // "ABCDE"

    row.insert_char(1, 2); // insert 2 blanks at col 1
    assert_eq!(row[0].ch, 'A');
    assert!(row[1].is_blank());
    assert!(row[2].is_blank());
    assert_eq!(row[3].ch, 'B');
    assert_eq!(row[4].ch, 'C');
}

#[test]
fn test_row_delete_char() {
    let mut row = Row::new(5);
    for i in 0..5 {
        row[i] = Cell::with_char((b'A' + i as u8) as char);
    }

    row.delete_char(0, 2); // delete 2 chars at col 0
    assert_eq!(row[0].ch, 'C');
    assert_eq!(row[1].ch, 'D');
    assert_eq!(row[2].ch, 'E');
    assert!(row[3].is_blank());
    assert!(row[4].is_blank());
}

#[test]
fn test_row_erase_char() {
    let mut row = Row::new(5);
    for i in 0..5 {
        row[i] = Cell::with_char((b'A' + i as u8) as char);
    }

    row.erase_char(2, 2);
    assert_eq!(row[0].ch, 'A');
    assert_eq!(row[1].ch, 'B');
    assert!(row[2].is_blank());
    assert!(row[3].is_blank());
    assert_eq!(row[4].ch, 'E', "col 4 unchanged");
}

// ===========================================================================
// put_char (Grid level — wide char support)
// ===========================================================================

#[test]
fn test_grid_put_char_ascii() {
    let mut grid = Grid::new(5, 1);
    let consumed = grid.put_char(0, 0, 'X');
    assert_eq!(consumed, 1);
    assert_eq!(grid[(0, 0)].ch, 'X');
    assert!(!grid[(0, 0)].is_wide());
}

#[test]
fn test_grid_put_char_wide_cjk() {
    let mut grid = Grid::new(10, 1);
    // 中 = U+4E2D, width 2
    let consumed = grid.put_char(0, 0, '中');
    assert_eq!(consumed, 2, "CJK char should consume 2 cells");
    assert_eq!(grid[(0, 0)].ch, '中');
    assert!(
        grid[(0, 0)].is_wide(),
        "lead cell should have WIDE_CHAR flag"
    );
    assert!(
        grid[(1, 0)].is_wide_spacer(),
        "next cell should be WIDE_SPACER"
    );
}

#[test]
fn test_grid_put_char_emoji() {
    let mut grid = Grid::new(10, 1);
    // 😀 = U+1F600, width 2
    let consumed = grid.put_char(2, 0, '😀');
    assert_eq!(consumed, 2);
    assert_eq!(grid[(2, 0)].ch, '😀');
    assert!(grid[(2, 0)].is_wide());
    assert!(grid[(3, 0)].is_wide_spacer());
}

#[test]
fn test_grid_put_char_zero_width_combining() {
    let mut grid = Grid::new(5, 1);
    // Combining diaeresis U+0308 = zero width
    let consumed = grid.put_char(0, 0, '\u{0308}');
    // put_char returns max(w, 1) so zero-width still consumes 1 cell
    assert_eq!(consumed, 1, "zero-width char consumes 1 cell (minimum)");
}

#[test]
fn test_grid_put_char_overwrites_existing_wide() {
    let mut grid = Grid::new(6, 1);
    // Place a wide char at col 0
    grid.put_char(0, 0, '中');
    assert!(grid[(1, 0)].is_wide_spacer());

    // Now place an ASCII char at col 0 — should clear the wide spacer
    grid.put_char(0, 0, 'A');
    assert_eq!(grid[(0, 0)].ch, 'A');
    assert!(!grid[(0, 0)].is_wide());
    assert!(!grid[(1, 0)].is_wide_spacer(), "spacer should be cleared");
}

#[test]
fn test_grid_put_char_wide_at_last_column() {
    let mut grid = Grid::new(3, 1);
    // Wide char at last column (col 2) — no room for spacer
    let consumed = grid.put_char(2, 0, '中');
    // Can't fit 2 cells at col 2 in width 3
    assert_eq!(consumed, 2, "still returns 2");
    assert!(grid[(2, 0)].is_wide());
    // No spacer (col 3 doesn't exist)
}

#[test]
fn test_grid_put_char_out_of_bounds() {
    let mut grid = Grid::new(5, 1);
    let consumed = grid.put_char(100, 100, 'X');
    assert_eq!(consumed, 0, "out-of-bounds put_char should return 0");
}

// ===========================================================================
// Cell wide-char methods
// ===========================================================================

#[test]
fn test_cell_set_char_narrow() {
    let mut cell = Cell::blank();
    let w = cell.set_char('A');
    assert_eq!(w, 1);
    assert_eq!(cell.ch, 'A');
    assert!(!cell.is_wide());
    assert!(!cell.is_wide_spacer());
}

#[test]
fn test_cell_set_char_wide() {
    let mut cell = Cell::blank();
    let w = cell.set_char('你');
    assert_eq!(w, 2);
    assert!(cell.is_wide());
}

#[test]
fn test_cell_set_char_zero_width() {
    let mut cell = Cell::blank();
    let w = cell.set_char('\u{0308}'); // combining diaeresis
    assert_eq!(w, 0);
}

#[test]
fn test_cell_set_wide_spacer() {
    let mut cell = Cell::blank();
    cell.set_wide_spacer();
    assert!(cell.is_wide_spacer());
    assert!(!cell.is_wide());
    assert_eq!(cell.ch, ' ');
}

#[test]
fn test_cell_is_wide_after_set_char_narrow() {
    let mut cell = Cell::blank();
    cell.set_char('你'); // wide
    assert!(cell.is_wide());
    cell.set_char('A'); // narrow — should clear WIDE_CHAR
    assert!(
        !cell.is_wide(),
        "WIDE_CHAR should be removed after setting narrow char"
    );
}

// ===========================================================================
// Row::visible_cells (wide spacer filtering)
// ===========================================================================

#[test]
fn test_row_visible_cells_no_wide() {
    let mut row = Row::new(5);
    for i in 0..5 {
        row[i] = Cell::with_char((b'A' + i as u8) as char);
    }
    let cells: Vec<(usize, char)> = row.visible_cells().map(|(i, c)| (i, c.ch)).collect();
    assert_eq!(cells.len(), 5);
    assert_eq!(cells[0], (0, 'A'));
    assert_eq!(cells[4], (4, 'E'));
}

#[test]
fn test_row_visible_cells_skips_wide_spacer() {
    let mut row = Row::new(5);
    row.put_char(0, '中'); // cols 0+1: wide + spacer
    row[2] = Cell::with_char('X');
    row[3] = Cell::with_char('Y');
    row[4] = Cell::with_char('Z');

    let cells: Vec<(usize, char)> = row.visible_cells().map(|(i, c)| (i, c.ch)).collect();
    // Should skip col 1 (spacer), so 4 visible
    assert_eq!(cells.len(), 4);
    assert_eq!(cells[0], (0, '中'));
    assert_eq!(cells[1], (2, 'X'));
    assert_eq!(cells[2], (3, 'Y'));
    assert_eq!(cells[3], (4, 'Z'));
}

// ===========================================================================
// Grid clear_line / clear_line_from / clear_line_to
// ===========================================================================

#[test]
fn test_grid_clear_line() {
    let mut grid = Grid::new(5, 3);
    for col in 0..5 {
        grid[(col, 1)] = Cell::with_char('X');
    }
    grid.clear_line(1);
    for col in 0..5 {
        assert!(
            grid.cell(col, 1).unwrap().is_blank(),
            "col {} should be blank",
            col
        );
    }
    // Other rows untouched
    assert!(grid.cell(0, 0).unwrap().is_blank());
}

#[test]
fn test_grid_clear_line_from() {
    let mut grid = Grid::new(6, 1);
    for col in 0..6 {
        grid[(col, 0)] = Cell::with_char((b'A' + col as u8) as char);
    }
    // "ABCDEF"
    grid.clear_line_from(3, 0);
    assert_eq!(grid[(0, 0)].ch, 'A');
    assert_eq!(grid[(2, 0)].ch, 'C');
    assert!(grid.cell(3, 0).unwrap().is_blank());
    assert!(grid.cell(5, 0).unwrap().is_blank());
}

#[test]
fn test_grid_clear_line_to() {
    let mut grid = Grid::new(6, 1);
    for col in 0..6 {
        grid[(col, 0)] = Cell::with_char((b'A' + col as u8) as char);
    }
    // "ABCDEF"
    grid.clear_line_to(2, 0); // clear cols 0..=2
    assert!(grid.cell(0, 0).unwrap().is_blank());
    assert!(grid.cell(1, 0).unwrap().is_blank());
    assert!(grid.cell(2, 0).unwrap().is_blank());
    assert_eq!(grid[(3, 0)].ch, 'D', "col 3 unchanged");
    assert_eq!(grid[(5, 0)].ch, 'F', "col 5 unchanged");
}

// ===========================================================================
// scrollback_row (indexed access to scrollback history)
// ===========================================================================

#[test]
fn test_scrollback_row_access() {
    let mut grid = Grid::new(3, 2);
    // Row 0 = A, Row 1 = B
    grid[(0, 0)] = Cell::with_char('A');
    grid[(0, 1)] = Cell::with_char('B');
    grid.scroll_up(1); // A → scrollback

    assert_eq!(grid.scrollback_len(), 1);
    let row0 = grid.scrollback_row(0).unwrap();
    assert_eq!(row0[0].ch, 'A', "scrollback row 0 should be 'A'");
}

#[test]
fn test_scrollback_row_multiple() {
    let mut grid = Grid::new(2, 2);
    for round in 0..3 {
        grid[(0, 0)] = Cell::with_char((b'A' + round as u8) as char);
        grid[(0, 1)] = Cell::with_char((b'a' + round as u8) as char);
        grid.scroll_up(1);
    }
    // scrollback: [A, B, C] (0=oldest)
    assert_eq!(grid.scrollback_len(), 3);
    assert_eq!(grid.scrollback_row(0).unwrap()[0].ch, 'A');
    assert_eq!(grid.scrollback_row(1).unwrap()[0].ch, 'B');
    assert_eq!(grid.scrollback_row(2).unwrap()[0].ch, 'C');
}

#[test]
fn test_scrollback_row_out_of_bounds() {
    let grid = Grid::new(3, 2);
    assert!(grid.scrollback_row(0).is_none());
    assert!(grid.scrollback_row(99).is_none());
}

// ===========================================================================
// Damage tracking (Grid level)
// ===========================================================================

#[test]
fn test_grid_is_dirty_after_write() {
    let mut grid = Grid::new(5, 3);
    // Initially not dirty (no changes since construction)
    assert!(!grid.is_dirty(), "fresh grid should not be dirty");
    // After a write, should be dirty
    grid.put_char(0, 0, 'X');
    assert!(grid.is_dirty());
}

#[test]
fn test_grid_dirty_after_put_char() {
    let mut grid = Grid::new(10, 3);
    // Grid::new marks all dirty; clear it
    grid.clear_damage();
    assert!(!grid.is_dirty());

    grid.put_char(2, 1, 'X');
    assert!(grid.is_dirty());
    let dirty = grid.dirty().unwrap();
    // The dirty rect should include (2, 1)
    assert!(dirty.x <= 2);
    assert!(dirty.y <= 1);
    assert!(dirty.right() > 2);
    assert!(dirty.bottom() > 1);
}

#[test]
fn test_grid_dirty_rect_bounds() {
    let mut grid = Grid::new(10, 5);
    // Write at (3, 2)
    grid.put_char(3, 2, 'X');
    let dirty = grid.dirty().unwrap();
    assert!(dirty.x <= 3, "dirty rect x ({}) should be <= 3", dirty.x);
    assert!(dirty.y <= 2, "dirty rect y ({}) should be <= 2", dirty.y);
    assert!(
        dirty.right() >= 4,
        "dirty rect right ({}) should be >= 4",
        dirty.right()
    );
    assert!(
        dirty.bottom() >= 3,
        "dirty rect bottom ({}) should be >= 3",
        dirty.bottom()
    );
}

#[test]
fn test_grid_mark_dirty() {
    let mut grid = Grid::new(10, 5);
    grid.mark_dirty(5, 3);
    let dirty = grid.dirty().unwrap();
    assert_eq!(dirty.x, 5);
    assert_eq!(dirty.y, 3);
    assert_eq!(dirty.width, 1);
    assert_eq!(dirty.height, 1);
}

#[test]
fn test_grid_mark_row_dirty() {
    let mut grid = Grid::new(10, 5);
    grid.mark_row_dirty(2);
    let dirty = grid.dirty().unwrap();
    assert_eq!(dirty.y, 2);
    assert_eq!(dirty.height, 1);
    assert_eq!(dirty.width, 10, "mark_row should cover full width");
}

#[test]
fn test_grid_dirty_after_clear() {
    let mut grid = Grid::new(5, 3);
    grid.clear();
    let dirty = grid.dirty().unwrap();
    // clear marks entire grid dirty
    assert_eq!(dirty.x, 0);
    assert_eq!(dirty.y, 0);
    assert_eq!(dirty.width, 5);
    assert_eq!(dirty.height, 3);
}

#[test]
fn test_grid_dirty_after_insert_line() {
    let mut grid = Grid::new(5, 4);
    grid.insert_line(1, 1);
    let dirty = grid.dirty().unwrap();
    // Should cover from row 1 to bottom
    assert!(dirty.y <= 1);
    assert!(dirty.bottom() >= 4);
}

#[test]
fn test_grid_dirty_after_delete_char() {
    let mut grid = Grid::new(5, 2);
    grid.delete_char(1, 0, 1);
    let dirty = grid.dirty().unwrap();
    assert_eq!(dirty.y, 0, "delete_char should dirty row 0");
}

// ===========================================================================
// DamageTracker direct tests (integration)
// ===========================================================================

#[test]
fn test_damage_tracker_via_grid() {
    let mut grid = Grid::new(10, 5);
    // Access DamageTracker reference
    let _tracker: &DamageTracker = grid.damage();
    // Initially not dirty (Grid::new doesn't mark_dirty)
    assert!(!grid.is_dirty());
    // After mutation
    grid.put_char(0, 0, 'X');
    assert!(grid.is_dirty());
}

#[test]
fn test_dirty_rect_union_via_grid() {
    let mut grid = Grid::new(20, 10);
    grid.clear_damage();
    // Mark two separate regions — dirty should be union of just those two
    grid.mark_dirty(1, 1);
    grid.mark_dirty(15, 8);
    let dirty = grid.dirty().unwrap();
    assert_eq!(dirty.x, 1);
    assert_eq!(dirty.y, 1);
    assert!(dirty.right() >= 16);
    assert!(dirty.bottom() >= 9);
}

// ===========================================================================
// DirtyRect direct tests
// ===========================================================================

#[test]
fn test_dirty_rect_right_bottom() {
    let rect = DirtyRect::new(2, 3, 5, 7);
    assert_eq!(rect.right(), 7); // 2 + 5
    assert_eq!(rect.bottom(), 10); // 3 + 7
}

#[test]
fn test_dirty_rect_zero_width_height() {
    let rect = DirtyRect::new(0, 0, 0, 0);
    assert_eq!(rect.right(), 0);
    assert_eq!(rect.bottom(), 0);
}

#[test]
fn test_dirty_rect_union_non_overlapping() {
    let a = DirtyRect::new(0, 0, 3, 3);
    let b = DirtyRect::new(5, 5, 3, 3);
    let u = a.union(&b);
    assert_eq!(u.x, 0);
    assert_eq!(u.y, 0);
    assert_eq!(u.width, 8); // right = max(3, 8) = 8
    assert_eq!(u.height, 8);
}

#[test]
fn test_dirty_rect_union_overlapping() {
    let a = DirtyRect::new(0, 0, 5, 5);
    let b = DirtyRect::new(3, 3, 5, 5);
    let u = a.union(&b);
    assert_eq!(u.x, 0);
    assert_eq!(u.y, 0);
    assert_eq!(u.right(), 8);
    assert_eq!(u.bottom(), 8);
}

#[test]
fn test_dirty_rect_union_contained() {
    let outer = DirtyRect::new(0, 0, 10, 10);
    let inner = DirtyRect::new(3, 3, 2, 2);
    let u = outer.union(&inner);
    // Outer unchanged
    assert_eq!(u, outer);
}

// ===========================================================================
// Integration: scroll region + editing
// ===========================================================================

#[test]
fn test_insert_line_in_scroll_region() {
    let mut grid = Grid::new(3, 6);
    grid.set_scroll_region(2, 5); // rows 2..5

    for row in 0..6 {
        for col in 0..3 {
            grid[(col, row)] = Cell::with_char((b'A' + row as u8) as char);
        }
    }

    grid.insert_line(3, 1); // insert at row 3 (within region 2..5)
    // Row 2 = C (untouched)
    // Row 3 = blank (inserted)
    // Row 4 = D (shifted)
    // Row outside region untouched
    assert_eq!(grid[(0, 2)].ch, 'C', "row 2 untouched");
    assert!(grid.cell(0, 3).unwrap().is_blank(), "row 3 = blank");
    assert_eq!(grid[(0, 4)].ch, 'D', "row 4 = old row 3");
    assert_eq!(grid[(0, 0)].ch, 'A', "row 0 outside region untouched");
}

#[test]
fn test_delete_line_in_scroll_region() {
    let mut grid = Grid::new(3, 6);
    grid.set_scroll_region(2, 5);

    for row in 0..6 {
        for col in 0..3 {
            grid[(col, row)] = Cell::with_char((b'A' + row as u8) as char);
        }
    }

    grid.delete_line(3, 1); // delete at row 3 (within region)
    // Row 2 = C (untouched)
    // Row 3 = E (shifted up from row 4)
    // Row 4 = blank (new)
    assert_eq!(grid[(0, 2)].ch, 'C');
    assert_eq!(grid[(0, 3)].ch, 'E');
    assert!(grid.cell(0, 4).unwrap().is_blank());
    assert_eq!(grid[(0, 0)].ch, 'A', "row 0 outside region untouched");
}
