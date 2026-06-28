//! Default render theme — colors and cursor configuration.

use ggterm_core::Color;

/// Cursor display style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorStyle {
    #[default]
    Block,
    Underline,
    Bar,
}

/// Theme controlling default colors and cursor appearance.
#[derive(Debug, Clone)]
pub struct RenderTheme {
    /// Default foreground color (when cell fg = Color::Default).
    pub default_fg: Color,
    /// Default background color (when cell bg = Color::Default).
    pub default_bg: Color,
    /// Cursor foreground (text color inside cursor).
    pub cursor_fg: Color,
    /// Cursor background.
    pub cursor_bg: Color,
    /// Cursor style.
    pub cursor_style: CursorStyle,
    /// 16-color ANSI palette for Indexed(0..15).
    pub palette: [Color; 16],
}

impl Default for RenderTheme {
    fn default() -> Self {
        Self {
            default_fg: Color::Rgb(0xe0, 0xe0, 0xe0),
            default_bg: Color::Rgb(0x1a, 0x1a, 0x1a),
            cursor_fg: Color::Rgb(0x1a, 0x1a, 0x1a),
            cursor_bg: Color::Rgb(0xe0, 0xe0, 0xe0),
            cursor_style: CursorStyle::Block,
            palette: Color::default_palette(),
        }
    }
}

impl RenderTheme {
    /// Resolve a `Color` value to an RGB triple.
    pub fn resolve(&self, color: &Color) -> (u8, u8, u8) {
        match color {
            Color::Default => self.default_fg_rgb(),
            Color::Indexed(n) => self.resolve_indexed(*n),
            Color::Rgb(r, g, b) => (*r, *g, *b),
        }
    }

    /// Resolve foreground color, falling back to default_fg.
    pub fn resolve_fg(&self, color: &Color) -> (u8, u8, u8) {
        match color {
            Color::Default => self.default_fg_rgb(),
            Color::Indexed(n) => self.resolve_indexed(*n),
            Color::Rgb(r, g, b) => (*r, *g, *b),
        }
    }

    /// Resolve background color, falling back to default_bg.
    pub fn resolve_bg(&self, color: &Color) -> (u8, u8, u8) {
        match color {
            Color::Default => self.default_bg_rgb(),
            Color::Indexed(n) => self.resolve_indexed(*n),
            Color::Rgb(r, g, b) => (*r, *g, *b),
        }
    }

    fn default_fg_rgb(&self) -> (u8, u8, u8) {
        match self.default_fg {
            Color::Rgb(r, g, b) => (r, g, b),
            Color::Indexed(n) => self.resolve_indexed(n),
            Color::Default => (0xe0, 0xe0, 0xe0),
        }
    }

    fn default_bg_rgb(&self) -> (u8, u8, u8) {
        match self.default_bg {
            Color::Rgb(r, g, b) => (r, g, b),
            Color::Indexed(n) => self.resolve_indexed(n),
            Color::Default => (0x1a, 0x1a, 0x1a),
        }
    }

    fn resolve_indexed(&self, n: u8) -> (u8, u8, u8) {
        match n {
            // Standard 16 colors
            0..=15 => {
                if let Color::Rgb(r, g, b) = self.palette[n as usize] {
                    (r, g, b)
                } else {
                    (0xff, 0xff, 0xff)
                }
            }
            // 216-color cube (16..=231)
            16..=231 => {
                let idx = (n - 16) as usize;
                let r = idx / 36;
                let g = (idx % 36) / 6;
                let b = idx % 6;
                let component = |v: usize| -> u8 {
                    if v == 0 {
                        0
                    } else {
                        55 + v as u8 * 40
                    }
                };
                (component(r), component(g), component(b))
            }
            // 24 grayscale shades (232..=255)
            232..=255 => {
                let v = 8 + (n - 232) as u8 * 10;
                (v, v, v)
            }
        }
    }
}
