//! Default render theme — colors and cursor configuration.
//!
//! Extended in Phase 5 with named color schemes (`NamedTheme`) and a
//! `ThemeManager` for hot-swapping.

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
    /// Selection background color (for mouse-selected text).
    pub selection_bg: Color,
}

/// Solarized 16-color ANSI palette (shared by both dark and light variants).
const SOLARIZED_PALETTE: [Color; 16] = [
    Color::Rgb(0x07, 0x36, 0x42), // 0  black (base02)
    Color::Rgb(0xdc, 0x32, 0x2f), // 1  red
    Color::Rgb(0x85, 0x99, 0x00), // 2  green
    Color::Rgb(0xb5, 0x89, 0x00), // 3  yellow
    Color::Rgb(0x26, 0x8b, 0xd2), // 4  blue
    Color::Rgb(0xd3, 0x36, 0x82), // 5  magenta
    Color::Rgb(0x2a, 0xa1, 0x98), // 6  cyan
    Color::Rgb(0xee, 0xe8, 0xd5), // 7  white (base2)
    Color::Rgb(0x00, 0x2b, 0x36), // 8  bright black (base03)
    Color::Rgb(0xcb, 0x4b, 0x16), // 9  bright red (orange)
    Color::Rgb(0x58, 0x6e, 0x75), // 10 bright green (base01)
    Color::Rgb(0x65, 0x7b, 0x83), // 11 bright yellow (base00)
    Color::Rgb(0x83, 0x94, 0x96), // 12 bright blue (base0)
    Color::Rgb(0x6c, 0x71, 0xc4), // 13 bright magenta (violet)
    Color::Rgb(0x93, 0xa1, 0xa1), // 14 bright cyan (base1)
    Color::Rgb(0xfd, 0xf6, 0xe3), // 15 bright white (base3)
];

/// Nord 16-color palette — Arctic, north-bluish colors.
const NORD_PALETTE: [Color; 16] = [
    Color::Rgb(0x3b, 0x42, 0x52), // 0  black (nord0)
    Color::Rgb(0xbf, 0x61, 0x6a), // 1  red (nord11)
    Color::Rgb(0xa3, 0xbe, 0x8c), // 2  green (nord14)
    Color::Rgb(0xeb, 0xcb, 0x8b), // 3  yellow (nord13)
    Color::Rgb(0x81, 0xa1, 0xc1), // 4  blue (nord9)
    Color::Rgb(0xb4, 0x8e, 0xad), // 5  magenta (nord15)
    Color::Rgb(0x88, 0xc0, 0xd0), // 6  cyan (nord8)
    Color::Rgb(0xe5, 0xe9, 0xf0), // 7  white (nord6)
    Color::Rgb(0x4c, 0x56, 0x6a), // 8  bright black (nord3)
    Color::Rgb(0xbf, 0x61, 0x6a), // 9  bright red (nord11)
    Color::Rgb(0xa3, 0xbe, 0x8c), // 10 bright green (nord14)
    Color::Rgb(0xeb, 0xcb, 0x8b), // 11 bright yellow (nord13)
    Color::Rgb(0x81, 0xa1, 0xc1), // 12 bright blue (nord9)
    Color::Rgb(0xb4, 0x8e, 0xad), // 13 bright magenta (nord15)
    Color::Rgb(0x8f, 0xbc, 0xbb), // 14 bright cyan (nord7)
    Color::Rgb(0xe5, 0xe9, 0xf0), // 15 bright white (nord6)
];

/// Tokyo Night 16-color palette.
const TOKYO_NIGHT_PALETTE: [Color; 16] = [
    Color::Rgb(0x15, 0x16, 0x1e), // 0  black
    Color::Rgb(0xf7, 0x76, 0x8e), // 1  red
    Color::Rgb(0x9e, 0xce, 0x6a), // 2  green
    Color::Rgb(0xe0, 0xaf, 0x68), // 3  yellow
    Color::Rgb(0x7a, 0xa2, 0xf7), // 4  blue
    Color::Rgb(0xbb, 0x9a, 0xf7), // 5  magenta
    Color::Rgb(0x7d, 0xc1, 0xd7), // 6  cyan
    Color::Rgb(0xa9, 0xb1, 0xd6), // 7  white
    Color::Rgb(0x41, 0x47, 0x57), // 8  bright black
    Color::Rgb(0xf7, 0x76, 0x8e), // 9  bright red
    Color::Rgb(0x9e, 0xce, 0x6a), // 10 bright green
    Color::Rgb(0xe0, 0xaf, 0x68), // 11 bright yellow
    Color::Rgb(0x7a, 0xa2, 0xf7), // 12 bright blue
    Color::Rgb(0xbb, 0x9a, 0xf7), // 13 bright magenta
    Color::Rgb(0x7d, 0xc1, 0xd7), // 14 bright cyan
    Color::Rgb(0xc0, 0xca, 0xf5), // 15 bright white
];

