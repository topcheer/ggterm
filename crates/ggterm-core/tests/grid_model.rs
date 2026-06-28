//! Integration tests for the Grid model (Cell, Row, Grid, Color, CellFlags).

use ggterm_core::{Cell, CellFlags, Color, Grid, Row};

// ---------------------------------------------------------------------------
// Cell tests
// ---------------------------------------------------------------------------

#[test]
fn test_cell_blank_is_default() {
    let cell = Cell::blank();
    assert_eq!(cell.ch, ' ');
    assert_eq!(cell.fg, Color::Default);
    assert_eq!(cell.bg, Color::Default);
    assert!(cell.flags.is_empty());
}

#[test]
fn test_cell_with_char() {
    let cell = Cell::with_char('X');
    assert_eq!(cell.ch, 'X');
    assert_eq!(cell.fg, Color::Default);
    assert_eq!(cell.bg, Color::Default);
    assert!(cell.flags.is_empty());
}

#[test]
fn test_cell_with_unicode() {
    let cell = Cell::with_char('你');
    assert_eq!(cell.ch, '你');
}

#[test]
fn test_cell_is_blank() {
    let blank = Cell::blank();
    assert!(blank.is_blank());

    let with_char = Cell::with_char('A');
    assert!(!with_char.is_blank());
}

#[test]
fn test_cell_is_not_blank_with_flags() {
    let mut cell = Cell::blank();
    cell.flags |= CellFlags::BOLD;
    assert!(!cell.is_blank(), "cell with BOLD flag should not be blank");
}

#[test]
fn test_cell_clear() {
    let mut cell = Cell::with_char('Z');
    cell.fg = Color::Indexed(1);
    cell.bg = Color::Indexed(2);
    cell.flags = CellFlags::BOLD | CellFlags::UNDERLINE;

    cell.clear();

    assert_eq!(cell.ch, ' ');
    assert_eq!(cell.fg, Color::Default);
    assert_eq!(cell.bg, Color::Default);
    assert!(cell.flags.is_empty());
    assert!(cell.is_blank());
}

#[test]
fn test_cell_set_fg_set_bg() {
    let mut cell = Cell::blank();
    cell.set_fg(Color::Indexed(3));
    cell.set_bg(Color::Rgb(10, 20, 30));

    assert_eq!(cell.fg, Color::Indexed(3));
    assert_eq!(cell.bg, Color::Rgb(10, 20, 30));
}

// ---------------------------------------------------------------------------
// CellFlags tests
// ---------------------------------------------------------------------------

#[test]
fn test_cellflags_individual_bits() {
    assert_eq!(CellFlags::BOLD.bits(), 0x001);
    assert_eq!(CellFlags::DIM.bits(), 0x002);
    assert_eq!(CellFlags::ITALIC.bits(), 0x004);
    assert_eq!(CellFlags::UNDERLINE.bits(), 0x008);
    assert_eq!(CellFlags::BLINK.bits(), 0x010);
    assert_eq!(CellFlags::REVERSE.bits(), 0x020);
    assert_eq!(CellFlags::HIDDEN.bits(), 0x040);
    assert_eq!(CellFlags::STRIKETHROUGH.bits(), 0x080);
    assert_eq!(CellFlags::WIDE_CHAR.bits(), 0x100);
    assert_eq!(CellFlags::WIDE_SPACER.bits(), 0x200);
}

#[test]
fn test_cellflags_combination() {
    let combo = CellFlags::BOLD | CellFlags::ITALIC | CellFlags::UNDERLINE;
    assert!(combo.contains(CellFlags::BOLD));
    assert!(combo.contains(CellFlags::ITALIC));
    assert!(combo.contains(CellFlags::UNDERLINE));
    assert!(!combo.contains(CellFlags::DIM));
    assert!(!combo.contains(CellFlags::BLINK));
    assert!(!combo.contains(CellFlags::REVERSE));
}

