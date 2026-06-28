//! Integration tests for ConsoleRenderer.
//!
//! Tests the Grid → ConsoleRenderer → ANSI string pipeline.

use ggterm_core::{CellFlags, Color, Grid, Parser, Terminal};
use ggterm_render::{ConsoleRenderer, CursorState, Renderer};

// ------------------------------------------------------------------
//  Helpers
// ------------------------------------------------------------------

/// Create a grid with the given text at row 0, starting at col 0.
fn grid_with_text(width: usize, height: usize, text: &str) -> Grid {
    let mut g = Grid::new(width, height);
    for (col, ch) in text.chars().enumerate() {
        if col >= width {
            break;
        }
        if let Some(cell) = g.cell_mut(col, 0) {
            cell.ch = ch;
        }
    }
    g
}

/// Render a grid to a plain-text snapshot (stripping ANSI escapes).
fn render_plain(grid: &Grid, cursor: &CursorState) -> String {
    let mut r = ConsoleRenderer::new(grid.width(), grid.height());
    r.render(grid, cursor, None);
    strip_ansi(r.output())
}

/// Strip ANSI escape sequences from a string.
fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(c) = chars.next() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                result.push(ch);
            }
        } else {
            result.push(ch);
        }
    }
    result
}

// ------------------------------------------------------------------
//  Construction
// ------------------------------------------------------------------

#[test]
fn test_new_renderer() {
    let r = ConsoleRenderer::new(80, 24);
    assert_eq!(r.cols(), 80);
    assert_eq!(r.rows(), 24);
    assert!(r.output().is_empty());
}

#[test]
fn test_resize() {
    let mut r = ConsoleRenderer::new(80, 24);
    r.resize(120, 40);
    assert_eq!(r.cols(), 120);
    assert_eq!(r.rows(), 40);
}

// ------------------------------------------------------------------
//  Basic rendering
// ------------------------------------------------------------------

#[test]
fn test_empty_grid() {
    let grid = Grid::new(10, 3);
    let cursor = CursorState::hidden();
    let output = render_plain(&grid, &cursor);
    for line in output.lines() {
        assert!(line.chars().all(|c| c == ' ' || c == '\0'));
    }
}

#[test]
fn test_simple_text() {
    let grid = grid_with_text(20, 1, "Hello");
    let cursor = CursorState::hidden();
    let output = render_plain(&grid, &cursor);
    let first_line = output.lines().next().unwrap();
    assert!(first_line.starts_with("Hello"));
}

#[test]
fn test_multi_row() {
    let mut grid = Grid::new(10, 3);
    for (col, ch) in "ABC".chars().enumerate() {
        if let Some(cell) = grid.cell_mut(col, 0) {
            cell.ch = ch;
        }
    }
    for (col, ch) in "DEF".chars().enumerate() {
        if let Some(cell) = grid.cell_mut(col, 1) {
            cell.ch = ch;
        }
    }
    let cursor = CursorState::hidden();
    let output = render_plain(&grid, &cursor);
    let lines: Vec<&str> = output.lines().collect();
    assert!(lines[0].contains("ABC"));
    assert!(lines[1].contains("DEF"));
}

// ------------------------------------------------------------------
//  SGR attributes
// ------------------------------------------------------------------

#[test]
fn test_bold_attribute() {
    let mut grid = Grid::new(10, 1);
    if let Some(cell) = grid.cell_mut(0, 0) {
        cell.ch = 'X';
        cell.flags |= CellFlags::BOLD;
    }
    let cursor = CursorState::hidden();
    let mut r = ConsoleRenderer::new(10, 1);
    r.render(&grid, &cursor, None);
    assert!(r.output().contains("\x1b[1m"), "Expected bold SGR");
}

#[test]
fn test_italic_attribute() {
    let mut grid = Grid::new(10, 1);
    if let Some(cell) = grid.cell_mut(0, 0) {
        cell.ch = 'I';
        cell.flags |= CellFlags::ITALIC;
    }
    let cursor = CursorState::hidden();
    let mut r = ConsoleRenderer::new(10, 1);
    r.render(&grid, &cursor, None);
    assert!(r.output().contains("\x1b[3m"), "Expected italic SGR");
}

#[test]
fn test_underline_attribute() {
    let mut grid = Grid::new(10, 1);
    if let Some(cell) = grid.cell_mut(0, 0) {
        cell.ch = 'U';
        cell.flags |= CellFlags::UNDERLINE;
    }
    let cursor = CursorState::hidden();
    let mut r = ConsoleRenderer::new(10, 1);
    r.render(&grid, &cursor, None);
    assert!(r.output().contains("\x1b[4m"), "Expected underline SGR");
}

#[test]
fn test_reverse_attribute() {
    let mut grid = Grid::new(10, 1);
    if let Some(cell) = grid.cell_mut(0, 0) {
        cell.ch = 'R';
        cell.flags |= CellFlags::REVERSE;
    }
    let cursor = CursorState::hidden();
    let mut r = ConsoleRenderer::new(10, 1);
    r.render(&grid, &cursor, None);
    assert!(r.output().contains("\x1b[7m"), "Expected reverse SGR");
}