/// Catppuccin Mocha 16-color palette.
const CATPPUCCIN_MOCHA_PALETTE: [Color; 16] = [
    Color::Rgb(0x45, 0x47, 0x59), // 0  black (surface1)
    Color::Rgb(0xf3, 0x8b, 0xa8), // 1  red
    Color::Rgb(0xa6, 0xe3, 0xa1), // 2  green
    Color::Rgb(0xf9, 0xe2, 0xaf), // 3  yellow
    Color::Rgb(0x89, 0xb4, 0xfa), // 4  blue
    Color::Rgb(0xfa, 0xe3, 0xb0), // 5  magenta (replaced with peach — catppuccin has no standard magenta index)
    Color::Rgb(0x94, 0xe2, 0xd5), // 6  cyan (teal)
    Color::Rgb(0xba, 0xc2, 0xde), // 7  white (subtext1)
    Color::Rgb(0x58, 0x5b, 0x70), // 8  bright black (surface2)
    Color::Rgb(0xf3, 0x8b, 0xa8), // 9  bright red
    Color::Rgb(0xa6, 0xe3, 0xa1), // 10 bright green
    Color::Rgb(0xf9, 0xe2, 0xaf), // 11 bright yellow
    Color::Rgb(0x89, 0xb4, 0xfa), // 12 bright blue
    Color::Rgb(0xf5, 0xc2, 0xe7), // 13 bright magenta (pink)
    Color::Rgb(0x94, 0xe2, 0xd5), // 14 bright cyan (teal)
    Color::Rgb(0xc6, 0xd0, 0xf5), // 15 bright white (subtext0)
];

