//! Terminal state machine.
//!
//! The [`Terminal`] struct implements the [`Perform`] trait, receiving
//! parsed VT/ANSI sequences from the VTE parser and applying them to
//! the [`Grid`] model. It manages cursor position, text attributes,
//! terminal modes, scroll regions, and tab stops.

use crate::grid::{Cell, CellFlags, Color, Grid};
use crate::vte::Perform;
use unicode_width::UnicodeWidthChar;

/// Terminal cursor state.
#[derive(Debug, Clone, Copy, Default)]
pub struct Cursor {
    /// Column (0-based).
    pub x: usize,
    /// Row (0-based).
    pub y: usize,
    /// Pending wrap flag (deferred wrap for DECAWM).
    pub pending_wrap: bool,
}

/// OSC 133 command mark kind (Shell Integration protocol).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandMarkKind {
    /// `OSC 133;A` — prompt start.
    PromptStart,
    /// `OSC 133;B` — command start (user typed Enter).
    CommandStart,
    /// `OSC 133;C` — output start (command begins producing output).
    OutputStart,
    /// `OSC 133;D[;exitcode]` — command end.
    CommandEnd,
}

/// A single OSC 133 mark emitted by the shell integration protocol.
/// A single OSC 133 mark emitted by the shell integration protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct CommandMark {
    /// What kind of mark this is.
    pub kind: CommandMarkKind,
    /// Row at which the mark was emitted (cursor Y).
    pub row: usize,
    /// Exit code, only meaningful for `CommandEnd` marks.
    pub exit_code: Option<i32>,
}

/// A grouped command block assembled from OSC 133 marks.
///
/// Represents the full lifecycle of a single command: prompt -> command -> output -> end.
/// Incomplete blocks (command still running) have `end_row = None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandBlock {
    /// Row where the prompt started (PromptStart / `OSC 133;A`).
    pub prompt_row: usize,
    /// Row where the command text was entered (CommandStart / `OSC 133;B`).
    /// `None` if only PromptStart has been seen (user is still at the prompt).
    pub command_row: Option<usize>,
    /// Row where command output began (OutputStart / `OSC 133;C`).
    /// `None` if the mark hasn't arrived yet or command produced no output.
    pub output_row: Option<usize>,
    /// Row where the command ended (CommandEnd / `OSC 133;D`).
    /// `None` means the command is still running.
    pub end_row: Option<usize>,
    /// Exit code from CommandEnd mark. `None` if command is still running
    /// or the mark didn't include an exit code.
    pub exit_code: Option<i32>,
}

impl CommandBlock {
    /// Returns true if the command completed successfully (exit code 0).
    pub fn is_success(&self) -> bool {
        self.exit_code == Some(0)
    }

    /// Returns true if the command failed (exit code non-zero).
    pub fn is_failure(&self) -> bool {
        matches!(self.exit_code, Some(code) if code != 0)
    }

    /// Returns true if the command is still running (no CommandEnd mark).
    pub fn is_running(&self) -> bool {
        self.command_row.is_some() && self.end_row.is_none()
    }

    /// Returns true if the user is at the prompt (no CommandStart yet).
    pub fn is_at_prompt(&self) -> bool {
        self.command_row.is_none()
    }

    /// Returns true if the command has completed (CommandEnd mark received).
    pub fn is_complete(&self) -> bool {
        self.end_row.is_some()
    }
}

/// Group a flat list of CommandMark entries into CommandBlocks.
///
/// Each PromptStart (A) mark starts a new block. Subsequent marks
/// (B, C, D) are attached to the current block until the next A mark.
pub fn group_command_blocks(marks: &[CommandMark]) -> Vec<CommandBlock> {
    let mut blocks = Vec::new();
    let mut current: Option<CommandBlock> = None;

    for mark in marks {
        match mark.kind {
            CommandMarkKind::PromptStart => {
                if let Some(b) = current.take() {
                    blocks.push(b);
                }
                current = Some(CommandBlock {
                    prompt_row: mark.row,
                    command_row: None,
                    output_row: None,
                    end_row: None,
                    exit_code: None,
                });
            }
            CommandMarkKind::CommandStart => {
                if current.is_none() {
                    current = Some(CommandBlock {
                        prompt_row: mark.row,
                        command_row: None,
                        output_row: None,
                        end_row: None,
                        exit_code: None,
                    });
                }
                if let Some(ref mut b) = current {
                    b.command_row = Some(mark.row);
                }
            }
            CommandMarkKind::OutputStart => {
                if current.is_none() {
                    current = Some(CommandBlock {
                        prompt_row: mark.row,
                        command_row: None,
                        output_row: None,
                        end_row: None,
                        exit_code: None,
                    });
                }
                if let Some(ref mut b) = current {
                    b.output_row = Some(mark.row);
                }
            }
            CommandMarkKind::CommandEnd => {
                if current.is_none() {
                    current = Some(CommandBlock {
                        prompt_row: mark.row,
                        command_row: None,
                        output_row: None,
                        end_row: None,
                        exit_code: None,
                    });
                }
                if let Some(ref mut b) = current {
                    b.end_row = Some(mark.row);
                    b.exit_code = mark.exit_code;
                }
                blocks.push(current.take().unwrap());
            }
        }
    }

    if let Some(b) = current.take() {
        blocks.push(b);
    }

    blocks
}

/// Character set designation (G0 or G1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Charset {
    /// US ASCII (default).
    #[default]
    Ascii,
    /// DEC Special Graphics (line drawing, block elements).
    DecSpecial,
}

impl Charset {
    /// Translate a character according to the active character set.
    pub fn translate(self, ch: char) -> char {
        match self {
            Charset::Ascii => ch,
            Charset::DecSpecial => {
                let b = ch as u32;
                if (0x5f..=0x7e).contains(&b) {
                    DEC_SPECIAL_GRAPHICS[(b - 0x5f) as usize]
                } else {
                    ch
                }
            }
        }
    }
}

/// DEC Special Graphics mapping for 0x5F-0x7E → Unicode.
static DEC_SPECIAL_GRAPHICS: [char; 32] = [
    '\u{00a0}', // 0x5F '_'
    '\u{25c6}', // 0x60 '`' diamond
    '\u{2592}', // 0x61 'a' medium shade
    '\u{2409}', // 0x62 'b' HT
    '\u{240c}', // 0x63 'c' FF
    '\u{240d}', // 0x64 'd' CR
    '\u{240a}', // 0x65 'e' LF
    '\u{00b0}', // 0x66 'f' degree
    '\u{00b1}', // 0x67 'g' plus-minus
    '\u{2424}', // 0x68 'h' NL
    '\u{240b}', // 0x69 'i' VT
    '\u{2518}', // 0x6A 'j' ┘
    '\u{2510}', // 0x6B 'k' ┐
    '\u{250c}', // 0x6C 'l' ┌
    '\u{2514}', // 0x6D 'm' └
    '\u{253c}', // 0x6E 'n' ┼
    '\u{239e}', // 0x6F 'o'
    '\u{239e}', // 0x70 'p'
    '\u{2500}', // 0x71 'q' ─
    '\u{23a0}', // 0x72 'r'
    '\u{23a2}', // 0x73 's'
    '\u{251c}', // 0x74 't' ├
    '\u{2524}', // 0x75 'u' ┤
    '\u{2534}', // 0x76 'v' ┴
    '\u{252c}', // 0x77 'w' ┬
    '\u{2502}', // 0x78 'x' │
    '\u{2264}', // 0x79 'y' ≤
    '\u{2265}', // 0x7A 'z' ≥
    '\u{03c0}', // 0x7B '{' π
    '\u{2260}', // 0x7C '|' ≠
    '\u{00a3}', // 0x7D '}' £
    '\u{00b7}', // 0x7E '~' ·
];

/// Cursor shape (DECSCUSR / `CSI Ps SP q`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorStyle {
    #[default]
    Default,
    BlinkBlock,
    SteadyBlock,
    BlinkUnderline,
    SteadyUnderline,
    BlinkBar,
    SteadyBar,
}

/// Terminal mode flags toggled by SM/RM (`CSI ? Pn h/l`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Modes {
    /// DECAWM — auto-wrap / line feed on right margin (default true).
    pub auto_wrap: bool,
    /// DECTCEM — text cursor enable (visibility).
    pub cursor_visible: bool,
    /// DECOM — origin mode.
    pub origin: bool,
    /// DECCKM — cursor keys application mode.
    pub cursor_keys_app: bool,
    /// Bracketed paste mode (mode 2004).
    pub bracketed_paste: bool,
    /// Alternate screen buffer active (modes 47/1047/1049).
    pub alt_screen: bool,
    /// Insert mode (IRM, SM/RM 4).
    pub insert: bool,
    /// Mouse tracking — X10 / normal mode (DECSET 1000).
    pub mouse_tracking: bool,
    /// Mouse tracking — button-event mode (DECSET 1002).
    pub mouse_button_event: bool,
    /// Mouse tracking — any-motion mode (DECSET 1003).
    pub mouse_any_event: bool,
    /// SGR mouse formatting (DECSET 1006).
    pub mouse_sgr: bool,
    /// UTF-8 mouse formatting (DECSET 1005).
    pub mouse_utf8: bool,
    /// URXVT mouse formatting (DECSET 1015).
    pub mouse_urxvt: bool,
}

impl Modes {
    /// Return the default mode set (auto_wrap + cursor_visible enabled).
    pub fn defaults() -> Self {
        Self {
            auto_wrap: true,
            cursor_visible: true,
            origin: false,
            cursor_keys_app: false,
            bracketed_paste: false,
            alt_screen: false,
            insert: false,
            mouse_tracking: false,
            mouse_button_event: false,
            mouse_any_event: false,
            mouse_sgr: false,
            mouse_utf8: false,
            mouse_urxvt: false,
        }
    }
}