#[test]
fn test_cellflags_empty_and_all() {
    let empty = CellFlags::empty();
    assert!(empty.is_empty());

    let all = CellFlags::all();
    assert!(!all.is_empty());
    assert!(all.contains(CellFlags::BOLD));
    assert!(all.contains(CellFlags::WIDE_SPACER));
}

#[test]
fn test_cellflags_toggle_remove() {
    let mut flags = CellFlags::BOLD;
    assert!(flags.contains(CellFlags::BOLD));

    flags |= CellFlags::UNDERLINE;
    assert!(flags.contains(CellFlags::BOLD));
    assert!(flags.contains(CellFlags::UNDERLINE));

    flags -= CellFlags::BOLD;
    assert!(!flags.contains(CellFlags::BOLD));
    assert!(flags.contains(CellFlags::UNDERLINE));
}

// ---------------------------------------------------------------------------
// Color tests
// ---------------------------------------------------------------------------

#[test]
fn test_color_default() {
    assert_eq!(Color::Default, Color::default());
}

#[test]
fn test_color_from_sgr_reset() {
    assert_eq!(Color::from_sgr(0), Some(Color::Default));
}

#[test]
fn test_color_from_sgr_standard_fg() {
    // 30-37 → Indexed 0-7
    assert_eq!(Color::from_sgr(30), Some(Color::Indexed(0))); // black
    assert_eq!(Color::from_sgr(31), Some(Color::Indexed(1))); // red
    assert_eq!(Color::from_sgr(32), Some(Color::Indexed(2))); // green
    assert_eq!(Color::from_sgr(33), Some(Color::Indexed(3))); // yellow
    assert_eq!(Color::from_sgr(34), Some(Color::Indexed(4))); // blue
    assert_eq!(Color::from_sgr(35), Some(Color::Indexed(5))); // magenta
    assert_eq!(Color::from_sgr(36), Some(Color::Indexed(6))); // cyan
    assert_eq!(Color::from_sgr(37), Some(Color::Indexed(7))); // white
}

#[test]
fn test_color_from_sgr_bright_fg() {
    // 90-97 → Indexed 8-15
    assert_eq!(Color::from_sgr(90), Some(Color::Indexed(8)));
    assert_eq!(Color::from_sgr(91), Some(Color::Indexed(9)));
    assert_eq!(Color::from_sgr(92), Some(Color::Indexed(10)));
    assert_eq!(Color::from_sgr(93), Some(Color::Indexed(11)));
    assert_eq!(Color::from_sgr(94), Some(Color::Indexed(12)));
    assert_eq!(Color::from_sgr(95), Some(Color::Indexed(13)));
    assert_eq!(Color::from_sgr(96), Some(Color::Indexed(14)));
    assert_eq!(Color::from_sgr(97), Some(Color::Indexed(15)));
}

#[test]
fn test_color_from_sgr_unsupported() {
    // Background colors (40-47), bright bg (100-107), and other params are not handled
    assert_eq!(Color::from_sgr(40), None);
    assert_eq!(Color::from_sgr(47), None);
    assert_eq!(Color::from_sgr(100), None);
    assert_eq!(Color::from_sgr(107), None);
    assert_eq!(Color::from_sgr(38), None);
    assert_eq!(Color::from_sgr(255), None);
}

#[test]
fn test_color_default_palette_has_16_entries() {
    let palette = Color::default_palette();
    assert_eq!(palette.len(), 16, "default palette should have exactly 16 entries");
}

#[test]
fn test_color_default_palette_all_rgb() {
    let palette = Color::default_palette();
    for (i, color) in palette.iter().enumerate() {
        match color {
            Color::Rgb(_, _, _) => {}
            other => panic!("palette[{}] should be Rgb, got {:?}", i, other),
        }
    }
}

#[test]
fn test_color_default_palette_known_values() {
    let palette = Color::default_palette();
    // black
    assert_eq!(palette[0], Color::Rgb(0x00, 0x00, 0x00));
    // red
    assert_eq!(palette[1], Color::Rgb(0xcc, 0x00, 0x00));
    // green
    assert_eq!(palette[2], Color::Rgb(0x4e, 0x9a, 0x06));
    // blue
    assert_eq!(palette[4], Color::Rgb(0x34, 0x65, 0xa4));
    // bright white
    assert_eq!(palette[15], Color::Rgb(0xee, 0xee, 0xec));
}