impl Default for RenderTheme {
    fn default() -> Self {
        Self::dark_default()
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
            Color::Default => (0xff, 0xff, 0xff),
        }
    }

    fn default_bg_rgb(&self) -> (u8, u8, u8) {
        match self.default_bg {
            Color::Rgb(r, g, b) => (r, g, b),
            Color::Indexed(n) => self.resolve_indexed(n),
            Color::Default => (0x00, 0x00, 0x00),
        }
    }

    fn resolve_indexed(&self, n: u8) -> (u8, u8, u8) {
        match n {
            0..=15 => {
                if let Color::Rgb(r, g, b) = self.palette[n as usize] {
                    (r, g, b)
                } else {
                    (0xff, 0xff, 0xff)
                }
            }
            16..=231 => {
                let idx = (n - 16) as usize;
                let r = idx / 36;
                let g = (idx % 36) / 6;
                let b = idx % 6;
                let component = |v: usize| -> u8 { if v == 0 { 0 } else { 55 + v as u8 * 40 } };
                (component(r), component(g), component(b))
            }
            232..=255 => {
                let v = 8 + (n - 232) * 10;
                (v, v, v)
            }
        }
    }

    // ── Built-in Themes ──────────────────────────────────────────

    /// Dark default theme — pure black bg + bright white fg for maximum contrast.
    pub fn dark_default() -> Self {
        Self {
            default_fg: Color::Rgb(0xff, 0xff, 0xff), // pure white
            default_bg: Color::Rgb(0x00, 0x00, 0x00), // pure black
            cursor_fg: Color::Rgb(0x00, 0x00, 0x00),
            cursor_bg: Color::Rgb(0xff, 0xff, 0xff),
            cursor_style: CursorStyle::Block,
            palette: [
                Color::Rgb(0x00, 0x00, 0x00), // 0  black
                Color::Rgb(0xff, 0x55, 0x55), // 1  red (bright)
                Color::Rgb(0x50, 0xfa, 0x7b), // 2  green (bright)
                Color::Rgb(0xf1, 0xfa, 0x8c), // 3  yellow (bright)
                Color::Rgb(0x6a, 0xbf, 0xff), // 4  blue (bright)
                Color::Rgb(0xff, 0x79, 0xc6), // 5  magenta (bright)
                Color::Rgb(0x8b, 0xe9, 0xfd), // 6  cyan (bright)
                Color::Rgb(0xb0, 0xb0, 0xb0), // 7  white (light gray)
                Color::Rgb(0x4d, 0x4d, 0x4d), // 8  bright black
                Color::Rgb(0xff, 0x6e, 0x67), // 9  bright red
                Color::Rgb(0x5a, 0xff, 0x7a), // 10 bright green
                Color::Rgb(0xf4, 0xf9, 0x9f), // 11 bright yellow
                Color::Rgb(0x8b, 0xd9, 0xff), // 12 bright blue
                Color::Rgb(0xff, 0x92, 0xd0), // 13 bright magenta
                Color::Rgb(0x9a, 0xff, 0xed), // 14 bright cyan
                Color::Rgb(0xff, 0xff, 0xff), // 15 bright white
            ],
            selection_bg: Color::Rgb(0x33, 0x33, 0x55),
        }
    }

    /// Light default theme for bright environments.
    pub fn light_default() -> Self {
        Self {
            default_fg: Color::Rgb(0x1a, 0x1a, 0x1a),
            default_bg: Color::Rgb(0xf5, 0xf5, 0xf5),
            cursor_fg: Color::Rgb(0xf5, 0xf5, 0xf5),
            cursor_bg: Color::Rgb(0x1a, 0x1a, 0x1a),
            cursor_style: CursorStyle::Block,
            palette: [
                Color::Rgb(0x00, 0x00, 0x00), // 0  black
                Color::Rgb(0xcc, 0x00, 0x00), // 1  red
                Color::Rgb(0x4e, 0x9a, 0x06), // 2  green
                Color::Rgb(0xc4, 0xa0, 0x00), // 3  yellow
                Color::Rgb(0x34, 0x65, 0xa4), // 4  blue
                Color::Rgb(0x75, 0x50, 0x7b), // 5  magenta
                Color::Rgb(0x06, 0x98, 0x9a), // 6  cyan
                // P18-B: Use dark gray instead of light gray for visibility on white.
                Color::Rgb(0x55, 0x57, 0x53), // 7  white (dark gray)
                Color::Rgb(0x55, 0x57, 0x53), // 8  bright black
                Color::Rgb(0xef, 0x29, 0x29), // 9  bright red
                Color::Rgb(0x8a, 0xe2, 0x34), // 10 bright green
                Color::Rgb(0xfc, 0xe9, 0x4f), // 11 bright yellow
                Color::Rgb(0x72, 0x9f, 0xcf), // 12 bright blue
                Color::Rgb(0xad, 0x7f, 0xa8), // 13 bright magenta
                Color::Rgb(0x34, 0xe2, 0xe2), // 14 bright cyan
                Color::Rgb(0xee, 0xee, 0xec), // 15 bright white
            ],
            selection_bg: Color::Rgb(0xaa, 0xcc, 0xff),
        }
    }

    /// Dracula — pure black bg variant for maximum contrast.
    pub fn dracula() -> Self {
        Self {
            default_fg: Color::Rgb(0xf8, 0xf8, 0xf2),
            default_bg: Color::Rgb(0x00, 0x00, 0x00), // pure black instead of #282a36
            cursor_fg: Color::Rgb(0x00, 0x00, 0x00),
            cursor_bg: Color::Rgb(0xf8, 0xf8, 0xf2),
            cursor_style: CursorStyle::Bar,
            palette: [
                Color::Rgb(0x00, 0x00, 0x00), // 0  black
                Color::Rgb(0xff, 0x55, 0x55), // 1  red
                Color::Rgb(0x50, 0xfa, 0x7b), // 2  green
                Color::Rgb(0xf1, 0xfa, 0x8c), // 3  yellow
                Color::Rgb(0xca, 0xa9, 0xfa), // 4  blue
                Color::Rgb(0xff, 0x79, 0xc6), // 5  magenta
                Color::Rgb(0x8b, 0xe9, 0xfd), // 6  cyan
                Color::Rgb(0xbf, 0xbf, 0xbf), // 7  white
                Color::Rgb(0x4d, 0x4d, 0x4d), // 8  bright black
                Color::Rgb(0xff, 0x6e, 0x67), // 9  bright red
                Color::Rgb(0x5a, 0xff, 0x7a), // 10 bright green
                Color::Rgb(0xf4, 0xf9, 0x9f), // 11 bright yellow
                Color::Rgb(0xca, 0xa9, 0xfa), // 12 bright blue
                Color::Rgb(0xff, 0x92, 0xd0), // 13 bright magenta
                Color::Rgb(0x9a, 0xff, 0xed), // 14 bright cyan
                Color::Rgb(0xe6, 0xe6, 0xe6), // 15 bright white
            ],
            selection_bg: Color::Rgb(0x6a, 0x4a, 0x8a),
        }
    }

    // ── P15-B: New built-in themes ──────────────────────────────

    /// Clean light theme with white background and dark text.
    pub fn light() -> Self {
        Self {
            default_fg: Color::Rgb(40, 40, 40),
            default_bg: Color::Rgb(250, 250, 250),
            cursor_fg: Color::Rgb(250, 250, 250),
            cursor_bg: Color::Rgb(40, 40, 40),
            cursor_style: CursorStyle::Block,
            palette: [
                Color::Rgb(0x00, 0x00, 0x00), // 0  black
                Color::Rgb(0xcc, 0x00, 0x00), // 1  red
                Color::Rgb(0x4e, 0x9a, 0x06), // 2  green
                Color::Rgb(0xc4, 0xa0, 0x00), // 3  yellow
                Color::Rgb(0x34, 0x65, 0xa4), // 4  blue
                Color::Rgb(0x75, 0x50, 0x7b), // 5  magenta
                Color::Rgb(0x06, 0x98, 0x9a), // 6  cyan
                // P18-B: Use dark gray for visibility on white.
                Color::Rgb(0x55, 0x57, 0x53), // 7  white (dark gray)
                Color::Rgb(0x55, 0x57, 0x53), // 8  bright black
                Color::Rgb(0xef, 0x29, 0x29), // 9  bright red
                Color::Rgb(0x8a, 0xe2, 0x34), // 10 bright green
                Color::Rgb(0xfc, 0xe9, 0x4f), // 11 bright yellow
                Color::Rgb(0x72, 0x9f, 0xcf), // 12 bright blue
                Color::Rgb(0xad, 0x7f, 0xa8), // 13 bright magenta
                Color::Rgb(0x34, 0xe2, 0xe2), // 14 bright cyan
                Color::Rgb(0xee, 0xee, 0xec), // 15 bright white
            ],
            selection_bg: Color::Rgb(0xb0, 0xc4, 0xde),
        }
    }

    /// Solarized Dark — high contrast variant: deep dark bg + bright fg.
    pub fn solarized_dark() -> Self {
        Self {
            default_fg: Color::Rgb(0xfd, 0xf6, 0xe3), // base3 — maximum brightness
            default_bg: Color::Rgb(0x00, 0x1a, 0x20), // darker than base03 for contrast
            cursor_fg: Color::Rgb(0x00, 0x1a, 0x20),
            cursor_bg: Color::Rgb(0xfd, 0xf6, 0xe3),
            cursor_style: CursorStyle::Block,
            palette: SOLARIZED_PALETTE,
            selection_bg: Color::Rgb(0x07, 0x36, 0x42),
        }
    }

    /// Solarized Light — improved contrast for maximum readability.
    pub fn solarized_light() -> Self {
        Self {
            // P18-B: Use base01 instead of base00 for higher contrast.
            default_fg: Color::Rgb(0x58, 0x6e, 0x75), // base01
            default_bg: Color::Rgb(0xfd, 0xf6, 0xe3), // base3
            cursor_fg: Color::Rgb(0xfd, 0xf6, 0xe3),
            cursor_bg: Color::Rgb(0x58, 0x6e, 0x75),
            cursor_style: CursorStyle::Block,
            palette: SOLARIZED_PALETTE,
            selection_bg: Color::Rgb(0xee, 0xe8, 0xd5), // base2
        }
    }

    /// Gruvbox Dark — high contrast variant.
    pub fn gruvbox() -> Self {
        Self {
            default_fg: Color::Rgb(0xfe, 0x80, 0x19), // bright orange instead of fg0 for contrast
            default_bg: Color::Rgb(0x00, 0x00, 0x00), // pure black instead of bg0
            cursor_fg: Color::Rgb(0x00, 0x00, 0x00),
            cursor_bg: Color::Rgb(0xfe, 0x80, 0x19),
            cursor_style: CursorStyle::Block,
            palette: [
                Color::Rgb(0x28, 0x28, 0x28), // 0  black (bg0)
                Color::Rgb(0xcc, 0x24, 0x1d), // 1  red
                Color::Rgb(0x98, 0x97, 0x1a), // 2  green
                Color::Rgb(0xd7, 0x99, 0x21), // 3  yellow
                Color::Rgb(0x45, 0x85, 0x88), // 4  blue
                Color::Rgb(0xb1, 0x62, 0x86), // 5  purple
                Color::Rgb(0x68, 0x9d, 0x6a), // 6  aqua
                Color::Rgb(0xa8, 0x99, 0x84), // 7  orange (fg1/gray)
                Color::Rgb(0x92, 0x83, 0x74), // 8  bright black
                Color::Rgb(0xfb, 0x49, 0x34), // 9  bright red
                Color::Rgb(0xb8, 0xbb, 0x26), // 10 bright green
                Color::Rgb(0xfa, 0xbd, 0x2f), // 11 bright yellow
                Color::Rgb(0x83, 0xa5, 0x98), // 12 bright blue
                Color::Rgb(0xd3, 0x86, 0x9b), // 13 bright purple
                Color::Rgb(0x8e, 0xc0, 0x7c), // 14 bright aqua
                Color::Rgb(0xfe, 0x80, 0x19), // 15 bright orange
            ],
            selection_bg: Color::Rgb(0x3c, 0x38, 0x36), // bg1
        }
    }

    /// Look up a built-in theme by name (case-insensitive).
    ///
    /// Returns `Some(theme)` for known names, `None` otherwise.
    /// Supported names: "dark", "light", "dracula", "solarized-dark",
    /// "solarized-light", "gruvbox", "nord", "tokyo-night", "catppuccin-mocha".
    pub fn by_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "dark" | "dark-default" | "default" => Some(Self::dark_default()),
            "light" | "light-default" => Some(Self::light_default()),
            "dracula" => Some(Self::dracula()),
            "solarized-dark" | "solarized_dark" => Some(Self::solarized_dark()),
            "solarized-light" | "solarized_light" => Some(Self::solarized_light()),
            "gruvbox" => Some(Self::gruvbox()),
            "nord" => Some(Self::nord()),
            "tokyo-night" | "tokyo_night" => Some(Self::tokyo_night()),
            "catppuccin-mocha" | "catppuccin_mocha" => Some(Self::catppuccin_mocha()),
            _ => None,
        }
    }

    /// Return all available built-in theme names.
    pub fn builtin_names() -> &'static [&'static str] {
        &[
            "dark",
            "light",
            "dracula",
            "solarized-dark",
            "solarized-light",
            "gruvbox",
            "nord",
            "tokyo-night",
            "catppuccin-mocha",
        ]
    }

    /// Nord theme — Arctic, north-bluish color palette.
    pub fn nord() -> Self {
        Self {
            default_fg: Color::Rgb(0xd8, 0xde, 0xe9),
            default_bg: Color::Rgb(0x2e, 0x34, 0x40),
            cursor_fg: Color::Rgb(0x2e, 0x34, 0x40),
            cursor_bg: Color::Rgb(0xd8, 0xde, 0xe9),
            cursor_style: CursorStyle::Block,
            palette: NORD_PALETTE,
            selection_bg: Color::Rgb(0x3b, 0x42, 0x52),
        }
    }

    /// Tokyo Night theme — A clean, dark color scheme inspired by Tokyo.
    pub fn tokyo_night() -> Self {
        Self {
            default_fg: Color::Rgb(0xa9, 0xb1, 0xd6),
            default_bg: Color::Rgb(0x1a, 0x1b, 0x26),
            cursor_fg: Color::Rgb(0x1a, 0x1b, 0x26),
            cursor_bg: Color::Rgb(0xc0, 0xca, 0xf5),
            cursor_style: CursorStyle::Block,
            palette: TOKYO_NIGHT_PALETTE,
            selection_bg: Color::Rgb(0x2a, 0x2b, 0x3c),
        }
    }

    /// Catppuccin Mocha — Soothing pastel theme for dark environments.
    pub fn catppuccin_mocha() -> Self {
        Self {
            default_fg: Color::Rgb(0xcd, 0xd6, 0xf4),
            default_bg: Color::Rgb(0x1e, 0x1e, 0x2e),
            cursor_fg: Color::Rgb(0x1e, 0x1e, 0x2e),
            cursor_bg: Color::Rgb(0xf5, 0xe0, 0xdc),
            cursor_style: CursorStyle::Block,
            palette: CATPPUCCIN_MOCHA_PALETTE,
            selection_bg: Color::Rgb(0x31, 0x31, 0x4e),
        }
    }
}