/// The main terminal state machine.
///
/// Owns the grid, cursor, current SGR attributes, mode flags, tab stops,
/// OSC 133 command marks, and character set state.
pub struct Terminal {
    /// Primary (and alternate) screen grid.
    pub(crate) grid: Grid,
    /// Active cursor position and pending-wrap flag.
    pub(crate) cursor: Cursor,
    /// Saved cursor (for DECSC/DECRC and alt-screen swap).
    pub(crate) saved_cursor: Cursor,
    /// Terminal mode flags.
    pub(crate) modes: Modes,
    /// Current foreground colour.
    pub(crate) fg: Color,
    /// Current background colour.
    pub(crate) bg: Color,
    /// Current cell flags (bold, italic, underline, ...).
    pub(crate) flags: CellFlags,
    /// Tab stop positions (one bool per column).
    pub(crate) tab_stops: Vec<bool>,
    /// OSC 133 command marks accumulated from shell integration.
    pub(crate) command_marks: Vec<CommandMark>,
    /// Terminal title (set via OSC 0/2).
    pub(crate) title: String,
    /// UTF-8 reassembly buffer for multi-byte sequences.
    pub(crate) utf8_buf: Vec<u8>,
    /// G0 character set designation.
    pub(crate) g0_charset: Charset,
    /// G1 character set designation.
    pub(crate) g1_charset: Charset,
    /// True when G1 is active (via SO/0x0E); false means G0 active (SI/0x0F).
    pub(crate) active_g1: bool,
    /// Last printed character (for REP / `CSI Ps b`).
    pub(crate) last_printed_char: Option<char>,
    /// Cursor style (DECSCUSR).
    pub(crate) cursor_style: CursorStyle,
    /// Device response buffer (DA/DSR replies).
    pub(crate) response_buffer: Vec<u8>,
    /// Pending OSC 52 clipboard set request (base64-decoded bytes).
    /// The app layer reads this and writes to the system clipboard.
    pub(crate) pending_clipboard_set: Option<Vec<u8>>,
    /// Bell flag — set when BEL (0x07) is received (P11-E).
    pub(crate) bell: bool,
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
            command_marks: Vec::new(),
            title: String::new(),
            utf8_buf: Vec::with_capacity(4),
            g0_charset: Charset::default(),
            g1_charset: Charset::default(),
            active_g1: false,
            last_printed_char: None,
            cursor_style: CursorStyle::default(),
            response_buffer: Vec::new(),
            pending_clipboard_set: None,
            bell: false,
        }
    }

    pub fn width(&self) -> usize {
        self.grid.width()
    }
    pub fn height(&self) -> usize {
        self.grid.height()
    }
    pub fn grid(&self) -> &Grid {
        &self.grid
    }
    pub fn grid_mut(&mut self) -> &mut Grid {
        &mut self.grid
    }
    pub fn cursor(&self) -> (usize, usize) {
        (self.cursor.x, self.cursor.y)
    }
    pub fn cursor_visible(&self) -> bool {
        self.modes.cursor_visible
    }

    /// Return true if any mouse tracking mode is active.
    pub fn mouse_tracking_enabled(&self) -> bool {
        self.modes.mouse_tracking || self.modes.mouse_button_event || self.modes.mouse_any_event
    }

    /// Return true if SGR mouse encoding is active (DECSET 1006).
    pub fn mouse_sgr_enabled(&self) -> bool {
        self.modes.mouse_sgr
    }

    /// Return true if URXVT mouse encoding is active (DECSET 1015).
    pub fn mouse_urxvt_enabled(&self) -> bool {
        self.modes.mouse_urxvt
    }

    /// Return true if any-event mouse tracking is active (DECSET 1003).
    pub fn mouse_any_event_enabled(&self) -> bool {
        self.modes.mouse_any_event
    }

    /// Return true if button-event mouse tracking is active (DECSET 1002).
    pub fn mouse_button_event_enabled(&self) -> bool {
        self.modes.mouse_button_event
    }

    /// Return true if bracketed paste mode is active (DECSET 2004).
    pub fn bracketed_paste(&self) -> bool {
        self.modes.bracketed_paste
    }

    pub fn cursor_style(&self) -> CursorStyle {
        self.cursor_style
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// Return the device response buffer (DA/DSR replies).
    pub fn response_buffer(&self) -> &[u8] {
        &self.response_buffer
    }

    /// Take the device response buffer, clearing it.
    pub fn take_response(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.response_buffer)
    }

    /// Return the active G0 character set.
    pub fn g0_charset(&self) -> Charset {
        self.g0_charset
    }
    /// Return the active G1 character set.
    pub fn g1_charset(&self) -> Charset {
        self.g1_charset
    }
    /// Return true if G1 is the currently active charset (via SO/ShiftOut).
    pub fn active_g1(&self) -> bool {
        self.active_g1
    }

    /// Return all OSC 133 command marks collected so far.
    pub fn command_marks(&self) -> &[CommandMark] {
        &self.command_marks
    }

    /// Return command marks grouped into logical command blocks.
    ///
    /// Each block represents a complete command lifecycle:
    /// PromptStart → CommandStart → OutputStart → CommandEnd.
    /// The final block may be incomplete (still running).
    pub fn command_blocks(&self) -> Vec<CommandBlock> {
        group_command_blocks(&self.command_marks)
    }

    /// Return the exit code of the most recent completed command.
    ///
    /// Returns `None` if no commands have completed yet.
    pub fn last_exit_code(&self) -> Option<i32> {
        self.command_marks
            .iter()
            .rev()
            .find(|m| m.kind == CommandMarkKind::CommandEnd)
            .and_then(|m| m.exit_code)
    }

    /// Return true if the most recent completed command succeeded (exit code 0).
    pub fn last_command_succeeded(&self) -> bool {
        self.last_exit_code() == Some(0)
    }

    /// Extract the text content of a grid row, trimming trailing spaces.
    ///
    /// Returns an empty string if the row is out of bounds.
    pub fn extract_row_text(&self, row: usize) -> String {
        let mut text = String::new();
        let width = self.grid.width();
        for x in 0..width {
            match self.grid.cell(x, row) {
                Some(cell) => {
                    if cell.flags.contains(CellFlags::WIDE_SPACER) {
                        continue;
                    }
                    text.push(cell.ch);
                }
                None => break,
            }
        }
        text.trim_end().to_string()
    }

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
        self.utf8_buf.clear();
    }

    // -- Helpers --

    #[allow(dead_code)]
    fn make_cell(&self, ch: char) -> Cell {
        Cell {
            ch,
            fg: self.fg,
            bg: self.bg,
            flags: self.flags,
        }
    }

    /// Flush the UTF-8 byte buffer: decode and write the reassembled character.
    fn flush_utf8(&mut self) {
        if self.utf8_buf.is_empty() {
            return;
        }
        match std::str::from_utf8(&self.utf8_buf) {
            Ok(s) => {
                if let Some(ch) = s.chars().next() {
                    self.put_printable_char(ch);
                }
            }
            Err(_) => {
                // Invalid UTF-8 sequence — emit replacement character
                self.put_printable_char('\u{FFFD}');
            }
        }
        self.utf8_buf.clear();
    }

    /// Write a decoded character to the grid with proper column advancement.
    ///
    /// Handles deferred wrap (DECAWM), insert mode (IRM), wide character
    /// boundary wrapping, zero-width skip, and attribute merging.
    fn put_printable_char(&mut self, ch: char) {
        let w = UnicodeWidthChar::width(ch).unwrap_or(1);

        // Skip zero-width combining characters (our Cell model can't represent them)
        if w == 0 {
            return;
        }

        // Track for REP (CSI Ps b)
        self.last_printed_char = Some(ch);

        // Handle deferred wrap (DECAWM) before writing
        if self.cursor.pending_wrap && self.modes.auto_wrap {
            self.cursor.x = 0;
            self.line_feed();
            self.cursor.pending_wrap = false;
        }

        let grid_width = self.grid.width();
        if grid_width == 0 {
            return;
        }

        // For wide chars (width 2), wrap to next line if not enough columns remain
        if w == 2 && self.cursor.x + 1 >= grid_width && self.modes.auto_wrap {
            self.cursor.x = 0;
            self.line_feed();
            self.cursor.pending_wrap = false;
        }

        // Insert mode: shift existing cells right to make room
        if self.modes.insert {
            self.grid.insert_char(self.cursor.x, self.cursor.y, w);
        }

        // Apply character set translation for ASCII range
        let ch = if ch.is_ascii() {
            let cs = if self.active_g1 {
                self.g1_charset
            } else {
                self.g0_charset
            };
            cs.translate(ch)
        } else {
            ch
        };

        // Write the character (grid.put_char handles wide char + spacer mechanics)
        let consumed = self.grid.put_char(self.cursor.x, self.cursor.y, ch);

        // Apply current text attributes — merge with flags set by put_char (e.g., WIDE_CHAR)
        if let Some(c) = self.grid.cell_mut(self.cursor.x, self.cursor.y) {
            c.fg = self.fg;
            c.bg = self.bg;
            c.flags |= self.flags;
        }
        // For wide chars, set bg on the spacer cell to avoid visual gaps
        if consumed == 2
            && self.cursor.x + 1 < grid_width
            && let Some(c) = self.grid.cell_mut(self.cursor.x + 1, self.cursor.y)
        {
            c.bg = self.bg;
        }

        // Advance cursor by the character's display width
        let advance = if consumed > 0 { consumed } else { w };
        if self.cursor.x + advance < grid_width {
            self.cursor.x += advance;
        } else if self.modes.auto_wrap {
            self.cursor.x = grid_width.saturating_sub(1);
            self.cursor.pending_wrap = true;
        }
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
            6 => {
                self.modes.origin = enable;
                self.set_cursor(0, 0);
            }
            1 => self.modes.cursor_keys_app = enable,
            2004 => self.modes.bracketed_paste = enable,
            47 | 1047 | 1049 => self.modes.alt_screen = enable,
            // Mouse tracking modes
            9 => self.modes.mouse_tracking = enable, // X10
            1000 => self.modes.mouse_tracking = enable, // Normal
            1002 => self.modes.mouse_button_event = enable, // Button-event
            1003 => self.modes.mouse_any_event = enable, // Any-motion
            1005 => self.modes.mouse_utf8 = enable,  // UTF-8 encoding
            1006 => self.modes.mouse_sgr = enable,   // SGR encoding
            1015 => self.modes.mouse_urxvt = enable, // URXVT encoding
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
                0 => {
                    self.fg = Color::Default;
                    self.bg = Color::Default;
                    self.flags = CellFlags::empty();
                }
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
                                    if p == 38 {
                                        self.fg = c;
                                    } else {
                                        self.bg = c;
                                    }
                                }
                                i += 2;
                            }
                            2 => {
                                if i + 4 < params.len() {
                                    let c = Color::Rgb(
                                        params[i + 2] as u8,
                                        params[i + 3] as u8,
                                        params[i + 4] as u8,
                                    );
                                    if p == 38 {
                                        self.fg = c;
                                    } else {
                                        self.bg = c;
                                    }
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

    /// Take the pending OSC 52 clipboard set data, if any.
    ///
    /// Called by the app layer to apply the clipboard change
    /// to the system clipboard.
    pub fn take_pending_clipboard_set(&mut self) -> Option<Vec<u8>> {
        self.pending_clipboard_set.take()
    }

    /// Take the bell flag (P11-E).
    ///
    /// Returns `true` if a BEL (0x07) was received since the last call.
    /// The app layer calls this in `about_to_wait` to trigger visual bell.
    pub fn take_bell(&mut self) -> bool {
        std::mem::replace(&mut self.bell, false)
    }

    /// Simple base64 decoder for OSC 52 payloads.
    fn decode_base64(input: &str) -> Option<Vec<u8>> {
        let bytes = input.as_bytes();
        let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
        let mut buf: u32 = 0;
        let mut bits = 0;
        for &b in bytes {
            let val = match b {
                b'A'..=b'Z' => b - b'A',
                b'a'..=b'z' => b - b'a' + 26,
                b'0'..=b'9' => b - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' => break,
                _ => continue,
            };
            buf = (buf << 6) | val as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buf >> bits) as u8);
                buf &= (1 << bits) - 1;
            }
        }
        Some(out)
    }
}