#[test]
fn test_color_equality() {
    assert_eq!(Color::Default, Color::Default);
    assert_eq!(Color::Indexed(5), Color::Indexed(5));
    assert_ne!(Color::Indexed(5), Color::Indexed(6));
    assert_eq!(Color::Rgb(1, 2, 3), Color::Rgb(1, 2, 3));
    assert_ne!(Color::Rgb(1, 2, 3), Color::Rgb(1, 2, 4));
    assert_ne!(Color::Default, Color::Indexed(0));
    assert_ne!(Color::Indexed(0), Color::Rgb(0, 0, 0));
}

// ---------------------------------------------------------------------------
// Row tests
// ---------------------------------------------------------------------------

#[test]
fn test_row_new() {
    let row = Row::new(10);
    assert_eq!(row.width(), 10);
    for i in 0..10 {
        assert!(row[i].is_blank(), "cell {} should be blank", i);
    }
}

#[test]
fn test_row_cell_access() {
    let mut row = Row::new(5);
    row.cell_mut(2).unwrap().ch = 'X';

    assert_eq!(row.cell(2).unwrap().ch, 'X');
    assert_eq!(row.cell(0).unwrap().ch, ' ');
    assert_eq!(row.cell(4).unwrap().ch, ' ');

    // Out of bounds
    assert!(row.cell(5).is_none());
    assert!(row.cell_mut(99).is_none());
}

#[test]
fn test_row_clear() {
    let mut row = Row::new(5);
    for i in 0..5 {
        row[i] = Cell::with_char('A');
    }
    assert_eq!(row.text(), "AAAAA");

    row.clear();

    for i in 0..5 {
        assert!(row[i].is_blank(), "cell {} should be blank after clear", i);
    }
    assert_eq!(row.text(), "");
}

#[test]
fn test_row_clear_from() {
    let mut row = Row::new(6);
    for i in 0..6 {
        row[i] = Cell::with_char((b'A' + i as u8) as char);
    }
    // row = "ABCDEF"
    row.clear_from(3);
    // row = "ABC   "
    assert_eq!(row.cell(0).unwrap().ch, 'A');
    assert_eq!(row.cell(2).unwrap().ch, 'C');
    assert!(row.cell(3).unwrap().is_blank());
    assert!(row.cell(5).unwrap().is_blank());
    assert_eq!(row.text(), "ABC");
}

#[test]
fn test_row_clear_to() {
    let mut row = Row::new(6);
    for i in 0..6 {
        row[i] = Cell::with_char((b'A' + i as u8) as char);
    }
    // row = "ABCDEF"
    row.clear_to(3);
    // row = "   DEF"
    assert!(row.cell(0).unwrap().is_blank());
    assert!(row.cell(2).unwrap().is_blank());
    assert_eq!(row.cell(3).unwrap().ch, 'D');
    assert_eq!(row.cell(5).unwrap().ch, 'F');
    assert_eq!(row.text(), "   DEF");
}

#[test]
fn test_row_clear_to_out_of_bounds() {
    let mut row = Row::new(4);
    for i in 0..4 {
        row[i] = Cell::with_char('Z');
    }
    // clear_to beyond row width should clear entire row (clamped)
    row.clear_to(100);
    for i in 0..4 {
        assert!(row[i].is_blank());
    }
}

#[test]
fn test_row_text_trims_trailing_spaces() {
    let mut row = Row::new(10);
    row[0] = Cell::with_char('H');
    row[1] = Cell::with_char('i');
    // cells 2-9 are blanks
    assert_eq!(row.text(), "Hi");
}

#[test]
fn test_row_text_all_blank() {
    let row = Row::new(5);
    assert_eq!(row.text(), "");
}

