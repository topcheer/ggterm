//! Color mapping: ggterm_core::Color → RGB triples.
//!
//! Uses [`RenderTheme`] for resolving `Color::Default` and `Color::Indexed`.

use ggterm_core::Color;
use ggterm_render::theme::RenderTheme;

/// Standard ANSI 16-color palette (0-15) as RGB.
pub const ANSI_16: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00), // 0  black
    (0xCC, 0x00, 0x00), // 1  red
    (0x4E, 0x9A, 0x06), // 2  green
    (0xC4, 0xA0, 0x00), // 3  yellow
    (0x34, 0x65, 0xA4), // 4  blue
    (0x75, 0x50, 0x7B), // 5  magenta
    (0x06, 0x98, 0x9A), // 6  cyan
    (0xD3, 0xD7, 0xCF), // 7  white (light gray)
    (0x55, 0x57, 0x53), // 8  bright black (dark gray)
    (0xEF, 0x29, 0x29), // 9  bright red
    (0x8A, 0xE2, 0x34), // 10 bright green
    (0xFC, 0xE9, 0x4F), // 11 bright yellow
    (0x72, 0x9F, 0xCF), // 12 bright blue
    (0xAD, 0x7F, 0xA8), // 13 bright magenta
    (0x34, 0xE2, 0xE2), // 14 bright cyan
    (0xEE, 0xEE, 0xEC), // 15 bright white
];

/// Default foreground RGB.
pub const DEFAULT_FG: (u8, u8, u8) = (0xE0, 0xE0, 0xE0);
/// Default background RGB.
pub const DEFAULT_BG: (u8, u8, u8) = (0x1A, 0x1A, 0x1A);

/// Convert an indexed ANSI color (0-255) to RGB.
pub fn indexed_to_rgb(idx: u8) -> (u8, u8, u8) {
    match idx {
        0..=15 => ANSI_16[idx as usize],
        16..=231 => {
            let i = (idx - 16) as usize;
            let component = |v: usize| -> u8 { if v == 0 { 0 } else { 55 + v as u8 * 40 } };
            (
                component(i / 36 % 6),
                component(i / 6 % 6),
                component(i % 6),
            )
        }
        232..=255 => {
            let v = 8 + (idx - 232) * 10;
            (v, v, v)
        }
    }
}

/// Map a terminal foreground color to RGB using the theme.
pub fn map_fg(color: Color, theme: &RenderTheme) -> (u8, u8, u8) {
    theme.resolve_fg(&color)
}

/// Map a terminal background color to RGB using the theme.
pub fn map_bg(color: Color, theme: &RenderTheme) -> (u8, u8, u8) {
    theme.resolve_bg(&color)
}

/// Map a terminal color with explicit default RGB fallback.
pub fn map_color(color: Color, default_rgb: (u8, u8, u8)) -> (u8, u8, u8) {
    match color {
        Color::Default => default_rgb,
        Color::Indexed(n) => indexed_to_rgb(n),
        Color::Rgb(r, g, b) => (r, g, b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_indexed_basic_colors() {
        assert_eq!(indexed_to_rgb(0), (0x00, 0x00, 0x00));
        assert_eq!(indexed_to_rgb(1), (0xCC, 0x00, 0x00));
        assert_eq!(indexed_to_rgb(7), (0xD3, 0xD7, 0xCF));
    }

    #[test]
    fn test_indexed_bright_colors() {
        assert_eq!(indexed_to_rgb(9), (0xEF, 0x29, 0x29));
        assert_eq!(indexed_to_rgb(15), (0xEE, 0xEE, 0xEC));
    }

    #[test]
    fn test_indexed_256_cube() {
        // Index 21 = 16 + 0*36 + 0*6 + 5 → pure blue (5 → 255)
        let (r, g, b) = indexed_to_rgb(21);
        assert_eq!((r, g, b), (0, 0, 255));

        // Index 196 = 16 + 5*36 + 0*6 + 0 → pure red (5 → 255)
        let (r, _, _) = indexed_to_rgb(196);
        assert_eq!(r, 255);
    }

    #[test]
    fn test_indexed_grayscale() {
        let (r, g, b) = indexed_to_rgb(232);
        assert_eq!((r, g, b), (8, 8, 8));

        let (r, _, _) = indexed_to_rgb(255);
        assert_eq!(r, 238); // 8 + 23*10
    }

    #[test]
    fn test_map_fg_default() {
        let theme = RenderTheme::default();
        let rgb = map_fg(Color::Default, &theme);
        assert_eq!(rgb, (0xFF, 0xFF, 0xFF)); // pure white
    }

    #[test]
    fn test_map_bg_default() {
        let theme = RenderTheme::default();
        let rgb = map_bg(Color::Default, &theme);
        assert_eq!(rgb, (0x00, 0x00, 0x00)); // pure black
    }

    #[test]
    fn test_map_fg_rgb() {
        let theme = RenderTheme::default();
        let rgb = map_fg(Color::Rgb(100, 150, 200), &theme);
        assert_eq!(rgb, (100, 150, 200));
    }

    #[test]
    fn test_map_color_with_fallback() {
        let rgb = map_color(Color::Default, (0xFF, 0x00, 0xFF));
        assert_eq!(rgb, (0xFF, 0x00, 0xFF));
    }
}