/// Determine the expected length of a UTF-8 sequence from its leading byte.
fn utf8_expected_len(lead: u8) -> usize {
    if lead & 0x80 == 0 {
        1
    }
    // 0xxxxxxx
    else if lead & 0xe0 == 0xc0 {
        2
    }
    // 110xxxxx
    else if lead & 0xf0 == 0xe0 {
        3
    }
    // 1110xxxx
    else if lead & 0xf8 == 0xf0 {
        4
    }
    // 11110xxx
    else {
        1
    } // invalid leading byte
}

impl Perform for Terminal {
    fn print(&mut self, byte: u8) {
        // ASCII: flush any pending UTF-8 buffer, then write directly
        if byte < 0x80 {
            self.flush_utf8();
            self.put_printable_char(byte as char);
            return;
        }
        // Flush pending incomplete sequence when a new leading byte arrives
        if !self.utf8_buf.is_empty() && byte >= 0xC0 {
            self.flush_utf8();
        }
        // Multi-byte UTF-8: buffer and decode when complete
        self.utf8_buf.push(byte);
        let expected = utf8_expected_len(self.utf8_buf[0]);
        if self.utf8_buf.len() >= expected {
            self.flush_utf8();
        }
    }

    fn execute(&mut self, byte: u8) {
        // Control characters interrupt pending UTF-8 sequences
        self.flush_utf8();
        match byte {
            0x07 => {
                self.bell = true;
            }
            0x08 => {
                if self.cursor.x > 0 {
                    self.cursor.x -= 1;
                }
                self.cursor.pending_wrap = false;
            }
            0x09 => {
                let width = self.grid.width();
                let mut next = self.cursor.x + 1;
                while next < width && !self.tab_stops.get(next).copied().unwrap_or(false) {
                    next += 1;
                }
                self.cursor.x = next.min(width.saturating_sub(1));
                self.cursor.pending_wrap = false;
            }
            0x0a..=0x0c => {
                self.line_feed();
            }
            0x0d => {
                self.cursor.x = 0;
                self.cursor.pending_wrap = false;
            }
            0x0e => {
                self.active_g1 = true;
            } // SO (Shift Out) — activate G1
            0x0f => {
                self.active_g1 = false;
            } // SI (Shift In)  — activate G0
            _ => {}
        }
    }

    fn csi(&mut self, intermediates: &[u8], params: &[u16], final_byte: u8) {
        let is_private = intermediates.contains(&b'?');
        match final_byte {
            b'A' => {
                let n = Self::param(params, 0, 1) as usize;
                let (top, _) = self.grid.scroll_region();
                self.cursor.y = self.cursor.y.saturating_sub(n).max(top);
                self.cursor.pending_wrap = false;
            }
            b'B' => {
                let n = Self::param(params, 0, 1) as usize;
                let (_, bottom) = self.grid.scroll_region();
                self.cursor.y = (self.cursor.y + n).min(bottom.saturating_sub(1));
                self.cursor.pending_wrap = false;
            }
            b'C' => {
                let n = Self::param(params, 0, 1) as usize;
                self.cursor.x = (self.cursor.x + n).min(self.grid.width().saturating_sub(1));
                self.cursor.pending_wrap = false;
            }
            b'D' => {
                let n = Self::param(params, 0, 1) as usize;
                self.cursor.x = self.cursor.x.saturating_sub(n);
                self.cursor.pending_wrap = false;
            }
            b'E' => {
                let n = Self::param(params, 0, 1) as usize;
                let (_, bottom) = self.grid.scroll_region();
                self.cursor.y = (self.cursor.y + n).min(bottom.saturating_sub(1));
                self.cursor.x = 0;
                self.cursor.pending_wrap = false;
            }
            b'F' => {
                let n = Self::param(params, 0, 1) as usize;
                let (top, _) = self.grid.scroll_region();
                self.cursor.y = self.cursor.y.saturating_sub(n).max(top);
                self.cursor.x = 0;
                self.cursor.pending_wrap = false;
            }
            b'G' => {
                let col = Self::param(params, 0, 1) as usize;
                self.set_cursor(col.saturating_sub(1), self.cursor.y);
            }
            b'H' | b'f' => {
                let row = Self::param(params, 0, 1) as usize;
                let col = Self::param(params, 1, 1) as usize;
                // Origin mode: CUP is relative to scroll region top
                let actual_row = if self.modes.origin {
                    let (top, _) = self.grid.scroll_region();
                    top + row.saturating_sub(1)
                } else {
                    row.saturating_sub(1)
                };
                self.set_cursor(col.saturating_sub(1), actual_row);
            }
            b'd' => {
                let row = Self::param(params, 0, 1) as usize;
                self.set_cursor(self.cursor.x, row.saturating_sub(1));
            }
            b'J' => {
                let mode = params.first().copied().unwrap_or(0);
                match mode {
                    0 => {
                        self.grid.clear_line_from(self.cursor.x, self.cursor.y);
                        for r in (self.cursor.y + 1)..self.grid.height() {
                            self.grid.clear_line(r);
                        }
                    }
                    1 => {
                        for r in 0..self.cursor.y {
                            self.grid.clear_line(r);
                        }
                        self.grid.clear_line_to(self.cursor.x + 1, self.cursor.y);
                    }
                    2 => {
                        self.grid.clear();
                    }
                    3 => {
                        self.grid.clear();
                        self.grid.clear_scrollback();
                    }
                    _ => {}
                }
            }
            b'K' => {
                let mode = params.first().copied().unwrap_or(0);
                match mode {
                    0 => self.grid.clear_line_from(self.cursor.x, self.cursor.y),
                    1 => self.grid.clear_line_to(self.cursor.x + 1, self.cursor.y),
                    2 => self.grid.clear_line(self.cursor.y),
                    _ => {}
                }
            }
            b'S' => {
                let n = Self::param(params, 0, 1) as usize;
                self.grid.scroll_up(n);
            }
            b'T' => {
                let n = Self::param(params, 0, 1) as usize;
                self.grid.scroll_down(n);
            }
            b'r' if !is_private => {
                // CSI r (no params) or CSI 0;0r → reset to full screen
                if params.is_empty() || params.iter().all(|&p| p == 0) {
                    self.grid.set_scroll_region(0, self.grid.height());
                    self.set_cursor(0, 0);
                } else {
                    let top = Self::param(params, 0, 1) as usize;
                    let bottom = params
                        .get(1)
                        .copied()
                        .unwrap_or(self.grid.height() as u16)
                        .max(1) as usize;
                    if top < bottom && bottom <= self.grid.height() {
                        self.grid.set_scroll_region(top.saturating_sub(1), bottom);
                        // In origin mode, cursor goes to scroll region top
                        let (st, _) = self.grid.scroll_region();
                        self.set_cursor(0, if self.modes.origin { st } else { 0 });
                    }
                }
            }
            b'm' => self.sgr(params),
            b'L' => {
                self.grid
                    .insert_line(self.cursor.y, Self::param(params, 0, 1) as usize);
            }
            b'M' => {
                self.grid
                    .delete_line(self.cursor.y, Self::param(params, 0, 1) as usize);
            }
            b'P' => {
                self.grid.delete_char(
                    self.cursor.x,
                    self.cursor.y,
                    Self::param(params, 0, 1) as usize,
                );
            }
            b'@' => {
                self.grid.insert_char(
                    self.cursor.x,
                    self.cursor.y,
                    Self::param(params, 0, 1) as usize,
                );
            }
            b'X' => {
                self.grid.erase_char(
                    self.cursor.x,
                    self.cursor.y,
                    Self::param(params, 0, 1) as usize,
                );
            }
            b'I' => {
                let n = Self::param(params, 0, 1);
                for _ in 0..n {
                    self.execute(0x09);
                }
            }
            b'Z' => {
                let n = Self::param(params, 0, 1);
                for _ in 0..n {
                    if self.cursor.x > 0 {
                        let mut p = self.cursor.x - 1;
                        while p > 0 && !self.tab_stops.get(p).copied().unwrap_or(false) {
                            p -= 1;
                        }
                        self.cursor.x = p;
                    }
                }
            }
            b'g' => {
                let m = params.first().copied().unwrap_or(0);
                match m {
                    0 => {
                        if self.cursor.x < self.tab_stops.len() {
                            self.tab_stops[self.cursor.x] = false;
                        }
                    }
                    3 => {
                        for s in &mut self.tab_stops {
                            *s = false;
                        }
                    }
                    _ => {}
                }
            }
            b'h' if is_private => {
                self.set_dec_mode(params.first().copied().unwrap_or(0), true);
            }
            b'l' if is_private => {
                self.set_dec_mode(params.first().copied().unwrap_or(0), false);
            }
            b'h' => {
                let m = params.first().copied().unwrap_or(0);
                if m == 4 {
                    self.modes.insert = true;
                }
            }
            b'l' => {
                let m = params.first().copied().unwrap_or(0);
                if m == 4 {
                    self.modes.insert = false;
                }
            }
            // REP — repeat preceding printable character N times
            b'b' => {
                let n = Self::param(params, 0, 1) as usize;
                if let Some(ch) = self.last_printed_char {
                    for _ in 0..n {
                        self.put_printable_char(ch);
                    }
                }
            }
            // DA1 — primary device attributes
            b'c' if !intermediates.contains(&b'>') => {
                // Respond: CSI ? 62 ; 1 ; 2 ; 4 ; 6 ; 9 ; 15 ; 16 ; 22 c
                // VT220-level, with basic capabilities
                self.response_buffer
                    .extend_from_slice(b"\x1b[?62;1;2;4;6;9;15;16;22c");
            }
            // DA2 — secondary device attributes (CSI > c)
            b'c' if intermediates.contains(&b'>') => {
                // Respond: CSI > 41 ; 0 ; 0 c (VT220)
                self.response_buffer.extend_from_slice(b"\x1b[>41;0;0c");
            }
            // DSR — device status report (CSI 6 n → cursor position)
            b'n' => {
                let mode = params.first().copied().unwrap_or(0);
                match mode {
                    5 => {
                        // Operating status: OK
                        self.response_buffer.extend_from_slice(b"\x1b[0n");
                    }
                    6 => {
                        // Cursor position report: CSI row;col R (1-based)
                        let (cx, cy) = (self.cursor.x + 1, self.cursor.y + 1);
                        let resp = format!("\x1b[{};{}R", cy, cx);
                        self.response_buffer.extend_from_slice(resp.as_bytes());
                    }
                    _ => {}
                }
            }
            // SCP — save cursor position (legacy ANSI.SYS)
            b's' => {
                self.saved_cursor = self.cursor;
            }
            // RCP — restore cursor position (legacy ANSI.SYS)
            b'u' => {
                self.cursor = self.saved_cursor;
                self.cursor.pending_wrap = false;
            }
            // DECSCUSR — cursor style (CSI Ps SP q)
            b'q' if intermediates.contains(&b' ') => {
                let style = params.first().copied().unwrap_or(0);
                self.cursor_style = match style {
                    0 => CursorStyle::Default,
                    1 => CursorStyle::BlinkBlock,
                    2 => CursorStyle::SteadyBlock,
                    3 => CursorStyle::BlinkUnderline,
                    4 => CursorStyle::SteadyUnderline,
                    5 => CursorStyle::BlinkBar,
                    6 => CursorStyle::SteadyBar,
                    _ => self.cursor_style,
                };
            }
            // DECSTR — soft terminal reset (CSI ! p)
            b'p' if intermediates.contains(&b'!') => {
                let w = self.grid.width();
                let h = self.grid.height();
                *self = Terminal::new(w, h);
            }
            _ => {}
        }
    }