#[test]
fn test_row_text_preserves_internal_spaces() {
    let mut row = Row::new(8);
    row[0] = Cell::with_char('A');
    row[1] = Cell::with_char(' ');
    row[2] = Cell::with_char('B');
    // trailing blanks
    assert_eq!(row.text(), "A B");
}

#[test]
fn test_row_resize_grow() {
    let mut row = Row::new(3);
    row[0] = Cell::with_char('X');

    row.resize(6);
    assert_eq!(row.width(), 6);
    assert_eq!(row.cell(0).unwrap().ch, 'X', "original content preserved");
    assert!(row.cell(3).unwrap().is_blank(), "new cells should be blank");
    assert!(row.cell(5).unwrap().is_blank());
}

#[test]
fn test_row_resize_shrink() {
    let mut row = Row::new(5);
    for i in 0..5 {
        row[i] = Cell::with_char((b'A' + i as u8) as char);
    }

    row.resize(3);
    assert_eq!(row.width(), 3);
    assert_eq!(row.cell(0).unwrap().ch, 'A');
    assert_eq!(row.cell(2).unwrap().ch, 'C');
}

#[test]
fn test_row_index_index_mut() {
    let mut row = Row::new(3);
    row[1] = Cell::with_char('Y');

    assert_eq!(row[0].ch, ' ');
    assert_eq!(row[1].ch, 'Y');
    assert_eq!(row[2].ch, ' ');
}

#[test]
fn test_row_equality() {
    let row1 = Row::new(3);
    let row2 = Row::new(3);
    assert_eq!(row1, row2);

    let mut row3 = Row::new(3);
    row3[0] = Cell::with_char('X');
    assert_ne!(row1, row3);
}

// ---------------------------------------------------------------------------
// Grid tests
// ---------------------------------------------------------------------------

#[test]
fn test_grid_new_dimensions() {
    let grid = Grid::new(80, 24);
    assert_eq!(grid.width(), 80);
    assert_eq!(grid.height(), 24);
    assert_eq!(grid.scrollback_len(), 0);
}

#[test]
fn test_grid_new_all_cells_blank() {
    let grid = Grid::new(10, 5);
    for row in 0..5 {
        for col in 0..10 {
            assert!(
                grid.cell(col, row).unwrap().is_blank(),
                "cell ({},{}) should be blank",
                col,
                row
            );
        }
    }
}

#[test]
fn test_grid_with_custom_scrollback() {
    let grid = Grid::with_scrollback(20, 10, 100);
    assert_eq!(grid.width(), 20);
    assert_eq!(grid.height(), 10);
    assert_eq!(grid.scrollback_len(), 0);
}

#[test]
fn test_grid_cell_access() {
    let mut grid = Grid::new(10, 5);

    // Write
    if let Some(cell) = grid.cell_mut(3, 2) {
        cell.ch = 'A';
    }

    // Read
    assert_eq!(grid.cell(3, 2).unwrap().ch, 'A');
    // Unmodified cells
    assert_eq!(grid.cell(0, 0).unwrap().ch, ' ');
}

#[test]
fn test_grid_cell_access_out_of_bounds() {
    let grid = Grid::new(5, 3);
    assert!(grid.cell(5, 0).is_none()); // col OOB
    assert!(grid.cell(0, 3).is_none()); // row OOB
    assert!(grid.cell(100, 100).is_none());
}

#[test]
fn test_grid_index_operator() {
    let mut grid = Grid::new(5, 3);

    // Write via IndexMut
    grid[(2, 1)] = Cell::with_char('X');

    // Read via Index
    assert_eq!(grid[(2, 1)].ch, 'X');
    assert_eq!(grid[(0, 0)].ch, ' ');
}

#[test]
fn test_grid_row_access() {
    let mut grid = Grid::new(5, 3);
    if let Some(row) = grid.row_mut(1) {
        row[0] = Cell::with_char('H');
        row[1] = Cell::with_char('i');
    }

    assert_eq!(grid.row(1).unwrap().text(), "Hi");
}

