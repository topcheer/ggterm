//! Console renderer: renders a [`Grid`] to an ANSI-escaped string.
//!
//! Used for headless testing and CI. No GPU required.

use crate::theme::RenderTheme;
use crate::{CursorState, Renderer};
use ggterm_core::{CellFlags, Color, DirtyRect, Grid};

/// Renders a terminal grid to an ANSI-escaped string.
///
/// Produces output suitable for stdout or snapshot comparison.
/// Implements the [`Renderer`] trait.
pub struct ConsoleRenderer {
    cols: usize,
    rows: usize,
    #[allow(dead_code)]
    theme: RenderTheme,
    output: String,
}

impl ConsoleRenderer {
    /// Create a new console renderer with default theme.
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols,
            rows,
            theme: RenderTheme::default(),
            output: String::new(),
        }
    }

    /// Create a renderer with a custom theme.
    pub fn with_theme(cols: usize, rows: usize, theme: RenderTheme) -> Self {
        Self {
            cols,
            rows,
            theme,
            output: String::new(),
        }
    }

    /// Get the rendered output string.
    pub fn output(&self) -> &str {
        &self.output
    }

    /// Get column count.
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Get row count.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Build the ANSI output for the entire grid.
    fn build_output(&self, grid: &Grid, cursor: &CursorState) -> String {
        let mut out = String::with_capacity(grid.width() * grid.height() * 4);

        // Track previous SGR state to minimize escape sequences.
        // Using Option so the first cell always emits its SGR.
        let mut prev_fg: Option<Color> = None;
        let mut prev_bg: Option<Color> = None;
        let mut prev_flags: CellFlags = CellFlags::empty();
        // Force SGR on first cell by setting a sentinel that won't match.
        let mut first_cell = true;

        for row in 0..grid.height() {
            if row > 0 {
                out.push('\n');
            }

            for col in 0..grid.width() {
                let cell = match grid.cell(col, row) {
                    Some(c) => c,
                    None => continue,
                };

                // Skip wide-char spacer (already rendered by lead cell)
                if cell.flags.contains(CellFlags::WIDE_SPACER) {
                    continue;
                }

                // Determine effective colors (handle cursor + REVERSE)
                let is_cursor = cursor.visible && cursor.x == col && cursor.y == row;
                let (fg, bg) = if is_cursor {
                    (cell.bg, cell.fg) // swap for cursor highlight
                } else if cell.flags.contains(CellFlags::REVERSE) {
                    (cell.bg, cell.fg)
                } else {
                    (cell.fg, cell.bg)
                };

                let effective_flags = if is_cursor {
                    cell.flags | CellFlags::REVERSE
                } else {
                    cell.flags
                };

                // Emit SGR changes only when attributes change
                let need_sgr = first_cell
                    || fg != prev_fg.unwrap_or(Color::Default)
                    || bg != prev_bg.unwrap_or(Color::Default)
                    || effective_flags != prev_flags;

                if need_sgr {
                    out.push_str(&Self::sgr_sequence(&fg, &bg, effective_flags));
                    prev_fg = Some(fg);
                    prev_bg = Some(bg);
                    prev_flags = effective_flags;
                    first_cell = false;
                }

                // Handle HIDDEN and null char
                let ch = if cell.flags.contains(CellFlags::HIDDEN) {
                    ' '
                } else if cell.ch == '\0' {
                    ' '
                } else {
                    cell.ch
                };

                out.push(ch);
            }
        }

        // Reset SGR at end
        out.push_str("\x1b[0m");

        out
    }

    /// Generate ANSI SGR escape sequence for the given colors and flags.
    fn sgr_sequence(fg: &Color, bg: &Color, flags: CellFlags) -> String {
        let mut params: Vec<String> = Vec::new();

        if flags.contains(CellFlags::BOLD) {
            params.push("1".into());
        }
        if flags.contains(CellFlags::DIM) {
            params.push("2".into());
        }
        if flags.contains(CellFlags::ITALIC) {
            params.push("3".into());
        }
        if flags.contains(CellFlags::UNDERLINE) {
            params.push("4".into());
        }
        if flags.contains(CellFlags::BLINK) {
            params.push("5".into());
        }
        if flags.contains(CellFlags::REVERSE) {
            params.push("7".into());
        }
        if flags.contains(CellFlags::HIDDEN) {
            params.push("8".into());
        }
        if flags.contains(CellFlags::STRIKETHROUGH) {
            params.push("9".into());
        }

        // Foreground color
        match fg {
            Color::Default => {}
            Color::Indexed(n) => {
                if *n < 8 {
                    params.push(format!("{}", 30 + n));
                } else if *n < 16 {
                    params.push(format!("{}", 90 + (n - 8)));
                } else {
                    params.push(format!("38;5;{}", n));
                }
            }
            Color::Rgb(r, g, b) => {
                params.push(format!("38;2;{};{};{}", r, g, b));
            }
        }

        // Background color
        match bg {
            Color::Default => {}
            Color::Indexed(n) => {
                if *n < 8 {
                    params.push(format!("{}", 40 + n));
                } else if *n < 16 {
                    params.push(format!("{}", 100 + (n - 8)));
                } else {
                    params.push(format!("48;5;{}", n));
                }
            }
            Color::Rgb(r, g, b) => {
                params.push(format!("48;2;{};{};{}", r, g, b));
            }
        }

        if params.is_empty() {
            "\x1b[0m".into()
        } else {
            format!("\x1b[{}m", params.join(";"))
        }
    }
}

impl Renderer for ConsoleRenderer {
    fn render(&mut self, grid: &Grid, cursor: &CursorState, _dirty: Option<&DirtyRect>) {
        self.output = self.build_output(grid, cursor);
    }

    fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.rows = rows;
    }
}
