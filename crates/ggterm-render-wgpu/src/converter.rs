//! Grid → glyphon text conversion.
//!
//! Converts terminal grid cells into styled text runs for glyphon rendering.
//! Each run groups adjacent cells with identical SGR attributes.

use ggterm_core::{CellFlags, Grid};
use ggterm_render::CursorState;
use ggterm_render::theme::RenderTheme;

/// A contiguous run of text with identical SGR attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRun {
    pub text: String,
    /// Starting column in the grid (0-based). Used for absolute positioning.
    pub start_col: usize,
    pub fg: (u8, u8, u8),
    pub bg: (u8, u8, u8),
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub blink: bool,
}

/// Background color used for search-match highlights (warm amber).
pub const HIGHLIGHT_BG: (u8, u8, u8) = (255, 200, 0);
/// Foreground color used on highlighted cells (black for contrast).
pub const HIGHLIGHT_FG: (u8, u8, u8) = (0, 0, 0);

/// Convert a single grid row into a vector of text runs.
///
/// Wide-spacer cells are skipped (the lead cell has the full char).
/// Cursor cell (if visible) swaps fg/bg for block cursor highlighting.
///
/// `highlights` is a slice of `(col_start, col_end)` ranges (inclusive).
/// Cells within a highlight range get `HIGHLIGHT_BG` / `HIGHLIGHT_FG`.
#[allow(clippy::too_many_arguments)]
pub fn row_to_runs(
    grid: &Grid,
    row: usize,
    theme: &RenderTheme,
    cursor: Option<&CursorState>,
    highlights: &[(usize, usize)],
    dynamic_fg: Option<(u8, u8, u8)>,
    dynamic_bg: Option<(u8, u8, u8)>,
    reverse_video: bool,
    palette_overrides: &std::collections::HashMap<u8, (u8, u8, u8)>,
) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut current: Option<TextRun> = None;

    for col in 0..grid.width() {
        let cell = match grid.display_cell(col, row) {
            Some(c) => c,
            None => continue,
        };

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

        let mut fg_rgb = if fg == ggterm_core::Color::Default {
            dynamic_fg.unwrap_or_else(|| crate::colors::map_fg(fg, theme))
        } else {
            crate::colors::map_fg(fg, theme)
        };
        let mut bg_rgb = if bg == ggterm_core::Color::Default {
            dynamic_bg.unwrap_or_else(|| crate::colors::map_bg(bg, theme))
        } else {
            crate::colors::map_bg(bg, theme)
        };

        // OSC 4: Apply custom palette overrides for indexed colors.
        if let ggterm_core::Color::Indexed(n) = &cell.fg
            && let Some(rgb) = palette_overrides.get(n)
        {
            fg_rgb = *rgb;
        }
        if let ggterm_core::Color::Indexed(n) = &cell.bg
            && let Some(rgb) = palette_overrides.get(n)
        {
            bg_rgb = *rgb;
        }

        // P13-A: DIM — reduce foreground brightness to ~60%.
        if cell.flags.contains(CellFlags::DIM) {
            fg_rgb = (
                (fg_rgb.0 as u16 * 6 / 10) as u8,
                (fg_rgb.1 as u16 * 6 / 10) as u8,
                (fg_rgb.2 as u16 * 6 / 10) as u8,
            );
        }

        // P13-A: HIDDEN — set fg = bg so text is invisible.
        if cell.flags.contains(CellFlags::HIDDEN) {
            fg_rgb = bg_rgb;
        }

        // OSC 8 hyperlink — tint foreground blue + force underline (like web links).
        if cell.hyperlink.is_some() {
            fg_rgb = (100, 160, 255);
        }

        // DECSCNM — reverse video: swap fg and bg globally.
        if reverse_video {
            std::mem::swap(&mut fg_rgb, &mut bg_rgb);
        }

        // P14-B: Search highlight — override to amber bg / black fg.
        let highlighted = highlights.iter().any(|&(s, e)| col >= s && col <= e);
        if highlighted {
            bg_rgb = HIGHLIGHT_BG;
            fg_rgb = HIGHLIGHT_FG;
        }

        // Cursor: swap resulting RGB for visibility (handles Default color case).
        // When a dynamic cursor color is set (OSC 12), use it as the cursor cell bg.
        // Highlight takes priority over cursor color swap.
        if !highlighted
            && let Some(c) = cursor
            && c.visible
            && c.x == col
            && c.y == row
        {
            if let Some((cr, cg, cb)) = c.color {
                bg_rgb = (cr, cg, cb);
            } else {
                std::mem::swap(&mut fg_rgb, &mut bg_rgb);
            }
        }

        let bold = cell.flags.contains(CellFlags::BOLD);
        let italic = cell.flags.contains(CellFlags::ITALIC);
        let has_link = cell.hyperlink.is_some();
        // OSC 8 hyperlinks render with underline even if cell doesn't have UNDERLINE flag.
        let underline = cell.flags.contains(CellFlags::UNDERLINE) || has_link;
        let strikethrough = cell.flags.contains(CellFlags::STRIKETHROUGH);
        let blink = cell.flags.contains(CellFlags::BLINK);

        let ch = cell.ch;
        let is_wide = cell.flags.contains(CellFlags::WIDE_CHAR);

        // P18-D: Always split runs at wide character boundaries.
        // This ensures each CJK/emoji char gets its own TextArea positioned
        // at the exact grid column, preventing cumulative drift.
        let can_extend = !is_wide
            && current.as_ref().is_some_and(|c: &TextRun| {
                c.fg == fg_rgb
                    && c.bg == bg_rgb
                    && c.bold == bold
                    && c.italic == italic
                    && c.underline == underline
                    && c.strikethrough == strikethrough
                    && c.blink == blink
                    && !has_link // hyperlink cells always start new runs
            });

        if can_extend && let Some(ref mut c) = current {
            c.text.push(ch);
        } else {
            #[allow(clippy::collapsible_if)]
            if let Some(r) = current.take() {
                runs.push(r);
            }
            current = Some(TextRun {
                text: ch.to_string(),
                start_col: col,
                fg: fg_rgb,
                bg: bg_rgb,
                bold,
                italic,
                underline,
                strikethrough,
                blink,
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
        let runs = row_to_runs(
            &grid,
            0,
            &theme,
            None,
            &[],
            None,
            None,
            false,
            &std::collections::HashMap::new(),
        );
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
        let runs = row_to_runs(
            &grid,
            0,
            &theme,
            None,
            &[],
            None,
            None,
            false,
            &std::collections::HashMap::new(),
        );
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
        let runs = row_to_runs(
            &grid,
            0,
            &theme,
            None,
            &[],
            None,
            None,
            false,
            &std::collections::HashMap::new(),
        );
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
        let runs = row_to_runs(
            &grid,
            0,
            &theme,
            None,
            &[],
            None,
            None,
            false,
            &std::collections::HashMap::new(),
        );
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

        let runs = row_to_runs(
            &grid,
            0,
            &theme,
            Some(&cursor),
            &[],
            None,
            None,
            false,
            &std::collections::HashMap::new(),
        );
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
        let runs = row_to_runs(
            &grid,
            0,
            &theme,
            None,
            &[],
            None,
            None,
            false,
            &std::collections::HashMap::new(),
        );
        let total: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(total.trim_end(), "A中B");
    }

    // ── P14-B: Search highlight tests ──────────────────────────────

    #[test]
    fn test_highlight_empty_no_change() {
        // No highlights → behavior identical to passing &[]
        let mut grid = Grid::new(4, 1);
        for (i, ch) in "ABCD".chars().enumerate() {
            grid[(i, 0)] = Cell::with_char(ch);
        }
        let theme = RenderTheme::default();
        let runs = row_to_runs(
            &grid,
            0,
            &theme,
            None,
            &[],
            None,
            None,
            false,
            &std::collections::HashMap::new(),
        );
        let runs2 = row_to_runs(
            &grid,
            0,
            &theme,
            None,
            &[(99, 99)],
            None,
            None,
            false,
            &std::collections::HashMap::new(),
        );
        // Neither should have highlight colors
        for r in &runs {
            assert_ne!(r.bg, HIGHLIGHT_BG);
        }
        for r in &runs2 {
            assert_ne!(r.bg, HIGHLIGHT_BG);
        }
    }

    #[test]
    fn test_highlight_single_range() {
        // Highlight cols 1-2 → those cells get HIGHLIGHT_BG / HIGHLIGHT_FG
        let mut grid = Grid::new(4, 1);
        for (i, ch) in "ABCD".chars().enumerate() {
            grid[(i, 0)] = Cell::with_char(ch);
        }
        let theme = RenderTheme::default();
        let runs = row_to_runs(
            &grid,
            0,
            &theme,
            None,
            &[(1, 2)],
            None,
            None,
            false,
            &std::collections::HashMap::new(),
        );

        // Find the run(s) containing 'B' and 'C' (cols 1,2)
        let mut found_highlight = false;
        let mut found_normal = false;
        for r in &runs {
            if r.bg == HIGHLIGHT_BG {
                assert_eq!(r.fg, HIGHLIGHT_FG);
                assert!(r.text.contains('B') || r.text.contains('C'));
                found_highlight = true;
            } else {
                found_normal = true;
            }
        }
        assert!(found_highlight, "expected at least one highlighted run");
        assert!(found_normal, "expected at least one normal run");
    }

    #[test]
    fn test_highlight_multiple_ranges() {
        // Two separate highlight ranges: (0,0) and (3,3)
        let mut grid = Grid::new(5, 1);
        for (i, ch) in "ABCDE".chars().enumerate() {
            grid[(i, 0)] = Cell::with_char(ch);
        }
        let theme = RenderTheme::default();
        let runs = row_to_runs(
            &grid,
            0,
            &theme,
            None,
            &[(0, 0), (3, 3)],
            None,
            None,
            false,
            &std::collections::HashMap::new(),
        );

        let mut highlight_count = 0;
        for r in &runs {
            if r.bg == HIGHLIGHT_BG {
                highlight_count += 1;
            }
        }
        // 'A' and 'D' are in separate ranges, so at least 2 highlight runs
        assert!(
            highlight_count >= 2,
            "expected >=2 highlighted runs, got {highlight_count}"
        );
    }

    #[test]
    fn test_highlight_outside_range_unaffected() {
        // Highlight cols 0-1, verify cols 2-3 are NOT highlighted
        let mut grid = Grid::new(4, 1);
        for (i, ch) in "ABCD".chars().enumerate() {
            grid[(i, 0)] = Cell::with_char(ch);
        }
        let theme = RenderTheme::default();
        let runs = row_to_runs(
            &grid,
            0,
            &theme,
            None,
            &[(0, 1)],
            None,
            None,
            false,
            &std::collections::HashMap::new(),
        );

        // Find run containing 'C' and 'D' — they must NOT be highlighted
        for r in &runs {
            if r.text.contains('C') || r.text.contains('D') {
                assert_ne!(
                    r.bg, HIGHLIGHT_BG,
                    "cell outside highlight range should not be highlighted"
                );
            }
        }
    }

    #[test]
    fn test_row_to_runs_decscnm_reverse_video() {
        // DECSCNM: when reverse_video=true, fg and bg are globally swapped.
        let mut grid = Grid::new(1, 1);
        let mut c = Cell::with_char('X');
        c.fg = Color::Rgb(255, 128, 0); // orange
        c.bg = Color::Rgb(0, 0, 255); // blue
        grid[(0, 0)] = c;

        let theme = RenderTheme::default();
        let runs = row_to_runs(
            &grid,
            0,
            &theme,
            None,
            &[],
            None,
            None,
            true,
            &std::collections::HashMap::new(),
        );

        assert_eq!(runs[0].fg, (0, 0, 255), "fg should be swapped to bg color");
        assert_eq!(
            runs[0].bg,
            (255, 128, 0),
            "bg should be swapped to fg color"
        );
    }
}
