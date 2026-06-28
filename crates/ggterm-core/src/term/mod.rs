//! Terminal state machine.
//!
//! The [`Terminal`] struct implements the [`Perform`] trait, receiving
//! parsed VT/ANSI sequences from the VTE parser and applying them to
//! the [`Grid`] model. It manages cursor position, text attributes,
//! terminal modes, scroll regions, and tab stops.

use crate::grid::{Cell, CellFlags, Color, Grid};
use crate::vte::Perform;

/// Terminal cursor state.
#[derive(Debug, Clone, Copy)]
pub struct Cursor {
    /// Column (0-based).
    pub x: usize,
    /// Row (0-based).
    pub y: usize,
    /// Pending wrap flag (deferred wrap for DECAWM).
    pub pending_wrap: bool,
}

impl Default for Cursor {
    fn default() -> Self {
        Self { x: 0, y: 0, pending_wrap: false }
    }
}

/// Terminal mode flags.
#[derive(Debug, Clone, Copy, Default)]
pub struct Modes {
    pub auto_wrap: bool,
    pub cursor_visible: bool,
    pub insert: bool,
    pub origin: bool,
    pub bracketed_paste: bool,
    pub cursor_keys_app: bool,
    pub alt_screen: bool,
}

impl Modes {
    fn defaults() -> Self {
        Self {
            auto_wrap: true,
            cursor_visible: true,
            insert: false,
            origin: false,
            bracketed_paste: false,
            cursor_keys_app: false,
            alt_screen: false,
        }
    }
}

/// The terminal state machine.
///
/// Connects the VTE parser to the Grid model via the [`Perform`] trait.
pub struct Terminal {
    grid: Grid,
    cursor: Cursor,
    saved_cursor: Cursor,
    modes: Modes,
    fg: Color,
    bg: Color,
    flags: CellFlags,
    tab_stops: Vec<bool>,
    title: String,
}

impl Terminal {
    /// Create a new terminal with the given dimensions.
    pub fn new(width: usize, height: usize) -> Self {
        let mut tab_stops = vec![false; width.max(1)];
        let mut col = 0;
        while col < width {
            tab_stops[col] = true;
            col += 8;
        }
        Self {
            grid: Grid::new(width, height),
            cursor: Cursor::default(),
            saved_cursor: Cursor::default(),
            modes: Modes::defaults(),
            fg: Color::Default,
            bg: Color::Default,
            flags: CellFlags::empty(),
            tab_stops,
            title: String::new(),
        }
    }

    pub fn width(&self) -> usize { self.grid.width() }
    pub fn height(&self) -> usize { self.grid.height() }
    pub fn grid(&self) -> &Grid { &self.grid }
    pub fn grid_mut(&mut self) -> &mut Grid { &mut self.grid }
    pub fn cursor(&self) -> (usize, usize) { (self.cursor.x, self.cursor.y) }
    pub fn title(&self) -> &str { &self.title }

    pub fn resize(&mut self, width: usize, height: usize) {
        self.grid.resize(width, height);
        self.tab_stops = vec![false; width.max(1)];
        let mut col = 0;
        while col < width {
            self.tab_stops[col] = true;
            col += 8;
        }
        self.cursor.x = self.cursor.x.min(width.saturating_sub(1));
        self.cursor.y = self.cursor.y.min(height.saturating_sub(1));
        self.cursor.pending_wrap = false;
    }

    // -- Helpers --

    fn make_cell(&self, ch: char) -> Cell {
        Cell { ch, fg: self.fg, bg: self.bg, flags: self.flags }
    }

    #[allow(dead_code)]
    fn advance_cursor(&mut self) {
        if self.cursor.x + 1 < self.grid.width() {
            self.cursor.x += 1;
        } else if self.modes.auto_wrap {
            self.cursor.pending_wrap = true;
        }
    }

    fn line_feed(&mut self) {
        let (_top, bottom) = self.grid.scroll_region();
        if self.cursor.y >= bottom.saturating_sub(1) {
            self.grid.scroll_up(1);
        } else {
            self.cursor.y += 1;
        }
    }

    fn reverse_line_feed(&mut self) {
        let (top, _bottom) = self.grid.scroll_region();
        if self.cursor.y == top {
            self.grid.scroll_down(1);
        } else if self.cursor.y > 0 {
            self.cursor.y -= 1;
        }
    }

