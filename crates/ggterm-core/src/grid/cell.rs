use bitflags::bitflags;
use unicode_width::UnicodeWidthChar;

bitflags! {
    /// Text attributes for a terminal cell.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct CellFlags: u16 {
        /// Bold / increased intensity.
        const BOLD       = 0x001;
        /// Dim / decreased intensity.
        const DIM        = 0x002;
        /// Italic.
        const ITALIC     = 0x004;
        /// Underlined.
        const UNDERLINE  = 0x008;
        /// Slow blink.
        const BLINK      = 0x010;
        /// Reverse video (swap fg/bg).
        const REVERSE    = 0x020;
        /// Hidden / invisible.
        const HIDDEN     = 0x040;
        /// Strikethrough.
        const STRIKETHROUGH = 0x080;
        /// Double-width (CJK / emoji).
        const WIDE_CHAR  = 0x100;
        /// Continuation cell of a wide character.
        const WIDE_SPACER = 0x200;
        /// Protected (DECSCA) — immune to DECSED selective erase.
        const PROTECTED = 0x400;
        // Underline style sub-flags (SGR 4:N):
        // 0x000 = none (UNDERLINE alone = single solid), 0x800+0x1000 combo:
        const UNDERLINE_DOUBLE = 0x800;  // SGR 4:2
        const UNDERLINE_CURLY  = 0x1000; // SGR 4:3
        const UNDERLINE_DOTTED = 0x2000; // SGR 4:4
        const UNDERLINE_DASHED = 0x4000; // SGR 4:5
        /// Overline (SGR 53) — line above the character.
        const OVERLINE = 0x8000;
    }
}

/// A terminal color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    /// Default foreground/background (uses terminal theme).
    #[default]
    Default,
    /// Indexed palette color (0-15: standard, 16-231: 6x6x6 cube, 232-255: grayscale).
    Indexed(u8),
    /// 24-bit true color.
    Rgb(u8, u8, u8),
}

impl Color {
    /// Parse an SGR color parameter.
    ///
    /// Supports: `30-37` (standard fg), `40-47` (standard bg),
    /// `90-97` (bright fg), `100-107` (bright bg),
    /// `38;5;n` / `48;5;n` (256-color), `38;2;r;g;b` / `48;2;r;g;b` (truecolor).
    pub fn from_sgr(param: u16) -> Option<Self> {
        match param {
            0 => Some(Color::Default),
            // Standard foreground: 30-37 → Indexed 0-7
            30..=37 => Some(Color::Indexed((param - 30) as u8)),
            // Bright foreground: 90-97 → Indexed 8-15
            90..=97 => Some(Color::Indexed((param - 90 + 8) as u8)),
            _ => None,
        }
    }

    /// Standard 16-color ANSI palette (default colors).
    pub fn default_palette() -> [Color; 16] {
        [
            Color::Rgb(0x00, 0x00, 0x00), // 0  black
            Color::Rgb(0xcc, 0x00, 0x00), // 1  red
            Color::Rgb(0x4e, 0x9a, 0x06), // 2  green
            Color::Rgb(0xc4, 0xa0, 0x00), // 3  yellow
            Color::Rgb(0x34, 0x65, 0xa4), // 4  blue
            Color::Rgb(0x75, 0x50, 0x7b), // 5  magenta
            Color::Rgb(0x06, 0x98, 0x9a), // 6  cyan
            Color::Rgb(0xd3, 0xd7, 0xcf), // 7  white
            Color::Rgb(0x55, 0x57, 0x53), // 8  bright black
            Color::Rgb(0xef, 0x29, 0x29), // 9  bright red
            Color::Rgb(0x8a, 0xe2, 0x34), // 10 bright green
            Color::Rgb(0xfc, 0xe9, 0x4f), // 11 bright yellow
            Color::Rgb(0x73, 0x9f, 0xcf), // 12 bright blue
            Color::Rgb(0xad, 0x7f, 0xa8), // 13 bright magenta
            Color::Rgb(0x34, 0xe2, 0xe2), // 14 bright cyan
            Color::Rgb(0xee, 0xee, 0xec), // 15 bright white
        ]
    }
}

/// A single terminal cell.
///
/// Stores the character(s), foreground/background colors, and text attributes.
/// Uses `SmallVec`-style inline storage (we use a simple `char` for now;
/// wide chars use a second spacer cell).
///
/// **Note**: `Cell` is `Clone` (not `Copy`) because it carries an optional
/// hyperlink (`Option<String>`). Use `.clone()` when you need an owned copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// The character to display (0 = blank / space).
    pub ch: char,
    /// Combining characters attached to this cell (P17-B).
    /// Empty for most cells. Non-empty when zero-width marks follow the base char.
    pub combining: Vec<char>,
    /// Foreground color.
    pub fg: Color,
    /// Background color.
    pub bg: Color,
    /// Text attributes.
    pub flags: CellFlags,
    /// OSC 8 hyperlink URI (None = no hyperlink).
    pub hyperlink: Option<String>,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            combining: Vec::new(),
            fg: Color::Default,
            bg: Color::Default,
            flags: CellFlags::empty(),
            hyperlink: None,
        }
    }
}

impl Cell {
    /// Create a blank cell.
    pub fn blank() -> Self {
        Self::default()
    }

    /// Create a cell with a character and default styling.
    pub fn with_char(ch: char) -> Self {
        Self {
            ch,
            ..Self::default()
        }
    }

    /// Is this cell blank (space, no attributes)?
    pub fn is_blank(&self) -> bool {
        self.ch == ' ' && self.flags.is_empty()
    }

    /// Reset to blank, preserving nothing.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Set the foreground color.
    pub fn set_fg(&mut self, color: Color) {
        self.fg = color;
    }

    /// Set the background color.
    pub fn set_bg(&mut self, color: Color) {
        self.bg = color;
    }

    /// Returns `true` if this cell is the lead cell of a wide character.
    pub fn is_wide(&self) -> bool {
        self.flags.contains(CellFlags::WIDE_CHAR)
    }

    /// Returns `true` if this cell is a continuation (spacer) of a wide character.
    pub fn is_wide_spacer(&self) -> bool {
        self.flags.contains(CellFlags::WIDE_SPACER)
    }

    /// Set the character and update wide-char flags.
    ///
    /// If `ch` is double-width (CJK, emoji), sets `WIDE_CHAR`.
    /// Returns the display width (0, 1, or 2).
    pub fn set_char(&mut self, ch: char) -> usize {
        self.ch = ch;
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w == 2 {
            self.flags |= CellFlags::WIDE_CHAR;
        } else {
            self.flags.remove(CellFlags::WIDE_CHAR);
        }
        self.flags.remove(CellFlags::WIDE_SPACER);
        w
    }

    /// Mark this cell as a wide-character spacer (continuation cell).
    pub fn set_wide_spacer(&mut self) {
        self.ch = ' ';
        self.flags = CellFlags::WIDE_SPACER;
    }
}

/// Compute the display width of a character.
///
/// Returns 0 (zero-width combining), 1 (normal), or 2 (wide / CJK / emoji).
pub fn char_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

/// Compute the display width of a string.
pub fn str_width(s: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    s.width()
}