#[test]
fn test_combined_attributes() {
    let mut grid = Grid::new(10, 1);
    if let Some(cell) = grid.cell_mut(0, 0) {
        cell.ch = 'C';
        cell.flags |= CellFlags::BOLD | CellFlags::ITALIC | CellFlags::UNDERLINE;
    }
    let cursor = CursorState::hidden();
    let mut r = ConsoleRenderer::new(10, 1);
    r.render(&grid, &cursor, None);
    assert!(
        r.output().contains("1;3;4"),
        "Expected combined SGR 1;3;4, got: {}",
        r.output()
    );
}

// ------------------------------------------------------------------
//  Colors
// ------------------------------------------------------------------

#[test]
fn test_indexed_color_foreground() {
    let mut grid = Grid::new(10, 1);
    if let Some(cell) = grid.cell_mut(0, 0) {
        cell.ch = 'R';
        cell.fg = Color::Indexed(1); // red
    }
    let cursor = CursorState::hidden();
    let mut r = ConsoleRenderer::new(10, 1);
    r.render(&grid, &cursor, None);
    assert!(
        r.output().contains("\x1b[31m"),
        "Expected red fg SGR 31, got: {}",
        r.output()
    );
}

#[test]
fn test_bright_indexed_color() {
    let mut grid = Grid::new(10, 1);
    if let Some(cell) = grid.cell_mut(0, 0) {
        cell.ch = 'B';
        cell.fg = Color::Indexed(9); // bright red
    }
    let cursor = CursorState::hidden();
    let mut r = ConsoleRenderer::new(10, 1);
    r.render(&grid, &cursor, None);
    assert!(
        r.output().contains("\x1b[91m"),
        "Expected bright red fg SGR 91, got: {}",
        r.output()
    );
}

#[test]
fn test_256_color() {
    let mut grid = Grid::new(10, 1);
    if let Some(cell) = grid.cell_mut(0, 0) {
        cell.ch = 'X';
        cell.fg = Color::Indexed(196); // 256-color red
    }
    let cursor = CursorState::hidden();
    let mut r = ConsoleRenderer::new(10, 1);
    r.render(&grid, &cursor, None);
    assert!(
        r.output().contains("\x1b[38;5;196m"),
        "Expected 256-color fg, got: {}",
        r.output()
    );
}

#[test]
fn test_truecolor() {
    let mut grid = Grid::new(10, 1);
    if let Some(cell) = grid.cell_mut(0, 0) {
        cell.ch = 'T';
        cell.fg = Color::Rgb(255, 128, 64);
    }
    let cursor = CursorState::hidden();
    let mut r = ConsoleRenderer::new(10, 1);
    r.render(&grid, &cursor, None);
    assert!(
        r.output().contains("\x1b[38;2;255;128;64m"),
        "Expected truecolor fg, got: {}",
        r.output()
    );
}

#[test]
fn test_background_color() {
    let mut grid = Grid::new(10, 1);
    if let Some(cell) = grid.cell_mut(0, 0) {
        cell.ch = 'B';
        cell.bg = Color::Indexed(4); // blue bg
    }
    let cursor = CursorState::hidden();
    let mut r = ConsoleRenderer::new(10, 1);
    r.render(&grid, &cursor, None);
    assert!(
        r.output().contains("\x1b[44m"),
        "Expected blue bg SGR 44, got: {}",
        r.output()
    );
}

// ------------------------------------------------------------------
//  Cursor
// ------------------------------------------------------------------

#[test]
fn test_cursor_visible() {
    let grid = grid_with_text(10, 1, "ABC");
    let cursor = CursorState::new(1, 0); // cursor on 'B'
    let mut r = ConsoleRenderer::new(10, 1);
    r.render(&grid, &cursor, None);
    assert!(
        r.output().contains("\x1b[7m"),
        "Expected reverse for cursor, got: {}",
        r.output()
    );
}

#[test]
fn test_cursor_invisible() {
    let grid = grid_with_text(10, 1, "ABC");
    let cursor = CursorState::hidden();
    let mut r = ConsoleRenderer::new(10, 1);
    r.render(&grid, &cursor, None);
    // Should not crash and should produce output
    assert!(!r.output().is_empty());
}

#[test]
fn test_cursor_at_different_position() {
    let grid = grid_with_text(10, 1, "ABCDE");
    let cursor = CursorState::new(3, 0);
    let mut r = ConsoleRenderer::new(10, 1);
    r.render(&grid, &cursor, None);
    assert!(
        r.output().contains("\x1b[7m"),
        "Expected cursor reverse video at position 3"
    );
}

// ------------------------------------------------------------------
//  Wide char / spacer
// ------------------------------------------------------------------

