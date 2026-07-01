//! P28-B: Color picker — hover over color codes to see a preview swatch.
//!
//! Detects `#RRGGBB`, `#RGB`, `rgb(r,g,b)`, and named CSS colors in the
//! terminal grid. When the mouse hovers over a color code, a small swatch
//! is displayed.

use std::collections::HashMap;

/// A detected color code in the grid.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorMatch {
    /// Absolute row in the grid.
    pub row: usize,
    /// Start column (0-based).
    pub col_start: usize,
    /// End column (exclusive).
    pub col_end: usize,
    /// The parsed (R, G, B) value.
    pub rgb: (u8, u8, u8),
    /// The original text matched.
    pub text: String,
}

/// State for the color picker overlay.
#[derive(Debug, Default)]
pub struct ColorPickerState {
    /// Currently hovered color match (if any).
    pub hovered: Option<ColorMatch>,
}

impl ColorPickerState {
    /// Create new color picker state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if the color picker is active (has a hovered color).
    pub fn is_active(&self) -> bool {
        self.hovered.is_some()
    }

    /// Get the hovered color as (R, G, B).
    pub fn hovered_rgb(&self) -> Option<(u8, u8, u8)> {
        self.hovered.as_ref().map(|c| c.rgb)
    }

    /// Clear the hovered color.
    pub fn clear(&mut self) {
        self.hovered = None;
    }
}