#[test]
fn test_grid_row_access_oob() {
    let grid = Grid::new(5, 3);
    assert!(grid.row(3).is_none());
    let mut grid_mut = Grid::new(5, 3);
    assert!(grid_mut.row_mut(99).is_none());
}

#[test]
fn test_grid_resize_grow() {
    let mut grid = Grid::new(5, 3);
    grid[(0, 0)] = Cell::with_char('X');

    grid.resize(10, 6);

    assert_eq!(grid.width(), 10);
    assert_eq!(grid.height(), 6);
    // Original content preserved
    assert_eq!(grid[(0, 0)].ch, 'X');
    // New row exists and is blank
    assert_eq!(grid.cell(0, 5).unwrap().ch, ' ');
    // Extended cells are blank
    assert_eq!(grid.cell(9, 0).unwrap().ch, ' ');
}

#[test]
fn test_grid_resize_shrink_width() {
    let mut grid = Grid::new(8, 4);
    grid[(7, 0)] = Cell::with_char('Z');

    grid.resize(4, 4);

    assert_eq!(grid.width(), 4);
    assert_eq!(grid.height(), 4);
    // Truncated cell is gone; remaining content accessible
    assert_eq!(grid[(0, 0)].ch, ' ');
}

#[test]
fn test_grid_resize_shrink_height() {
    let mut grid = Grid::new(5, 6);
    for row in 0..6 {
        grid[(0, row)] = Cell::with_char((b'A' + row as u8) as char);
    }

    grid.resize(5, 3);

    assert_eq!(grid.height(), 3);
    // Excess rows moved to scrollback
    assert_eq!(grid.scrollback_len(), 3);
    // Top 3 rows (A, B, C) went to scrollback; visible now has D, E, F
    assert_eq!(grid[(0, 0)].ch, 'D');
    assert_eq!(grid[(0, 2)].ch, 'F');
}

#[test]
fn test_grid_resize_grow_then_shrink() {
    let mut grid = Grid::new(4, 2);
    grid[(0, 0)] = Cell::with_char('P');

    // Grow
    grid.resize(8, 4);
    assert_eq!(grid[(0, 0)].ch, 'P');

    // Shrink back: height 4→2, top 2 rows (incl. 'P') move to scrollback
    grid.resize(4, 2);
    assert_eq!(grid.height(), 2);
    assert_eq!(grid.scrollback_len(), 2);
    // 'P' was in row 0, now in scrollback; visible rows are the ones that were at bottom
    assert_eq!(grid[(0, 0)].ch, ' ', "visible content after shrink is from lower rows");
}

#[test]
fn test_grid_scroll_up_basic() {
    let mut grid = Grid::new(5, 4);
    // Fill visible rows
    grid[(0, 0)] = Cell::with_char('A');
    grid[(0, 1)] = Cell::with_char('B');
    grid[(0, 2)] = Cell::with_char('C');
    grid[(0, 3)] = Cell::with_char('D');

    grid.scroll_up(1);

    // B, C, D should move up; new blank row at bottom
    assert_eq!(grid[(0, 0)].ch, 'B');
    assert_eq!(grid[(0, 1)].ch, 'C');
    assert_eq!(grid[(0, 2)].ch, 'D');
    assert_eq!(grid[(0, 3)].ch, ' ', "new bottom row should be blank");

    // 'A' row is now in scrollback
    assert_eq!(grid.scrollback_len(), 1);
}

#[test]
fn test_grid_scroll_up_multiple() {
    let mut grid = Grid::new(3, 4);
    grid[(0, 0)] = Cell::with_char('1');
    grid[(0, 1)] = Cell::with_char('2');
    grid[(0, 2)] = Cell::with_char('3');
    grid[(0, 3)] = Cell::with_char('4');

    grid.scroll_up(2);

    // 3, 4 move up; two blank rows at bottom
    assert_eq!(grid[(0, 0)].ch, '3');
    assert_eq!(grid[(0, 1)].ch, '4');
    assert_eq!(grid[(0, 2)].ch, ' ');
    assert_eq!(grid[(0, 3)].ch, ' ');
    assert_eq!(grid.scrollback_len(), 2);
}