    fn set_cursor(&mut self, x: usize, y: usize) {
        self.cursor.x = x.min(self.grid.width().saturating_sub(1));
        self.cursor.y = y.min(self.grid.height().saturating_sub(1));
        self.cursor.pending_wrap = false;
    }

    fn param(params: &[u16], idx: usize, default: u16) -> u16 {
        params.get(idx).copied().unwrap_or(default).max(1)
    }

    fn set_dec_mode(&mut self, mode: u16, enable: bool) {
        match mode {
            7 => self.modes.auto_wrap = enable,
            25 => self.modes.cursor_visible = enable,
            6 => { self.modes.origin = enable; self.set_cursor(0, 0); }
            1 => self.modes.cursor_keys_app = enable,
            2004 => self.modes.bracketed_paste = enable,
            47 | 1047 | 1049 => self.modes.alt_screen = enable,
            _ => {}
        }
    }

    /// Process SGR parameters.
    fn sgr(&mut self, params: &[u16]) {
        if params.is_empty() {
            self.fg = Color::Default;
            self.bg = Color::Default;
            self.flags = CellFlags::empty();
            return;
        }
        let mut i = 0;
        while i < params.len() {
            let p = params[i];
            match p {
                0 => { self.fg = Color::Default; self.bg = Color::Default; self.flags = CellFlags::empty(); }
                1 => self.flags |= CellFlags::BOLD,
                2 => self.flags |= CellFlags::DIM,
                3 => self.flags |= CellFlags::ITALIC,
                4 => self.flags |= CellFlags::UNDERLINE,
                5 => self.flags |= CellFlags::BLINK,
                7 => self.flags |= CellFlags::REVERSE,
                8 => self.flags |= CellFlags::HIDDEN,
                9 => self.flags |= CellFlags::STRIKETHROUGH,
                22 => self.flags &= !(CellFlags::BOLD | CellFlags::DIM),
                23 => self.flags &= !CellFlags::ITALIC,
                24 => self.flags &= !CellFlags::UNDERLINE,
                25 => self.flags &= !CellFlags::BLINK,
                27 => self.flags &= !CellFlags::REVERSE,
                28 => self.flags &= !CellFlags::HIDDEN,
                29 => self.flags &= !CellFlags::STRIKETHROUGH,
                30..=37 => self.fg = Color::Indexed((p - 30) as u8),
                39 => self.fg = Color::Default,
                40..=47 => self.bg = Color::Indexed((p - 40) as u8),
                49 => self.bg = Color::Default,
                90..=97 => self.fg = Color::Indexed((p - 90 + 8) as u8),
                100..=107 => self.bg = Color::Indexed((p - 100 + 8) as u8),
                38 | 48 => {
                    if i + 1 < params.len() {
                        match params[i + 1] {
                            5 => {
                                if i + 2 < params.len() {
                                    let c = Color::Indexed(params[i + 2] as u8);
                                    if p == 38 { self.fg = c; } else { self.bg = c; }
                                }
                                i += 2;
                            }
                            2 => {
                                if i + 4 < params.len() {
                                    let c = Color::Rgb(params[i+2] as u8, params[i+3] as u8, params[i+4] as u8);
                                    if p == 38 { self.fg = c; } else { self.bg = c; }
                                }
                                i += 4;
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
}

impl Perform for Terminal {
    fn print(&mut self, byte: u8) {
        let ch = if byte < 0x80 { byte as char } else { byte as char };
        // Handle deferred wrap (DECAWM) BEFORE writing
        if self.cursor.pending_wrap && self.modes.auto_wrap {
            self.cursor.x = 0;
            self.line_feed();
            self.cursor.pending_wrap = false;
        }
        if self.modes.insert {
            self.grid.insert_char(self.cursor.x, self.cursor.y, 1);
        }
        let cell = self.make_cell(ch);
        if let Some(c) = self.grid.cell_mut(self.cursor.x, self.cursor.y) {
            *c = cell;
        }
        // Advance cursor: if at right margin, set pending_wrap
        if self.cursor.x + 1 < self.grid.width() {
            self.cursor.x += 1;
        } else if self.modes.auto_wrap {
            self.cursor.pending_wrap = true;
        }
        // else: no auto-wrap, cursor stays at last column
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x07 => {} // BEL
            0x08 => { if self.cursor.x > 0 { self.cursor.x -= 1; } self.cursor.pending_wrap = false; }
            0x09 => {
                let width = self.grid.width();
                let mut next = self.cursor.x + 1;
                while next < width && !self.tab_stops.get(next).copied().unwrap_or(false) { next += 1; }
                self.cursor.x = next.min(width.saturating_sub(1));
                self.cursor.pending_wrap = false;
            }
            0x0a | 0x0b | 0x0c => { self.line_feed(); }
            0x0d => { self.cursor.x = 0; self.cursor.pending_wrap = false; }
            _ => {}
        }
    }

    fn csi(&mut self, intermediates: &[u8], params: &[u16], final_byte: u8) {
        let is_private = intermediates.contains(&b'?');
        match final_byte {
            b'A' => { let n = Self::param(params,0,1) as usize; let (top,_) = self.grid.scroll_region(); self.cursor.y = self.cursor.y.saturating_sub(n).max(top); self.cursor.pending_wrap = false; }
            b'B' => { let n = Self::param(params,0,1) as usize; let (_,bottom) = self.grid.scroll_region(); self.cursor.y = (self.cursor.y+n).min(bottom.saturating_sub(1)); self.cursor.pending_wrap = false; }
            b'C' => { let n = Self::param(params,0,1) as usize; self.cursor.x = (self.cursor.x+n).min(self.grid.width().saturating_sub(1)); self.cursor.pending_wrap = false; }
            b'D' => { let n = Self::param(params,0,1) as usize; self.cursor.x = self.cursor.x.saturating_sub(n); self.cursor.pending_wrap = false; }
            b'E' => { let n = Self::param(params,0,1) as usize; let (_,bottom) = self.grid.scroll_region(); self.cursor.y = (self.cursor.y+n).min(bottom.saturating_sub(1)); self.cursor.x = 0; self.cursor.pending_wrap = false; }
            b'F' => { let n = Self::param(params,0,1) as usize; let (top,_) = self.grid.scroll_region(); self.cursor.y = self.cursor.y.saturating_sub(n).max(top); self.cursor.x = 0; self.cursor.pending_wrap = false; }
            b'G' => { let col = Self::param(params,0,1) as usize; self.set_cursor(col.saturating_sub(1), self.cursor.y); }
            b'H' | b'f' => { let row = Self::param(params,0,1) as usize; let col = Self::param(params,1,1) as usize; self.set_cursor(col.saturating_sub(1), row.saturating_sub(1)); }
            b'd' => { let row = Self::param(params,0,1) as usize; self.set_cursor(self.cursor.x, row.saturating_sub(1)); }
            b'J' => {
                let mode = params.first().copied().unwrap_or(0);
                match mode {
                    0 => { self.grid.clear_line_from(self.cursor.x, self.cursor.y); for r in (self.cursor.y+1)..self.grid.height() { self.grid.clear_line(r); } }
                    1 => { for r in 0..self.cursor.y { self.grid.clear_line(r); } self.grid.clear_line_to(self.cursor.x+1, self.cursor.y); }
                    2 | 3 => { self.grid.clear(); }
                    _ => {}
                }
            }
            b'K' => {
                let mode = params.first().copied().unwrap_or(0);
                match mode {
                    0 => self.grid.clear_line_from(self.cursor.x, self.cursor.y),
                    1 => self.grid.clear_line_to(self.cursor.x+1, self.cursor.y),
                    2 => self.grid.clear_line(self.cursor.y),
                    _ => {}
                }
            }
            b'S' => { let n = Self::param(params,0,1) as usize; self.grid.scroll_up(n); }
            b'T' => { let n = Self::param(params,0,1) as usize; self.grid.scroll_down(n); }
            b'r' if !is_private => {
                let top = Self::param(params,0,1) as usize;
                let bottom = params.get(1).copied().unwrap_or(self.grid.height() as u16).max(1) as usize;
                if top < bottom && bottom <= self.grid.height() {
                    self.grid.set_scroll_region(top.saturating_sub(1), bottom);
                    self.set_cursor(0, 0);
                }
            }
            b'm' => self.sgr(params),
            b'L' => { self.grid.insert_line(self.cursor.y, Self::param(params,0,1) as usize); }
            b'M' => { self.grid.delete_line(self.cursor.y, Self::param(params,0,1) as usize); }
            b'P' => { self.grid.delete_char(self.cursor.x, self.cursor.y, Self::param(params,0,1) as usize); }
            b'@' => { self.grid.insert_char(self.cursor.x, self.cursor.y, Self::param(params,0,1) as usize); }
            b'X' => { self.grid.erase_char(self.cursor.x, self.cursor.y, Self::param(params,0,1) as usize); }
            b'I' => { let n = Self::param(params,0,1); for _ in 0..n { self.execute(0x09); } }
            b'Z' => { let n = Self::param(params,0,1); for _ in 0..n { if self.cursor.x > 0 { let mut p = self.cursor.x-1; while p > 0 && !self.tab_stops.get(p).copied().unwrap_or(false) { p -= 1; } self.cursor.x = p; } } }
            b'g' => { let m = params.first().copied().unwrap_or(0); match m { 0 => { if self.cursor.x < self.tab_stops.len() { self.tab_stops[self.cursor.x] = false; } } 3 => { for s in &mut self.tab_stops { *s = false; } } _ => {} } }
            b'h' if is_private => { self.set_dec_mode(params.first().copied().unwrap_or(0), true); }
            b'l' if is_private => { self.set_dec_mode(params.first().copied().unwrap_or(0), false); }
            b'h' => { let m = params.first().copied().unwrap_or(0); if m == 4 { self.modes.insert = true; } }
            b'l' => { let m = params.first().copied().unwrap_or(0); if m == 4 { self.modes.insert = false; } }
            _ => {}
        }
    }

    fn esc(&mut self, _intermediates: &[u8], final_byte: u8) {
        match final_byte {
            b'7' => self.saved_cursor = self.cursor,
            b'8' => self.cursor = self.saved_cursor,
            b'c' => { let w = self.grid.width(); let h = self.grid.height(); *self = Terminal::new(w, h); }
            b'D' => self.line_feed(),
            b'E' => { self.cursor.x = 0; self.line_feed(); self.cursor.pending_wrap = false; }
            b'M' => self.reverse_line_feed(),
            b'H' => { if self.cursor.x < self.tab_stops.len() { self.tab_stops[self.cursor.x] = true; } }
            _ => {}
        }
    }

    fn osc(&mut self, data: &[u8]) {
        let s = String::from_utf8_lossy(data);
        let mut parts = s.splitn(2, ';');
        let cmd = parts.next().and_then(|s| s.parse::<u16>().ok());
        match cmd {
            Some(0) | Some(2) => { self.title = parts.next().unwrap_or("").to_string(); }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vte::Parser;

    fn feed(term: &mut Terminal, data: &[u8]) {
        let mut p = Parser::new();
        p.feed(data, term);
    }

    #[test]
    fn t_print_basic() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"Hi");
        assert_eq!(t.grid().cell(0,0).unwrap().ch, 'H');
        assert_eq!(t.grid().cell(1,0).unwrap().ch, 'i');
        assert_eq!(t.cursor(), (2, 0));
    }

    #[test]
    fn t_auto_wrap() {
        let mut t = Terminal::new(4, 4);
        feed(&mut t, b"ABCDE");
        assert_eq!(t.grid().cell(0,0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(3,0).unwrap().ch, 'D');
        assert_eq!(t.grid().cell(0,1).unwrap().ch, 'E');
    }

    #[test]
    fn t_cr_lf() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"AB\r\nCD");
        assert_eq!(t.grid().cell(0,0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(0,1).unwrap().ch, 'C');
        assert_eq!(t.grid().cell(1,1).unwrap().ch, 'D');
    }

    #[test]
    fn t_tab() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 8);
    }

    #[test]
    fn t_backspace() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"ABC\x08");
        assert_eq!(t.cursor().0, 2);
    }

    #[test]
    fn t_csi_cup() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[10;20H");
        assert_eq!(t.cursor(), (19, 9));
    }

    #[test]
    fn t_csi_cuu_cud() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5;1H\x1b[3A");
        assert_eq!(t.cursor().1, 1);
        feed(&mut t, b"\x1b[1B");
        assert_eq!(t.cursor().1, 2);
    }

    #[test]
    fn t_csi_cuf_cub() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[10C\x1b[3D");
        assert_eq!(t.cursor().0, 7);
    }

    #[test]
    fn t_csi_cha() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[30G");
        assert_eq!(t.cursor().0, 29);
    }

    #[test]
    fn t_ed_clear_all() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"Hello\x1b[2J");
        assert_eq!(t.grid().cell(0,0).unwrap().ch, ' ');
    }

    #[test]
    fn t_ed_clear_to_end() {
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"ABC\x1b[0J");
        assert_eq!(t.grid().cell(0,0).unwrap().ch, 'A');
    }