/// Manages the active theme and provides hot-swap functionality.
pub struct ThemeManager {
    current: RenderTheme,
    current_name: String,
}

impl ThemeManager {
    /// Create a new ThemeManager with the given theme.
    pub fn new(theme: RenderTheme, name: impl Into<String>) -> Self {
        Self {
            current: theme,
            current_name: name.into(),
        }
    }

    /// Create a ThemeManager starting with the dark default theme.
    pub fn with_default() -> Self {
        Self::new(RenderTheme::dark_default(), "dark")
    }

    /// Get the current theme.
    pub fn current(&self) -> &RenderTheme {
        &self.current
    }

    /// Get the current theme name.
    pub fn current_name(&self) -> &str {
        &self.current_name
    }

    /// Swap to a new theme. Returns `true` if the theme was found and applied.
    pub fn set_by_name(&mut self, name: &str) -> bool {
        if let Some(theme) = RenderTheme::by_name(name) {
            self.current = theme;
            self.current_name = name.to_lowercase();
            true
        } else {
            false
        }
    }

    /// Set a custom theme directly.
    pub fn set_theme(&mut self, theme: RenderTheme, name: impl Into<String>) {
        self.current = theme;
        self.current_name = name.into();
    }

    /// List all available theme names.
    pub fn available_themes() -> &'static [&'static str] {
        RenderTheme::builtin_names()
    }

    /// Cycle to the next theme in the built-in list.
    ///
    /// Wraps around after the last theme. Returns the new theme name.
    pub fn cycle_next(&mut self) -> &str {
        let names = Self::available_themes();
        let idx = names
            .iter()
            .position(|&n| n == self.current_name)
            .map(|i| (i + 1) % names.len())
            .unwrap_or(0);
        self.set_by_name(names[idx]);
        &self.current_name
    }
}