#[test]
fn test_grid_scroll_up_more_than_height() {
    let mut grid = Grid::new(3, 3);
    grid[(0, 0)] = Cell::with_char('A');
    grid[(0, 1)] = Cell::with_char('B');
    grid[(0, 2)] = Cell::with_char('C');

    // scroll_up(10) should clamp to height (3)
    grid.scroll_up(10);

    // All rows now blank
    for row in 0..3 {
        assert_eq!(grid[(0, row)].ch, ' ', "row {} should be blank", row);
    }
    // All 3 original rows in scrollback
    assert_eq!(grid.scrollback_len(), 3);
}

#[test]
fn test_grid_scroll_down_restores_content() {
    let mut grid = Grid::new(5, 4);
    grid[(0, 0)] = Cell::with_char('A');
    grid[(0, 1)] = Cell::with_char('B');
    grid[(0, 2)] = Cell::with_char('C');
    grid[(0, 3)] = Cell::with_char('D');

    // Scroll up 2 → scrollback = [A, B] (pushed in order)
    grid.scroll_up(2);
    assert_eq!(grid.scrollback_len(), 2);
    // Visible: C, D, blank, blank
    assert_eq!(grid[(0, 0)].ch, 'C');
    assert_eq!(grid[(0, 1)].ch, 'D');

    // Scroll down 1 → pop_back from scrollback = B (LIFO)
    grid.scroll_down(1);
    assert_eq!(grid.scrollback_len(), 1);
    // B restored to top
    assert_eq!(grid[(0, 0)].ch, 'B');
    assert_eq!(grid[(0, 1)].ch, 'C');
    assert_eq!(grid[(0, 2)].ch, 'D');
}

#[test]
fn test_grid_scroll_down_empty_scrollback() {
    let mut grid = Grid::new(5, 3);
    grid[(0, 0)] = Cell::with_char('X');

    // No scrollback to restore; scroll_down should just insert blank at top
    grid.scroll_down(1);
    assert_eq!(grid[(0, 0)].ch, ' ', "new top row should be blank");
    assert_eq!(grid.scrollback_len(), 0);
}

#[test]
fn test_grid_scroll_up_down_roundtrip() {
    let mut grid = Grid::new(3, 3);
    grid[(0, 0)] = Cell::with_char('1');
    grid[(0, 1)] = Cell::with_char('2');
    grid[(0, 2)] = Cell::with_char('3');

    // Scroll up 1 → scrollback has '1'
    grid.scroll_up(1);
    assert_eq!(grid.scrollback_len(), 1);

    // Scroll down 1 → '1' restored, bottom lost
    grid.scroll_down(1);
    assert_eq!(grid.scrollback_len(), 0);
    assert_eq!(grid[(0, 0)].ch, '1');
}