    fn esc(&mut self, intermediates: &[u8], final_byte: u8) {
        // SCS: ESC ( <final> — designate G0 character set
        if intermediates.contains(&b'(') {
            match final_byte {
                b'B' => self.g0_charset = Charset::Ascii,      // US ASCII
                b'0' => self.g0_charset = Charset::DecSpecial, // DEC Special Graphics
                _ => {}                                        // Other charsets ignored (UK, etc.)
            }
            return;
        }
        // SCS: ESC ) <final> — designate G1 character set
        if intermediates.contains(&b')') {
            match final_byte {
                b'B' => self.g1_charset = Charset::Ascii,
                b'0' => self.g1_charset = Charset::DecSpecial,
                _ => {}
            }
            return;
        }
        // Handle intermediate-byte escape sequences (e.g. DECALN = ESC # 8).
        if intermediates.contains(&b'#') {
            if final_byte == b'8' {
                // DECALN — fill the entire screen with 'E' for alignment testing.
                // This also tests that scroll regions are NOT affected (they stay set).
                for row in 0..self.grid.height() {
                    for col in 0..self.grid.width() {
                        if let Some(c) = self.grid.cell_mut(col, row) {
                            c.ch = 'E';
                            c.fg = Color::Default;
                            c.bg = Color::Default;
                            c.flags = CellFlags::empty();
                        }
                    }
                }
                self.grid.mark_all_dirty();
            }
            return;
        }
        match final_byte {
            b'7' => self.saved_cursor = self.cursor,
            b'8' => self.cursor = self.saved_cursor,
            b'c' => {
                let w = self.grid.width();
                let h = self.grid.height();
                *self = Terminal::new(w, h);
            }
            b'D' => self.line_feed(),
            b'E' => {
                self.cursor.x = 0;
                self.line_feed();
                self.cursor.pending_wrap = false;
            }
            b'M' => self.reverse_line_feed(),
            b'H' => {
                if self.cursor.x < self.tab_stops.len() {
                    self.tab_stops[self.cursor.x] = true;
                }
            }
            _ => {}
        }
    }