#[test]
fn test_wide_spacer_skipped() {
    let mut grid = Grid::new(10, 1);
    if let Some(cell) = grid.cell_mut(0, 0) {
        cell.ch = '你';
        cell.flags |= CellFlags::WIDE_CHAR;
    }
    if let Some(cell) = grid.cell_mut(1, 0) {
        cell.flags |= CellFlags::WIDE_SPACER;
    }
    let cursor = CursorState::hidden();
    let output = render_plain(&grid, &cursor);
    let first_line = output.lines().next().unwrap();
    assert!(first_line.starts_with("你"));
}

// ------------------------------------------------------------------
//  Dirty rect (ignored by ConsoleRenderer)
// ------------------------------------------------------------------

#[test]
fn test_dirty_rect_ignored() {
    let grid = grid_with_text(10, 1, "Hello");
    let cursor = CursorState::hidden();
    let dirty = ggterm_core::DirtyRect::new(0, 0, 5, 1);

    let mut r1 = ConsoleRenderer::new(10, 1);
    r1.render(&grid, &cursor, Some(&dirty));
    let with_dirty = r1.output().to_string();

    let mut r2 = ConsoleRenderer::new(10, 1);
    r2.render(&grid, &cursor, None);
    let without_dirty = r2.output().to_string();

    assert_eq!(with_dirty, without_dirty);
}

// ------------------------------------------------------------------
//  End-to-end: Parser → Terminal → Grid → ConsoleRenderer
// ------------------------------------------------------------------

#[test]
fn test_end_to_simple_text() {
    let mut term = Terminal::new(40, 3);
    let mut parser = Parser::new();
    parser.feed(b"Hello World", &mut term);

    let cursor = CursorState::hidden();
    let output = render_plain(term.grid(), &cursor);
    let first_line = output.lines().next().unwrap();
    assert!(
        first_line.starts_with("Hello World"),
        "Expected 'Hello World', got: {}",
        first_line
    );
}

#[test]
fn test_end_to_end_with_colors() {
    let mut term = Terminal::new(40, 3);
    let mut parser = Parser::new();
    parser.feed(b"\x1b[1;31mError\x1b[0m", &mut term);

    let cursor = CursorState::hidden();
    let mut r = ConsoleRenderer::new(40, 3);
    r.render(term.grid(), &cursor, None);

    // Should have bold + red fg in output
    assert!(
        r.output().contains("1;31") || r.output().contains("31;1"),
        "Expected bold+red SGR, got: {}",
        r.output()
    );
}

#[test]
fn test_end_to_end_multiline() {
    let mut term = Terminal::new(20, 5);
    let mut parser = Parser::new();
    parser.feed(b"Line 1\r\nLine 2\r\nLine 3", &mut term);

    let cursor = CursorState::hidden();
    let output = render_plain(term.grid(), &cursor);
    let lines: Vec<&str> = output.lines().collect();
    assert!(lines[0].contains("Line 1"));
    assert!(lines[1].contains("Line 2"));
    assert!(lines[2].contains("Line 3"));
}

#[test]
fn test_end_to_end_with_cursor() {
    let mut term = Terminal::new(40, 3);
    let mut parser = Parser::new();
    parser.feed(b"ABCD", &mut term);

    let (cx, cy) = term.cursor();
    let cursor = CursorState {
        x: cx,
        y: cy,
        visible: true,
        ..Default::default()
    };
    let mut r = ConsoleRenderer::new(40, 3);
    r.render(term.grid(), &cursor, None);

    // Cursor at end of "ABCD" (position 4, 0) → reverse video
    assert!(
        r.output().contains("\x1b[7m"),
        "Expected cursor reverse video"
    );
}

// ------------------------------------------------------------------
//  ANSI output structure
// ------------------------------------------------------------------

#[test]
fn test_output_ends_with_reset() {
    let grid = grid_with_text(10, 1, "Test");
    let cursor = CursorState::hidden();
    let mut r = ConsoleRenderer::new(10, 1);
    r.render(&grid, &cursor, None);
    assert!(
        r.output().ends_with("\x1b[0m"),
        "Expected output to end with reset SGR"
    );
}

#[test]
fn test_renderer_trait_object() {
    let grid = Grid::new(5, 1);
    let cursor = CursorState::hidden();
    let mut renderer: Box<dyn Renderer> = Box::new(ConsoleRenderer::new(5, 1));
    renderer.render(&grid, &cursor, None);
    renderer.resize(10, 2);
}

#[test]
fn test_with_custom_theme() {
    use ggterm_render::RenderTheme;
    let theme = RenderTheme::default();
    let mut renderer = ConsoleRenderer::with_theme(10, 3, theme);
    let grid = Grid::new(10, 3);
    let cursor = CursorState::hidden();
    renderer.render(&grid, &cursor, None);
    assert!(!renderer.output().is_empty());
}

#[test]
fn test_cjk_multi_line() {
    let mut term = Terminal::new(10, 5);
    let mut parser = Parser::new();
    parser.feed("你好\r\n世界\r\n测试".as_bytes(), &mut term);

    let cursor = CursorState::hidden();
    let output = render_plain(term.grid(), &cursor);
    assert!(output.contains("你好"));
    assert!(output.contains("世界"));
    assert!(output.contains("测试"));
}