#[test]
fn test_grid_clear() {
    let mut grid = Grid::new(5, 3);
    grid[(0, 0)] = Cell::with_char('A');
    grid[(2, 1)] = Cell::with_char('B');

    grid.clear();

    for row in 0..3 {
        for col in 0..5 {
            assert!(
                grid[(col, row)].is_blank(),
                "cell ({},{}) should be blank after clear",
                col,
                row
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Scrollback tests
// ---------------------------------------------------------------------------

#[test]
fn test_grid_scrollback_capacity_limit() {
    // Small scrollback: max 3
    let mut grid = Grid::with_scrollback(2, 2, 3);
    // grid visible rows: row0=A, row1=B
    grid[(0, 0)] = Cell::with_char('A');
    grid[(0, 1)] = Cell::with_char('B');

    // Each scroll_up pushes row 0 to scrollback, adds blank row at bottom
    grid.scroll_up(1); // scrollback: [A]
    assert_eq!(grid.scrollback_len(), 1);

    grid[(0, 0)] = Cell::with_char('C');
    grid[(0, 1)] = Cell::with_char('D');
    grid.scroll_up(1); // scrollback: [A, B]

    assert_eq!(grid.scrollback_len(), 2);
    grid[(0, 0)] = Cell::with_char('E');
    grid[(0, 1)] = Cell::with_char('F');
    grid.scroll_up(1); // scrollback: [A, B, C]

    assert_eq!(grid.scrollback_len(), 3);

    grid[(0, 0)] = Cell::with_char('G');
    grid[(0, 1)] = Cell::with_char('H');
    grid.scroll_up(1); // scrollback: [B, C, E] (A evicted)

    assert_eq!(
        grid.scrollback_len(),
        3,
        "scrollback should be capped at max_scrollback=3"
    );
}

#[test]
fn test_grid_scrollback_grows_with_scrolling() {
    let mut grid = Grid::new(3, 2);
    // Initial state
    assert_eq!(grid.scrollback_len(), 0);

    // Fill rows and scroll
    grid[(0, 0)] = Cell::with_char('A');
    grid[(0, 1)] = Cell::with_char('B');
    grid.scroll_up(1);

    assert_eq!(grid.scrollback_len(), 1);

    grid[(0, 0)] = Cell::with_char('C');
    grid[(0, 1)] = Cell::with_char('D');
    grid.scroll_up(1);

    assert_eq!(grid.scrollback_len(), 2);
}

#[test]
fn test_grid_scrollback_default_capacity_10000() {
    // Verify Grid::new() uses 10_000 default scrollback
    // We verify indirectly: scrolling many times should not cap at a low value
    let mut grid = Grid::new(2, 1);

    for i in 0..50 {
        grid[(0, 0)] = Cell::with_char((b'a' + (i % 26) as u8) as char);
        grid.scroll_up(1);
    }

    assert_eq!(grid.scrollback_len(), 50, "50 scrolls should produce 50 scrollback rows");
}

// ---------------------------------------------------------------------------
// Integration: combined operations
// ---------------------------------------------------------------------------

#[test]
fn test_grid_write_scroll_write_pattern() {
    let mut grid = Grid::new(4, 3);

    // Write row 0
    grid[(0, 0)] = Cell::with_char('H');
    grid[(1, 0)] = Cell::with_char('i');
    // Scroll up 1
    grid.scroll_up(1);
    // Write new content on visible
    grid[(0, 0)] = Cell::with_char('X');

    // Row 0 is now 'X', row 1 and 2 are blank (from scroll)
    assert_eq!(grid.row(0).unwrap().text(), "X");
    assert_eq!(grid.row(1).unwrap().text(), "");
    // Scrollback has "Hi"
    assert_eq!(grid.scrollback_len(), 1);
}

#[test]
fn test_grid_write_all_cells_in_grid() {
    let mut grid = Grid::new(3, 3);
    let chars = [
        ['1', '2', '3'],
        ['4', '5', '6'],
        ['7', '8', '9'],
    ];

    for (row_idx, row_chars) in chars.iter().enumerate() {
        for (col_idx, &ch) in row_chars.iter().enumerate() {
            grid[(col_idx, row_idx)] = Cell::with_char(ch);
        }
    }

    for (row_idx, row_chars) in chars.iter().enumerate() {
        for (col_idx, &ch) in row_chars.iter().enumerate() {
            assert_eq!(
                grid[(col_idx, row_idx)].ch,
                ch,
                "mismatch at ({},{})",
                col_idx,
                row_idx
            );
        }
    }
}

#[test]
fn test_grid_large_dimensions() {
    let mut grid = Grid::new(200, 100);
    assert_eq!(grid.width(), 200);
    assert_eq!(grid.height(), 100);

    // Write to corners
    grid[(0, 0)] = Cell::with_char('T');
    grid[(199, 99)] = Cell::with_char('B');

    assert_eq!(grid[(0, 0)].ch, 'T');
    assert_eq!(grid[(199, 99)].ch, 'B');
    assert_eq!(grid[(100, 50)].ch, ' ');
}