    fn osc(&mut self, data: &[u8]) {
        let s = String::from_utf8_lossy(data);
        let mut parts = s.splitn(2, ';');
        let cmd = parts.next().and_then(|s| s.parse::<u16>().ok());
        match cmd {
            Some(0) | Some(2) => {
                self.title = parts.next().unwrap_or("").to_string();
            }
            Some(52) => {
                // OSC 52 — Clipboard manipulation.
                // Format: OSC 52 ; <selector> ; <base64-data> ST
                // <selector>: 'c' = clipboard, 'p' = primary selection.
                // With data: set clipboard.  Without data (empty): clear clipboard.
                let payload = parts.next().unwrap_or("");
                let base64_data = if let Some(idx) = payload.find(';') {
                    &payload[idx + 1..]
                } else {
                    payload
                };
                if base64_data.is_empty() {
                    self.pending_clipboard_set = Some(Vec::new());
                } else if let Some(decoded) = Self::decode_base64(base64_data) {
                    self.pending_clipboard_set = Some(decoded);
                }
            }
            Some(133) => {
                let payload = parts.next().unwrap_or("");
                let mut sub_parts = payload.splitn(2, ';');
                let mark_char = sub_parts.next().unwrap_or("");
                let exit_code = sub_parts.next().and_then(|code| code.parse::<i32>().ok());
                let (kind, has_exit) = match mark_char.chars().next() {
                    Some('A') => (CommandMarkKind::PromptStart, false),
                    Some('B') => (CommandMarkKind::CommandStart, false),
                    Some('C') => (CommandMarkKind::OutputStart, false),
                    Some('D') => (CommandMarkKind::CommandEnd, true),
                    _ => return,
                };
                self.command_marks.push(CommandMark {
                    kind,
                    row: self.cursor.y,
                    exit_code: if has_exit { exit_code } else { None },
                });
            }
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
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'H');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'i');
        assert_eq!(t.cursor(), (2, 0));
    }

    #[test]
    fn t_auto_wrap() {
        let mut t = Terminal::new(4, 4);
        feed(&mut t, b"ABCDE");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'D');
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'E');
    }

    #[test]
    fn t_cr_lf() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"AB\r\nCD");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'C');
        assert_eq!(t.grid().cell(1, 1).unwrap().ch, 'D');
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
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ');
    }

    #[test]
    fn t_ed_clear_to_end() {
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"ABC\x1b[0J");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
    }

    #[test]
    fn t_el_clear_line() {
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"Hello\x1b[2K");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ');
    }

    #[test]
    fn t_el_clear_to_end() {
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"Hello\x1b[0K");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'H');
        assert_eq!(t.grid().cell(5, 0).unwrap().ch, ' ');
    }

    #[test]
    fn t_sgr_bold() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[1mX");
        assert!(t.grid().cell(0, 0).unwrap().flags.contains(CellFlags::BOLD));
    }

    #[test]
    fn t_sgr_underline() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[4mU");
        assert!(
            t.grid()
                .cell(0, 0)
                .unwrap()
                .flags
                .contains(CellFlags::UNDERLINE)
        );
    }

    #[test]
    fn t_sgr_color_fg() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[31mR");
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Indexed(1));
    }

    #[test]
    fn t_sgr_color_bg() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[42mG");
        assert_eq!(t.grid().cell(0, 0).unwrap().bg, Color::Indexed(2));
    }

    #[test]
    fn t_sgr_bright_color() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[91mR");
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Indexed(9));
    }

    #[test]
    fn t_sgr_truecolor() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[38;2;255;128;0mX");
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Rgb(255, 128, 0));
    }

    #[test]
    fn t_sgr_256color() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[38;5;200mX");
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Indexed(200));
    }

    #[test]
    fn t_sgr_reset() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[1;31mA\x1b[0mB");
        assert!(t.grid().cell(0, 0).unwrap().flags.contains(CellFlags::BOLD));
        assert!(!t.grid().cell(1, 0).unwrap().flags.contains(CellFlags::BOLD));
        assert_eq!(t.grid().cell(1, 0).unwrap().fg, Color::Default);
    }

    #[test]
    fn t_sgr_multi_attr() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[1;3;4mX");
        let c = t.grid().cell(0, 0).unwrap();
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
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'R');
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
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ');
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
    fn t_osc_52_set_clipboard() {
        let mut t = Terminal::new(80, 24);
        // "hello" in base64 = "aGVsbG8="
        feed(&mut t, b"\x1b]52;c;aGVsbG8=\x07");
        assert_eq!(t.take_pending_clipboard_set(), Some(b"hello".to_vec()));
    }

    #[test]
    fn t_osc_52_set_clipboard_st_terminated() {
        let mut t = Terminal::new(80, 24);
        // "world" in base64 = "d29ybGQ="
        feed(&mut t, b"\x1b]52;c;d29ybGQ=\x1b\\");
        assert_eq!(t.take_pending_clipboard_set(), Some(b"world".to_vec()));
    }

    #[test]
    fn t_osc_52_clear_clipboard() {
        let mut t = Terminal::new(80, 24);
        // Empty data = clear clipboard
        feed(&mut t, b"\x1b]52;c;\x07");
        assert_eq!(t.take_pending_clipboard_set(), Some(Vec::new()));
    }

    #[test]
    fn t_osc_52_no_data() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]52;c;\x07");
        // Should set empty clipboard
        assert!(t.take_pending_clipboard_set().is_some());
    }

    #[test]
    fn t_osc_52_take_clears() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]52;c;aGVsbG8=\x07");
        assert!(t.take_pending_clipboard_set().is_some());
        // Second take should return None
        assert!(t.take_pending_clipboard_set().is_none());
    }

    #[test]
    fn t_base64_decode_basic() {
        assert_eq!(Terminal::decode_base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(Terminal::decode_base64("d29ybGQ=").unwrap(), b"world");
        assert_eq!(Terminal::decode_base64("Zm9v").unwrap(), b"foo");
    }

    #[test]
    fn t_base64_decode_empty() {
        assert_eq!(Terminal::decode_base64("").unwrap(), b"");
    }

    #[test]
    fn t_base64_decode_padding() {
        assert_eq!(Terminal::decode_base64("Zg==").unwrap(), b"f");
        assert_eq!(Terminal::decode_base64("Zm8=").unwrap(), b"fo");
    }

    #[test]
    fn t_bracketed_paste_accessor() {
        let mut t = Terminal::new(80, 24);
        assert!(!t.bracketed_paste());
        feed(&mut t, b"\x1b[?2004h");
        assert!(t.bracketed_paste());
        feed(&mut t, b"\x1b[?2004l");
        assert!(!t.bracketed_paste());
    }

    #[test]
    fn t_bell_sets_flag() {
        let mut t = Terminal::new(80, 24);
        assert!(!t.take_bell());
        feed(&mut t, b"\x07");
        assert!(t.take_bell());
    }

    #[test]
    fn t_bell_take_clears() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x07");
        assert!(t.take_bell());
        assert!(!t.take_bell());
    }

    #[test]
    fn t_bell_in_text() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"hello\x07world");
        assert!(t.take_bell());
    }

    #[test]
    fn t_bell_multiple() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x07\x07\x07");
        // Multiple bells — still just true (we only track that bell occurred).
        assert!(t.take_bell());
        assert!(!t.take_bell());
    }

    #[test]
    fn t_bell_no_false_positive() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"hello world");
        assert!(!t.take_bell());
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
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ');
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'A');
    }

    #[test]
    fn t_delete_line() {
        let mut t = Terminal::new(10, 4);
        feed(
            &mut t,
            b"\x1b[1;1HA\x1b[2;1HB\x1b[3;1HC\x1b[4;1HD\x1b[1;1H\x1b[M",
        );
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'B');
    }

    #[test]
    fn t_insert_char() {
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"ABC\x1b[1G\x1b[@");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'A');
    }

    #[test]
    fn t_delete_char() {
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"ABC\x1b[1G\x1b[P");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'B');
    }

    #[test]
    fn t_erase_char() {
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"ABC\x1b[1G\x1b[X");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B');
    }

    #[test]
    fn t_irm_insert_mode() {
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"\x1b[4hAB");
        // In insert mode, each char pushes existing chars right.
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B');
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
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'H');
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Indexed(2));
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'W');
        assert_eq!(t.grid().cell(0, 1).unwrap().fg, Color::Default);
    }

    #[test]
    fn t_tab_clear() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[3g"); // Clear all tabs
        feed(&mut t, b"\t"); // Tab now does nothing (no stops)
        assert_eq!(t.cursor().0, 79); // Moved to end (no tab stop found)
    }

    #[test]
    fn t_hts_set_tab() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5G\x1bH"); // Move to col 5, set tab stop
        feed(&mut t, b"\x1b[1G\t"); // Home, tab → should hit col 5
        assert_eq!(t.cursor().0, 4);
    }

    #[test]
    fn t_concurrent_feed() {
        // Simulate concurrent feeding from different chunks.
        let mut t = Terminal::new(80, 24);
        let mut p = Parser::new();
        p.feed(b"\x1b[31", &mut t);
        p.feed(b"mRed", &mut t);
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Indexed(1));
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'R');
    }

    // -- UTF-8 tests --

    #[test]
    fn t_utf8_ascii() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"Hi");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'H');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'i');
        assert_eq!(t.cursor().0, 2);
    }

    #[test]
    fn t_utf8_chinese_3byte() {
        // "你好" = E4BDA0 E5A5BD in UTF-8 (3 bytes per char, display width=2 each)
        let mut t = Terminal::new(80, 24);
        feed(&mut t, "你好".as_bytes());
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '你');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, '好');
        assert_eq!(t.cursor().0, 4); // 2 chars * 2 cells each
    }

    #[test]
    fn t_utf8_emoji_4byte() {
        // 😀 = F09F9880 in UTF-8 (4 bytes, display width=2)
        let mut t = Terminal::new(80, 24);
        feed(&mut t, "😀".as_bytes());
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '😀');
        assert_eq!(t.cursor().0, 2); // 1 emoji * 2 cells
    }

    #[test]
    fn t_utf8_mixed_ascii_cjk() {
        // "AB你好CD" — mix ASCII (1 cell) and CJK (2 cells)
        let mut t = Terminal::new(80, 24);
        feed(&mut t, "AB你好CD".as_bytes());
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, '你');
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, '好');
        assert_eq!(t.grid().cell(6, 0).unwrap().ch, 'C');
        assert_eq!(t.grid().cell(7, 0).unwrap().ch, 'D');
        assert_eq!(t.cursor().0, 8); // 2+2+2+2 = 8
    }

    #[test]
    fn t_utf8_split_across_feeds() {
        // Feed the 3 bytes of '你' (E4 BD A0) in separate feed calls
        let mut t = Terminal::new(80, 24);
        let mut p = Parser::new();
        p.feed(&[0xE4], &mut t);
        p.feed(&[0xBD], &mut t);
        p.feed(&[0xA0], &mut t);
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '你');
        assert_eq!(t.cursor().0, 2); // CJK = 2 cells
    }

    #[test]
    fn t_utf8_cjk_wraps_at_margin() {
        // Grid width=4: write "AB你" — CJK fills cols 2-3, then 'C' wraps
        let mut t = Terminal::new(4, 24);
        feed(&mut t, b"AB");
        assert_eq!(t.cursor().0, 2);
        feed(&mut t, "你".as_bytes()); // fills to end of line
        assert_eq!(t.cursor().0, 3); // cursor at last col, pending_wrap set
        feed(&mut t, "C".as_bytes()); // wrap + write C
        assert_eq!(t.cursor().0, 1); // C at col 0, cursor at 1
        assert_eq!(t.cursor().1, 1); // wrapped to row 1
    }

    #[test]
    fn t_utf8_control_interrupts_buffer() {
        // Start a CJK sequence but interrupt with BS before completing
        let mut t = Terminal::new(80, 24);
        let mut p = Parser::new();
        p.feed(&[0xE4, 0xBD], &mut t); // incomplete '你'
        p.feed(b"\x08", &mut t); // BS (execute) — should flush (drop incomplete)
        feed(&mut t, b"X");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X');
    }

    #[test]
    fn t_utf8_invalid_sequence_emits_replacement() {
        // Invalid UTF-8 bytes should emit U+FFFD (replacement character)
        let mut t = Terminal::new(80, 24);
        feed(&mut t, &[0xFF]);
        // 0xFF is invalid → flush_utf8 emits U+FFFD when next byte arrives
        feed(&mut t, b"A");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{FFFD}');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'A');
    }

    #[test]
    fn t_utf8_split_emoji_across_feeds() {
        // 😀 = F0 9F 98 80 (4 bytes)
        let mut t = Terminal::new(80, 24);
        let mut p = Parser::new();
        p.feed(&[0xF0, 0x9F], &mut t);
        p.feed(&[0x98, 0x80], &mut t);
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '😀');
        assert_eq!(t.cursor().0, 2);
    }

    #[test]
    fn t_utf8_styled_wide_char_preserves_flags() {
        // Bold red CJK char — SGR attributes must merge with WIDE_CHAR flag
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[1;31m");
        feed(&mut t, "好".as_bytes());
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.ch, '好');
        assert!(cell.is_wide(), "must preserve WIDE_CHAR flag");
        assert!(cell.flags.contains(CellFlags::BOLD), "must preserve BOLD");
        assert_eq!(cell.fg, Color::Indexed(1));
    }

    #[test]
    fn t_utf8_multiple_cjk_sequence() {
        // Write 3 CJK chars in a row
        let mut t = Terminal::new(80, 24);
        feed(&mut t, "你好世".as_bytes());
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '你');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, '好');
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, '世');
        assert_eq!(t.cursor().0, 6); // 3 * 2 = 6
    }

    #[test]
    fn t_utf8_truncated_then_new_sequence() {
        // Truncated E4 BD (incomplete '你') then valid E5 A5 BD = '好'
        // The new leading byte E5 should flush the old incomplete sequence
        let mut t = Terminal::new(80, 24);
        let mut p = Parser::new();
        p.feed(&[0xE4, 0xBD, 0xE5, 0xA5, 0xBD], &mut t);
        // E4 BD is incomplete → U+FFFD at col 0 (width 1)
        // E5 A5 BD = '好' → col 1-2 (width 2)
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{FFFD}');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, '好');
        assert_eq!(t.cursor().0, 3);
    }

    #[test]
    fn t_utf8_cjk_followed_by_ascii() {
        // CJK immediately followed by ASCII in same feed
        let mut t = Terminal::new(80, 24);
        feed(&mut t, "你X".as_bytes());
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '你');
        assert!(t.grid().cell(0, 0).unwrap().is_wide());
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'X');
        assert_eq!(t.cursor().0, 3); // 2 (CJK) + 1 (ASCII)
    }

    #[test]
    fn t_utf8_wide_char_bg_on_spacer() {
        // Wide char with background color — spacer cell should inherit bg
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[42m"); // green bg
        feed(&mut t, "中".as_bytes());
        assert_eq!(t.grid().cell(0, 0).unwrap().bg, Color::Indexed(2));
        assert_eq!(
            t.grid().cell(1, 0).unwrap().bg,
            Color::Indexed(2),
            "spacer cell should inherit bg color"
        );
    }

    #[test]
    fn t_utf8_cjk_at_penultimate_col() {
        // Width=4: write ABC → cursor at col 3. CJK (width 2) doesn't fit at col 3.
        // Should wrap to next line when auto_wrap is on.
        let mut t = Terminal::new(4, 24);
        feed(&mut t, b"ABC"); // A=col0, B=col1, C=col2, cursor at col3
        feed(&mut t, "你".as_bytes()); // doesn't fit at col 3 → wrap
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'C');
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, '你');
        assert!(t.grid().cell(0, 1).unwrap().is_wide());
    }

    // -- dd_dev bug review regression tests --

    #[test]
    fn t_utf8_wide_char_flag_preserved() {
        // Bug 1 regression: put_printable_char must not overwrite WIDE_CHAR flag.
        // After writing a CJK char, the cell must still have WIDE_CHAR set
        // even when SGR attributes (bold, italic, etc.) are active.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[1;3m"); // Bold + italic
        feed(&mut t, "你".as_bytes());
        let cell = t.grid().cell(0, 0).unwrap();
        assert!(
            cell.is_wide(),
            "WIDE_CHAR flag must be preserved after SGR merge"
        );
        assert!(
            cell.flags.contains(CellFlags::BOLD),
            "BOLD flag must be set"
        );
        assert!(
            cell.flags.contains(CellFlags::ITALIC),
            "ITALIC flag must be set"
        );
    }

    #[test]
    fn t_utf8_invalid_emits_replacement_char() {
        // Bug 2: flush_utf8 should emit U+FFFD for invalid UTF-8, not silently drop.
        // 0xFF is never a valid UTF-8 leading byte.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, &[0xFF]);
        // After execute/feed completes, the invalid byte should have been flushed
        // as U+FFFD. We feed a trailing ASCII to force the flush.
        feed(&mut t, b"A");
        // Cell (0,0) should have U+FFFD, cell (1,0) should have 'A'
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{FFFD}');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'A');
    }

    // ==================================================================
    // P2-3: DECALN (ESC # 8) + Scroll Region + Tab Stop edge cases
    // ==================================================================

    #[test]
    fn t_decaln_fills_screen_with_e() {
        let mut t = Terminal::new(10, 5);
        // Write some content first
        feed(&mut t, b"ABCD");
        feed(&mut t, b"\x1b#8"); // DECALN
        for y in 0..5 {
            for x in 0..10 {
                assert_eq!(
                    t.grid().cell(x, y).unwrap().ch,
                    'E',
                    "cell ({},{}) should be 'E' after DECALN",
                    x,
                    y
                );
            }
        }
    }

    #[test]
    fn t_decaln_resets_attributes() {
        let mut t = Terminal::new(10, 3);
        // Set bold + colors, then DECALN
        feed(&mut t, b"\x1b[1;31m");
        feed(&mut t, b"\x1b#8");
        let cell = t.grid().cell(0, 0).unwrap();
        assert!(
            !cell.flags.contains(CellFlags::BOLD),
            "DECALN should reset attributes"
        );
        assert_eq!(cell.fg, Color::Default, "DECALN should reset fg");
        assert_eq!(cell.bg, Color::Default, "DECALN should reset bg");
    }

    #[test]
    fn t_decaln_preserves_scroll_region() {
        let mut t = Terminal::new(10, 6);
        // Set scroll region to rows 1-4 (0-based)
        feed(&mut t, b"\x1b[2;5r");
        // DECALN fills entire screen regardless of scroll region
        feed(&mut t, b"\x1b#8");
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(top, 1, "scroll region top preserved after DECALN");
        assert_eq!(bottom, 5, "scroll region bottom preserved after DECALN");
    }

    #[test]
    fn t_decstbm_reset_no_params() {
        let mut t = Terminal::new(80, 24);
        // Set scroll region
        feed(&mut t, b"\x1b[5;15r");
        assert_eq!(t.grid().scroll_region(), (4, 15));
        // Reset with no params: CSI r → full screen
        feed(&mut t, b"\x1b[r");
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(top, 0, "DECSTBM reset: top should be 0");
        assert_eq!(bottom, 24, "DECSTBM reset: bottom should be height");
    }

    #[test]
    fn t_decstbm_invalid_params_ignored() {
        let mut t = Terminal::new(80, 24);
        // top >= bottom → ignored (reset to full screen)
        feed(&mut t, b"\x1b[15;5r");
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(top, 0);
        assert_eq!(bottom, 24);
    }

    #[test]
    fn t_decstbm_bottom_exceeds_height() {
        let mut t = Terminal::new(80, 24);
        // bottom > height → reset to full screen
        feed(&mut t, b"\x1b[5;30r");
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(top, 0);
        assert_eq!(bottom, 24);
    }

    #[test]
    fn t_scroll_region_isolation() {
        // Scrolling inside the region should not affect rows outside.
        let mut t = Terminal::new(10, 6);
        // Fill all rows
        for row in 0..6 {
            feed(&mut t, format!("R{}\n", row).as_bytes());
        }
        // Set scroll region to rows 2-4 (0-indexed: 1-3)
        feed(&mut t, b"\x1b[2;4r");
        // Move cursor inside region and scroll up
        feed(&mut t, b"\x1b[2;1H"); // row 2, col 1
        feed(&mut t, b"\x1b[S"); // scroll up 1
        // Row 0 and row 5 should be unaffected
        // (Content may shift inside region but rows outside stay)
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(top, 1);
        assert_eq!(bottom, 4);
    }

    #[test]
    fn t_tab_at_last_column_no_panic() {
        // HT at last column should not panic or go out of bounds.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1;10H"); // move to last column
        feed(&mut t, b"\t"); // HT
        assert!(t.cursor().0 < 10, "cursor should not exceed width");
    }

    #[test]
    fn t_cbt_at_first_column_no_panic() {
        // CBT (reverse tab) at column 0 should not panic.
        let mut t = Terminal::new(10, 3);
        // cursor at (0,0)
        feed(&mut t, b"\x1b[Z"); // CBT
        assert_eq!(t.cursor().0, 0, "CBT at column 0 stays at 0");
    }

    #[test]
    fn t_tbc_clear_all_tab_stops() {
        let mut t = Terminal::new(40, 3);
        // Set some tab stops
        feed(&mut t, b"\x1b[1;5H\x1bH"); // HTS at column 5
        feed(&mut t, b"\x1b[1;15H\x1bH"); // HTS at column 15
        // Clear all
        feed(&mut t, b"\x1b[3g");
        // Tab should now only stop at default positions (or none)
        // After TBC 3, all tab stops are cleared
        feed(&mut t, b"\x1b[1;1H");
        feed(&mut t, b"\t");
        // With no tab stops, tab should move to end of line
        assert!(t.cursor().0 <= 40);
    }

    #[test]
    fn t_decstbm_moves_cursor_home() {
        // After DECSTBM, cursor should move to (0,0) of the screen
        // (or origin of scroll region if origin mode is on).
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[10;10H"); // move cursor away
        feed(&mut t, b"\x1b[5;15r"); // set scroll region
        // Per VT spec, DECSTBM moves cursor to home position
        assert_eq!(t.cursor(), (0, 0), "DECSTBM should home cursor");
    }

    // ==================================================================
    // P3-A: OSC 133 Shell Integration (Command Marks)
    // ==================================================================

    #[test]
    fn t_osc133_prompt_start() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;A\x07"); // BEL terminated
        let marks = t.command_marks();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].kind, CommandMarkKind::PromptStart);
        assert_eq!(marks[0].row, 0);
        assert_eq!(marks[0].exit_code, None);
    }

    #[test]
    fn t_osc133_command_start() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;B\x07");
        let marks = t.command_marks();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].kind, CommandMarkKind::CommandStart);
    }

    #[test]
    fn t_osc133_output_start() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;C\x07");
        let marks = t.command_marks();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].kind, CommandMarkKind::OutputStart);
    }

    #[test]
    fn t_osc133_command_end() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;D\x07");
        let marks = t.command_marks();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].kind, CommandMarkKind::CommandEnd);
        assert_eq!(marks[0].exit_code, None, "D without exit code → None");
    }

    #[test]
    fn t_osc133_command_end_with_exit_code_zero() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;D;0\x07");
        let marks = t.command_marks();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].kind, CommandMarkKind::CommandEnd);
        assert_eq!(marks[0].exit_code, Some(0));
    }

    #[test]
    fn t_osc133_command_end_with_error_code() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;D;127\x07");
        let marks = t.command_marks();
        assert_eq!(marks[0].kind, CommandMarkKind::CommandEnd);
        assert_eq!(marks[0].exit_code, Some(127));
    }

    #[test]
    fn t_osc133_st_terminated() {
        // ST = ESC \ (0x1b 0x5c)
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;A\x1b\\");
        let marks = t.command_marks();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].kind, CommandMarkKind::PromptStart);
    }

    #[test]
    fn t_osc133_full_cycle() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;A\x07"); // prompt start at row 0
        feed(&mut t, b"ls -la\r\n");
        feed(&mut t, b"\x1b]133;B\x07"); // command start (Enter pressed)
        feed(&mut t, b"\x1b]133;C\x07"); // output start
        feed(&mut t, b"file1 file2\r\n");
        feed(&mut t, b"\x1b]133;D;0\x07"); // command end, exit 0
        let marks = t.command_marks();
        assert_eq!(marks.len(), 4);
        assert_eq!(marks[0].kind, CommandMarkKind::PromptStart);
        assert_eq!(marks[1].kind, CommandMarkKind::CommandStart);
        assert_eq!(marks[2].kind, CommandMarkKind::OutputStart);
        assert_eq!(marks[3].kind, CommandMarkKind::CommandEnd);
        assert_eq!(marks[3].exit_code, Some(0));
    }

    #[test]
    fn t_osc133_truncated_command_new_prompt() {
        // A → B → A without D (user Ctrl+C'd then new prompt)
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;A\x07"); // prompt
        feed(&mut t, b"\x1b]133;B\x07"); // command start
        feed(&mut t, b"\x1b]133;A\x07"); // new prompt (no D for previous)
        let marks = t.command_marks();
        assert_eq!(marks.len(), 3);
        assert_eq!(marks[0].kind, CommandMarkKind::PromptStart);
        assert_eq!(marks[1].kind, CommandMarkKind::CommandStart);
        assert_eq!(marks[2].kind, CommandMarkKind::PromptStart);
    }

    #[test]
    fn t_osc133_row_tracking() {
        // Command marks should record the cursor row.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;A\x07"); // row 0
        feed(&mut t, b"\n\n"); // cursor now at row 2
        feed(&mut t, b"\x1b]133;C\x07"); // row 2
        let marks = t.command_marks();
        assert_eq!(marks[0].row, 0);
        assert_eq!(marks[1].row, 2);
    }

    #[test]
    fn t_osc133_unknown_subcommand_ignored() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;X\x07"); // unknown subcommand
        assert_eq!(
            t.command_marks().len(),
            0,
            "unknown subcommand should be ignored"
        );
    }

    #[test]
    fn t_osc133_empty_payload() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133\x07"); // OSC 133 with no sub-mark
        assert_eq!(
            t.command_marks().len(),
            0,
            "OSC 133 with empty payload should be ignored"
        );
    }

    #[test]
    fn t_osc133_negative_exit_code() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;D;-1\x07");
        let marks = t.command_marks();
        assert_eq!(marks[0].exit_code, Some(-1));
    }

    // -- P2-2: CSI extensions tests --

    #[test]
    fn t_rep_basic() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"A");
        feed(&mut t, b"\x1b[3b"); // REP 3 times → total "AAAA"
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'A');
        assert_eq!(t.cursor().0, 4);
    }

    #[test]
    fn t_rep_default_count() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"X");
        feed(&mut t, b"\x1b[b"); // REP with default = 1
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'X');
        assert_eq!(t.cursor().0, 2);
    }

    #[test]
    fn t_rep_no_preceding_char() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5b"); // REP without preceding char → no-op
        assert_eq!(t.cursor().0, 0);
    }

    #[test]
    fn t_dsr_cursor_position() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"Hello\x1b[6n"); // cursor at col 5, row 0
        let resp = t.take_response();
        let expected = b"\x1b[1;6R"; // row 1, col 6 (1-based)
        assert_eq!(resp, expected);
    }

    #[test]
    fn t_dsr_device_status() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5n"); // device status report
        let resp = t.take_response();
        assert_eq!(resp, b"\x1b[0n"); // OK
    }

    #[test]
    fn t_da1_primary() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[c"); // primary DA
        let resp = t.take_response();
        assert!(resp.starts_with(b"\x1b[?"));
        assert!(resp.ends_with(b"c"));
    }

    #[test]
    fn t_da2_secondary() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[>c"); // secondary DA
        let resp = t.take_response();
        assert!(resp.starts_with(b"\x1b[>"));
    }

    #[test]
    fn t_decscusr_steady_block() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[2 q"); // steady block
        assert_eq!(t.cursor_style(), CursorStyle::SteadyBlock);
    }

    #[test]
    fn t_decscusr_blinking_underline() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[3 q"); // blinking underline
        assert_eq!(t.cursor_style(), CursorStyle::BlinkUnderline);
    }

    #[test]
    fn t_decscusr_steady_bar() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[6 q"); // steady bar
        assert_eq!(t.cursor_style(), CursorStyle::SteadyBar);
    }

    #[test]
    fn t_decscusr_default() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[2 q"); // change first
        feed(&mut t, b"\x1b[0 q"); // reset to default
        assert_eq!(t.cursor_style(), CursorStyle::Default);
    }

    #[test]
    fn t_scp_rcp() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5;10H"); // move to row 5, col 10
        feed(&mut t, b"\x1b[s"); // SCP — save position
        feed(&mut t, b"\x1b[1;1H"); // move to home
        feed(&mut t, b"\x1b[u"); // RCP — restore
        assert_eq!(t.cursor(), (9, 4)); // 0-based: col 9, row 4
    }

    #[test]
    fn t_decstr_soft_reset() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5;10H"); // move cursor
        feed(&mut t, b"\x1b[31m"); // set color
        feed(&mut t, b"\x1b[!p"); // DECSTR — soft reset
        assert_eq!(t.cursor(), (0, 0));
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Default);
    }

    #[test]
    fn t_origin_mode_cup() {
        let mut t = Terminal::new(80, 24);
        // Set scroll region to rows 5-15 (0-based: 4-14)
        feed(&mut t, b"\x1b[5;15r");
        // Enable origin mode
        feed(&mut t, b"\x1b[?6h");
        // CUP to row 1, col 1 → should be relative to scroll region top
        feed(&mut t, b"\x1b[1;1H");
        assert_eq!(t.cursor().1, 4); // row 4 (0-based) = scroll top
    }

    #[test]
    fn t_origin_mode_disabled_cup() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5;15r"); // set scroll region
        // Origin mode NOT enabled
        feed(&mut t, b"\x1b[1;1H");
        assert_eq!(t.cursor().1, 0); // row 0 (absolute)
    }

    #[test]
    fn t_ed_mode3_clear_scrollback() {
        let mut t = Terminal::new(80, 4);
        // Fill visible screen, then scroll to create scrollback
        feed(&mut t, b"AAAA\r\nBBBB\r\nCCCC\r\nDDDD\r\nEEEE");
        assert!(t.grid().scrollback_len() > 0);
        // ED mode 3 — clear scrollback only
        feed(&mut t, b"\x1b[3J");
        assert_eq!(t.grid().scrollback_len(), 0);
    }

    #[test]
    fn t_decestbm_reset_no_params() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5;15r"); // set region
        feed(&mut t, b"\x1b[r"); // reset with no params
        feed(&mut t, b"\x1b[1;1H");
        assert_eq!(t.cursor(), (0, 0));
    }

    #[test]
    fn t_decestbm_reset_zero_params() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5;15r"); // set region
        feed(&mut t, b"\x1b[0;0r"); // reset with 0;0
        feed(&mut t, b"\x1b[1;1H");
        assert_eq!(t.cursor(), (0, 0));
    }

    #[test]
    fn t_response_buffer_drain() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[6n"); // DSR
        assert!(!t.response_buffer().is_empty());
        let drained = t.take_response();
        assert!(!drained.is_empty());
        assert!(t.response_buffer().is_empty()); // drained
    }

    #[test]
    fn t_cursor_style_default() {
        let t = Terminal::new(80, 24);
        assert_eq!(t.cursor_style(), CursorStyle::Default);
    }

    // ---- P2-4: G0/G1 Character Set tests ----

    #[test]
    fn t_charset_default_state() {
        let t = Terminal::new(80, 24);
        assert_eq!(t.g0_charset(), Charset::Ascii);
        assert_eq!(t.g1_charset(), Charset::Ascii);
        assert!(!t.active_g1(), "G0 should be active by default");
    }

    #[test]
    fn t_charset_scs_g0_dec_special() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b(0"); // ESC ( 0
        assert_eq!(t.g0_charset(), Charset::DecSpecial);
        assert!(!t.active_g1());
    }

    #[test]
    fn t_charset_scs_g0_ascii_restore() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b(0");
        feed(&mut t, b"\x1b(B");
        assert_eq!(t.g0_charset(), Charset::Ascii);
    }

    #[test]
    fn t_charset_scs_g1_dec_special() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b)0");
        assert_eq!(t.g1_charset(), Charset::DecSpecial);
    }

    #[test]
    fn t_charset_scs_g1_ascii_restore() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b)0");
        feed(&mut t, b"\x1b)B");
        assert_eq!(t.g1_charset(), Charset::Ascii);
    }

    #[test]
    fn t_charset_so_shift_out_activates_g1() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b)0");
        feed(&mut t, b"\x0e"); // SO
        assert!(t.active_g1());
    }

    #[test]
    fn t_charset_si_shift_in_activates_g0() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b)0\x0e\x0f");
        assert!(!t.active_g1());
    }

    #[test]
    fn t_charset_dec_special_g0_translation() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b(0q");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{2500}'); // ─
    }

    #[test]
    fn t_charset_dec_special_g1_via_so() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b)0\x0ex");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{2502}'); // │
    }

    #[test]
    fn t_charset_dec_special_corner_chars() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b(0lk mj");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{250C}'); // ┌
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, '\u{2510}'); // ┐
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, '\u{2514}'); // └
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, '\u{2518}'); // ┘
    }

    #[test]
    fn t_charset_dec_special_cross_tee() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b(0ntuvw");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{253C}'); // ┼
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, '\u{251C}'); // ├
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, '\u{2524}'); // ┤
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, '\u{2534}'); // ┴
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, '\u{252C}'); // ┬
    }

    #[test]
    fn t_charset_dec_special_special_chars() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b(0`afg");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{25C6}'); // ◆
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, '\u{2592}'); // ▒
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, '\u{00B0}'); // °
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, '\u{00B1}'); // ±
    }

    #[test]
    fn t_charset_ascii_passes_through() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"Hello");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'H');
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'o');
    }

    #[test]
    fn t_charset_dec_special_below_range_unchanged() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b(0A1");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, '1');
    }

    #[test]
    fn t_charset_switch_back_to_ascii_restores_text() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b(0q\x1b(Bq");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{2500}'); // ─
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'q');
    }

    #[test]
    fn t_charset_so_si_toggle() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b(0\x1b)B");
        // G0=DEC, G1=ASCII
        feed(&mut t, b"q");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{2500}'); // G0 → ─
        feed(&mut t, b"\x0eq"); // shift to G1
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'q'); // G1 → q
        feed(&mut t, b"\x0fq"); // shift to G0
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, '\u{2500}'); // G0 → ─
    }

    #[test]
    fn t_charset_ris_resets() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b(0\x1b)0\x0e");
        feed(&mut t, b"\x1bc"); // RIS
        assert_eq!(t.g0_charset(), Charset::Ascii);
        assert_eq!(t.g1_charset(), Charset::Ascii);
        assert!(!t.active_g1());
    }

    #[test]
    fn t_charset_dec_special_box_drawing() {
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b(0");
        feed(&mut t, b"lqqqk\r"); // ┌───┐
        feed(&mut t, b"\nx   x\r"); // │   │
        feed(&mut t, b"\nmqqqj"); // └───┘
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{250C}'); // ┌
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, '\u{2500}'); // ─
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, '\u{2510}'); // ┐
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, '\u{2502}'); // │
        assert_eq!(t.grid().cell(4, 1).unwrap().ch, '\u{2502}'); // │
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, '\u{2514}'); // └
        assert_eq!(t.grid().cell(4, 2).unwrap().ch, '\u{2518}'); // ┘
    }

    #[test]
    fn t_charset_scs_unknown_final_ignored() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b(Z");
        assert_eq!(t.g0_charset(), Charset::Ascii);
    }

    #[test]
    fn t_charset_scs_uk_treated_as_ascii() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b(A");
        assert_eq!(t.g0_charset(), Charset::Ascii);
    }

    // -- P3-B: CommandBlock data model tests --

    #[test]
    fn t_command_blocks_empty() {
        let t = Terminal::new(80, 24);
        assert!(t.command_blocks().is_empty());
    }

    #[test]
    fn t_command_blocks_single_command() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;A\x07"); // PromptStart
        feed(&mut t, b"\x1b]133;C\x07"); // OutputStart
        feed(&mut t, b"\x1b]133;D;0\x07"); // CommandEnd exit 0
        let blocks = t.command_blocks();
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].is_complete());
        assert!(blocks[0].is_success());
    }

    #[test]
    fn t_command_blocks_failed_exit() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;A\x07");
        feed(&mut t, b"\x1b]133;C\x07");
        feed(&mut t, b"\x1b]133;D;127\x07");
        let blocks = t.command_blocks();
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].is_failure());
        assert!(!blocks[0].is_success());
    }

    #[test]
    fn t_command_blocks_multiple() {
        let mut t = Terminal::new(80, 24);
        // First command
        feed(&mut t, b"\x1b]133;A\x07\x1b]133;C\x07\x1b]133;D;0\x07");
        // Second command
        feed(&mut t, b"\x1b]133;A\x07\x1b]133;C\x07\x1b]133;D;1\x07");
        let blocks = t.command_blocks();
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].is_success());
        assert!(blocks[1].is_failure());
    }

    #[test]
    fn t_command_blocks_running() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;A\x07"); // PromptStart only
        let blocks = t.command_blocks();
        assert_eq!(blocks.len(), 1);
        assert!(!blocks[0].is_complete());
        assert!(blocks[0].is_at_prompt());
    }

    #[test]
    fn t_command_blocks_last_exit_code() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;A\x07\x1b]133;C\x07\x1b]133;D;42\x07");
        assert_eq!(t.last_exit_code(), Some(42));
        assert!(!t.last_command_succeeded());
    }

    #[test]
    fn t_command_blocks_last_exit_code_none() {
        let t = Terminal::new(80, 24);
        assert_eq!(t.last_exit_code(), None);
    }

    #[test]
    fn t_command_blocks_extract_row_text() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"Hello World");
        assert_eq!(t.extract_row_text(0), "Hello World");
        assert_eq!(t.extract_row_text(1), "");
    }

    #[test]
    fn t_group_command_blocks_empty() {
        assert!(group_command_blocks(&[]).is_empty());
    }

    #[test]
    fn t_group_command_blocks_stress() {
        let mut marks = Vec::new();
        for i in 0..100 {
            marks.push(CommandMark {
                kind: CommandMarkKind::PromptStart,
                row: i * 3,
                exit_code: None,
            });
            marks.push(CommandMark {
                kind: CommandMarkKind::OutputStart,
                row: i * 3,
                exit_code: None,
            });
            marks.push(CommandMark {
                kind: CommandMarkKind::CommandEnd,
                row: i * 3 + 1,
                exit_code: Some(if i % 7 == 0 { 1 } else { 0 }),
            });
        }
        let blocks = group_command_blocks(&marks);
        assert_eq!(blocks.len(), 100);
        for (i, b) in blocks.iter().enumerate() {
            assert!(b.is_complete());
            if i % 7 == 0 {
                assert!(b.is_failure());
            } else {
                assert!(b.is_success());
            }
        }
    }

    // ── Mouse mode tracking ──────────────────────────────────────────

    #[test]
    fn t_mouse_tracking_mode_1000() {
        let mut t = Terminal::new(80, 24);
        assert!(!t.mouse_tracking_enabled());
        feed(&mut t, b"\x1b[?1000h");
        assert!(t.mouse_tracking_enabled());
        assert!(t.modes.mouse_tracking);
        feed(&mut t, b"\x1b[?1000l");
        assert!(!t.mouse_tracking_enabled());
    }

    #[test]
    fn t_mouse_tracking_mode_9() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?9h");
        assert!(t.modes.mouse_tracking);
        assert!(t.mouse_tracking_enabled());
    }

    #[test]
    fn t_mouse_button_event_mode_1002() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?1002h");
        assert!(t.modes.mouse_button_event);
        assert!(t.mouse_tracking_enabled());
        assert!(t.mouse_button_event_enabled());
        feed(&mut t, b"\x1b[?1002l");
        assert!(!t.mouse_button_event_enabled());
    }

    #[test]
    fn t_mouse_any_event_mode_1003() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?1003h");
        assert!(t.modes.mouse_any_event);
        assert!(t.mouse_tracking_enabled());
        assert!(t.mouse_any_event_enabled());
        feed(&mut t, b"\x1b[?1003l");
        assert!(!t.mouse_any_event_enabled());
    }

    #[test]
    fn t_mouse_sgr_mode_1006() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?1006h");
        assert!(t.mouse_sgr_enabled());
        feed(&mut t, b"\x1b[?1006l");
        assert!(!t.mouse_sgr_enabled());
    }

    #[test]
    fn t_mouse_urxvt_mode_1015() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?1015h");
        assert!(t.mouse_urxvt_enabled());
        feed(&mut t, b"\x1b[?1015l");
        assert!(!t.mouse_urxvt_enabled());
    }

    #[test]
    fn t_mouse_utf8_mode_1005() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?1005h");
        assert!(t.modes.mouse_utf8);
        feed(&mut t, b"\x1b[?1005l");
        assert!(!t.modes.mouse_utf8);
    }
}