impl Default for ThemeManager {
    fn default() -> Self {
        Self::with_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_default_is_dark() {
        let theme = RenderTheme::default();
        let (r, g, b) = theme.default_bg_rgb();
        assert!(
            r < 10 && g < 10 && b < 10,
            "default theme should have near-pure black bg"
        );
    }

    #[test]
    fn t_dark_default_colors() {
        let theme = RenderTheme::dark_default();
        let (r, _g, _b) = theme.resolve_fg(&Color::Default);
        assert!(r > 200, "dark theme fg should be very bright");
        let (r, g, b) = theme.resolve_bg(&Color::Default);
        assert_eq!((r, g, b), (0, 0, 0), "dark theme bg should be pure black");
    }

    #[test]
    fn t_light_default_colors() {
        let theme = RenderTheme::light_default();
        let (r, _, _) = theme.resolve_fg(&Color::Default);
        assert!(r < 128, "light theme fg should be dark");
        let (r, _, _) = theme.resolve_bg(&Color::Default);
        assert!(r > 128, "light theme bg should be light");
    }

    #[test]
    fn t_dracula_colors() {
        let theme = RenderTheme::dracula();
        // Dracula bg now pure black
        let (r, g, b) = theme.resolve_bg(&Color::Default);
        assert_eq!((r, g, b), (0x00, 0x00, 0x00));
        // Dracula foreground is #f8f8f2
        let (r, g, b) = theme.resolve_fg(&Color::Default);
        assert_eq!((r, g, b), (0xf8, 0xf8, 0xf2));
    }

    #[test]
    fn t_dracula_cursor_style_is_bar() {
        let theme = RenderTheme::dracula();
        assert_eq!(theme.cursor_style, CursorStyle::Bar);
    }

    #[test]
    fn t_by_name_dark() {
        let theme = RenderTheme::by_name("dark").unwrap();
        assert_eq!(theme.default_bg, Color::Rgb(0x00, 0x00, 0x00));
    }

    #[test]
    fn t_by_name_light() {
        let theme = RenderTheme::by_name("light").unwrap();
        assert_eq!(theme.default_bg, Color::Rgb(0xf5, 0xf5, 0xf5));
    }

    #[test]
    fn t_by_name_dracula() {
        let theme = RenderTheme::by_name("dracula").unwrap();
        assert_eq!(theme.default_bg, Color::Rgb(0x00, 0x00, 0x00));
    }

    #[test]
    fn t_by_name_case_insensitive() {
        assert!(RenderTheme::by_name("DARK").is_some());
        assert!(RenderTheme::by_name("Light").is_some());
        assert!(RenderTheme::by_name("DRACULA").is_some());
    }

    #[test]
    fn t_by_name_alias() {
        assert!(RenderTheme::by_name("default").is_some());
        assert!(RenderTheme::by_name("dark-default").is_some());
        assert!(RenderTheme::by_name("light-default").is_some());
    }

    #[test]
    fn t_by_name_unknown() {
        assert!(RenderTheme::by_name("nonexistent").is_none());
    }

    #[test]
    fn t_builtin_names() {
        let names = RenderTheme::builtin_names();
        assert!(names.contains(&"dark"));
        assert!(names.contains(&"light"));
        assert!(names.contains(&"dracula"));
        assert!(names.contains(&"solarized-dark"));
        assert!(names.contains(&"solarized-light"));
        assert!(names.contains(&"gruvbox"));
        assert!(names.contains(&"nord"));
        assert!(names.contains(&"tokyo-night"));
        assert!(names.contains(&"catppuccin-mocha"));
        assert_eq!(names.len(), 9);
    }

    #[test]
    fn t_resolve_rgb_passthrough() {
        let theme = RenderTheme::dark_default();
        let c = Color::Rgb(0xff, 0x00, 0xff);
        let (r, g, b) = theme.resolve(&c);
        assert_eq!((r, g, b), (0xff, 0x00, 0xff));
    }

    #[test]
    fn t_resolve_indexed_0() {
        let theme = RenderTheme::dark_default();
        let c = Color::Indexed(0);
        let (_, g, b) = theme.resolve(&c);
        // Index 0 = black
        assert_eq!((g, b), (0, 0));
    }

    #[test]
    fn t_resolve_indexed_cube() {
        let theme = RenderTheme::dark_default();
        let c = Color::Indexed(16); // start of color cube
        let (r, g, b) = theme.resolve(&c);
        assert_eq!((r, g, b), (0, 0, 0));
    }

    #[test]
    fn t_resolve_indexed_grayscale() {
        let theme = RenderTheme::dark_default();
        let c = Color::Indexed(232); // start of grayscale
        let (r, g, b) = theme.resolve(&c);
        assert_eq!(r, 8);
        assert_eq!(g, 8);
        assert_eq!(b, 8);
    }

    #[test]
    fn t_selection_bg_dark() {
        let theme = RenderTheme::dark_default();
        assert!(matches!(theme.selection_bg, Color::Rgb(0x33, 0x33, 0x55)));
    }

    #[test]
    fn t_selection_bg_light() {
        let theme = RenderTheme::light_default();
        assert!(matches!(theme.selection_bg, Color::Rgb(0xaa, 0xcc, 0xff)));
    }

    // --- ThemeManager tests ---

    #[test]
    fn t_manager_default() {
        let mgr = ThemeManager::default();
        assert_eq!(mgr.current_name(), "dark");
    }

    #[test]
    fn t_manager_with_default() {
        let mgr = ThemeManager::with_default();
        assert_eq!(mgr.current_name(), "dark");
    }

    #[test]
    fn t_manager_current() {
        let mgr = ThemeManager::with_default();
        let theme = mgr.current();
        assert_eq!(theme.default_bg, Color::Rgb(0x00, 0x00, 0x00));
    }

    #[test]
    fn t_manager_set_by_name_success() {
        let mut mgr = ThemeManager::with_default();
        assert!(mgr.set_by_name("light"));
        assert_eq!(mgr.current_name(), "light");
        assert_eq!(mgr.current().default_bg, Color::Rgb(0xf5, 0xf5, 0xf5));
    }

    #[test]
    fn t_manager_set_by_name_case_insensitive() {
        let mut mgr = ThemeManager::with_default();
        assert!(mgr.set_by_name("DRACULA"));
        assert_eq!(mgr.current_name(), "dracula");
    }

    #[test]
    fn t_manager_set_by_name_unknown() {
        let mut mgr = ThemeManager::with_default();
        assert!(!mgr.set_by_name("nonexistent"));
        // Theme should not change
        assert_eq!(mgr.current_name(), "dark");
    }

    #[test]
    fn t_manager_set_custom_theme() {
        let mut mgr = ThemeManager::with_default();
        let custom = RenderTheme::light_default();
        mgr.set_theme(custom, "custom");
        assert_eq!(mgr.current_name(), "custom");
    }

    #[test]
    fn t_manager_available_themes() {
        let names = ThemeManager::available_themes();
        assert!(names.contains(&"dark"));
        assert!(names.contains(&"light"));
        assert!(names.contains(&"dracula"));
    }

    #[test]
    fn t_manager_swap_multiple() {
        let mut mgr = ThemeManager::with_default();
        mgr.set_by_name("light");
        assert_eq!(mgr.current_name(), "light");
        mgr.set_by_name("dracula");
        assert_eq!(mgr.current_name(), "dracula");
        mgr.set_by_name("dark");
        assert_eq!(mgr.current_name(), "dark");
    }

    // ── P15-B: New theme tests ──────────────────────────────────

    #[test]
    fn t_light_theme_colors() {
        let theme = RenderTheme::light();
        assert_eq!(theme.default_bg, Color::Rgb(250, 250, 250));
        assert_eq!(theme.default_fg, Color::Rgb(40, 40, 40));
    }

    #[test]
    fn t_solarized_dark_colors() {
        let theme = RenderTheme::solarized_dark();
        assert_eq!(theme.default_bg, Color::Rgb(0x00, 0x1a, 0x20));
        assert_eq!(theme.default_fg, Color::Rgb(0xfd, 0xf6, 0xe3));
    }

    #[test]
    fn t_solarized_light_colors() {
        let theme = RenderTheme::solarized_light();
        // bg = base3 (253, 246, 227)
        assert_eq!(theme.default_bg, Color::Rgb(0xfd, 0xf6, 0xe3));
        // P18-B: fg = base01 (88, 110, 117) for higher contrast
        assert_eq!(theme.default_fg, Color::Rgb(0x58, 0x6e, 0x75));
    }

    #[test]
    fn t_gruvbox_colors() {
        let theme = RenderTheme::gruvbox();
        assert_eq!(theme.default_bg, Color::Rgb(0x00, 0x00, 0x00));
        assert_eq!(theme.default_fg, Color::Rgb(0xfe, 0x80, 0x19));
    }

    #[test]
    fn t_by_name_new_themes() {
        assert!(RenderTheme::by_name("solarized-dark").is_some());
        assert!(RenderTheme::by_name("solarized-light").is_some());
        assert!(RenderTheme::by_name("gruvbox").is_some());
    }

    #[test]
    fn t_by_name_solarized_underscore() {
        // Underscore variant should also work
        assert!(RenderTheme::by_name("solarized_dark").is_some());
        assert!(RenderTheme::by_name("solarized_light").is_some());
    }

    #[test]
    fn t_builtin_names_includes_new() {
        let names = RenderTheme::builtin_names();
        assert!(names.contains(&"solarized-dark"));
        assert!(names.contains(&"solarized-light"));
        assert!(names.contains(&"gruvbox"));
        assert!(names.contains(&"nord"));
        assert!(names.contains(&"tokyo-night"));
        assert!(names.contains(&"catppuccin-mocha"));
        assert_eq!(names.len(), 9);
    }

    #[test]
    fn t_nord_theme_colors() {
        let theme = RenderTheme::nord();
        assert_eq!(theme.default_fg, Color::Rgb(0xd8, 0xde, 0xe9));
        assert_eq!(theme.default_bg, Color::Rgb(0x2e, 0x34, 0x40));
    }

    #[test]
    fn t_tokyo_night_colors() {
        let theme = RenderTheme::tokyo_night();
        assert_eq!(theme.default_fg, Color::Rgb(0xa9, 0xb1, 0xd6));
        assert_eq!(theme.default_bg, Color::Rgb(0x1a, 0x1b, 0x26));
    }

    #[test]
    fn t_catppuccin_mocha_colors() {
        let theme = RenderTheme::catppuccin_mocha();
        assert_eq!(theme.default_fg, Color::Rgb(0xcd, 0xd6, 0xf4));
        assert_eq!(theme.default_bg, Color::Rgb(0x1e, 0x1e, 0x2e));
    }

    #[test]
    fn t_by_name_new_p23b_themes() {
        assert!(RenderTheme::by_name("nord").is_some());
        assert!(RenderTheme::by_name("tokyo-night").is_some());
        assert!(RenderTheme::by_name("tokyo_night").is_some());
        assert!(RenderTheme::by_name("catppuccin-mocha").is_some());
        assert!(RenderTheme::by_name("catppuccin_mocha").is_some());
    }

    #[test]
    fn t_cycle_next_wraps_around() {
        let mut mgr = ThemeManager::with_default();
        let all = ThemeManager::available_themes().to_vec();
        // Cycle through all themes and verify wrap-around
        for i in 0..all.len() * 2 {
            let name = mgr.cycle_next();
            let expected = all[(i + 1) % all.len()];
            assert_eq!(name, expected, "cycle {i}: expected {expected}, got {name}");
        }
    }

    #[test]
    fn t_cycle_next_visits_all_themes() {
        let mut mgr = ThemeManager::with_default();
        let total = ThemeManager::available_themes().len();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..total {
            let name = mgr.cycle_next().to_string();
            seen.insert(name);
        }
        assert_eq!(seen.len(), total, "should have visited all {total} themes");
    }
}
