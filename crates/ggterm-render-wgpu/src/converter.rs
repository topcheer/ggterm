//! Grid → glyphon text conversion.
//!
//! Converts terminal grid cells into styled text runs for glyphon rendering.
//! Each run groups adjacent cells with identical SGR attributes.

use ggterm_core::{CellFlags, Grid};
use ggterm_render::theme::RenderTheme;
use ggterm_render::CursorState;

/// A contiguous run of text with identical SGR attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRun {
    pub text: String,
    pub fg: (u8, u8, u8),
    pub bg: (u8, u8, u8),
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

/// Convert a single grid row into a vector of text runs.
///
/// Wide-spacer cells are skipped (the lead cell has the full char).
/// Cursor cell (if visible) swaps fg/bg for block cursor highlighting.
pub fn row_to_runs(
    grid: &Grid,
    row: usize,
    theme: &RenderTheme,
    cursor: Option<&CursorState>,
) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut current: Option<TextRun> = None;

    for col in 0..grid.width() {
        let cell = &grid[(col, row)];

        // Skip wide spacers
        if cell.flags.contains(CellFlags::WIDE_SPACER) {
            continue;
        }

        // Determine effective fg/bg
        let mut fg = cell.fg;
        let mut bg = cell.bg;

        // REVERSE: swap fg/bg
        if cell.flags.contains(CellFlags::REVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }

        let mut fg_rgb = crate::colors::map_fg(fg, theme);
        let mut bg_rgb = crate::colors::map_bg(bg, theme);

        // Cursor: swap resulting RGB for visibility (handles Default color case)
        if let Some(c) = cursor {
            if c.visible && c.x == col && c.y == row {
                std::mem::swap(&mut fg_rgb, &mut bg_rgb);
            }
        }

        let bold = cell.flags.contains(CellFlags::BOLD);
        let italic = cell.flags.contains(CellFlags::ITALIC);
        let underline = cell.flags.contains(CellFlags::UNDERLINE);

        let ch = cell.ch;

        let can_extend = current.as_ref().is_some_and(|c| {
            c.fg == fg_rgb && c.bg == bg_rgb && c.bold == bold && c.italic == italic && c.underline == underline
        });

        if can_extend {
            current.as_mut().unwrap().text.push(ch);
        } else {
            if let Some(r) = current.take() {
                runs.push(r);
            }
            current = Some(TextRun {
                text: ch.to_string(),
                fg: fg_rgb,
                bg: bg_rgb,
                bold,
                italic,
                underline,
            });
        }
    }

    if let Some(mut r) = current {
        // Trim trailing spaces from last run (empty grid cells)
        while r.text.ends_with(' ') {
            r.text.pop();
        }
        if !r.text.is_empty() {
            runs.push(r);
        }
    }

    runs
}

/// Build a plain text string from a grid row (for debugging/testing).
pub fn row_to_text(grid: &Grid, row: usize) -> String {
    let mut text = String::with_capacity(grid.width());
    for col in 0..grid.width() {
        let cell = &grid[(col, row)];
        if !cell.flags.contains(CellFlags::WIDE_SPACER) {
            text.push(cell.ch);
        }
    }
    text.trim_end_matches(' ').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ggterm_core::{Cell, Color, Grid};

    #[test]
    fn test_row_to_text_basic() {
        let mut grid = Grid::new(5, 1);
        for (i, ch) in "Hello".chars().enumerate() {
            grid[(i, 0)] = Cell::with_char(ch);
        }
        assert_eq!(row_to_text(&grid, 0), "Hello");
    }

    #[test]
    fn test_row_to_runs_single() {
        let mut grid = Grid::new(3, 1);
        grid[(0, 0)] = Cell::with_char('A');
        grid[(1, 0)] = Cell::with_char('B');
        grid[(2, 0)] = Cell::with_char('C');

        let theme = RenderTheme::default();
        let runs = row_to_runs(&grid, 0, &theme, None);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "ABC");
    }

    #[test]
    fn test_row_to_runs_color_split() {
        let mut grid = Grid::new(3, 1);
        grid[(0, 0)] = Cell::with_char('A');
        let mut b = Cell::with_char('B');
        b.fg = Color::Rgb(255, 0, 0);
        grid[(1, 0)] = b;
        grid[(2, 0)] = Cell::with_char('C');

        let theme = RenderTheme::default();
        let runs = row_to_runs(&grid, 0, &theme, None);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].text, "A");
        assert_eq!(runs[1].text, "B");
        assert_eq!(runs[1].fg, (255, 0, 0));
    }

    #[test]
    fn test_row_to_runs_bold() {
        let mut grid = Grid::new(3, 1);
        grid[(0, 0)] = Cell::with_char('A');
        let mut b = Cell::with_char('B');
        b.flags |= CellFlags::BOLD;
        grid[(1, 0)] = b;
        grid[(2, 0)] = Cell::with_char('C');

        let theme = RenderTheme::default();
        let runs = row_to_runs(&grid, 0, &theme, None);
        assert_eq!(runs.len(), 3);
        assert!(!runs[0].bold);
        assert!(runs[1].bold);
    }

    #[test]
    fn test_row_to_runs_reverse() {
        let mut grid = Grid::new(2, 1);
        grid[(0, 0)] = Cell::with_char('A');
        let mut b = Cell::with_char('B');
        b.fg = Color::Rgb(255, 0, 0);
        b.bg = Color::Rgb(0, 0, 255);
        b.flags |= CellFlags::REVERSE;
        grid[(1, 0)] = b;

        let theme = RenderTheme::default();
        let runs = row_to_runs(&grid, 0, &theme, None);
        assert_eq!(runs.len(), 2);
        // REVERSE: fg becomes bg color
        assert_eq!(runs[1].fg, (0, 0, 255)); // was bg
        assert_eq!(runs[1].bg, (255, 0, 0)); // was fg
    }

    #[test]
    fn test_row_to_runs_cursor_swaps() {
        let mut grid = Grid::new(3, 1);
        grid[(0, 0)] = Cell::with_char('A');
        grid[(1, 0)] = Cell::with_char('B');
        grid[(2, 0)] = Cell::with_char('C');

        let theme = RenderTheme::default();
        let cursor = CursorState::new(1, 0);

        let runs = row_to_runs(&grid, 0, &theme, Some(&cursor));
        // Cursor cell should have swapped colors → different run
        assert!(runs.len() >= 2);
    }

    #[test]
    fn test_row_to_text_wide_spacer_skipped() {
        let mut grid = Grid::new(5, 1);
        grid.put_char(0, 0, 'A');
        grid.put_char(1, 0, '中');
        grid.put_char(3, 0, 'B');

        let text = row_to_text(&grid, 0);
        assert_eq!(text.trim_end(), "A中B");
    }

    #[test]
    fn test_row_to_runs_wide_spacer() {
        let mut grid = Grid::new(5, 1);
        grid.put_char(0, 0, 'A');
        grid.put_char(1, 0, '中');
        grid.put_char(3, 0, 'B');

        let theme = RenderTheme::default();
        let runs = row_to_runs(&grid, 0, &theme, None);
        let total: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(total.trim_end(), "A中B");
    }
}