    #[test]
    fn t_el_clear_line() {
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"Hello\x1b[2K");
        assert_eq!(t.grid().cell(0,0).unwrap().ch, ' ');
    }

    #[test]
    fn t_el_clear_to_end() {
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"Hello\x1b[0K");
        assert_eq!(t.grid().cell(0,0).unwrap().ch, 'H');
        assert_eq!(t.grid().cell(5,0).unwrap().ch, ' ');
    }

    #[test]
    fn t_sgr_bold() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[1mX");
        assert!(t.grid().cell(0,0).unwrap().flags.contains(CellFlags::BOLD));
    }

    #[test]
    fn t_sgr_underline() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[4mU");
        assert!(t.grid().cell(0,0).unwrap().flags.contains(CellFlags::UNDERLINE));
    }

    #[test]
    fn t_sgr_color_fg() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[31mR");
        assert_eq!(t.grid().cell(0,0).unwrap().fg, Color::Indexed(1));
    }

    #[test]
    fn t_sgr_color_bg() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[42mG");
        assert_eq!(t.grid().cell(0,0).unwrap().bg, Color::Indexed(2));
    }

    #[test]
    fn t_sgr_bright_color() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[91mR");
        assert_eq!(t.grid().cell(0,0).unwrap().fg, Color::Indexed(9));
    }

    #[test]
    fn t_sgr_truecolor() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[38;2;255;128;0mX");
        assert_eq!(t.grid().cell(0,0).unwrap().fg, Color::Rgb(255,128,0));
    }

    #[test]
    fn t_sgr_256color() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[38;5;200mX");
        assert_eq!(t.grid().cell(0,0).unwrap().fg, Color::Indexed(200));
    }

    #[test]
    fn t_sgr_reset() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[1;31mA\x1b[0mB");
        assert!(t.grid().cell(0,0).unwrap().flags.contains(CellFlags::BOLD));
        assert!(!t.grid().cell(1,0).unwrap().flags.contains(CellFlags::BOLD));
        assert_eq!(t.grid().cell(1,0).unwrap().fg, Color::Default);
    }

    #[test]
    fn t_sgr_multi_attr() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[1;3;4mX");
        let c = t.grid().cell(0,0).unwrap();
        assert!(c.flags.contains(CellFlags::BOLD));
        assert!(c.flags.contains(CellFlags::ITALIC));
        assert!(c.flags.contains(CellFlags::UNDERLINE));
    }

    #[test]
    fn t_scroll_at_bottom() {
        let mut t = Terminal::new(10, 3);
        // Use CUP to fill each row at column 0, then scroll by going past the bottom
        feed(&mut t, b"\x1b[1;1HR1\x1b[2;1HR2\x1b[3;1HR3\r\nR4");
        // After R3 on row 3 (0-indexed=2, the last row), \r\n triggers scroll_up
        assert_eq!(t.grid().scrollback_len(), 1);
        // After scroll, row 0 has old row 1 content (R2)
        assert_eq!(t.grid().cell(0,0).unwrap().ch, 'R');
    }

    #[test]
    fn t_csi_su() {
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"\x1b[2S");
        assert_eq!(t.grid().scrollback_len(), 2);
    }

    #[test]
    fn t_dec_show_hide_cursor() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?25l");
        assert!(!t.modes.cursor_visible);
        feed(&mut t, b"\x1b[?25h");
        assert!(t.modes.cursor_visible);
    }

    #[test]
    fn t_dec_bracketed_paste() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?2004h");
        assert!(t.modes.bracketed_paste);
        feed(&mut t, b"\x1b[?2004l");
        assert!(!t.modes.bracketed_paste);
    }

    #[test]
    fn t_dec_alt_screen() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?1049h");
        assert!(t.modes.alt_screen);
        feed(&mut t, b"\x1b[?1049l");
        assert!(!t.modes.alt_screen);
    }

    #[test]
    fn t_esc_save_restore_cursor() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5;10H\x1b7\x1b[1;1H\x1b8");
        assert_eq!(t.cursor(), (9, 4));
    }

    #[test]
    fn t_esc_ris_reset() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[31mHello\x1bc");
        assert_eq!(t.grid().cell(0,0).unwrap().ch, ' ');
        assert_eq!(t.cursor(), (0, 0));
    }

    #[test]
    fn t_esc_ri_reverse_index() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[1;1H\x1bM");
        assert_eq!(t.cursor().1, 0);
    }

    #[test]
    fn t_osc_title() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]0;My Title\x07");
        assert_eq!(t.title(), "My Title");
    }

    #[test]
    fn t_osc_title_st_terminated() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]2;Title2\x1b\\");
        assert_eq!(t.title(), "Title2");
    }

    #[test]
    fn t_decstbm() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5;20r");
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(top, 4);
        assert_eq!(bottom, 20);
        assert_eq!(t.cursor(), (0, 0));
    }

    #[test]
    fn t_insert_line() {
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"A\nB\nC\x1b[1;1H\x1b[L");
        assert_eq!(t.grid().cell(0,0).unwrap().ch, ' ');
        assert_eq!(t.grid().cell(0,1).unwrap().ch, 'A');
    }

    #[test]
    fn t_delete_line() {
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"\x1b[1;1HA\x1b[2;1HB\x1b[3;1HC\x1b[4;1HD\x1b[1;1H\x1b[M");
        assert_eq!(t.grid().cell(0,0).unwrap().ch, 'B');
    }

    #[test]
    fn t_insert_char() {
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"ABC\x1b[1G\x1b[@");
        assert_eq!(t.grid().cell(0,0).unwrap().ch, ' ');
        assert_eq!(t.grid().cell(1,0).unwrap().ch, 'A');
    }

    #[test]
    fn t_delete_char() {
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"ABC\x1b[1G\x1b[P");
        assert_eq!(t.grid().cell(0,0).unwrap().ch, 'B');
    }

    #[test]
    fn t_erase_char() {
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"ABC\x1b[1G\x1b[X");
        assert_eq!(t.grid().cell(0,0).unwrap().ch, ' ');
        assert_eq!(t.grid().cell(1,0).unwrap().ch, 'B');
    }

    #[test]
    fn t_irm_insert_mode() {
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"\x1b[4hAB");
        // In insert mode, each char pushes existing chars right.
        assert_eq!(t.grid().cell(0,0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1,0).unwrap().ch, 'B');
    }

    #[test]
    fn t_resize() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[50;50H");
        t.resize(40, 10);
        assert_eq!(t.width(), 40);
        assert_eq!(t.height(), 10);
        assert!(t.cursor().0 < 40);
        assert!(t.cursor().1 < 10);
    }

    #[test]
    fn t_decom_origin_mode() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5;20r\x1b[?6h");
        assert!(t.modes.origin);
        assert_eq!(t.cursor(), (0, 0));
    }

    #[test]
    fn t_cnl_cpl() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5;5H\x1b[2E");
        assert_eq!(t.cursor().1, 6);
        assert_eq!(t.cursor().0, 0);
        feed(&mut t, b"\x1b[2F");
        assert_eq!(t.cursor().1, 4);
    }

    #[test]
    fn t_vpa() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[10d");
        assert_eq!(t.cursor().1, 9);
    }

    #[test]
    fn t_complex_seq() {
        // Clear, home, set color, print text, reset, newline.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[2J\x1b[1;1H\x1b[32mHello\x1b[0m\r\nWorld");
        assert_eq!(t.grid().cell(0,0).unwrap().ch, 'H');
        assert_eq!(t.grid().cell(0,0).unwrap().fg, Color::Indexed(2));
        assert_eq!(t.grid().cell(0,1).unwrap().ch, 'W');
        assert_eq!(t.grid().cell(0,1).unwrap().fg, Color::Default);
    }

    #[test]
    fn t_tab_clear() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[3g"); // Clear all tabs
        feed(&mut t, b"\t");     // Tab now does nothing (no stops)
        assert_eq!(t.cursor().0, 79); // Moved to end (no tab stop found)
    }

    #[test]
    fn t_hts_set_tab() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5G\x1bH"); // Move to col 5, set tab stop
        feed(&mut t, b"\x1b[1G\t");    // Home, tab → should hit col 5
        assert_eq!(t.cursor().0, 4);
    }

    #[test]
    fn t_concurrent_feed() {
        // Simulate concurrent feeding from different chunks.
        let mut t = Terminal::new(80, 24);
        let mut p = Parser::new();
        p.feed(b"\x1b[31", &mut t);
        p.feed(b"mRed", &mut t);
        assert_eq!(t.grid().cell(0,0).unwrap().fg, Color::Indexed(1));
        assert_eq!(t.grid().cell(0,0).unwrap().ch, 'R');
    }
}