/// Parse a hex color string like `#RRGGBB` or `#RGB`.
/// Returns Some((r, g, b)) if valid, None otherwise.
pub fn parse_hex_color(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.strip_prefix('#')?;
    match s.len() {
        6 => {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            Some((r, g, b))
        }
        3 => {
            let r = u8::from_str_radix(&s[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&s[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&s[2..3].repeat(2), 16).ok()?;
            Some((r, g, b))
        }
        8 => {
            // #RRGGBBAA — ignore alpha for display
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            Some((r, g, b))
        }
        _ => None,
    }
}

/// Parse `rgb(r, g, b)` or `rgba(r, g, b, a)` color string.
pub fn parse_rgb_color(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim();
    let s = s.strip_prefix("rgb(").or_else(|| s.strip_prefix("rgba("))?;
    let s = s.strip_suffix(')')?;
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() < 3 {
        return None;
    }
    let r = parts[0].trim().parse::<u8>().ok()?;
    let g = parts[1].trim().parse::<u8>().ok()?;
    let b = parts[2].trim().parse::<u8>().ok()?;
    Some((r, g, b))
}

/// Parse an ANSI 24-bit color spec: `\x1b[38;2;R;G;Bm` or `\x1b[48;2;R;G;Bm`.
/// Extracted from raw escape sequence text.
pub fn parse_ansi_24bit(s: &str) -> Option<(u8, u8, u8)> {
    // Look for pattern: 38;2;R;G;B or 48;2;R;G;B
    let s = s.strip_suffix('m')?;
    let parts: Vec<&str> = s.split(';').collect();
    if parts.len() < 5 {
        return None;
    }
    // Check for 38;2 or 48;2 prefix
    let prefix = format!("{};{}", parts[0], parts[1]);
    if prefix != "38;2" && prefix != "48;2" {
        return None;
    }
    let r = parts[2].parse::<u8>().ok()?;
    let g = parts[3].parse::<u8>().ok()?;
    let b = parts[4].parse::<u8>().ok()?;
    Some((r, g, b))
}

/// Get a map of CSS named colors to RGB values.
/// Only the most common ones.
pub fn css_named_colors() -> HashMap<&'static str, (u8, u8, u8)> {
    let mut m = HashMap::new();
    m.insert("black", (0, 0, 0));
    m.insert("white", (255, 255, 255));
    m.insert("red", (255, 0, 0));
    m.insert("green", (0, 128, 0));
    m.insert("blue", (0, 0, 255));
    m.insert("yellow", (255, 255, 0));
    m.insert("cyan", (0, 255, 255));
    m.insert("magenta", (255, 0, 255));
    m.insert("gray", (128, 128, 128));
    m.insert("grey", (128, 128, 128));
    m.insert("orange", (255, 165, 0));
    m.insert("purple", (128, 0, 128));
    m.insert("pink", (255, 192, 203));
    m.insert("brown", (165, 42, 42));
    m.insert("navy", (0, 0, 128));
    m.insert("teal", (0, 128, 128));
    m.insert("maroon", (128, 0, 0));
    m.insert("lime", (0, 255, 0));
    m.insert("olive", (128, 128, 0));
    m.insert("silver", (192, 192, 192));
    m
}

/// Scan a line of text for color codes.
/// Returns all matches found.
pub fn scan_line_for_colors(text: &str) -> Vec<ColorMatch> {
    let mut results = Vec::new();
    let bytes = text.as_bytes();

    let mut i = 0;
    while i < bytes.len() {
        // Try hex color (#RRGGBB)
        if bytes[i] == b'#' && i + 1 < bytes.len() {
            let remaining = &text[i..];
            // Find the longest hex sequence
            let mut hex_len = 0;
            for (j, c) in remaining[1..].chars().enumerate() {
                if c.is_ascii_hexdigit() {
                    hex_len = j + 1;
                } else {
                    break;
                }
            }
            if hex_len == 6 || hex_len == 3 || hex_len == 8 {
                let hex_str = &remaining[..=hex_len];
                if let Some(rgb) = parse_hex_color(hex_str) {
                    results.push(ColorMatch {
                        row: 0, // Set by caller
                        col_start: i,
                        col_end: i + hex_len + 1,
                        rgb,
                        text: hex_str.to_string(),
                    });
                    i += hex_len + 1;
                    continue;
                }
            }
        }

        // Try rgb()/rgba() — use byte slice for prefix check to avoid
        // slicing inside multi-byte UTF-8 sequences.
        let is_rgb = i + 4 <= bytes.len() && &bytes[i..i + 4] == b"rgb(";
        let is_rgba = i + 5 <= bytes.len() && &bytes[i..i + 5] == b"rgba(";
        if (is_rgb || is_rgba)
            && let Some(end) = text[i..].find(')')
        {
            let candidate = &text[i..=i + end];
            if let Some(rgb) = parse_rgb_color(candidate) {
                results.push(ColorMatch {
                    row: 0,
                    col_start: i,
                    col_end: i + end + 1,
                    rgb,
                    text: candidate.to_string(),
                });
                i += end + 1;
                continue;
            }
        }

        // Advance by the full UTF-8 character width to avoid landing
        // inside a multi-byte sequence (e.g. ✘ = 3 bytes).
        let char_len = text[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        i += char_len;
    }

    results
}

/// Check if the mouse position (col, row) is over a color match.
pub fn find_color_at(matches: &[ColorMatch], col: usize, row: usize) -> Option<&ColorMatch> {
    matches
        .iter()
        .find(|m| m.row == row && col >= m.col_start && col < m.col_end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_parse_hex_6digit() {
        assert_eq!(parse_hex_color("#FF5733"), Some((0xFF, 0x57, 0x33)));
        assert_eq!(parse_hex_color("#000000"), Some((0, 0, 0)));
        assert_eq!(parse_hex_color("#ffffff"), Some((255, 255, 255)));
    }

    #[test]
    fn t_parse_hex_3digit() {
        assert_eq!(parse_hex_color("#F00"), Some((255, 0, 0)));
        assert_eq!(parse_hex_color("#0F0"), Some((0, 255, 0)));
        assert_eq!(parse_hex_color("#00F"), Some((0, 0, 255)));
    }

    #[test]
    fn t_parse_hex_8digit() {
        assert_eq!(parse_hex_color("#FF573380"), Some((0xFF, 0x57, 0x33)));
    }

    #[test]
    fn t_parse_hex_invalid() {
        assert_eq!(parse_hex_color("FF5733"), None);
        assert_eq!(parse_hex_color("#GGG"), None);
        assert_eq!(parse_hex_color("#1"), None);
    }

    #[test]
    fn t_parse_rgb() {
        assert_eq!(parse_rgb_color("rgb(255, 0, 0)"), Some((255, 0, 0)));
        assert_eq!(parse_rgb_color("rgb(100, 200, 50)"), Some((100, 200, 50)));
    }

    #[test]
    fn t_parse_rgba() {
        assert_eq!(parse_rgb_color("rgba(255, 0, 0, 0.5)"), Some((255, 0, 0)));
    }

    #[test]
    fn t_parse_rgb_invalid() {
        assert_eq!(parse_rgb_color("rgb(300, 0, 0)"), None);
        assert_eq!(parse_rgb_color("rgb(0)"), None);
    }

    #[test]
    fn t_parse_ansi_24bit_fg() {
        assert_eq!(parse_ansi_24bit("38;2;255;100;50m"), Some((255, 100, 50)));
    }

    #[test]
    fn t_parse_ansi_24bit_bg() {
        assert_eq!(parse_ansi_24bit("48;2;0;0;255m"), Some((0, 0, 255)));
    }

    #[test]
    fn t_parse_ansi_invalid() {
        assert_eq!(parse_ansi_24bit("38;5;255m"), None); // 256-color, not 24-bit
    }

    #[test]
    fn t_scan_line_hex() {
        let matches = scan_line_for_colors("color: #FF5733; background: #000000");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].rgb, (0xFF, 0x57, 0x33));
        assert_eq!(matches[1].rgb, (0, 0, 0));
    }

    #[test]
    fn t_scan_line_rgb() {
        let matches = scan_line_for_colors("color: rgb(255, 100, 50);");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].rgb, (255, 100, 50));
    }

    #[test]
    fn t_scan_line_mixed() {
        let matches = scan_line_for_colors("#F00 rgb(0,255,0) #00FF00");
        assert_eq!(matches.len(), 3);
    }

    #[test]
    fn t_scan_line_no_colors() {
        let matches = scan_line_for_colors("hello world");
        assert!(matches.is_empty());
    }

    #[test]
    fn t_scan_line_short_hex_rejected() {
        let matches = scan_line_for_colors("#12 not a color");
        assert!(matches.is_empty());
    }

    #[test]
    fn t_find_color_at() {
        let matches = vec![ColorMatch {
            row: 5,
            col_start: 10,
            col_end: 17,
            rgb: (255, 87, 51),
            text: "#FF5733".to_string(),
        }];
        assert!(find_color_at(&matches, 10, 5).is_some());
        assert!(find_color_at(&matches, 16, 5).is_some());
        assert!(find_color_at(&matches, 17, 5).is_none()); // exclusive end
        assert!(find_color_at(&matches, 10, 6).is_none()); // wrong row
    }

    #[test]
    fn t_css_named_colors() {
        let m = css_named_colors();
        assert_eq!(m.get("red"), Some(&(255, 0, 0)));
        assert_eq!(m.get("blue"), Some(&(0, 0, 255)));
        assert_eq!(m.get("nonexistent"), None);
    }

    #[test]
    fn t_color_picker_state() {
        let mut state = ColorPickerState::new();
        assert!(!state.is_active());
        state.hovered = Some(ColorMatch {
            row: 0,
            col_start: 0,
            col_end: 7,
            rgb: (255, 0, 0),
            text: "#FF0000".to_string(),
        });
        assert!(state.is_active());
        assert_eq!(state.hovered_rgb(), Some((255, 0, 0)));
        state.clear();
        assert!(!state.is_active());
    }

    #[test]
    fn t_scan_line_multibyte_no_panic() {
        // Multi-byte UTF-8 characters (✘ = 3 bytes, → = 3 bytes) should
        // not cause a panic when scanning for colors.
        let matches = scan_line_for_colors("✘ error → fix #FF5733 ✘");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].rgb, (0xFF, 0x57, 0x33));
    }

    #[test]
    fn t_scan_line_rgb_with_emoji() {
        let matches = scan_line_for_colors("🎨 rgb(100, 200, 50) done");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].rgb, (100, 200, 50));
    }
}
