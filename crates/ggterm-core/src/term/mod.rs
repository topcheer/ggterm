//! Terminal state machine.
//!
//! The [`Terminal`] struct implements the [`Perform`] trait, receiving
//! parsed VT/ANSI sequences from the VTE parser and applying them to
//! the [`Grid`] model. It manages cursor position, text attributes,
//! terminal modes, scroll regions, and tab stops.

use crate::grid::{Cell, CellFlags, Color, Grid};
use crate::vte::Perform;
use std::collections::HashMap;
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

/// State saved/restored by DECSC/DECRC (ESC 7 / ESC 8).
///
/// Per the VT220/xterm specification, DECSC saves:
/// - Cursor position (x, y) and pending wrap
/// - Current SGR attributes (fg, bg, underline color, flags)
/// - Character set designation (G0, G1, active set)
/// - Autowrap (DECAWM) mode
/// - Origin (DECOM) mode
/// - Character protection (DECSCA) attribute
#[derive(Debug, Clone, Copy)]
pub(crate) struct DecscState {
    pub(crate) cursor: Cursor,
    pub(crate) fg: Color,
    pub(crate) bg: Color,
    pub(crate) underline_color: Color,
    pub(crate) flags: CellFlags,
    pub(crate) g0_charset: Charset,
    pub(crate) g1_charset: Charset,
    pub(crate) active_g1: bool,
    pub(crate) auto_wrap: bool,
    pub(crate) origin: bool,
    pub(crate) protected_attr: bool,
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

    /// Number of lines of output produced by this command.
    /// Returns `None` if the command hasn't finished or produced no output.
    pub fn output_line_count(&self) -> Option<usize> {
        let output = self.output_row?;
        let end = self.end_row?;
        if end > output {
            Some(end - output)
        } else {
            Some(0)
        }
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
                if let Some(b) = current.take() {
                    blocks.push(b);
                }
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
    /// SGR pixel mouse formatting (DECSET 1016).
    /// Like SGR (1006) but reports pixel coordinates instead of cell coords.
    pub mouse_sgr_pixel: bool,
    /// Focus event reporting (DECSET 1004) — P12-D.
    pub focus_event: bool,
    /// Synchronized output mode (DECSET 2026) — P24-A.
    /// When enabled, the terminal should defer rendering until disabled.
    pub synchronized_output: bool,
    /// Text reflow on resize (DECSET 2027) — P24-B.
    /// When enabled, content reflows when the terminal is resized.
    /// Default: true.
    pub reflow: bool,
    /// DECSET 7727 — alternate scroll mode.
    /// When in the alternate screen and mouse tracking is off, mouse wheel
    /// events are converted to Up/Down arrow key sequences so the user can
    /// scroll in full-screen apps (less, man, vim) without mouse mode.
    /// Default: true (matches xterm).
    pub alternate_scroll: bool,
    /// DECPAM — keypad application mode (ESC =).
    /// When enabled, numeric keypad keys send SS3 sequences instead of digits.
    pub keypad_app: bool,
    /// DECSET 12 — cursor blink attribute.
    /// Programs can control whether the cursor should blink.
    pub cursor_blink: bool,
    /// DECSET 5 — DECSCNM screen mode (reverse video).
    /// When enabled, foreground and background colors are swapped.
    pub reverse_video: bool,
    /// modifyOtherKeys — xterm enhanced keyboard protocol.
    /// 0 = disabled, 1 = mode 1, 2 = mode 2.
    pub modify_other_keys: u8,
    /// LNM — Line Feed/New Line Mode (ANSI mode 20).
    /// When enabled, LF/VT/FF also produce a carriage return (CR+LF behavior).
    pub new_line_mode: bool,
    /// Kitty keyboard protocol active flags.
    /// Bit 0 = disambiguate escape keys
    /// Bit 1 = report event types
    /// Bit 2 = report alternate keys
    /// Bit 3 = report all keys as escapes
    pub kitty_keyboard: u32,
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
            mouse_sgr_pixel: false,
            focus_event: false,
            synchronized_output: false,
            reflow: true,
            alternate_scroll: true,
            keypad_app: false,
            cursor_blink: true,
            reverse_video: false,
            modify_other_keys: 0,
            kitty_keyboard: 0,
            new_line_mode: false,
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
    /// Saved cursor (for alt-screen swap only).
    pub(crate) saved_cursor: Cursor,
    /// Full saved state for DECSC/DECRC (ESC 7 / ESC 8).
    /// Per xterm spec, DECSC saves cursor position, SGR attributes,
    /// character set designation, and autowrap flag.
    pub(crate) decsc_state: Option<DecscState>,
    /// Terminal mode flags.
    pub(crate) modes: Modes,
    /// Current foreground colour.
    pub(crate) fg: Color,
    /// Current background colour.
    pub(crate) bg: Color,
    /// Current underline colour (SGR 58; set to Default by SGR 59).
    pub(crate) underline_color: Color,
    /// Current cell flags (bold, italic, underline, ...).
    pub(crate) flags: CellFlags,
    /// Tab stop positions (one bool per column).
    pub(crate) tab_stops: Vec<bool>,
    /// OSC 133 command marks accumulated from shell integration.
    pub(crate) command_marks: Vec<CommandMark>,
    /// Terminal title (set via OSC 0/2).
    pub(crate) title: String,
    /// Title stack for CSI 22t/23t (push/pop title).
    pub(crate) title_stack: Vec<String>,
    /// Kitty keyboard protocol flag stack (for push/pop via CSI > u / CSI < u).
    pub(crate) kitty_kb_stack: Vec<u32>,
    /// User variables from OSC 1337 SetUserVar (tmux integration).
    pub(crate) user_vars: std::collections::HashMap<String, String>,
    /// Progress report from OSC 9;4 (iTerm2 / xterm extension).
    /// Value 0.0–1.0 represents task progress; None = no progress bar.
    pub(crate) progress: Option<f32>,
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
    /// True when a program queried the clipboard via OSC 52 (?).
    /// The window layer should respond with the clipboard contents.
    pub(crate) pending_clipboard_query: bool,
    /// Current OSC 8 hyperlink URI (applied to new cells in put_printable_char).
    pub(crate) current_hyperlink: Option<String>,
    /// Bell flag — set when BEL (0x07) is received (P11-E).
    pub(crate) bell: bool,
    /// Saved primary grid for alt-screen swap (P15-A).
    /// When alt-screen is activated, the primary grid is saved here
    /// and a fresh grid is installed. On exit, the primary grid is restored.
    pub(crate) alt_saved_grid: Option<Grid>,
    /// Saved DECSC state for alt-screen swap (P15-A).
    /// Used by DECSET 1049 which saves/restores full cursor+SGR state.
    pub(crate) alt_saved_state: Option<DecscState>,
    /// Saved tab stops for alt-screen swap.
    /// Alt screen gets default tab stops; primary stops are restored on exit.
    pub(crate) alt_saved_tab_stops: Option<Vec<bool>>,
    /// Dynamic foreground color set via OSC 10 (P17-A).
    /// When set, overrides the theme default foreground.
    pub(crate) dynamic_fg: Option<Color>,
    /// Dynamic background color set via OSC 11 (P17-A).
    /// When set, overrides the theme default background.
    pub(crate) dynamic_bg: Option<Color>,
    /// Dynamic cursor color set via OSC 12.
    pub(crate) dynamic_cursor: Option<Color>,
    /// Current working directory set via OSC 7 (P22-D).
    /// Format: `OSC 7 ; file://hostname/path ST`
    pub(crate) cwd: Option<std::path::PathBuf>,
    /// DECSCA protected attribute (P24-D).
    /// When true, newly printed characters get the PROTECTED flag.
    pub(crate) protected_attr: bool,
    /// Pending desktop notification from OSC 9/777 (P24-E).
    /// (title, body) pair. Consumed by the event loop.
    pub(crate) pending_notification: Option<(String, String)>,
    /// Remote SSH host (from OSC 1337 RemoteHost=).
    pub(crate) remote_host: Option<String>,
    /// Scrollback mark row (from OSC 1337 SetMark).
    pub(crate) mark_row: Option<usize>,
    /// Custom palette overrides set via OSC 4.
    /// Maps color index → (R, G, B). Programs like base16-shell use this
    /// to change the terminal's color scheme.
    pub(crate) palette_overrides: HashMap<u8, (u8, u8, u8)>,
    /// Real cell dimensions in physical pixels (width, height).
    /// Set by the renderer after font measurement.
    pub(crate) cell_dimensions: Option<(u32, u32)>,
    /// Instant when the current command started (OSC 133;B received).
    /// `None` when no command is running or shell integration is inactive.
    pub(crate) command_start_time: Option<std::time::Instant>,
    /// Duration of the most recently completed command.
    /// `None` when no command has completed yet.
    pub(crate) last_command_duration: Option<std::time::Duration>,
    /// Cached last exit code, cleared on new prompt (PromptStart).
    pub(crate) last_exit_code_cache: Option<i32>,
    /// Instant of the last received output from the PTY.
    /// Used for idle detection in the status bar.
    pub(crate) last_output_time: Option<std::time::Instant>,
}

/// Parse an OSC 7 working directory URI.
///
/// Format: `file://hostname/path`
/// Returns the path component as a `PathBuf`.
/// P22-D: used by OSC 7 handler.
fn parse_osc7_cwd(payload: &str) -> Option<std::path::PathBuf> {
    // Strip the `file://` scheme prefix.
    let rest = payload.strip_prefix("file://")?;
    // Skip the hostname (everything up to the first `/`).
    let idx = rest.find('/')?;
    let path = &rest[idx..];
    // Percent-decode common sequences (%20 → space, etc).
    let decoded = percent_decode(path);
    Some(std::path::PathBuf::from(decoded))
}

/// Minimal percent-decoding for file URIs.
fn percent_decode(input: &str) -> String {
    // Collect decoded bytes first, then convert to String via UTF-8.
    // This correctly handles multi-byte sequences like %E6%A1%8C (CJK).
    let mut bytes: Vec<u8> = Vec::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            let hi = chars.next();
            let lo = chars.next();
            if let (Some(hi), Some(lo)) = (hi, lo) {
                if let Ok(byte) = u8::from_str_radix(&format!("{hi}{lo}"), 16) {
                    bytes.push(byte);
                    continue;
                }
                // Failed decode — keep the original.
                bytes.extend_from_slice(b"%");
                bytes.push(hi as u8);
                bytes.push(lo as u8);
            } else {
                bytes.push(b'%');
            }
        } else {
            // Non-% chars: encode as UTF-8 (handles non-ASCII in path).
            let mut buf = [0u8; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        }
    }
    String::from_utf8(bytes).unwrap_or_default()
}

/// Parse an X11 color specification string into a Color.
/// Format: `rgb:RR/GG/BB` (hex, 1-4 digits per channel).
/// P17-A: used by OSC 10/11/12.
fn parse_xcolor(spec: &str) -> Option<Color> {
    let spec = spec
        .strip_prefix("rgb:")
        .or_else(|| spec.strip_prefix("#"))?;
    let parts: Vec<&str> = spec.split('/').collect();
    if parts.len() == 3 {
        let r = u8::from_str_radix(parts[0], 16).ok()?;
        let g = u8::from_str_radix(parts[1], 16).ok()?;
        let b = u8::from_str_radix(parts[2], 16).ok()?;
        Some(Color::Rgb(r, g, b))
    } else if parts.len() == 1 && spec.len() == 6 {
        // #RRGGBB format
        let r = u8::from_str_radix(&spec[0..2], 16).ok()?;
        let g = u8::from_str_radix(&spec[2..4], 16).ok()?;
        let b = u8::from_str_radix(&spec[4..6], 16).ok()?;
        Some(Color::Rgb(r, g, b))
    } else {
        None
    }
}

/// Lookup the RGB value for a 16-color palette index.
/// P17-A: used by OSC 10/11 query responses.
pub fn color_for_index(idx: u8) -> (u8, u8, u8) {
    match idx {
        0 => (0, 0, 0),        // black
        1 => (205, 0, 0),      // red
        2 => (0, 205, 0),      // green
        3 => (205, 205, 0),    // yellow
        4 => (0, 0, 238),      // blue
        5 => (205, 0, 205),    // magenta
        6 => (0, 205, 205),    // cyan
        7 => (229, 229, 229),  // white
        8 => (127, 127, 127),  // bright black
        9 => (255, 0, 0),      // bright red
        10 => (0, 255, 0),     // bright green
        11 => (255, 255, 0),   // bright yellow
        12 => (92, 92, 255),   // bright blue
        13 => (255, 0, 255),   // bright magenta
        14 => (0, 255, 255),   // bright cyan
        15 => (255, 255, 255), // bright white
        // xterm 216-color cube: indices 16-231
        16..=231 => {
            let cube = [0u8, 95, 135, 175, 215, 255];
            let offset = (idx - 16) as usize;
            let r = cube[offset / 36];
            let g = cube[(offset / 6) % 6];
            let b = cube[offset % 6];
            (r, g, b)
        }
        // Grayscale ramp: indices 232-255 (24 steps from 8 to 238)
        232..=255 => {
            let v = 8 + (idx - 232) * 10;
            (v, v, v)
        }
    }
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
            decsc_state: None,
            modes: Modes::defaults(),
            fg: Color::Default,
            bg: Color::Default,
            underline_color: Color::Default,
            flags: CellFlags::empty(),
            tab_stops,
            command_marks: Vec::new(),
            title: String::new(),
            title_stack: Vec::new(),
            kitty_kb_stack: Vec::new(),
            user_vars: std::collections::HashMap::new(),
            progress: None,
            utf8_buf: Vec::with_capacity(4),
            g0_charset: Charset::default(),
            g1_charset: Charset::default(),
            active_g1: false,
            last_printed_char: None,
            cursor_style: CursorStyle::default(),
            response_buffer: Vec::new(),
            pending_clipboard_set: None,
            pending_clipboard_query: false,
            current_hyperlink: None,
            bell: false,
            alt_saved_grid: None,
            alt_saved_state: None,
            alt_saved_tab_stops: None,
            dynamic_fg: None,
            dynamic_bg: None,
            dynamic_cursor: None,
            cwd: None,
            protected_attr: false,
            pending_notification: None,
            remote_host: None,
            mark_row: None,
            palette_overrides: HashMap::new(),
            cell_dimensions: None,
            command_start_time: None,
            last_command_duration: None,
            last_exit_code_cache: None,
            last_output_time: None,
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

    /// Return true if SGR pixel mouse encoding is active (DECSET 1016).
    pub fn mouse_sgr_pixel_enabled(&self) -> bool {
        self.modes.mouse_sgr_pixel
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

    /// Return true if focus event reporting is active (DECSET 1004) — P12-D.
    pub fn focus_event_enabled(&self) -> bool {
        self.modes.focus_event
    }

    /// Generate focus-in report sequence (P12-D).
    /// Returns `\x1b[I` if focus reporting is enabled, otherwise empty.
    pub fn focus_in_report(&self) -> Vec<u8> {
        if self.modes.focus_event {
            b"\x1b[I".to_vec()
        } else {
            Vec::new()
        }
    }

    /// Generate focus-out report sequence (P12-D).
    /// Returns `\x1b[O` if focus reporting is enabled, otherwise empty.
    pub fn focus_out_report(&self) -> Vec<u8> {
        if self.modes.focus_event {
            b"\x1b[O".to_vec()
        } else {
            Vec::new()
        }
    }

    pub fn cursor_style(&self) -> CursorStyle {
        self.cursor_style
    }

    /// Set the cursor style (used by config to override default).
    pub fn set_cursor_style(&mut self, style: CursorStyle) {
        self.cursor_style = style;
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// Return true if the alternate screen buffer is active (P16-D).
    pub fn is_alt_screen(&self) -> bool {
        self.modes.alt_screen
    }

    /// Return true if synchronized output mode is active (DECSET 2026, P24-A).
    /// When active, the renderer should defer updates until mode is disabled.
    pub fn is_synchronized(&self) -> bool {
        self.modes.synchronized_output
    }

    /// Return true if text reflow on resize is enabled (DECSET 2027, P24-B).
    pub fn reflow_enabled(&self) -> bool {
        self.modes.reflow
    }

    /// Return true if alternate scroll mode is enabled (DECSET 7727).
    /// When in alt screen + no mouse tracking, wheel events become arrow keys.
    pub fn alternate_scroll_enabled(&self) -> bool {
        self.modes.alternate_scroll
    }

    /// Return true if cursor keys are in application mode (DECCKM).
    pub fn cursor_keys_app(&self) -> bool {
        self.modes.cursor_keys_app
    }

    /// Return true if keypad is in application mode (DECPAM).
    pub fn keypad_app(&self) -> bool {
        self.modes.keypad_app
    }

    /// Return the modifyOtherKeys mode (0=off, 1=mode1, 2=mode2).
    pub fn modify_other_keys(&self) -> u8 {
        self.modes.modify_other_keys
    }

    /// Return the active kitty keyboard protocol flags (0 = disabled).
    pub fn kitty_keyboard_flags(&self) -> u32 {
        self.modes.kitty_keyboard
    }

    /// Return true if LNM (Line Feed/New Line Mode) is enabled.
    pub fn new_line_mode(&self) -> bool {
        self.modes.new_line_mode
    }

    /// Get a user variable set via OSC 1337 SetUserVar.
    pub fn user_var(&self, name: &str) -> Option<&str> {
        self.user_vars.get(name).map(|s| s.as_str())
    }

    /// Return the current progress report (0.0–1.0) from OSC 9;4, or None.
    pub fn progress(&self) -> Option<f32> {
        self.progress
    }

    /// Return true if cursor blink is enabled (DECSET 12).
    pub fn cursor_blink_enabled(&self) -> bool {
        self.modes.cursor_blink
    }

    /// Return true if reverse video mode is active (DECSET 5 / DECSCNM).
    pub fn reverse_video(&self) -> bool {
        self.modes.reverse_video
    }

    /// Return a reference to the current underline color (SGR 58).
    pub fn underline_color_ref(&self) -> &Color {
        &self.underline_color
    }

    /// Take and clear a pending desktop notification (P24-E).
    /// Returns (title, body) if OSC 9 or OSC 777 was received.
    pub fn take_pending_notification(&mut self) -> Option<(String, String)> {
        self.pending_notification.take()
    }

    /// Perform a full terminal reset (RIS — ESC c).
    ///
    /// Resets the terminal to its initial state: clears the grid,
    /// resets cursor, modes, attributes, and charset.
    pub fn ris(&mut self) {
        let w = self.grid.width();
        let h = self.grid.height();
        *self = Terminal::new(w, h);
    }

    /// Return the dynamic foreground color if set via OSC 10 (P17-A).
    pub fn dynamic_fg(&self) -> Option<&Color> {
        self.dynamic_fg.as_ref()
    }

    /// Return the dynamic background color if set via OSC 11 (P17-A).
    pub fn dynamic_bg(&self) -> Option<&Color> {
        self.dynamic_bg.as_ref()
    }

    /// Return the dynamic cursor color if set via OSC 12.
    pub fn dynamic_cursor(&self) -> Option<&Color> {
        self.dynamic_cursor.as_ref()
    }

    /// Return the current working directory set via OSC 7 (P22-D).
    pub fn cwd(&self) -> Option<&std::path::Path> {
        self.cwd.as_deref()
    }

    /// Return the remote SSH host (from OSC 1337 RemoteHost=).
    pub fn remote_host(&self) -> Option<&str> {
        self.remote_host.as_deref()
    }

    /// Return the scrollback mark row (from OSC 1337 SetMark).
    pub fn mark_row(&self) -> Option<usize> {
        self.mark_row
    }

    /// Set the real cell dimensions in physical pixels (width, height).
    /// Called by the window layer after font measurement.
    /// Enables accurate CSI 14t/15t/16t pixel-size reports for tmux/nvim.
    pub fn set_cell_dimensions(&mut self, width: u32, height: u32) {
        self.cell_dimensions = Some((width.max(1), height.max(1)));
    }

    /// Return cell dimensions as (width, height) in pixels.
    /// Falls back to (10, 20) when the renderer hasn't provided values.
    fn cell_dims(&self) -> (usize, usize) {
        match self.cell_dimensions {
            Some((w, h)) => (w as usize, h as usize),
            None => (10, 20),
        }
    }

    /// Return the custom palette overrides (OSC 4).
    /// Maps color index → (R, G, B). Used by the renderer to resolve
    /// Color::Indexed values with program-set colors.
    pub fn palette_overrides(&self) -> &HashMap<u8, (u8, u8, u8)> {
        &self.palette_overrides
    }

    /// Resolve a color index to RGB, considering custom palette overrides (OSC 4).
    pub fn resolve_palette_color(&self, idx: u8) -> (u8, u8, u8) {
        self.palette_overrides
            .get(&idx)
            .copied()
            .unwrap_or_else(|| color_for_index(idx))
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
        self.last_exit_code_cache
    }

    /// Return true if the most recent completed command succeeded (exit code 0).
    pub fn last_command_succeeded(&self) -> bool {
        self.last_exit_code() == Some(0)
    }

    /// Returns the duration of the most recently completed command.
    /// `None` if no command has completed or shell integration is inactive.
    pub fn last_command_duration(&self) -> Option<std::time::Duration> {
        self.last_command_duration
    }

    /// Returns the number of output lines from the last completed command.
    /// `None` if no command has completed or shell integration is inactive.
    pub fn last_command_output_lines(&self) -> Option<usize> {
        self.command_blocks()
            .last()
            .filter(|b| b.is_complete())
            .and_then(|b| b.output_line_count())
    }

    /// Extract the text output of the most recent completed command.
    ///
    /// Uses OSC 133 marks to identify the output region (from OutputStart
    /// to CommandEnd). Returns `None` if no completed command exists or
    /// the output region cannot be determined.
    pub fn last_command_output_text(&self) -> Option<String> {
        let block = self.command_blocks().into_iter().last()?;
        if !block.is_complete() {
            return None;
        }
        let start = block.output_row?;
        let end = block.end_row?;
        if start >= end {
            return None;
        }
        let mut lines = Vec::new();
        for row in start..end {
            // extract_row_text already trims trailing whitespace.
            lines.push(self.extract_row_text(row));
        }
        // Remove trailing empty lines.
        while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
            lines.pop();
        }
        Some(lines.join("\n"))
    }

    /// Extract the command text AND its output for the most recent completed command.
    ///
    /// Returns a string like "$ ls -la\nfile1\nfile2\n" — useful for sharing
    /// error reports or command results.
    pub fn last_command_with_output_text(&self) -> Option<String> {
        let block = self.command_blocks().into_iter().last()?;
        if !block.is_complete() {
            return None;
        }
        let cmd_row = block.command_row?;
        let end_row = block.end_row?;
        if cmd_row >= end_row {
            return None;
        }
        let mut lines = Vec::new();
        // Command line (from command_row to output_row)
        let output_row = block.output_row.unwrap_or(cmd_row + 1);
        for row in cmd_row..output_row {
            // extract_row_text already trims trailing whitespace.
            let text = self.extract_row_text(row);
            if !text.is_empty() {
                lines.push(format!("$ {text}"));
            }
        }
        // Output lines
        for row in output_row..end_row {
            // extract_row_text already trims trailing whitespace.
            lines.push(self.extract_row_text(row));
        }
        while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
            lines.pop();
        }
        Some(lines.join("\n"))
    }
    pub fn is_command_running(&self) -> bool {
        self.command_start_time.is_some()
    }

    /// Returns elapsed time of the currently running command.
    /// `None` if no command is running.
    pub fn running_command_elapsed(&self) -> Option<std::time::Duration> {
        self.command_start_time.map(|t| t.elapsed())
    }

    /// Returns the instant of the last received terminal output.
    /// Used for idle detection.
    pub fn last_output_time(&self) -> Option<std::time::Instant> {
        self.last_output_time
    }

    /// Extract the text content of a grid row, trimming trailing spaces.
    ///
    /// Returns an empty string if the row is out of bounds.
    pub fn extract_row_text(&self, row: usize) -> String {
        let width = self.grid.width();
        let mut text = String::with_capacity(width);
        for x in 0..width {
            match self.grid.cell(x, row) {
                Some(cell) => {
                    if cell.flags.contains(CellFlags::WIDE_SPACER) {
                        continue;
                    }
                    text.push(cell.ch);
                    // Append combining characters (zero-width marks like accents)
                    for &mc in &cell.combining {
                        text.push(mc);
                    }
                }
                None => break,
            }
        }
        // Trim trailing whitespace in-place to avoid trim_end().to_string() allocation.
        while text.ends_with(|c: char| c.is_whitespace()) {
            text.pop();
        }
        text
    }

    /// Reset tab stops to default (every 8 columns).
    fn reset_tab_stops(&mut self) {
        let width = self.grid.width();
        self.tab_stops = vec![false; width.max(1)];
        let mut col = 0;
        while col < width {
            self.tab_stops[col] = true;
            col += 8;
        }
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        if self.modes.reflow {
            self.grid.reflow_resize(width, height);
        } else {
            self.grid.resize(width, height);
        }
        // Preserve existing custom tab stops across resize.
        // If wider, extend with default stops at every 8 columns in the new area.
        // If narrower, truncate (custom stops in the clipped area are lost).
        let old_width = self.tab_stops.len();
        if width > old_width {
            self.tab_stops.resize(width, false);
            let mut col = (old_width / 8 + 1) * 8;
            while col < width {
                self.tab_stops[col] = true;
                col += 8;
            }
        } else {
            self.tab_stops.truncate(width.max(1));
        }
        self.cursor.x = self.cursor.x.min(width.saturating_sub(1));
        self.cursor.y = self.cursor.y.min(height.saturating_sub(1));
        self.cursor.pending_wrap = false;
        self.utf8_buf.clear();
    }

    // -- Helpers --

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
        // Update last output time here — once per character, not per byte.
        self.last_output_time = Some(std::time::Instant::now());
        // Fast path for ASCII: width is always 1, skip UnicodeWidthChar lookup.
        let w = if (ch as u32) < 0x80 {
            1
        } else {
            UnicodeWidthChar::width(ch).unwrap_or(1)
        };

        // P17-B: Combining characters (zero-width) are merged into the preceding cell.
        if w == 0 {
            let cx = self.cursor.x;
            let cy = self.cursor.y;
            if cx > 0
                && let Some(c) = self.grid.cell_mut(cx.saturating_sub(1), cy)
                && !c.flags.contains(CellFlags::WIDE_SPACER)
                && !c.is_blank()
            {
                // Cap combining chars to prevent memory exhaustion from
                // sequences that emit many zero-width characters.
                if c.combining.len() < 8 {
                    c.combining.push(ch);
                }
                return;
            }
            if cx == 0 && cy > 0 {
                let prev_w = self.grid.width();
                if let Some(c) = self.grid.cell_mut(prev_w.saturating_sub(1), cy - 1)
                    && !c.flags.contains(CellFlags::WIDE_SPACER)
                    && !c.is_blank()
                {
                    if c.combining.len() < 8 {
                        c.combining.push(ch);
                    }
                    return;
                }
            }
            // No preceding cell to attach to — silently drop.
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
            if self.protected_attr {
                c.flags |= CellFlags::PROTECTED;
            }
            // Only clone hyperlink when active (avoid None allocation).
            if let Some(ref hl) = self.current_hyperlink {
                c.hyperlink = Some(hl.clone());
            } else {
                c.hyperlink = None;
            }
        }
        // For wide chars, set bg on the spacer cell to avoid visual gaps
        if consumed == 2
            && self.cursor.x + 1 < grid_width
            && let Some(c) = self.grid.cell_mut(self.cursor.x + 1, self.cursor.y)
        {
            c.bg = self.bg;
            // Spacer cell: set hyperlink only when active (avoid clone of None).
            if let Some(ref hl) = self.current_hyperlink {
                c.hyperlink = Some(hl.clone());
            } else {
                c.hyperlink = None;
            }
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

    fn line_feed(&mut self) {
        let (top, bottom) = self.grid.scroll_region();
        // Only scroll when cursor is at the bottom of the scroll region.
        // If cursor is below the scroll region, just advance the row.
        if self.cursor.y >= top
            && self.cursor.y < bottom
            && self.cursor.y >= bottom.saturating_sub(1)
        {
            self.grid.scroll_up(1);
        } else {
            self.cursor.y = (self.cursor.y + 1).min(self.grid.height().saturating_sub(1));
        }
        // Always clear pending_wrap on line feed — the cursor has moved
        // to a new line regardless of LNM mode. Without this, bare LF
        // (without CR) when LNM is off would leave pending_wrap=true,
        // causing the next printable char to wrap an extra line.
        self.cursor.pending_wrap = false;
        // LNM (mode 20): LF also performs a carriage return.
        if self.modes.new_line_mode {
            self.cursor.x = 0;
        }
    }

    fn reverse_line_feed(&mut self) {
        let (top, bottom) = self.grid.scroll_region();
        // Only scroll down when cursor is at the top of the scroll region.
        // If cursor is above or outside the scroll region, just move up.
        if self.cursor.y == top && self.cursor.y < bottom {
            self.grid.scroll_down(1);
        } else if self.cursor.y > 0 {
            self.cursor.y -= 1;
        }
        self.cursor.pending_wrap = false;
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
            5 => self.modes.reverse_video = enable, // DECSCNM
            12 => self.modes.cursor_blink = enable,
            25 => self.modes.cursor_visible = enable,
            6 => {
                self.modes.origin = enable;
                self.set_cursor(0, 0);
            }
            1 => self.modes.cursor_keys_app = enable,
            2004 => self.modes.bracketed_paste = enable,
            // Alt-screen modes — P15-A: properly save/restore grid
            47 | 1047 => {
                if enable && !self.modes.alt_screen {
                    // Enter alt-screen: save primary grid + tab stops
                    self.alt_saved_grid = Some(self.grid.clone());
                    self.alt_saved_tab_stops = Some(self.tab_stops.clone());
                    self.grid = Grid::new(self.width(), self.height());
                    self.reset_tab_stops();
                    if mode == 1047 {
                        // 1047: clear the alt screen (already fresh)
                    }
                    self.modes.alt_screen = true;
                } else if !enable && self.modes.alt_screen {
                    // Exit alt-screen: restore primary grid + tab stops
                    if let Some(saved) = self.alt_saved_grid.take() {
                        self.grid = saved;
                    }
                    if let Some(stops) = self.alt_saved_tab_stops.take() {
                        self.tab_stops = stops;
                    }
                    self.modes.alt_screen = false;
                }
            }
            1049 => {
                if enable && !self.modes.alt_screen {
                    // Enter alt-screen: save cursor, grid, tab stops
                    self.alt_saved_state = Some(DecscState {
                        cursor: self.cursor,
                        fg: self.fg,
                        bg: self.bg,
                        underline_color: self.underline_color,
                        flags: self.flags,
                        g0_charset: self.g0_charset,
                        g1_charset: self.g1_charset,
                        active_g1: self.active_g1,
                        auto_wrap: self.modes.auto_wrap,
                        origin: self.modes.origin,
                        protected_attr: self.protected_attr,
                    });
                    self.alt_saved_grid = Some(self.grid.clone());
                    self.alt_saved_tab_stops = Some(self.tab_stops.clone());
                    self.grid = Grid::new(self.width(), self.height());
                    self.reset_tab_stops();
                    self.cursor = Cursor::default();
                    self.modes.alt_screen = true;
                } else if !enable && self.modes.alt_screen {
                    // Exit alt-screen: restore grid, cursor, tab stops
                    if let Some(saved) = self.alt_saved_grid.take() {
                        self.grid = saved;
                    }
                    if let Some(stops) = self.alt_saved_tab_stops.take() {
                        self.tab_stops = stops;
                    }
                    if let Some(state) = self.alt_saved_state.take() {
                        self.cursor = state.cursor;
                        self.fg = state.fg;
                        self.bg = state.bg;
                        self.underline_color = state.underline_color;
                        self.flags = state.flags;
                        self.g0_charset = state.g0_charset;
                        self.g1_charset = state.g1_charset;
                        self.active_g1 = state.active_g1;
                        self.modes.auto_wrap = state.auto_wrap;
                        self.modes.origin = state.origin;
                        self.protected_attr = state.protected_attr;
                    }
                    self.modes.alt_screen = false;
                }
            }
            // Mouse tracking modes
            9 => self.modes.mouse_tracking = enable, // X10
            1000 => self.modes.mouse_tracking = enable, // Normal
            1002 => self.modes.mouse_button_event = enable, // Button-event
            1003 => self.modes.mouse_any_event = enable, // Any-motion
            1005 => self.modes.mouse_utf8 = enable,  // UTF-8 encoding
            1006 => self.modes.mouse_sgr = enable,   // SGR encoding
            1015 => self.modes.mouse_urxvt = enable, // URXVT encoding
            1016 => self.modes.mouse_sgr_pixel = enable, // SGR pixel encoding
            1004 => self.modes.focus_event = enable, // Focus event reporting
            2026 => self.modes.synchronized_output = enable, // Synchronized output
            2027 => self.modes.reflow = enable,      // Text reflow on resize
            7727 => self.modes.alternate_scroll = enable, // Alternate scroll
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
                    self.underline_color = Color::Default;
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
                // SGR 21 — doubly underlined (xterm). Equivalent to SGR 4:2.
                21 => self.flags |= CellFlags::UNDERLINE | CellFlags::UNDERLINE_DOUBLE,
                22 => self.flags &= !(CellFlags::BOLD | CellFlags::DIM),
                23 => self.flags &= !CellFlags::ITALIC,
                24 => {
                    self.flags &= !CellFlags::UNDERLINE;
                    self.flags &= !(CellFlags::UNDERLINE_DOUBLE
                        | CellFlags::UNDERLINE_CURLY
                        | CellFlags::UNDERLINE_DOTTED
                        | CellFlags::UNDERLINE_DASHED);
                }
                25 => self.flags &= !CellFlags::BLINK,
                27 => self.flags &= !CellFlags::REVERSE,
                28 => self.flags &= !CellFlags::HIDDEN,
                29 => self.flags &= !CellFlags::STRIKETHROUGH,
                // SGR 53 — overline on. SGR 55 — overline off.
                53 => self.flags |= CellFlags::OVERLINE,
                55 => self.flags &= !CellFlags::OVERLINE,
                30..=37 => self.fg = Color::Indexed((p - 30) as u8),
                39 => self.fg = Color::Default,
                40..=47 => self.bg = Color::Indexed((p - 40) as u8),
                49 => self.bg = Color::Default,
                59 => self.underline_color = Color::Default,
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
                // SGR 58 — set underline color (extended: 5 = palette, 2 = RGB)
                58 => {
                    match (
                        params.get(i + 1).copied(),
                        i + 2 < params.len(),
                        i + 4 < params.len(),
                    ) {
                        (Some(5), true, _) => {
                            self.underline_color = Color::Indexed(params[i + 2] as u8);
                            i += 2;
                        }
                        (Some(2), _, true) => {
                            self.underline_color = Color::Rgb(
                                params[i + 2] as u8,
                                params[i + 3] as u8,
                                params[i + 4] as u8,
                            );
                            i += 4;
                        }
                        _ => {}
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

    /// Check and clear the OSC 52 clipboard query flag.
    ///
    /// Returns true if a program queried the clipboard via `OSC 52;?c`.
    /// The window layer should respond with `OSC 52;c;<base64> ST`.
    pub fn take_pending_clipboard_query(&mut self) -> bool {
        std::mem::take(&mut self.pending_clipboard_query)
    }

    /// Take the bell flag (P11-E).
    ///
    /// Returns `true` if a BEL (0x07) was received since the last call.
    /// The app layer calls this in `about_to_wait` to trigger visual bell.
    pub fn take_bell(&mut self) -> bool {
        std::mem::replace(&mut self.bell, false)
    }

    // ---------- P24-D: Selective erase helpers ----------

    /// Erase non-protected cells from cursor position to end of screen.
    fn selective_erase_from(&mut self, col: usize, row: usize) {
        let width = self.grid.width();
        let height = self.grid.height();
        // Erase from cursor to end of current row
        for c in col..width {
            if let Some(cell) = self.grid.cell_mut(c, row)
                && !cell.flags.contains(CellFlags::PROTECTED)
            {
                *cell = Cell::blank();
            }
        }
        // Erase all subsequent rows
        for r in (row + 1)..height {
            for c in 0..width {
                if let Some(cell) = self.grid.cell_mut(c, r)
                    && !cell.flags.contains(CellFlags::PROTECTED)
                {
                    *cell = Cell::blank();
                }
            }
        }
    }

    /// Erase non-protected cells from start of screen to cursor position.
    fn selective_erase_to(&mut self, col: usize, row: usize) {
        let width = self.grid.width();
        // Erase all rows before cursor row
        for r in 0..row {
            for c in 0..width {
                if let Some(cell) = self.grid.cell_mut(c, r)
                    && !cell.flags.contains(CellFlags::PROTECTED)
                {
                    *cell = Cell::blank();
                }
            }
        }
        // Erase from start of current row to cursor (inclusive)
        for c in 0..=col.min(width.saturating_sub(1)) {
            if let Some(cell) = self.grid.cell_mut(c, row)
                && !cell.flags.contains(CellFlags::PROTECTED)
            {
                *cell = Cell::blank();
            }
        }
    }

    /// Erase all non-protected cells on the screen.
    fn selective_erase_all(&mut self) {
        let width = self.grid.width();
        let height = self.grid.height();
        for r in 0..height {
            for c in 0..width {
                if let Some(cell) = self.grid.cell_mut(c, r)
                    && !cell.flags.contains(CellFlags::PROTECTED)
                {
                    *cell = Cell::blank();
                }
            }
        }
    }

    /// Simple base64 decoder for OSC 52 payloads.
    fn decode_base64(input: &str) -> Vec<u8> {
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
        out
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

/// Hex-encode bytes as lowercase hex string (for XTGETTCAP).
fn hex_encode(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for b in data {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Hex-decode a hex string to bytes (for XTGETTCAP).
fn hex_decode(data: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(data).ok()?;
    if s.len() % 2 != 0 {
        return None;
    }
    let bytes: Result<Vec<u8>, _> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16))
        .collect();
    String::from_utf8(bytes.ok()?).ok()
}

impl Perform for Terminal {
    fn print(&mut self, byte: u8) {
        // Defer Instant::now() to flush_utf8 to avoid per-byte syscall
        // in high-throughput scenarios.
        if byte < 0x80 {
            self.flush_utf8();
            self.put_printable_char(byte as char);
            return;
        }
        // Flush pending incomplete sequence when a new leading byte arrives
        if !self.utf8_buf.is_empty() && byte >= 0xC0 {
            self.flush_utf8();
        }
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
            0x05 => {
                // ENQ — transmit answerback message.
                // Respond with a terminal identification string.
                self.response_buffer.extend_from_slice(b"ggterm");
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
                // CUU stops at scroll region top when cursor is inside it.
                // When cursor is above the scroll region, stops at row 0.
                self.cursor.y = if self.cursor.y >= top {
                    self.cursor.y.saturating_sub(n).max(top)
                } else {
                    self.cursor.y.saturating_sub(n)
                };
                self.cursor.pending_wrap = false;
            }
            b'B' => {
                let n = Self::param(params, 0, 1) as usize;
                let (_, bottom) = self.grid.scroll_region();
                // CUD stops at scroll region bottom when cursor is inside it.
                // When cursor is below the scroll region, stops at last row.
                self.cursor.y = if self.cursor.y < bottom {
                    (self.cursor.y + n).min(bottom.saturating_sub(1))
                } else {
                    (self.cursor.y + n).min(self.grid.height().saturating_sub(1))
                };
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
                // CNL stops at scroll region bottom when cursor is inside it.
                // When cursor is below the scroll region, stops at last row.
                self.cursor.y = if self.cursor.y < bottom {
                    (self.cursor.y + n).min(bottom.saturating_sub(1))
                } else {
                    (self.cursor.y + n).min(self.grid.height().saturating_sub(1))
                };
                self.cursor.x = 0;
                self.cursor.pending_wrap = false;
            }
            b'F' => {
                let n = Self::param(params, 0, 1) as usize;
                let (top, _) = self.grid.scroll_region();
                // CPL stops at scroll region top when cursor is inside it.
                // When cursor is above the scroll region, stops at row 0.
                self.cursor.y = if self.cursor.y >= top {
                    self.cursor.y.saturating_sub(n).max(top)
                } else {
                    self.cursor.y.saturating_sub(n)
                };
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
                // Origin mode: CUP is relative to scroll region top,
                // and cursor is clamped to the scroll region.
                let actual_row = if self.modes.origin {
                    let (top, bottom) = self.grid.scroll_region();
                    (top + row.saturating_sub(1)).min(bottom.saturating_sub(1))
                } else {
                    row.saturating_sub(1)
                };
                self.set_cursor(col.saturating_sub(1), actual_row);
            }
            b'd' => {
                let row = Self::param(params, 0, 1) as usize;
                // Origin mode: VPA is relative to scroll region top,
                // and cursor is clamped to the scroll region.
                let actual_row = if self.modes.origin {
                    let (top, bottom) = self.grid.scroll_region();
                    (top + row.saturating_sub(1)).min(bottom.saturating_sub(1))
                } else {
                    row.saturating_sub(1)
                };
                self.set_cursor(self.cursor.x, actual_row);
            }
            // DECSED — selective erase in display (CSI ? Ps J) (P24-D)
            // Must come BEFORE regular ED to take priority when `?` prefix is present.
            b'J' if is_private => {
                let mode = params.first().copied().unwrap_or(0);
                match mode {
                    0 => self.selective_erase_from(self.cursor.x, self.cursor.y),
                    1 => self.selective_erase_to(self.cursor.x, self.cursor.y),
                    2 => self.selective_erase_all(),
                    _ => {}
                }
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
                        // xterm: CSI 3J clears scrollback only.
                        // Do NOT clear the visible screen.
                        self.grid.clear_scrollback();
                        self.grid.reset_viewport();
                    }
                    _ => {}
                }
            }
            // DECSEL — selective erase in line (CSI ? Ps K)
            // Must come BEFORE regular EL to take priority when `?` prefix is present.
            b'K' if is_private => {
                let mode = params.first().copied().unwrap_or(0);
                let width = self.grid.width();
                let (cx, cy) = (self.cursor.x, self.cursor.y);
                match mode {
                    0 => {
                        // Erase from cursor to end of line (non-protected only)
                        for c in cx..width {
                            if let Some(cell) = self.grid.cell_mut(c, cy)
                                && !cell.flags.contains(CellFlags::PROTECTED)
                            {
                                *cell = Cell::blank();
                            }
                        }
                    }
                    1 => {
                        // Erase from start of line to cursor (non-protected only)
                        for c in 0..=cx.min(width.saturating_sub(1)) {
                            if let Some(cell) = self.grid.cell_mut(c, cy)
                                && !cell.flags.contains(CellFlags::PROTECTED)
                            {
                                *cell = Cell::blank();
                            }
                        }
                    }
                    2 => {
                        // Erase entire line (non-protected only)
                        for c in 0..width {
                            if let Some(cell) = self.grid.cell_mut(c, cy)
                                && !cell.flags.contains(CellFlags::PROTECTED)
                            {
                                *cell = Cell::blank();
                            }
                        }
                    }
                    _ => {}
                }
                self.grid_mut().mark_row_dirty(cy);
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
                    }
                    // Per VT spec, DECSTBM always homes the cursor,
                    // even when parameters are invalid and region is unchanged.
                    let (st, _) = self.grid.scroll_region();
                    self.set_cursor(0, if self.modes.origin { st } else { 0 });
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
                    0 if self.cursor.x < self.tab_stops.len() => {
                        self.tab_stops[self.cursor.x] = false;
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
            // modifyOtherKeys: CSI > 4 ; Nm h / CSI > 4 ; Nm l
            b'h' if intermediates.contains(&b'>') => {
                let m = params.first().copied().unwrap_or(0);
                if m == 4 {
                    self.modes.modify_other_keys = params.get(1).copied().unwrap_or(1) as u8;
                }
            }
            b'l' if intermediates.contains(&b'>') => {
                let m = params.first().copied().unwrap_or(0);
                if m == 4 {
                    self.modes.modify_other_keys = 0;
                }
            }
            b'h' => {
                let m = params.first().copied().unwrap_or(0);
                if m == 4 {
                    self.modes.insert = true;
                } else if m == 20 {
                    self.modes.new_line_mode = true;
                }
            }
            b'l' => {
                let m = params.first().copied().unwrap_or(0);
                if m == 4 {
                    self.modes.insert = false;
                } else if m == 20 {
                    self.modes.new_line_mode = false;
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
            b'c' if !intermediates.contains(&b'>') && !intermediates.contains(&b'=') => {
                // Respond: CSI ? 62 ; 1 ; 6 ; 9 ; 16 ; 22 ; 29 c
                // VT220-level capabilities — only features we actually support:
                //   62 = VT220, 1 = 132-cols,
                //   6 = selective erase, 9 = national charset,
                //   16 = locator port,
                //   22 = ANSI color, 29 = ANSI text locator (OSC 8 hyperlinks)
                // Removed: 4 (sixel — not rendered), 2 (printer), 15 (DEC tech)
                self.response_buffer
                    .extend_from_slice(b"\x1b[?62;1;6;9;16;22;29c");
            }
            // DA2 — secondary device attributes (CSI > c)
            b'c' if intermediates.contains(&b'>') => {
                // Respond: CSI > 41 ; 0 ; 0 c (VT220)
                self.response_buffer.extend_from_slice(b"\x1b[>41;0;0c");
            }
            // DA3 — tertiary device attributes (CSI = c)
            // Response: DCS ! | <8 hex digits> ST
            // xterm returns the terminal session ID as 8 hex digits.
            b'c' if intermediates.contains(&b'=') => {
                self.response_buffer
                    .extend_from_slice(b"\x1bP!|00000000\x1b\\");
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
                        // In origin mode, report relative to scroll region top.
                        let (cx, cy) = (self.cursor.x + 1, self.cursor.y + 1);
                        let report_row = if self.modes.origin {
                            let (top, _) = self.grid.scroll_region();
                            cy.saturating_sub(top + 1).max(1)
                        } else {
                            cy
                        };
                        let resp = format!("\x1b[{};{}R", report_row, cx);
                        self.response_buffer.extend_from_slice(resp.as_bytes());
                    }
                    _ => {}
                }
            }
            // Text area size report (CSI Ps t)
            b't' => {
                let mode = params.first().copied().unwrap_or(0);
                match mode {
                    18 => {
                        // Report text area size in characters: CSI 8 ; rows ; cols t
                        let resp = format!("\x1b[8;{};{}t", self.grid.height(), self.grid.width());
                        self.response_buffer.extend_from_slice(resp.as_bytes());
                    }
                    19 => {
                        // Report screen size in characters: CSI 9 ; rows ; cols t
                        // We don't know actual screen size, report terminal size.
                        let resp = format!("\x1b[9;{};{}t", self.grid.height(), self.grid.width());
                        self.response_buffer.extend_from_slice(resp.as_bytes());
                    }
                    14 => {
                        // Report text area size in pixels: CSI 4 ; height ; width t
                        // We don't know the actual pixel size from the terminal model,
                        // so estimate based on a standard cell size.
                        let (cw, ch) = self.cell_dims();
                        let h = self.grid.height() * ch;
                        let w = self.grid.width() * cw;
                        let resp = format!("\x1b[4;{};{}t", h, w);
                        self.response_buffer.extend_from_slice(resp.as_bytes());
                    }
                    11 => {
                        // Report window iconified state: CSI 1 t (not iconified).
                        // xterm extension — programs query window visibility.
                        self.response_buffer.extend_from_slice(b"\x1b[1t");
                    }
                    13 => {
                        // Report window position: CSI 3 ; x ; y t
                        // We don't track real position, report (0,0).
                        self.response_buffer.extend_from_slice(b"\x1b[3;0;0t");
                    }
                    15 => {
                        // Report screen size in pixels: CSI 5 ; height ; width t
                        // Estimate from grid + standard cell size.
                        let (cw, ch) = self.cell_dims();
                        let h = self.grid.height() * ch;
                        let w = self.grid.width() * cw;
                        let resp = format!("\x1b[5;{};{}t", h, w);
                        self.response_buffer.extend_from_slice(resp.as_bytes());
                    }
                    16 => {
                        // Report character cell size in pixels.
                        // Response: CSI 6 ; cell_height ; cell_width t
                        // We use approximate standard cell dimensions.
                        let (cw, ch) = self.cell_dims();
                        let resp = format!("\x1b[6;{};{}t", ch, cw);
                        self.response_buffer.extend_from_slice(resp.as_bytes());
                    }
                    22 => {
                        // Push title onto stack (xterm windowops).
                        // Param 2 = icon title, param 1 = window title.
                        // We only track one title, so save it regardless of param.
                        let kind = params.get(1).copied().unwrap_or(0);
                        if kind == 0 || kind == 2 || kind == 1 {
                            self.title_stack.push(self.title.clone());
                            // Prevent unbounded growth (malicious programs).
                            if self.title_stack.len() > 100 {
                                self.title_stack.remove(0);
                            }
                        }
                    }
                    23 => {
                        // Pop title from stack (xterm windowops).
                        let kind = params.get(1).copied().unwrap_or(0);
                        if (kind == 0 || kind == 2 || kind == 1)
                            && let Some(popped) = self.title_stack.pop()
                        {
                            self.title = popped;
                        }
                    }
                    21 => {
                        // Report window title: OSC l <title> ST
                        // xterm windowops — tmux queries this to detect the
                        // terminal's title for session naming.
                        let resp = format!("\x1b]l{}\x1b\\", self.title);
                        self.response_buffer.extend_from_slice(resp.as_bytes());
                    }
                    _ => {}
                }
            }
            // SCP — save cursor position (legacy ANSI.SYS)
            b's' => {
                self.saved_cursor = self.cursor;
            }
            // Kitty keyboard protocol: push flags (CSI > Ps u)
            // Saves current flags onto an internal stack and ORs the new flags.
            b'u' if intermediates.contains(&b'>') => {
                let new_flags = params.first().copied().unwrap_or(0) as u32;
                self.kitty_kb_stack.push(self.modes.kitty_keyboard);
                // Prevent unbounded growth (malicious programs).
                if self.kitty_kb_stack.len() > 100 {
                    self.kitty_kb_stack.remove(0);
                }
                self.modes.kitty_keyboard |= new_flags;
            }
            // Kitty keyboard protocol: pop flags (CSI < Ps u)
            // Restores the previous flags from the stack (N times).
            b'u' if intermediates.contains(&b'<') => {
                let count = params.first().copied().unwrap_or(1) as usize;
                for _ in 0..count {
                    if let Some(prev) = self.kitty_kb_stack.pop() {
                        self.modes.kitty_keyboard = prev;
                    } else {
                        self.modes.kitty_keyboard = 0;
                        break;
                    }
                }
            }
            // Kitty keyboard protocol: set/report flags (CSI = Ps ; Pu u)
            // Ps = 1: set flags to Pu. Ps = 2: query current flags.
            b'u' if intermediates.contains(&b'=') => {
                let action = params.first().copied().unwrap_or(0);
                match action {
                    1 => {
                        self.modes.kitty_keyboard = params.get(1).copied().unwrap_or(0) as u32;
                    }
                    2 => {
                        // Report current flags: CSI ? flags u
                        let resp = format!("\x1b[?{}u", self.modes.kitty_keyboard);
                        self.response_buffer.extend_from_slice(resp.as_bytes());
                    }
                    _ => {}
                }
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
            // Resets SGR attributes, cursor position, scroll region, and modes
            // but preserves scrollback, grid content, and terminal size.
            b'p' if intermediates.contains(&b'!') => {
                // Reset cursor
                self.cursor = Cursor::default();
                // Reset SGR attributes
                self.fg = Color::Default;
                self.bg = Color::Default;
                self.underline_color = Color::Default;
                self.flags = CellFlags::empty();
                self.protected_attr = false;
                // Reset character set
                self.g0_charset = Charset::Ascii;
                self.g1_charset = Charset::Ascii;
                self.active_g1 = false;
                // Reset scroll region to full screen
                self.grid_mut().reset_scroll_region();
                // Reset modes (but preserve alt_screen, mouse modes)
                self.modes.auto_wrap = true;
                self.modes.cursor_visible = true;
                self.modes.origin = false;
                self.modes.cursor_keys_app = false;
                self.modes.insert = false;
                self.modes.bracketed_paste = false;
                self.modes.new_line_mode = false;
                self.modes.keypad_app = false;
                self.modes.synchronized_output = false;
                self.modes.reflow = true;
                self.modes.focus_event = false;
                // Reset tab stops
                let width = self.grid.width();
                self.tab_stops = vec![false; width.max(1)];
                let mut col = 0;
                while col < width {
                    self.tab_stops[col] = true;
                    col += 8;
                }
                // Reset hyperlinks
                self.current_hyperlink = None;
                // Clear partial UTF-8 sequence and REP tracking
                self.utf8_buf.clear();
                self.last_printed_char = None;
                // Reset dynamic colors
                self.dynamic_fg = None;
                self.dynamic_bg = None;
                self.dynamic_cursor = None;
                self.palette_overrides.clear();
                self.grid_mut().mark_all_dirty();
            }
            // DECSCA — select character protection attribute (CSI " Ps q)
            // 0 = unprotected (default), 1 = protected, 2 = unprotected (same as 0)
            b'q' if intermediates.contains(&b'"') => {
                let mode = params.first().copied().unwrap_or(0);
                self.protected_attr = mode == 1;
            }
            // XTVERSION — query terminal identification (CSI > Ps q)
            // Programs like tmux use this to detect the terminal type.
            // We respond: DCS >| ggterm(<version>) ST
            b'q' if intermediates.contains(&b'>') => {
                let resp = format!("\x1bP>|ggterm({})\x1b\\", env!("CARGO_PKG_VERSION"));
                self.response_buffer.extend_from_slice(resp.as_bytes());
            }
            // DECRQM — request mode (CSI ? Pm $ p for DEC private modes)
            // Programs query whether a mode is set. We respond with:
            // CSI ? Pm ; Ps $ y  where Ps: 0=not recognized, 1=set, 2=reset, 3=permanently set, 4=permanently reset
            b'p' if intermediates.contains(&b'$') && is_private => {
                let mode = params.first().copied().unwrap_or(0);
                let is_set = match mode {
                    1 => self.modes.cursor_keys_app,        // DECCKM
                    5 => self.modes.reverse_video,          // DECSCNM
                    6 => self.modes.origin,                 // DECOM
                    7 => self.modes.auto_wrap,              // DECAWM
                    12 => self.modes.cursor_blink,          // Cursor blink
                    25 => self.modes.cursor_visible,        // DECTCEM
                    47 => self.modes.alt_screen,            // Alt screen (47)
                    45 => false, // DECRIVM: reverse wraparound (not supported)
                    9 => self.modes.mouse_tracking, // X10 mouse tracking
                    1000 => self.modes.mouse_tracking, // Mouse tracking
                    1002 => self.modes.mouse_button_event, // Button-event mouse
                    1003 => self.modes.mouse_any_event, // Any-event mouse
                    1004 => self.modes.focus_event, // Focus event reporting
                    1005 => self.modes.mouse_utf8, // UTF-8 mouse
                    1006 => self.modes.mouse_sgr, // SGR mouse
                    1015 => self.modes.mouse_urxvt, // URXVT mouse
                    1016 => self.modes.mouse_sgr_pixel, // SGR-pixel mouse
                    1047 => self.modes.alt_screen, // Alt screen (1047)
                    1049 => self.modes.alt_screen, // Alt screen + cursor save (1049)
                    2004 => self.modes.bracketed_paste, // Bracketed paste
                    2026 => self.modes.synchronized_output, // Synchronized output
                    2027 => self.modes.reflow, // Text reflow
                    7727 => self.modes.alternate_scroll, // Alternate scroll
                    _ => false,
                };
                let status = if is_set { 1 } else { 2 };
                let resp = format!("\x1b[?{};{}$y", mode, status);
                self.response_buffer.extend_from_slice(resp.as_bytes());
            }
            // DECRQM for modifyOtherKeys (CSI > Ps $ p)
            // Must be checked BEFORE the ANSI-mode DECRQM below because the
            // '>' intermediate is not a private '?' marker, so is_private=false.
            b'p' if intermediates.contains(&b'$') && intermediates.contains(&b'>') => {
                let mode = params.first().copied().unwrap_or(0);
                if mode == 4 {
                    let m = self.modes.modify_other_keys;
                    let status: u8 = if m > 0 { 1 } else { 2 }; // 1=set, 2=reset
                    let resp = format!("\x1b[>{mode};{status}$y");
                    self.response_buffer.extend_from_slice(resp.as_bytes());
                } else {
                    let resp = format!("\x1b[>{mode};0$y");
                    self.response_buffer.extend_from_slice(resp.as_bytes());
                }
            }
            // DECRQM for ANSI modes (CSI Ps $ p, no private '?')
            b'p' if intermediates.contains(&b'$') && !is_private => {
                let mode = params.first().copied().unwrap_or(0);
                // status: 0=not recognized, 1=set, 2=reset, 3=permanently set, 4=permanently reset
                let (is_set, permanent) = match mode {
                    4 => (self.modes.insert, false),         // IRM — insert mode
                    7 => (self.modes.auto_wrap, false),      // DECAWM — autowrap
                    12 => (self.modes.cursor_blink, false),  // Cursor blink
                    20 => (self.modes.new_line_mode, false), // LNM — line feed/new line mode
                    8 => (true, true), // ARM — auto-repeat, always on (permanently set)
                    _ => (false, false),
                };
                let status = if permanent {
                    3 // permanently set
                } else if is_set {
                    1 // set
                } else {
                    2 // reset
                };
                let resp = format!("\x1b[{};{}$y", mode, status);
                self.response_buffer.extend_from_slice(resp.as_bytes());
            }
            // DECRQSS fallback (CSI Ps $ q)
            // DECRQSS is properly handled via DCS ($ q) in the dcs() method.
            // This CSI variant can't receive string parameters, so respond
            // "not recognized" — programs use the DCS form.
            b'q' if intermediates.contains(&b'$') => {
                self.response_buffer.extend_from_slice(b"\x1bP0$r\x1b\\");
            }
            // DECREQTPARM — Request Terminal Parameters (CSI Ps x)
            // Programs use this during startup to detect terminal type.
            // Response: CSI 2 ; 1 ; 0 ; 0 ; 0 ; 0 x
            //   2 = respond to request, 1 = no parity, rest = unused
            b'x' => {
                self.response_buffer.extend_from_slice(b"\x1b[2;1;0;0;0;0x");
            }
            _ => {}
        }
    }

    fn csi_with_subs(
        &mut self,
        intermediates: &[u8],
        params: &[u16],
        subs: &[u16],
        final_byte: u8,
    ) {
        // Handle SGR 4:N underline styles when colon syntax is used.
        // With the new parser, `4:3` produces params=[4, 3] with subs=[0, 1].
        // subs[i+1] != 0 means params[i+1] was colon-derived from params[i].
        if final_byte == b'm' && !intermediates.contains(&b'?') && subs.iter().any(|&s| s != 0) {
            let mut handled = false;
            let mut i = 0;
            while i < params.len() {
                let p = params[i];
                // Check if the NEXT param is colon-derived (sub != 0).
                let next_is_colon = subs.get(i + 1).copied().unwrap_or(0) != 0;
                if next_is_colon {
                    let val = params.get(i + 1).copied().unwrap_or(0);
                    match (p, val) {
                        (4, 0) | (4, 1) => {
                            self.flags |= CellFlags::UNDERLINE;
                            self.flags &= !(CellFlags::UNDERLINE_DOUBLE
                                | CellFlags::UNDERLINE_CURLY
                                | CellFlags::UNDERLINE_DOTTED
                                | CellFlags::UNDERLINE_DASHED);
                            handled = true;
                        }
                        (4, 2) => {
                            self.flags |= CellFlags::UNDERLINE | CellFlags::UNDERLINE_DOUBLE;
                            handled = true;
                        }
                        (4, 3) => {
                            self.flags |= CellFlags::UNDERLINE | CellFlags::UNDERLINE_CURLY;
                            handled = true;
                        }
                        (4, 4) => {
                            self.flags |= CellFlags::UNDERLINE | CellFlags::UNDERLINE_DOTTED;
                            handled = true;
                        }
                        (4, 5) => {
                            self.flags |= CellFlags::UNDERLINE | CellFlags::UNDERLINE_DASHED;
                            handled = true;
                        }
                        (24, _) => {
                            self.flags &= !CellFlags::UNDERLINE;
                            self.flags &= !(CellFlags::UNDERLINE_DOUBLE
                                | CellFlags::UNDERLINE_CURLY
                                | CellFlags::UNDERLINE_DOTTED
                                | CellFlags::UNDERLINE_DASHED);
                            handled = true;
                        }
                        _ => {}
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            }
            // Only return early if ALL params were colon-derived underline
            // styles. If there are non-colon params mixed in (e.g. 4:3;31),
            // fall through to csi() so the regular SGR handler processes them.
            if handled {
                // Check if any params were NOT consumed by colon pairs.
                let mut all_colon = true;
                let mut j = 0;
                while j < params.len() {
                    let next_is_colon = subs.get(j + 1).copied().unwrap_or(0) != 0;
                    let curr_is_colon = subs.get(j).copied().unwrap_or(0) != 0;
                    if next_is_colon {
                        j += 2;
                    } else if curr_is_colon {
                        j += 1; // skip colon-derived value
                    } else {
                        // This param is a regular SGR value (e.g. 31=red).
                        all_colon = false;
                        break;
                    }
                }
                if all_colon {
                    return;
                }
                // Fall through to csi() to process remaining regular SGR params.
            }
        }
        // Default: delegate to regular csi()
        self.csi(intermediates, params, final_byte);
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
            b'=' => {
                // DECPAM — keypad application mode
                self.modes.keypad_app = true;
            }
            b'>' => {
                // DECPNM — keypad normal mode
                self.modes.keypad_app = false;
            }
            // DECSC — save cursor and terminal state (ESC 7).
            // Saves: cursor position, pending wrap, SGR attributes,
            // character set designation, autowrap mode.
            b'7' => {
                self.decsc_state = Some(DecscState {
                    cursor: self.cursor,
                    fg: self.fg,
                    bg: self.bg,
                    underline_color: self.underline_color,
                    flags: self.flags,
                    g0_charset: self.g0_charset,
                    g1_charset: self.g1_charset,
                    active_g1: self.active_g1,
                    auto_wrap: self.modes.auto_wrap,
                    origin: self.modes.origin,
                    protected_attr: self.protected_attr,
                });
            }
            // DECRC — restore cursor and terminal state (ESC 8).
            b'8' => {
                if let Some(state) = &self.decsc_state {
                    self.cursor = state.cursor;
                    self.fg = state.fg;
                    self.bg = state.bg;
                    self.underline_color = state.underline_color;
                    self.flags = state.flags;
                    self.g0_charset = state.g0_charset;
                    self.g1_charset = state.g1_charset;
                    self.active_g1 = state.active_g1;
                    self.modes.auto_wrap = state.auto_wrap;
                    self.modes.origin = state.origin;
                    self.protected_attr = state.protected_attr;
                } else {
                    // No saved state — restore defaults (VT220 spec).
                    self.cursor = Cursor::default();
                    self.fg = Color::Default;
                    self.bg = Color::Default;
                    self.underline_color = Color::Default;
                    self.flags = CellFlags::empty();
                    self.modes.auto_wrap = true;
                    self.modes.origin = false;
                }
            }
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
            b'H' if self.cursor.x < self.tab_stops.len() => {
                self.tab_stops[self.cursor.x] = true;
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
                // Strip control characters to prevent terminal injection
                // and log forging via window title. Cap at 256 chars
                // (well beyond any reasonable title length) to avoid
                // wasting memory on malformed sequences.
                let raw = parts.next().unwrap_or("");
                self.title = raw.chars().filter(|c| !c.is_control()).take(256).collect();
            }
            Some(8) => {
                // OSC 8 — Hyperlink.
                // Format: OSC 8 ; params ; URI ST
                // Empty URI clears the hyperlink, non-empty sets it.
                let payload = parts.next().unwrap_or("");
                // Split off optional params before the URI.
                let uri = if let Some(idx) = payload.find(';') {
                    &payload[idx + 1..]
                } else {
                    payload
                };
                if uri.is_empty() {
                    self.current_hyperlink = None;
                } else {
                    // Cap URI length to prevent memory exhaustion from
                    // malformed or malicious OSC 8 sequences.
                    let uri = if uri.len() > 2048 {
                        // Use floor_char_boundary for UTF-8 safety.
                        &uri[..uri.floor_char_boundary(2048)]
                    } else {
                        uri
                    };
                    self.current_hyperlink = Some(uri.to_string());
                }
            }
            Some(52) => {
                // OSC 52 — Clipboard manipulation.
                // Format: OSC 52 ; <selector>[;<base64-data>] ST
                // <selector>: 'c' = clipboard, 'p' = primary selection.
                // With data: set clipboard.  Without data (empty): clear clipboard.
                // With '?' as data (e.g. "c;?"): query clipboard.
                let payload = parts.next().unwrap_or("");
                // Check for query: '?' prefix on selector (e.g. "?c")
                if payload.starts_with('?') {
                    self.pending_clipboard_query = true;
                } else if payload.contains(';') {
                    let parts2: Vec<&str> = payload.splitn(2, ';').collect();
                    if parts2.len() == 2 && parts2[1] == "?" {
                        // Alternative query format: selector;?
                        self.pending_clipboard_query = true;
                    } else {
                        // Normal set/clear with selector;data
                        let base64_data = parts2.get(1).copied().unwrap_or("");
                        if base64_data.is_empty() {
                            self.pending_clipboard_set = Some(Vec::new());
                        } else {
                            // Cap at ~1MB base64 (~750KB decoded) to prevent
                            // memory exhaustion from malicious OSC 52 payloads.
                            let decoded = if base64_data.len() > 1_400_000 {
                                Self::decode_base64(&base64_data[..1_400_000])
                            } else {
                                Self::decode_base64(base64_data)
                            };
                            self.pending_clipboard_set = Some(decoded);
                        }
                    }
                } else {
                    // Normal set/clear without selector prefix
                    let base64_data = if let Some(idx) = payload.find(';') {
                        &payload[idx + 1..]
                    } else {
                        payload
                    };
                    if base64_data.is_empty() {
                        self.pending_clipboard_set = Some(Vec::new());
                    } else {
                        // Cap at ~1MB base64 (~750KB decoded) to prevent
                        // memory exhaustion from malicious OSC 52 payloads.
                        let decoded = if base64_data.len() > 1_400_000 {
                            Self::decode_base64(&base64_data[..1_400_000])
                        } else {
                            Self::decode_base64(base64_data)
                        };
                        self.pending_clipboard_set = Some(decoded);
                    }
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
                // Prevent unbounded growth: keep at most 2000 marks (~500 commands).
                // Command marks reference absolute row numbers that become stale
                // when scrollback is trimmed, so old marks are useless anyway.
                if self.command_marks.len() > 2000 {
                    let drain_count = self.command_marks.len() - 2000;
                    self.command_marks.drain(0..drain_count);
                }
                // Track command execution time.
                match kind {
                    CommandMarkKind::CommandStart => {
                        self.command_start_time = Some(std::time::Instant::now());
                        self.last_command_duration = None;
                    }
                    CommandMarkKind::CommandEnd => {
                        if let Some(start) = self.command_start_time.take() {
                            self.last_command_duration = Some(start.elapsed());
                        }
                        // Cache exit code from the mark for status bar display.
                        self.last_exit_code_cache = exit_code;
                    }
                    CommandMarkKind::PromptStart => {
                        // Safety: clear any stale command_start_time.
                        // If a CommandStart (B) was received without a matching
                        // CommandEnd (D), the spinner would spin forever.
                        // PromptStart (A) always means we're back at the prompt,
                        // so any running command must have finished.
                        self.command_start_time = None;
                        // Clear last command duration so the status bar
                        // doesn't show stale timing from the previous command
                        // while the user is at a new prompt.
                        self.last_command_duration = None;
                        // Clear cached exit code so status bar doesn't show
                        // "exit:0" from the previous command at the new prompt.
                        self.last_exit_code_cache = None;
                    }
                    _ => {}
                }
            }
            // OSC 10/11/12 — dynamic colors (P17-A)
            Some(cmd_num @ 10..=12) => {
                let payload = parts.next().unwrap_or("");
                if payload == "?" {
                    // Query: report current color
                    let current = match cmd {
                        Some(10) => &self.fg,
                        Some(11) => &self.bg,
                        _ => self.dynamic_cursor.as_ref().unwrap_or(&self.fg),
                    };
                    let resp = match current {
                        Color::Rgb(r, g, b) => {
                            format!("\x1b]{};rgb:{:02x}/{:02x}/{:02x}\x1b\\", cmd_num, r, g, b)
                        }
                        Color::Indexed(i) => {
                            let (r, g, b) = color_for_index(*i);
                            format!("\x1b]{};rgb:{:02x}/{:02x}/{:02x}\x1b\\", cmd_num, r, g, b)
                        }
                        Color::Default => {
                            // OSC 10/12 (fg/cursor) default = white,
                            // OSC 11 (bg) default = black.
                            let (r, g, b) = match cmd {
                                Some(11) => (0u8, 0u8, 0u8),
                                _ => (0xff, 0xff, 0xff),
                            };
                            format!("\x1b]{};rgb:{:02x}/{:02x}/{:02x}\x1b\\", cmd_num, r, g, b)
                        }
                    };
                    self.response_buffer.extend_from_slice(resp.as_bytes());
                } else if let Some(color) = parse_xcolor(payload) {
                    match cmd {
                        Some(10) => self.dynamic_fg = Some(color),
                        Some(11) => self.dynamic_bg = Some(color),
                        Some(12) => self.dynamic_cursor = Some(color),
                        _ => {}
                    }
                }
            }
            Some(7) => {
                // OSC 7 — Current working directory.
                // Format: `OSC 7 ; file://hostname/path ST`
                // We extract the path component and store it.
                let payload = parts.next().unwrap_or("");
                if let Some(path) = parse_osc7_cwd(payload) {
                    self.cwd = Some(path);
                }
            }
            // OSC 9 — iTerm2-style extensions
            // OSC 9 ; message ST          → desktop notification
            // OSC 9 ; 4 ; state ; progress ST → progress report (iTerm2)
            Some(9) => {
                let payload = parts.next().unwrap_or("");
                if payload.starts_with("4;") {
                    // Progress report: "4;state;progress" or "4;state"
                    let sub_fields: Vec<&str> = payload.splitn(3, ';').collect();
                    let state = sub_fields.get(1).copied().unwrap_or("0");
                    match state {
                        "0" => {
                            // Start/progress update (value range: 0–100)
                            let pct = sub_fields
                                .get(2)
                                .and_then(|s| s.parse::<f32>().ok())
                                .unwrap_or(0.0)
                                / 100.0;
                            self.progress = Some(pct.clamp(0.0, 1.0));
                        }
                        "1" => {
                            // Hide / completed
                            self.progress = None;
                        }
                        "2" => {
                            // Error state (red badge in some terminals)
                            self.progress = None;
                        }
                        _ => {}
                    }
                } else {
                    // Desktop notification
                    if !payload.is_empty() {
                        self.pending_notification =
                            Some(("Terminal".to_string(), payload.to_string()));
                    }
                }
            }
            // OSC 777 — urxvt desktop notification (P24-E)
            // Format: `OSC 777 ; notify ; title ; body ST`
            Some(777) => {
                let payload = parts.next().unwrap_or("");
                let mut fields = payload.splitn(3, ';');
                let _kind = fields.next().unwrap_or(""); // should be "notify"
                let title_raw = fields.next().unwrap_or("");
                let title = if title_raw.is_empty() {
                    "Terminal"
                } else {
                    title_raw
                }
                .to_string();
                let body = fields.next().unwrap_or("").to_string();
                if !body.is_empty() {
                    self.pending_notification = Some((title, body));
                }
            }
            // OSC 21 — query window title (xterm extension).
            // Respond with: OSC l <title> ST
            Some(21) => {
                let resp = format!("\x1b]l{}\x1b\\", self.title);
                self.response_buffer.extend_from_slice(resp.as_bytes());
            }
            // OSC 4 — set/query color palette entries.
            // Query format: OSC 4 ; index ; ? ST → responds OSC 4 ; index ; rgb:RR/GG/BB ST
            // Set format: OSC 4 ; index ; rgb:RR/GG/BB ST
            // Multiple pairs can appear: OSC 4 ; 0 ; ? ; 1 ; ? ST
            Some(4) => {
                let payload = parts.next().unwrap_or("");
                let mut fields = payload.split(';');
                while let Some(idx_str) = fields.next() {
                    let Ok(idx) = idx_str.parse::<u8>() else {
                        continue;
                    };
                    let spec = fields.next().unwrap_or("");
                    if spec == "?" {
                        // Query: respond with current palette color
                        // (use override if set, otherwise built-in palette)
                        let (r, g, b) = self
                            .palette_overrides
                            .get(&idx)
                            .copied()
                            .unwrap_or_else(|| color_for_index(idx));
                        let resp =
                            format!("\x1b]4;{};rgb:{:02x}/{:02x}/{:02x}\x1b\\", idx, r, g, b);
                        self.response_buffer.extend_from_slice(resp.as_bytes());
                    } else if let Some(color) = parse_xcolor(spec) {
                        // Set: store the override in the palette map.
                        let Color::Rgb(r, g, b) = color else {
                            continue;
                        };
                        self.palette_overrides.insert(idx, (r, g, b));
                    }
                }
            }
            // OSC 1337 — iTerm2 shell integration protocol.
            // Key sub-protocols we support:
            //   OSC 1337 ; CurrentDir=<path>    — update cwd (like OSC 7)
            //   OSC 1337 ; RemoteHost=user@host — track remote SSH host
            //   OSC 1337 ; SetMark              — set a scrollback mark
            //   OSC 1337 ; ClearScrollback      — clear scrollback history
            //   OSC 1337 ; SetUserVar=var=value — store user variable (tmux)
            // Other 1337 extensions (inline images, profile switching) are ignored.
            Some(1337) => {
                let payload = parts.next().unwrap_or("");
                if let Some(path) = payload.strip_prefix("CurrentDir=") {
                    if let Ok(p) = std::path::PathBuf::from(path).canonicalize() {
                        self.cwd = Some(p);
                    } else {
                        self.cwd = Some(std::path::PathBuf::from(path));
                    }
                } else if let Some(host) = payload.strip_prefix("RemoteHost=") {
                    self.remote_host = Some(host.to_string());
                } else if payload == "SetMark" {
                    self.mark_row = Some(self.cursor().1);
                } else if payload == "ClearScrollback" {
                    self.grid_mut().clear_scrollback();
                } else if let Some(rest) = payload.strip_prefix("SetUserVar=") {
                    // SetUserVar=name=value — store user variable
                    if let Some(eq_pos) = rest.find('=') {
                        let (name, value) = rest.split_at(eq_pos);
                        // Prevent unbounded growth (malicious programs).
                        if self.user_vars.len() >= 100 && !self.user_vars.contains_key(name) {
                            self.user_vars.clear();
                        }
                        self.user_vars
                            .insert(name.to_string(), value[1..].to_string());
                    }
                }
            }
            // OSC 104 — reset color palette entries.
            // OSC 104 ; index ST → reset specific entry
            // OSC 104 ST          → reset ALL entries
            Some(104) => {
                let payload = parts.next().unwrap_or("");
                if payload.is_empty() {
                    // Reset all palette overrides.
                    self.palette_overrides.clear();
                } else {
                    // Reset specific entries.
                    for idx_str in payload.split(';') {
                        if let Ok(idx) = idx_str.parse::<u8>() {
                            self.palette_overrides.remove(&idx);
                        }
                    }
                }
            }
            // OSC 110 / 111 / 112 — reset dynamic colors (fg/bg/cursor).
            // OSC 110 ST → reset foreground (OSC 10)
            // OSC 111 ST → reset background (OSC 11)
            // OSC 112 ST → reset cursor (OSC 12)
            Some(110) => {
                self.dynamic_fg = None;
            }
            Some(111) => {
                self.dynamic_bg = None;
            }
            Some(112) => {
                self.dynamic_cursor = None;
            }
            _ => {}
        }
    }

    fn dcs(&mut self, intermediates: &[u8], _params: &[u16], final_byte: u8, data: &[u8]) {
        // XTGETTCAP — request terminal capability (DCS + q <hex-name> ST)
        // Response: DCS + r <hex-name> = <hex-value> ST
        // Programs (tmux, nvim) query capabilities like "TN" (terminal name),
        // "Co" (number of colors), "RGB" (truecolor support).
        if final_byte == b'q' && intermediates.contains(&b'+') {
            // Decode hex-encoded capability name
            if let Some(cap_name) = hex_decode(data) {
                let cap_upper = cap_name.to_ascii_uppercase();
                let value = match cap_upper.as_str() {
                    // Terminal name
                    "TN" => Some("ggterm".to_string()),
                    // Number of colors — report 256 (xterm-256color compatible)
                    "CO" | "COLORS" => Some("256".to_string()),
                    // Truecolor support
                    "RGB" => Some("8".to_string()),
                    // Background color (xterm extension)
                    "BG" => Some("rgb:0000/0000/0000".to_string()),
                    // Foreground color
                    "FG" => Some("rgb:cccc/cccc/cccc".to_string()),
                    _ => None,
                };
                match value {
                    Some(v) => {
                        // Encode response: DCS + r <hex-name> = <hex-value> ST
                        let hex_name = hex_encode(cap_upper.as_bytes());
                        let hex_val = hex_encode(v.as_bytes());
                        let resp = format!("\x1bP1+r{hex_name}={hex_val}\x1b\\");
                        self.response_buffer.extend_from_slice(resp.as_bytes());
                    }
                    None => {
                        // Unknown capability: DCS 0 + r ST
                        self.response_buffer.extend_from_slice(b"\x1bP0+r\x1b\\");
                    }
                }
            }
        }
        // Sixel graphics (DCS ... q) — acknowledged but not rendered
        // tmux passthrough (DCS tmux ;) — ignored

        // DECRQSS — Request Status String (DCS $ q <selector> ST)
        // Response: DCS 1 $ r <value> ST for known settings
        //           DCS 0 $ r ST for unknown settings
        if final_byte == b'q' && intermediates.contains(&b'$') {
            let selector = std::str::from_utf8(data).unwrap_or("");
            let response = match selector {
                // SGR — report current SGR attributes
                "m" => {
                    let mut sgr_parts: Vec<String> = Vec::new();
                    let mut has_attr = false;
                    if self.flags.contains(CellFlags::BOLD) {
                        sgr_parts.push("1".into());
                        has_attr = true;
                    }
                    if self.flags.contains(CellFlags::DIM) {
                        sgr_parts.push("2".into());
                        has_attr = true;
                    }
                    if self.flags.contains(CellFlags::ITALIC) {
                        sgr_parts.push("3".into());
                        has_attr = true;
                    }
                    if self.flags.contains(CellFlags::UNDERLINE) {
                        sgr_parts.push("4".into());
                        has_attr = true;
                    }
                    if self.flags.contains(CellFlags::BLINK) {
                        sgr_parts.push("5".into());
                        has_attr = true;
                    }
                    if self.flags.contains(CellFlags::REVERSE) {
                        sgr_parts.push("7".into());
                        has_attr = true;
                    }
                    if self.flags.contains(CellFlags::HIDDEN) {
                        sgr_parts.push("8".into());
                        has_attr = true;
                    }
                    if self.flags.contains(CellFlags::STRIKETHROUGH) {
                        sgr_parts.push("9".into());
                        has_attr = true;
                    }
                    let sgr = if has_attr {
                        sgr_parts.join(";")
                    } else {
                        "0".into()
                    };
                    format!("\x1bP1$r{sgr}m\x1b\\")
                }
                // DECSTBM — scroll region (top;bottom)
                "r" => {
                    let (top, bottom) = self.grid.scroll_region();
                    format!("\x1bP1$r{};{}r\x1b\\", top + 1, bottom)
                }
                // DECSCA — select character protection attribute
                // Response: 1$r Ps " q where Ps = 1 (protected) or 0 (unprotected)
                "\"q" => {
                    let val = if self.protected_attr { 1 } else { 0 };
                    format!("\x1bP1$r{val}\"q\x1b\\")
                }
                // DECSCUSR — cursor style
                // Response: 1$r Ps SP q where Ps = current style number
                " q" => {
                    let style_num = match self.cursor_style {
                        CursorStyle::Default => 0,
                        CursorStyle::BlinkBlock => 1,
                        CursorStyle::SteadyBlock => 2,
                        CursorStyle::BlinkUnderline => 3,
                        CursorStyle::SteadyUnderline => 4,
                        CursorStyle::BlinkBar => 5,
                        CursorStyle::SteadyBar => 6,
                    };
                    format!("\x1bP1$r{style_num} q\x1b\\")
                }
                _ => "\x1bP0$r\x1b\\".to_string(),
            };
            self.response_buffer.extend_from_slice(response.as_bytes());
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
    fn t_lf_clears_pending_wrap_without_lnm() {
        // Fill a full line to set pending_wrap, then bare LF (no CR).
        // Without the fix, pending_wrap stays true and the next char
        // would wrap an extra line.
        let mut t = Terminal::new(4, 4);
        // LNM is off by default.
        feed(&mut t, b"ABCD"); // fills row 0, pending_wrap=true at col 3
        feed(&mut t, b"\n"); // bare LF — should clear pending_wrap
        feed(&mut t, b"E");
        // LNM off → LF keeps column. E should be at col 3 of row 1.
        // If pending_wrap wasn't cleared, E would wrap to col 0 of row 2.
        assert_eq!(t.grid().cell(3, 1).unwrap().ch, 'E');
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, ' ');
    }

    #[test]
    fn t_lf_outside_scroll_region_no_scroll() {
        // Scroll region rows 0-2; cursor at row 3 (below region).
        // LF should move cursor down without scrolling the region.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[1;3r"); // region top=0, bottom=2
        feed(&mut t, b"\x1b[4;1H"); // cursor to row 3 (0-based), below region
        feed(&mut t, b"\n"); // LF
        // Cursor should move to row 4, region should NOT scroll.
        assert_eq!(t.cursor().1, 4, "LF below scroll region should move cursor");
    }

    #[test]
    fn t_ril_outside_scroll_region_no_scroll() {
        // Scroll region rows 2-4; cursor at row 0 (above region).
        // RI should move cursor up without scrolling the region.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;5r"); // region top=2, bottom=4
        feed(&mut t, b"\x1b[2;1H"); // cursor to row 1 (above region)
        feed(&mut t, b"\x1bM"); // RI (reverse line feed)
        // Cursor should move to row 0, region should NOT scroll.
        assert_eq!(t.cursor().1, 0, "RI above scroll region should move cursor");
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
    fn t_cuu_outside_scroll_region() {
        // Scroll region rows 3-6 (0-based). Cursor at row 1 (above region).
        // CUU should move up to row 0, NOT clamp to scroll_top.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[4;6r"); // region top=3, bottom=6
        feed(&mut t, b"\x1b[2;1H"); // cursor row 1 (above region)
        feed(&mut t, b"\x1b[1A"); // CUU 1
        assert_eq!(t.cursor().1, 0, "CUU above scroll region should not clamp");
    }

    #[test]
    fn t_cud_outside_scroll_region() {
        // Scroll region rows 0-3 (0-based). Cursor at row 7 (below region).
        // CUD should move down to row 8, NOT clamp to scroll_bottom.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[1;4r"); // region top=0, bottom=3
        feed(&mut t, b"\x1b[8;1H"); // cursor row 7 (below region)
        feed(&mut t, b"\x1b[1B"); // CUD 1
        assert_eq!(t.cursor().1, 8, "CUD below scroll region should not clamp");
    }

    #[test]
    fn t_cnl_outside_scroll_region() {
        // Scroll region rows 0-3. Cursor at row 7 (below region).
        // CNL should move to row 8, NOT clamp to scroll_bottom.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[1;4r"); // region top=0, bottom=3
        feed(&mut t, b"\x1b[8;1H"); // cursor row 7 (below region)
        feed(&mut t, b"\x1b[1E"); // CNL 1
        assert_eq!(t.cursor().1, 8, "CNL below scroll region should not clamp");
        assert_eq!(t.cursor().0, 0, "CNL should set column to 0");
    }

    #[test]
    fn t_cpl_outside_scroll_region() {
        // Scroll region rows 3-6. Cursor at row 1 (above region).
        // CPL should move to row 0, NOT clamp to scroll_top.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[4;6r"); // region top=3, bottom=6
        feed(&mut t, b"\x1b[2;1H"); // cursor row 1 (above region)
        feed(&mut t, b"\x1b[1F"); // CPL 1
        assert_eq!(t.cursor().1, 0, "CPL above scroll region should not clamp");
        assert_eq!(t.cursor().0, 0, "CPL should set column to 0");
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
    fn t_sgr_underline_double() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[4:2mD");
        let flags = t.grid().cell(0, 0).unwrap().flags;
        assert!(flags.contains(CellFlags::UNDERLINE));
        assert!(flags.contains(CellFlags::UNDERLINE_DOUBLE));
    }

    #[test]
    fn t_sgr_underline_curly() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[4:3mC");
        let flags = t.grid().cell(0, 0).unwrap().flags;
        assert!(flags.contains(CellFlags::UNDERLINE));
        assert!(flags.contains(CellFlags::UNDERLINE_CURLY));
    }

    #[test]
    fn t_sgr_underline_style_mixed_with_color() {
        // ESC[4:3;31m — curly underline AND red foreground
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[4:3;31mX");
        let cell = t.grid().cell(0, 0).unwrap();
        assert!(
            cell.flags.contains(CellFlags::UNDERLINE_CURLY),
            "should have curly underline"
        );
        assert_eq!(cell.fg, Color::Indexed(1), "should have red fg");
    }

    #[test]
    fn t_sgr_underline_dotted() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[4:4m.");
        let flags = t.grid().cell(0, 0).unwrap().flags;
        assert!(flags.contains(CellFlags::UNDERLINE));
        assert!(flags.contains(CellFlags::UNDERLINE_DOTTED));
    }

    #[test]
    fn t_sgr_underline_dashed() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[4:5m-");
        let flags = t.grid().cell(0, 0).unwrap().flags;
        assert!(flags.contains(CellFlags::UNDERLINE));
        assert!(flags.contains(CellFlags::UNDERLINE_DASHED));
    }

    #[test]
    fn t_sgr_underline_style_reset() {
        // SGR 24 (no sub) clears all underline styles.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[4:3mC\x1b[24mR");
        let flags_after = t.grid().cell(1, 0).unwrap().flags;
        assert!(!flags_after.contains(CellFlags::UNDERLINE));
        assert!(!flags_after.contains(CellFlags::UNDERLINE_CURLY));
    }

    #[test]
    fn t_sgr21_double_underline() {
        // SGR 21 = double underline (xterm convention).
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[21mD");
        let flags = t.grid().cell(0, 0).unwrap().flags;
        assert!(flags.contains(CellFlags::UNDERLINE));
        assert!(flags.contains(CellFlags::UNDERLINE_DOUBLE));
    }

    #[test]
    fn t_sgr53_overline() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[53mO");
        let flags = t.grid().cell(0, 0).unwrap().flags;
        assert!(flags.contains(CellFlags::OVERLINE));
    }

    #[test]
    fn t_sgr55_overline_off() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[53mO\x1b[55mR");
        let flags = t.grid().cell(1, 0).unwrap().flags;
        assert!(!flags.contains(CellFlags::OVERLINE));
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
    fn t_sgr_truecolor_colon_syntax() {
        // Colon-separated truecolor: 38:2:R:G:B (used by kitty, foot, etc.)
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[38:2:255:128:0mX");
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Rgb(255, 128, 0));
    }

    #[test]
    fn t_sgr_256color_colon_syntax() {
        // Colon-separated 256-color: 38:5:N
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[38:5:200mX");
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Indexed(200));
    }

    #[test]
    fn t_sgr_bg_truecolor_colon_syntax() {
        // Background truecolor with colon syntax: 48:2:R:G:B
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[48:2:0:128:255mX");
        assert_eq!(t.grid().cell(0, 0).unwrap().bg, Color::Rgb(0, 128, 255));
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

    // ── P15-A: Alt-screen grid swap tests ───────────────────────────

    #[test]
    fn t_alt_screen_1049_saves_and_restores_content() {
        let mut t = Terminal::new(10, 3);
        // Write "Hello" on the primary screen.
        feed(&mut t, b"Hello");
        // Enter alt-screen (mode 1049).
        feed(&mut t, b"\x1b[?1049h");
        assert!(t.modes.alt_screen);
        // The alt screen should be blank (not contain "Hello").
        let cell = t.grid().cell(0, 0).unwrap();
        assert!(
            cell.ch == '\0' || cell.ch == ' ',
            "alt screen should be blank, got '{}'",
            cell.ch
        );
        // Write "World" on the alt screen.
        feed(&mut t, b"World");
        // Exit alt-screen.
        feed(&mut t, b"\x1b[?1049l");
        assert!(!t.modes.alt_screen);
        // Primary screen should have "Hello" restored.
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.ch, 'H');
    }

    #[test]
    fn t_alt_screen_1049_saves_and_restores_cursor() {
        let mut t = Terminal::new(10, 3);
        // Move cursor to row 2, col 5.
        feed(&mut t, b"\x1b[2;5H");
        assert_eq!(t.cursor(), (4, 1));
        // Enter alt-screen (saves cursor).
        feed(&mut t, b"\x1b[?1049h");
        // Cursor should be at origin on alt screen.
        assert_eq!(t.cursor(), (0, 0));
        // Move cursor on alt screen.
        feed(&mut t, b"\x1b[3;3H");
        assert_eq!(t.cursor(), (2, 2));
        // Exit alt-screen (restores cursor).
        feed(&mut t, b"\x1b[?1049l");
        assert_eq!(t.cursor(), (4, 1));
    }

    #[test]
    fn t_alt_screen_1049_saves_and_restores_sgr() {
        // DECSET 1049 should save/restore full DECSC state including SGR.
        let mut t = Terminal::new(10, 3);
        // Set bold + red foreground.
        feed(&mut t, b"\x1b[1;31m");
        // Enter alt-screen.
        feed(&mut t, b"\x1b[?1049h");
        // Change SGR on alt screen.
        feed(&mut t, b"\x1b[0;32m");
        assert_eq!(t.fg, Color::Indexed(2));
        // Exit alt-screen — original SGR should be restored.
        feed(&mut t, b"\x1b[?1049l");
        assert!(t.flags.contains(CellFlags::BOLD), "bold should be restored");
        assert_eq!(t.fg, Color::Indexed(1), "red fg should be restored");
    }

    #[test]
    fn t_alt_screen_1049_preserves_custom_tab_stops() {
        let mut t = Terminal::new(40, 3);
        // Clear all default tab stops, set custom one at col 20.
        feed(&mut t, b"\x1b[3g"); // TBC 3: clear all tab stops
        feed(&mut t, b"\x1b[1;21H"); // CUP: row 1, col 21 → x=20
        feed(&mut t, b"\x1bH"); // HTS: set tab stop here
        // Verify custom tab stop exists at col 20.
        assert!(t.tab_stops.get(20).copied().unwrap_or(false));
        // Enter alt-screen.
        feed(&mut t, b"\x1b[?1049h");
        // Alt screen should have default tab stops (every 8), not custom.
        assert!(!t.tab_stops.get(20).copied().unwrap_or(false));
        assert!(t.tab_stops.get(8).copied().unwrap_or(false));
        // Exit alt-screen.
        feed(&mut t, b"\x1b[?1049l");
        // Custom tab stop should be restored.
        assert!(t.tab_stops.get(20).copied().unwrap_or(false));
        assert!(!t.tab_stops.get(8).copied().unwrap_or(false));
    }

    #[test]
    fn t_alt_screen_47_swaps_without_cursor_save() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[2;5H");
        assert_eq!(t.cursor(), (4, 1));
        // Enter alt-screen with mode 47 (no cursor save).
        feed(&mut t, b"\x1b[?47h");
        assert!(t.modes.alt_screen);
        // Cursor is NOT reset by mode 47.
        assert_eq!(t.cursor(), (4, 1), "mode 47 should not reset cursor");
        // Exit.
        feed(&mut t, b"\x1b[?47l");
        assert!(!t.modes.alt_screen);
    }

    #[test]
    fn t_alt_screen_content_preserved_through_swap() {
        let mut t = Terminal::new(10, 3);
        // Write line 1: "AAA"
        feed(&mut t, b"AAA");
        // Enter alt-screen.
        feed(&mut t, b"\x1b[?1049h");
        // Write on alt screen: "BBB"
        feed(&mut t, b"BBB");
        // Verify alt screen has BBB.
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'B');
        // Exit alt-screen.
        feed(&mut t, b"\x1b[?1049l");
        // Primary screen should still have AAA.
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'A');
    }

    #[test]
    fn t_alt_screen_multiple_enter_exit_cycles() {
        let mut t = Terminal::new(10, 3);
        for _ in 0..3 {
            feed(&mut t, b"X");
            feed(&mut t, b"\x1b[?1049h");
            assert!(t.modes.alt_screen);
            feed(&mut t, b"\x1b[?1049l");
            assert!(!t.modes.alt_screen);
        }
        // After 3 cycles with 3 X's, cursor should be at col 3.
        assert_eq!(t.cursor().0, 3);
    }

    #[test]
    fn t_alt_screen_idempotent_enter() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?1049h");
        feed(&mut t, b"\x1b[?1049h"); // Double enter — should be no-op
        assert!(t.modes.alt_screen);
        assert!(t.alt_saved_grid.is_some());
    }

    #[test]
    fn t_alt_screen_idempotent_exit() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?1049h");
        feed(&mut t, b"\x1b[?1049l");
        feed(&mut t, b"\x1b[?1049l"); // Double exit — should be no-op
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
    fn t_osc_52_query() {
        let mut t = Terminal::new(80, 24);
        // OSC 52 clipboard query: OSC 52;c;? ST
        feed(&mut t, b"\x1b]52;c;?\x07");
        assert!(t.take_pending_clipboard_query(), "query flag should be set");
        // Second take clears it
        assert!(
            !t.take_pending_clipboard_query(),
            "query flag should be cleared"
        );
        // Should NOT trigger clipboard set
        assert!(t.take_pending_clipboard_set().is_none());
    }

    #[test]
    fn t_base64_decode_basic() {
        assert_eq!(Terminal::decode_base64("aGVsbG8="), b"hello");
        assert_eq!(Terminal::decode_base64("d29ybGQ="), b"world");
        assert_eq!(Terminal::decode_base64("Zm9v"), b"foo");
    }

    #[test]
    fn t_base64_decode_empty() {
        assert_eq!(Terminal::decode_base64(""), b"");
    }

    #[test]
    fn t_base64_decode_padding() {
        assert_eq!(Terminal::decode_base64("Zg=="), b"f");
        assert_eq!(Terminal::decode_base64("Zm8="), b"fo");
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
    fn t_osc_8_set_hyperlink() {
        // OSC 8 ; params ; URI ST → set current hyperlink
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]8;;https://example.com\x1b\\");
        assert_eq!(t.current_hyperlink.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn t_osc_8_set_hyperlink_with_params() {
        // OSC 8 ; id=123 ; URI ST → params ignored, URI captured
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]8;id=42;https://rust-lang.org\x07");
        assert_eq!(
            t.current_hyperlink.as_deref(),
            Some("https://rust-lang.org")
        );
    }

    #[test]
    fn t_osc_8_clear_hyperlink() {
        // OSC 8 with empty URI clears the hyperlink
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]8;;https://example.com\x1b\\");
        assert!(t.current_hyperlink.is_some());
        feed(&mut t, b"\x1b]8;;\x1b\\");
        assert!(t.current_hyperlink.is_none());
    }

    #[test]
    fn t_osc_8_hyperlink_applied_to_cells() {
        // Set hyperlink, print text, verify cells carry the URI.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]8;;https://example.com\x1b\\");
        feed(&mut t, b"Hi");
        let cell0 = t.grid().cell(0, 0).unwrap();
        let cell1 = t.grid().cell(1, 0).unwrap();
        assert_eq!(cell0.hyperlink.as_deref(), Some("https://example.com"));
        assert_eq!(cell1.hyperlink.as_deref(), Some("https://example.com"));
        assert_eq!(cell0.ch, 'H');
        assert_eq!(cell1.ch, 'i');
    }

    #[test]
    fn t_osc_8_hyperlink_cleared_on_subsequent_text() {
        // Set hyperlink, print, clear hyperlink, print more text.
        // Subsequent cells should NOT carry the hyperlink.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]8;;https://example.com\x1b\\");
        feed(&mut t, b"A");
        feed(&mut t, b"\x1b]8;;\x1b\\");
        feed(&mut t, b"B");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().hyperlink.as_deref(),
            Some("https://example.com")
        );
        assert_eq!(t.grid().cell(1, 0).unwrap().hyperlink, None);
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B');
    }

    #[test]
    fn t_osc_8_multichar_continuation() {
        // Multiple characters under same hyperlink all carry it.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]8;;https://rust-lang.org\x07");
        feed(&mut t, b"Click"); // 5 chars at positions 0-4
        for i in 0..5 {
            let cell = t.grid().cell(i, 0).unwrap();
            assert_eq!(
                cell.hyperlink.as_deref(),
                Some("https://rust-lang.org"),
                "cell {i} should have hyperlink"
            );
        }
        // After clearing, more text has no hyperlink.
        feed(&mut t, b"\x1b]8;;\x07");
        feed(&mut t, b"X"); // at position 5
        assert_eq!(t.grid().cell(5, 0).unwrap().hyperlink, None);
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
    fn t_focus_event_disabled_by_default() {
        let t = Terminal::new(80, 24);
        assert!(!t.focus_event_enabled());
        assert!(t.focus_in_report().is_empty());
        assert!(t.focus_out_report().is_empty());
    }

    #[test]
    fn t_focus_event_enabled() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?1004h");
        assert!(t.focus_event_enabled());
        assert_eq!(t.focus_in_report(), b"\x1b[I");
        assert_eq!(t.focus_out_report(), b"\x1b[O");
    }

    #[test]
    fn t_focus_event_disabled() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?1004h");
        feed(&mut t, b"\x1b[?1004l");
        assert!(!t.focus_event_enabled());
        assert!(t.focus_in_report().is_empty());
    }

    #[test]
    fn t_csi_18t_text_area_size_chars() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[18t");
        // Response: CSI 8 ; rows ; cols t
        let resp = String::from_utf8_lossy(t.response_buffer());
        assert!(resp.contains("8;24;80"), "got: {resp}");
    }

    #[test]
    fn t_csi_14t_text_area_size_pixels() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[14t");
        // Response: CSI 4 ; height ; width t
        let resp = String::from_utf8_lossy(t.response_buffer());
        assert!(resp.starts_with("\x1b[4;"), "got: {resp}");
        assert!(resp.ends_with('t'));
    }

    #[test]
    fn t_csi_16t_cell_size_pixels() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[16t");
        // Response: CSI 6 ; height ; width t
        let resp = String::from_utf8_lossy(t.response_buffer());
        assert!(
            resp.starts_with("\x1b[6;"),
            "CSI 16t should respond with cell size, got: {resp}"
        );
        assert!(resp.ends_with('t'));
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
    fn t_vpa_origin_mode() {
        let mut t = Terminal::new(80, 24);
        // Set scroll region to rows 5-15 (0-based: 4-14)
        feed(&mut t, b"\x1b[5;15r");
        // Enable origin mode
        feed(&mut t, b"\x1b[?6h");
        // VPA to row 1 → should be relative to scroll region top (row 4)
        feed(&mut t, b"\x1b[1d");
        assert_eq!(
            t.cursor().1,
            4,
            "VPA row 1 in origin mode should be scroll top"
        );
        // VPA to row 3 → row 6 (4 + 3 - 1)
        feed(&mut t, b"\x1b[3d");
        assert_eq!(t.cursor().1, 6, "VPA row 3 in origin mode should be 4+2");
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

    #[test]
    fn t_resize_preserves_custom_tab_stops() {
        let mut t = Terminal::new(20, 5);
        // Set custom tab stop at column 3.
        feed(&mut t, b"\x1b[1;4H\x1bH");
        assert!(t.tab_stops[3]);
        // Widen to 30 — custom stop at column 3 should be preserved.
        t.resize(30, 5);
        assert!(
            t.tab_stops[3],
            "custom tab stop at col 3 should survive resize"
        );
        // Default stops at col 8, 16, 24 should also be set in new area.
        assert!(t.tab_stops[8], "default stop at col 8");
        assert!(t.tab_stops[24], "default stop at col 24 in new area");
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

    // ── P17-B: Combining Character tests ─────────────────────────────

    #[test]
    fn t_combining_char_merges_into_preceding_cell() {
        let mut t = Terminal::new(80, 24);
        // 'e' followed by combining acute accent (U+0301)
        feed(&mut t, "e\u{0301}".as_bytes());
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.ch, 'e');
        assert_eq!(cell.combining, vec!['\u{0301}']);
        // Cursor should have advanced only for 'e' (width 1), not for combining.
        assert_eq!(t.cursor().0, 1);
    }

    #[test]
    fn t_combining_char_multiple_marks() {
        let mut t = Terminal::new(80, 24);
        // 'a' with combining diaeresis (U+0308) and combining grave (U+0300)
        feed(&mut t, "a\u{0308}\u{0300}".as_bytes());
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.ch, 'a');
        assert_eq!(cell.combining, vec!['\u{0308}', '\u{0300}']);
        assert_eq!(t.cursor().0, 1);
    }

    #[test]
    fn t_combining_char_at_line_start_dropped() {
        let mut t = Terminal::new(80, 24);
        // Combining char at position (0,0) — no preceding cell, should be dropped.
        feed(&mut t, "\u{0301}".as_bytes());
        assert!(t.grid().cell(0, 0).unwrap().combining.is_empty());
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ');
    }

    #[test]
    fn t_combining_char_preserves_fg_bg() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[31;42m"); // red fg, green bg
        feed(&mut t, "e\u{0301}".as_bytes());
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.fg, Color::Indexed(1)); // red
        assert_eq!(cell.bg, Color::Indexed(2)); // green
        assert_eq!(cell.combining, vec!['\u{0301}']);
    }

    #[test]
    fn t_combining_char_does_not_advance_cursor() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, "ab\u{0301}c".as_bytes());
        // 'a' at col 0, 'b' at col 1 (with combining), 'c' at col 2
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'a');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'b');
        assert_eq!(t.grid().cell(1, 0).unwrap().combining, vec!['\u{0301}']);
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'c');
        assert_eq!(t.cursor().0, 3);
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

    #[test]
    fn t_decstbm_invalid_params_still_homes() {
        // DECSTBM with invalid params (top >= bottom) should NOT change the
        // scroll region, but per VT spec MUST still home the cursor.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5;15r"); // set a valid scroll region first
        feed(&mut t, b"\x1b[10;10H"); // move cursor away
        feed(&mut t, b"\x1b[10;5r"); // invalid: top(10) >= bottom(5)
        // Cursor should still be homed
        assert_eq!(
            t.cursor(),
            (0, 0),
            "DECSTBM should home cursor even with invalid params"
        );
        // Scroll region should be unchanged
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(top, 4, "Scroll region top should be unchanged");
        assert_eq!(bottom, 15, "Scroll region bottom should be unchanged");
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
    fn t_osc133_command_end_non_numeric_exit_code() {
        // Some shells may emit non-numeric exit codes — must not panic.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;D;abc\x07");
        let marks = t.command_marks();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].kind, CommandMarkKind::CommandEnd);
        assert_eq!(marks[0].exit_code, None, "non-numeric exit code → None");
    }

    #[test]
    fn t_osc133_command_end_st_terminated() {
        // ST-terminated (ESC \) instead of BEL — both must work.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;D;0\x1b\\");
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
    fn t_osc133_command_duration_tracked() {
        let mut t = Terminal::new(80, 24);
        // Before any command: no duration.
        assert!(t.last_command_duration().is_none());
        assert!(!t.is_command_running());

        // Command starts.
        feed(&mut t, b"\x1b]133;B\x07");
        assert!(t.is_command_running());
        assert!(t.last_command_duration().is_none());

        // Command ends.
        feed(&mut t, b"\x1b]133;D;0\x07");
        assert!(!t.is_command_running());
        let dur = t.last_command_duration();
        assert!(
            dur.is_some(),
            "duration should be tracked after command end"
        );
        assert!(dur.unwrap().as_nanos() < 1_000_000_000, "should be fast");
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
    fn t_dsr_cursor_position_origin_mode() {
        let mut t = Terminal::new(80, 24);
        // Set scroll region to rows 5-15 (0-based: 4-14)
        feed(&mut t, b"\x1b[5;15r");
        // Enable origin mode
        feed(&mut t, b"\x1b[?6h");
        // Move to origin (row 1, col 1 in origin mode = row 5, col 1)
        feed(&mut t, b"\x1b[1;1H");
        // Query cursor position — should report relative to scroll region
        feed(&mut t, b"\x1b[6n");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        // Should report row 1 (relative to scroll top), col 1
        assert!(
            s.contains("1;1R"),
            "DSR in origin mode should report relative position, got: {s}"
        );
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
    fn t_decstr_preserves_scrollback() {
        // DECSTR should NOT destroy scrollback — only hard reset (RIS, ESC c) does.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"AAAA\nBBBB\nCCCC\nDDDD"); // creates scrollback
        let sb = t.grid().scrollback_len();
        assert!(sb > 0);
        feed(&mut t, b"\x1b[!p"); // DECSTR — soft reset
        assert_eq!(
            t.grid().scrollback_len(),
            sb,
            "DECSTR should preserve scrollback"
        );
    }

    #[test]
    fn t_decstr_resets_modes() {
        let mut t = Terminal::new(80, 24);
        // Set various modes
        feed(&mut t, b"\x1b[?2004h"); // bracketed paste
        feed(&mut t, b"\x1b[?1h"); // cursor keys app mode
        feed(&mut t, b"\x1b[20h"); // LNM
        feed(&mut t, b"\x1b[4h"); // insert mode
        // Soft reset
        feed(&mut t, b"\x1b[!p");
        assert!(!t.bracketed_paste(), "DECSTR should reset bracketed paste");
        assert!(!t.cursor_keys_app(), "DECSTR should reset cursor keys app");
        assert!(!t.new_line_mode(), "DECSTR should reset LNM");
        assert!(!t.modes.insert, "DECSTR should reset insert mode");
        // Auto-wrap and cursor visible should be restored to defaults
        assert!(t.modes.auto_wrap, "DECSTR should restore auto_wrap=true");
        assert!(
            t.modes.cursor_visible,
            "DECSTR should restore cursor_visible=true"
        );
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
    fn t_origin_mode_cup_clamps_to_scroll_region() {
        let mut t = Terminal::new(80, 24);
        // Set scroll region to rows 5-15 (0-based: 4-14)
        feed(&mut t, b"\x1b[5;15r");
        feed(&mut t, b"\x1b[?6h"); // origin mode on
        // CUP to row 100 — should clamp to scroll bottom (row 14)
        feed(&mut t, b"\x1b[100;1H");
        assert_eq!(
            t.cursor().1,
            14,
            "Origin mode CUP should clamp to scroll region bottom"
        );
    }

    #[test]
    fn t_ed_mode3_clear_scrollback() {
        let mut t = Terminal::new(80, 4);
        // Fill visible screen, then scroll to create scrollback
        feed(&mut t, b"AAAA\r\nBBBB\r\nCCCC\r\nDDDD\r\nEEEE");
        assert!(t.grid().scrollback_len() > 0);
        // ED mode 3 — clear scrollback only, screen content must survive
        feed(&mut t, b"\x1b[3J");
        assert_eq!(t.grid().scrollback_len(), 0);
        // Screen content should still be there (EEEE on last visible row).
        assert_eq!(t.grid().cell(0, 3).unwrap().ch, 'E');
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
    fn t_command_block_output_line_count() {
        let mut t = Terminal::new(80, 24);
        // Command with 3 lines of output.
        feed(&mut t, b"\x1b]133;A\x07"); // PromptStart at row 0
        feed(&mut t, b"\x1b]133;B\x07"); // CommandStart
        feed(&mut t, b"\x1b]133;C\x07"); // OutputStart
        feed(&mut t, b"line1\nline2\nline3\n");
        feed(&mut t, b"\x1b]133;D;0\x07"); // CommandEnd
        let blocks = t.command_blocks();
        assert_eq!(blocks.len(), 1);
        let count = blocks[0].output_line_count();
        assert!(count.is_some(), "should have output line count");
        assert!(
            count.unwrap() >= 3,
            "should have at least 3 lines of output"
        );
    }

    #[test]
    fn t_command_block_output_line_count_none_running() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;A\x07"); // PromptStart
        feed(&mut t, b"\x1b]133;C\x07"); // OutputStart
        // No CommandEnd — command still running.
        let blocks = t.command_blocks();
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].output_line_count().is_none());
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
    fn t_last_command_output_text_basic() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;A\x07"); // PromptStart
        feed(&mut t, b"\x1b]133;B\x07"); // CommandStart
        feed(&mut t, b"\x1b]133;C\x07"); // OutputStart
        feed(&mut t, b"hello world\nfoo bar\n");
        feed(&mut t, b"\x1b]133;D;0\x07"); // CommandEnd
        let text = t.last_command_output_text();
        assert!(text.is_some(), "should have output text");
        let text = text.unwrap();
        assert!(
            text.contains("hello world"),
            "should contain first line: {text}"
        );
        assert!(
            text.contains("foo bar"),
            "should contain second line: {text}"
        );
    }

    #[test]
    fn t_last_command_output_text_none_running() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;A\x07");
        feed(&mut t, b"\x1b]133;C\x07");
        // No CommandEnd — command still running.
        assert!(t.last_command_output_text().is_none());
    }

    #[test]
    fn t_last_command_output_text_none_no_marks() {
        let t = Terminal::new(80, 24);
        assert!(t.last_command_output_text().is_none());
    }

    #[test]
    fn t_last_command_with_output_text_basic() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;A\x07"); // PromptStart
        feed(&mut t, b"ls -la"); // command text on row 0
        feed(&mut t, b"\x1b]133;B\x07"); // CommandStart
        feed(&mut t, b"\x1b]133;C\x07"); // OutputStart
        feed(&mut t, b"file1\nfile2\n");
        feed(&mut t, b"\x1b]133;D;0\x07"); // CommandEnd
        let text = t.last_command_with_output_text();
        assert!(text.is_some(), "should have command+output text");
        let text = text.unwrap();
        assert!(text.contains("file1"), "should contain output: {text}");
        assert!(text.contains("file2"), "should contain output: {text}");
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
    fn t_prompt_start_clears_stale_command_running() {
        // Simulate: CommandStart (B) received but CommandEnd (D) missed.
        // Then PromptStart (A) arrives — should clear stale running state.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;B\x07"); // CommandStart
        assert!(t.is_command_running());

        feed(&mut t, b"\x1b]133;A\x07"); // PromptStart (next prompt)
        assert!(
            !t.is_command_running(),
            "PromptStart should clear stale command_running"
        );
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
    fn t_mouse_sgr_pixel_mode_1016() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?1016h");
        assert!(t.mouse_sgr_pixel_enabled());
        feed(&mut t, b"\x1b[?1016l");
        assert!(!t.mouse_sgr_pixel_enabled());
    }

    #[test]
    fn t_mouse_utf8_mode_1005() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?1005h");
        assert!(t.modes.mouse_utf8);
        feed(&mut t, b"\x1b[?1005l");
        assert!(!t.modes.mouse_utf8);
    }

    // ── P17-A: OSC 10/11/12 Dynamic Color tests ──────────────────────

    #[test]
    fn t_parse_xcolor_rgb_slash_format() {
        assert_eq!(parse_xcolor("rgb:ff/00/ff"), Some(Color::Rgb(255, 0, 255)));
        assert_eq!(parse_xcolor("rgb:00/80/ff"), Some(Color::Rgb(0, 128, 255)));
    }

    #[test]
    fn t_parse_xcolor_hash_format() {
        assert_eq!(parse_xcolor("#ff8000"), Some(Color::Rgb(255, 128, 0)));
    }

    #[test]
    fn t_parse_xcolor_invalid() {
        assert_eq!(parse_xcolor("invalid"), None);
        assert_eq!(parse_xcolor("rgb:xyz"), None);
    }

    #[test]
    fn t_osc10_set_dynamic_fg() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]10;rgb:ff/80/00\x1b\\");
        assert_eq!(t.dynamic_fg(), Some(&Color::Rgb(255, 128, 0)));
    }

    #[test]
    fn t_osc11_set_dynamic_bg() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]11;rgb:1a/1a/2e\x1b\\");
        assert_eq!(t.dynamic_bg(), Some(&Color::Rgb(26, 26, 46)));
    }

    #[test]
    fn t_osc12_set_dynamic_cursor() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]12;rgb:ff/00/ff\x1b\\");
        assert_eq!(t.dynamic_cursor(), Some(&Color::Rgb(255, 0, 255)));
    }

    #[test]
    fn t_osc12_query_response() {
        let mut t = Terminal::new(80, 24);
        // Set cursor color first
        feed(&mut t, b"\x1b]12;rgb:aa/bb/cc\x1b\\");
        t.take_response(); // clear
        // Query cursor color
        feed(&mut t, b"\x1b]12;?\x1b\\");
        let resp = String::from_utf8_lossy(t.response_buffer());
        assert!(
            resp.contains("12;rgb:aa/bb/cc"),
            "OSC 12 query should return set cursor color, got: {resp}"
        );
    }

    #[test]
    fn t_osc10_query_response() {
        let mut t = Terminal::new(80, 24);
        // Set fg to red first
        feed(&mut t, b"\x1b[31m");
        // Query fg color
        feed(&mut t, b"\x1b]10;?\x1b\\");
        let resp = String::from_utf8_lossy(t.response_buffer());
        assert!(
            resp.contains("rgb:"),
            "query response should contain rgb: spec"
        );
    }

    #[test]
    fn t_osc11_query_default_bg_is_black() {
        let mut t = Terminal::new(80, 24);
        // Query default bg color (no dynamic bg set)
        feed(&mut t, b"\x1b]11;?\x1b\\");
        let resp = String::from_utf8_lossy(t.response_buffer());
        assert!(
            resp.contains("rgb:00/00/00"),
            "default bg should be black, got: {resp}"
        );
    }

    #[test]
    fn t_osc10_query_default_fg_is_white() {
        let mut t = Terminal::new(80, 24);
        // Reset fg to default first
        feed(&mut t, b"\x1b[39m");
        // Query default fg color
        feed(&mut t, b"\x1b]10;?\x1b\\");
        let resp = String::from_utf8_lossy(t.response_buffer());
        assert!(
            resp.contains("rgb:ff/ff/ff"),
            "default fg should be white, got: {resp}"
        );
    }

    #[test]
    fn t_dynamic_colors_default_none() {
        let t = Terminal::new(80, 24);
        assert!(t.dynamic_fg().is_none());
        assert!(t.dynamic_bg().is_none());
    }

    #[test]
    fn t_osc10_hash_format() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]10;#abcdef\x1b\\");
        assert_eq!(t.dynamic_fg(), Some(&Color::Rgb(171, 205, 239)));
    }

    // ── P22-D: OSC 7 working directory tests ──────────────────

    #[test]
    fn t_osc7_basic_file_uri() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]7;file://localhost/home/user\x1b\\");
        assert_eq!(t.cwd(), Some(std::path::Path::new("/home/user")));
    }

    #[test]
    fn t_osc7_with_hostname() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]7;file://myhost.example.com/var/log\x1b\\");
        assert_eq!(t.cwd(), Some(std::path::Path::new("/var/log")));
    }

    #[test]
    fn t_osc7_empty_path() {
        let mut t = Terminal::new(80, 24);
        // file://hostname with no trailing path → no cwd set
        feed(&mut t, b"\x1b]7;file://hostname\x1b\\");
        assert!(t.cwd().is_none());
    }

    #[test]
    fn t_osc7_not_file_scheme() {
        let mut t = Terminal::new(80, 24);
        // Non-file:// schemes are ignored
        feed(&mut t, b"\x1b]7;http://example.com/path\x1b\\");
        assert!(t.cwd().is_none());
    }

    #[test]
    fn t_osc7_percent_encoded() {
        let mut t = Terminal::new(80, 24);
        // %20 → space
        feed(&mut t, b"\x1b]7;file://host/home/my%20dir\x1b\\");
        assert_eq!(t.cwd(), Some(std::path::Path::new("/home/my dir")));
    }

    #[test]
    fn t_osc7_percent_encoded_multibyte() {
        let mut t = Terminal::new(80, 24);
        // %E6%A1%8C%E9%9D%A2 → 桌面 (CJK multibyte UTF-8)
        feed(
            &mut t,
            b"\x1b]7;file://host/Users/test/%E6%A1%8C%E9%9D%A2\x1b\\",
        );
        assert_eq!(t.cwd(), Some(std::path::Path::new("/Users/test/桌面")));
    }

    #[test]
    fn t_osc7_overwrites_previous() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]7;file://host/home/a\x1b\\");
        assert_eq!(t.cwd(), Some(std::path::Path::new("/home/a")));

        feed(&mut t, b"\x1b]7;file://host/home/b\x1b\\");
        assert_eq!(t.cwd(), Some(std::path::Path::new("/home/b")));
    }

    #[test]
    fn t_osc7_default_none() {
        let t = Terminal::new(80, 24);
        assert!(t.cwd().is_none());
    }

    #[test]
    fn t_parse_osc7_cwd_direct() {
        assert_eq!(
            parse_osc7_cwd("file://localhost/home/user"),
            Some(std::path::PathBuf::from("/home/user"))
        );
        assert_eq!(
            parse_osc7_cwd("file://host/path/to/dir"),
            Some(std::path::PathBuf::from("/path/to/dir"))
        );
        assert_eq!(parse_osc7_cwd("file://host"), None);
        assert_eq!(parse_osc7_cwd("not-a-uri"), None);
    }

    // ===== P24-A: Synchronized output tests =====

    #[test]
    fn t_sync_output_enable_disable() {
        let mut t = Terminal::new(80, 24);
        assert!(!t.is_synchronized());
        feed(&mut t, b"\x1b[?2026h");
        assert!(t.is_synchronized());
        feed(&mut t, b"\x1b[?2026l");
        assert!(!t.is_synchronized());
    }

    // ===== P24-B: Text reflow mode tests =====

    #[test]
    fn t_reflow_mode_default() {
        let t = Terminal::new(80, 24);
        assert!(t.reflow_enabled(), "reflow should be enabled by default");
    }

    #[test]
    fn t_reflow_mode_toggle() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?2027l");
        assert!(!t.reflow_enabled());
        feed(&mut t, b"\x1b[?2027h");
        assert!(t.reflow_enabled());
    }

    // ===== DECSET 7727: Alternate scroll mode tests =====

    #[test]
    fn t_alternate_scroll_default() {
        let t = Terminal::new(80, 24);
        assert!(
            t.alternate_scroll_enabled(),
            "alternate scroll should be enabled by default"
        );
    }

    #[test]
    fn t_alternate_scroll_toggle() {
        let mut t = Terminal::new(80, 24);
        // Disable: DECSET 7727 off
        feed(&mut t, b"\x1b[?7727l");
        assert!(!t.alternate_scroll_enabled());
        // Enable: DECSET 7727 on
        feed(&mut t, b"\x1b[?7727h");
        assert!(t.alternate_scroll_enabled());
    }

    // ===== P24-D: DECSCA / DECSED selective erase tests =====

    #[test]
    fn t_decsca_sets_protected_attr() {
        let mut t = Terminal::new(80, 24);
        // Set protected attribute: CSI 1 " q
        feed(&mut t, b"\x1b[1\"q");
        feed(&mut t, b"A");
        // Set unprotected: CSI 0 " q
        feed(&mut t, b"\x1b[0\"q");
        feed(&mut t, b"B");
        assert!(
            t.grid()
                .cell(0, 0)
                .unwrap()
                .flags
                .contains(CellFlags::PROTECTED)
        );
        assert!(
            !t.grid()
                .cell(1, 0)
                .unwrap()
                .flags
                .contains(CellFlags::PROTECTED)
        );
    }

    #[test]
    fn t_decsed_preserves_protected() {
        let mut t = Terminal::new(80, 24);
        // Write protected 'A': CSI 1 " q
        feed(&mut t, b"\x1b[1\"qA");
        // Write unprotected 'B': CSI 0 " q
        feed(&mut t, b"\x1b[0\"qB");
        // Selective erase all
        feed(&mut t, b"\x1b[?2J");
        // Protected cell should survive
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert!(
            t.grid()
                .cell(0, 0)
                .unwrap()
                .flags
                .contains(CellFlags::PROTECTED)
        );
        // Unprotected cell should be erased
        assert!(t.grid().cell(1, 0).unwrap().is_blank());
    }

    #[test]
    fn t_decsed_from_cursor() {
        let mut t = Terminal::new(80, 24);
        // Protected A at (0,0), unprotected B at (1,0)
        feed(&mut t, b"\x1b[1\"qA\x1b[0\"qB");
        // Move cursor to (1,0)
        feed(&mut t, b"\x1b[1;1H");
        // Selective erase from cursor to end
        feed(&mut t, b"\x1b[?0J");
        // A survives (protected), B erased (unprotected)
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert!(t.grid().cell(1, 0).unwrap().is_blank());
    }

    #[test]
    fn t_decsed_to_cursor() {
        let mut t = Terminal::new(80, 24);
        // Protected A at (0,0), unprotected B at (1,0)
        feed(&mut t, b"\x1b[1\"qA\x1b[0\"qB");
        // Move cursor to (1,0)
        feed(&mut t, b"\x1b[2;1H");
        // Selective erase from start to cursor (inclusive)
        feed(&mut t, b"\x1b[?1J");
        // A survives (protected), B erased (unprotected)
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert!(t.grid().cell(1, 0).unwrap().is_blank());
    }

    #[test]
    fn t_decsca_2_is_unprotected() {
        let mut t = Terminal::new(80, 24);
        // DECSCA 2 = unprotected (same as 0): CSI 2 " q
        feed(&mut t, b"\x1b[2\"qA");
        assert!(
            !t.grid()
                .cell(0, 0)
                .unwrap()
                .flags
                .contains(CellFlags::PROTECTED)
        );
    }

    #[test]
    fn t_decsel_preserves_protected() {
        let mut t = Terminal::new(10, 3);
        // Set protected, print "AB", then unprotected, print "CD"
        feed(&mut t, b"\x1b[1\"qAB\x1b[0\"qCD");
        // DECSEL mode 2: erase entire line (non-protected only)
        feed(&mut t, b"\x1b[?2K");
        // Protected cells "AB" should survive
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B');
        // Non-protected "CD" should be erased
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, ' ');
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, ' ');
    }

    #[test]
    fn t_decsel_from_cursor() {
        let mut t = Terminal::new(10, 3);
        // Print "ABCDE" at cols 0-4
        feed(&mut t, b"ABCDE");
        // Overwrite col 2 with protected "X"
        feed(&mut t, b"\x1b[1;3H"); // move to col 2 (0-based)
        feed(&mut t, b"\x1b[1\"qX\x1b[0\"q"); // protected X
        // Move cursor to col 3 (0-based)
        feed(&mut t, b"\x1b[1;4H");
        // DECSEL mode 0: erase from cursor to end of line
        feed(&mut t, b"\x1b[?0K");
        // Cols 0-2 should survive (before cursor)
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'X'); // protected survived
        // Cols 3-4 should be erased
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, ' ');
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, ' ');
    }

    // ===== P24-E: OSC 9 / OSC 777 notification tests =====

    #[test]
    fn t_osc9_notification() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]9;Build complete\x1b\\");
        let note = t.take_pending_notification();
        assert_eq!(
            note,
            Some(("Terminal".to_string(), "Build complete".to_string()))
        );
        // Second call returns None
        assert!(t.take_pending_notification().is_none());
    }

    #[test]
    fn t_osc9_empty_ignored() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]9;\x1b\\");
        assert!(t.take_pending_notification().is_none());
    }

    #[test]
    fn t_osc9_progress_start() {
        let mut t = Terminal::new(80, 24);
        // OSC 9;4;0;50.0 — start progress at 50%
        feed(&mut t, b"\x1b]9;4;0;50.0\x1b\\");
        assert_eq!(t.progress(), Some(0.5));
    }

    #[test]
    fn t_osc9_progress_update() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]9;4;0;25.0\x1b\\");
        assert_eq!(t.progress(), Some(0.25));
        feed(&mut t, b"\x1b]9;4;0;75.0\x1b\\");
        assert_eq!(t.progress(), Some(0.75));
    }

    #[test]
    fn t_osc9_progress_hide() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]9;4;0;50.0\x1b\\");
        assert!(t.progress().is_some());
        // State 1 = hide/completed
        feed(&mut t, b"\x1b]9;4;1\x1b\\");
        assert!(t.progress().is_none());
    }

    #[test]
    fn t_osc9_progress_clamp() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]9;4;0;200.0\x1b\\");
        assert_eq!(t.progress(), Some(1.0));
        feed(&mut t, b"\x1b]9;4;0;-50.0\x1b\\");
        assert_eq!(t.progress(), Some(0.0));
    }

    #[test]
    fn t_osc777_notification() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]777;notify;Test Title;Body text\x1b\\");
        let note = t.take_pending_notification();
        assert_eq!(
            note,
            Some(("Test Title".to_string(), "Body text".to_string()))
        );
    }

    #[test]
    fn t_osc777_default_title() {
        let mut t = Terminal::new(80, 24);
        // Missing title — should default to "Terminal"
        feed(&mut t, b"\x1b]777;notify;;Body only\x1b\\");
        let note = t.take_pending_notification();
        assert_eq!(
            note,
            Some(("Terminal".to_string(), "Body only".to_string()))
        );
    }

    #[test]
    fn t_decpam_keypad_app_mode() {
        let mut t = Terminal::new(80, 24);
        assert!(!t.modes.keypad_app);
        feed(&mut t, b"\x1b="); // DECPAM
        assert!(t.modes.keypad_app);
    }

    #[test]
    fn t_decpnm_keypad_normal_mode() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b="); // DECPAM
        assert!(t.modes.keypad_app);
        feed(&mut t, b"\x1b>"); // DECPNM
        assert!(!t.modes.keypad_app);
    }

    #[test]
    fn t_sgr_blink_flag() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5m");
        assert!(t.flags.contains(CellFlags::BLINK));
        feed(&mut t, b"\x1b[25m");
        assert!(!t.flags.contains(CellFlags::BLINK));
    }

    #[test]
    fn t_modify_other_keys_set() {
        let mut t = Terminal::new(80, 24);
        assert_eq!(t.modes.modify_other_keys, 0);
        feed(&mut t, b"\x1b[>4;1h"); // Enable mode 1
        assert_eq!(t.modes.modify_other_keys, 1);
        feed(&mut t, b"\x1b[>4;2h"); // Enable mode 2
        assert_eq!(t.modes.modify_other_keys, 2);
    }

    #[test]
    fn t_modify_other_keys_reset() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[>4;2h");
        assert_eq!(t.modes.modify_other_keys, 2);
        feed(&mut t, b"\x1b[>4l"); // Disable
        assert_eq!(t.modes.modify_other_keys, 0);
    }

    #[test]
    fn t_osc21_title_query() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]0;My Title\x1b\\");
        assert_eq!(t.title(), "My Title");
        // Query title
        feed(&mut t, b"\x1b]21\x1b\\");
        let resp = t.take_response();
        assert!(
            resp.windows(3).any(|w| w == b"\x1b]l"),
            "response should contain OSC l"
        );
        assert!(resp.windows(8).any(|w| w == b"My Title"));
    }

    #[test]
    fn t_modify_other_keys_does_not_affect_insert_mode() {
        let mut t = Terminal::new(80, 24);
        // CSI > 4 ; 1 h should set modifyOtherKeys, NOT insert mode
        feed(&mut t, b"\x1b[>4;1h");
        assert_eq!(t.modes.modify_other_keys, 1);
        assert!(
            !t.modes.insert,
            "insert should NOT be set by modifyOtherKeys"
        );
    }

    #[test]
    fn t_kitty_keyboard_push_or_flags() {
        let mut t = Terminal::new(80, 24);
        assert_eq!(t.kitty_keyboard_flags(), 0);
        // Push flags: CSI > 1 u sets bit 0
        feed(&mut t, b"\x1b[>1u");
        assert_eq!(t.kitty_keyboard_flags(), 1);
        // Push more flags: CSI > 2 u sets bit 1
        feed(&mut t, b"\x1b[>2u");
        assert_eq!(t.kitty_keyboard_flags(), 3);
    }

    #[test]
    fn t_decrqm_modify_other_keys_default() {
        // Query modifyOtherKeys when disabled: CSI > 4 $ p
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[>4$p");
        let resp_bytes = t.take_response();
        let resp = String::from_utf8_lossy(&resp_bytes);
        assert!(
            resp.contains("\x1b[>4;2$y"),
            "DECRQM modifyOtherKeys default should be reset (2): got {resp:?}"
        );
    }

    #[test]
    fn t_decrqm_modify_other_keys_set() {
        // Set modifyOtherKeys mode 1, then query
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[>4;1h");
        feed(&mut t, b"\x1b[>4$p");
        let resp_bytes = t.take_response();
        let resp = String::from_utf8_lossy(&resp_bytes);
        assert!(
            resp.contains("\x1b[>4;1$y"),
            "DECRQM modifyOtherKeys mode 1 should be set (1): got {resp:?}"
        );
    }

    #[test]
    fn t_kitty_keyboard_pop_restores() {
        let mut t = Terminal::new(80, 24);
        // Push flag 1, then push flag 2
        feed(&mut t, b"\x1b[>1u");
        feed(&mut t, b"\x1b[>2u");
        assert_eq!(t.kitty_keyboard_flags(), 3);
        // Pop once: restores to 1
        feed(&mut t, b"\x1b[<1u");
        assert_eq!(t.kitty_keyboard_flags(), 1);
        // Pop again: restores to 0
        feed(&mut t, b"\x1b[<1u");
        assert_eq!(t.kitty_keyboard_flags(), 0);
    }

    #[test]
    fn t_kitty_keyboard_set_and_query() {
        let mut t = Terminal::new(80, 24);
        // Set flags directly: CSI = 1 ; 5 u
        feed(&mut t, b"\x1b[=1;5u");
        assert_eq!(t.kitty_keyboard_flags(), 5);
        // Query flags: CSI = 2 u
        feed(&mut t, b"\x1b[=2u");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("\x1b[?5u"),
            "kitty keyboard query should report flags 5, got: {s}"
        );
    }

    #[test]
    fn t_kitty_keyboard_rcp_still_works() {
        // Plain CSI u (RCP) should still restore cursor position
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5;3H"); // move to row 5, col 3
        feed(&mut t, b"\x1b[s"); // save cursor
        feed(&mut t, b"\x1b[10;10H"); // move elsewhere
        feed(&mut t, b"\x1b[u"); // restore cursor
        assert_eq!(t.cursor().0, 2, "col should be restored to 2 (0-based)");
        assert_eq!(t.cursor().1, 4, "row should be restored to 4 (0-based)");
    }

    #[test]
    fn t_xtgettcap_terminal_name() {
        // XTGETTCAP for "TN" (terminal name)
        // DCS + q 544e ST → "TN" in hex
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1bP+q544e\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        // Response should be DCS 1+r 544e=67677465726d ST
        // (TN=ggterm in hex encoding)
        assert!(
            s.contains("1+r544e="),
            "XTGETTCAP TN should start with 1+r544e=, got: {s}"
        );
        assert!(
            s.contains("67677465726d"),
            "XTGETTCAP TN response should contain hex 'ggterm' (67677465726d), got: {s}"
        );
    }

    #[test]
    fn t_xtgettcap_colors() {
        // XTGETTCAP for "Co" (number of colors)
        // "Co" in hex = 436f
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1bP+q436f\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        // Response should contain hex "256" = 323536
        assert!(
            s.contains("1+r") && s.contains("323536"),
            "XTGETTCAP Co should return hex 256 (323536), got: {s}"
        );
    }

    #[test]
    fn t_xtgettcap_rgb() {
        // XTGETTCAP for "RGB" (truecolor support)
        // "RGB" in hex = 524742
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1bP+q524742\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("1+r"),
            "XTGETTCAP RGB should return success, got: {s}"
        );
    }

    #[test]
    fn t_dcs_passthrough_ignored() {
        // tmux DCS passthrough should not crash or produce garbage
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1bPtmux;\x1b\x1b[?1000h\x1b\\");
        // Should not crash — grid should still be at default state
        assert_eq!(t.cursor().0, 0);
    }

    #[test]
    fn t_hex_encode_decode() {
        assert_eq!(hex_encode(b"TN"), "544e");
        assert_eq!(hex_encode(b"ggterm"), "67677465726d");
        assert_eq!(hex_decode(b"544e").as_deref(), Some("TN"));
        assert_eq!(hex_decode(b"67677465726d").as_deref(), Some("ggterm"));
        assert!(hex_decode(b"xyz").is_none()); // odd length
        assert!(hex_decode(b"zz").is_none()); // invalid hex
    }

    #[test]
    fn t_decrqss_sgr_default() {
        // DECRQSS for SGR: DCS $ q m ST
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1bP$qm\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("1$r0m"),
            "DECRQSS SGR default should return 0m, got: {s}"
        );
    }

    #[test]
    fn t_decrqss_sgr_bold() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[1m"); // bold on
        feed(&mut t, b"\x1bP$qm\x1b\\"); // query SGR
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("1$r1m"),
            "DECRQSS SGR with bold should return 1m, got: {s}"
        );
    }

    #[test]
    fn t_decrqss_scroll_region() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5;20r"); // set scroll region rows 5-20
        feed(&mut t, b"\x1bP$qr\x1b\\"); // query DECSTBM
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("1$r5;20r"),
            "DECRQSS DECSTBM should return 5;20r, got: {s}"
        );
    }

    #[test]
    fn t_decrqss_decsca() {
        let mut t = Terminal::new(80, 24);
        // Default unprotected → 0
        feed(&mut t, b"\x1bP$q\"q\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(s.contains("1$r0\"q"), "DECRQSS DECSCA default: {s}");

        // Set protected
        feed(&mut t, b"\x1b[1\"q");
        feed(&mut t, b"\x1bP$q\"q\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(s.contains("1$r1\"q"), "DECRQSS DECSCA protected: {s}");
    }

    #[test]
    fn t_decrqss_decscusr() {
        let mut t = Terminal::new(80, 24);
        // Default cursor style
        feed(&mut t, b"\x1bP$q q\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(s.contains("1$r0 q"), "DECRQSS DECSCUSR default: {s}");

        // Set to steady block (2)
        feed(&mut t, b"\x1b[2 q");
        feed(&mut t, b"\x1bP$q q\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(s.contains("1$r2 q"), "DECRQSS DECSCUSR steady block: {s}");
    }

    #[test]
    fn t_lnm_default_off() {
        let t = Terminal::new(80, 24);
        assert!(!t.new_line_mode(), "LNM should be off by default");
    }

    #[test]
    fn t_lnm_set_and_reset() {
        let mut t = Terminal::new(80, 24);
        // CSI 20 h — set LNM
        feed(&mut t, b"\x1b[20h");
        assert!(t.new_line_mode(), "LNM should be on after CSI 20 h");
        // CSI 20 l — reset LNM
        feed(&mut t, b"\x1b[20l");
        assert!(!t.new_line_mode(), "LNM should be off after CSI 20 l");
    }

    #[test]
    fn t_lnm_lf_produces_crlf() {
        let mut t = Terminal::new(10, 5);
        // Enable LNM, print text, then LF should move to col 0
        feed(&mut t, b"\x1b[20h");
        feed(&mut t, b"ABC");
        assert_eq!(t.cursor().0, 3); // at col 3
        feed(&mut t, b"\n"); // LF
        assert_eq!(t.cursor().0, 0, "LNM: LF should reset column to 0");
        assert_eq!(t.cursor().1, 1, "LNM: LF should move to next row");
    }

    #[test]
    fn t_lnm_off_lf_preserves_column() {
        let mut t = Terminal::new(10, 5);
        // LNM is off by default
        feed(&mut t, b"ABC");
        assert_eq!(t.cursor().0, 3);
        feed(&mut t, b"\n"); // LF
        assert_eq!(t.cursor().0, 3, "LNM off: LF should preserve column");
        assert_eq!(t.cursor().1, 1, "LF should move to next row");
    }

    #[test]
    fn t_decrm_mode_20_reports_lnm() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[20h"); // set LNM
        feed(&mut t, b"\x1b[20$p"); // DECRQM for mode 20
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("20;1$y"),
            "DECRQM should report LNM as set (1), got: {s}"
        );
    }

    #[test]
    fn t_xtversion_query() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[>q"); // XTVERSION query
        let resp = t.take_response();
        assert!(
            resp.windows(7).any(|w| w == b"ggterm("),
            "response should contain ggterm version, got: {:?}",
            String::from_utf8_lossy(&resp)
        );
    }

    #[test]
    fn t_da1_primary_device_attributes() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[c"); // DA1 query
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.starts_with("\x1b[?62;"),
            "DA1 should start with VT220 code, got: {s}"
        );
        assert!(
            s.contains(";29c") || s.contains(";29;"),
            "DA1 should report text locator (29) for OSC 8 hyperlinks, got: {s}"
        );
    }

    #[test]
    fn t_da2_secondary_device_attributes() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[>c"); // DA2 query
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.starts_with("\x1b[>41;"),
            "DA2 should report terminal class 41, got: {s}"
        );
    }

    #[test]
    fn t_da3_tertiary_device_attributes() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[=c"); // DA3 query
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.starts_with("\x1bP!|") && s.ends_with("\x1b\\"),
            "DA3 should respond with DCS format, got: {s}"
        );
    }

    #[test]
    fn t_decrqm_cursor_visible_set() {
        let mut t = Terminal::new(80, 24);
        // Cursor is visible by default (DECSET 25)
        feed(&mut t, b"\x1b[?25$p"); // Query DEC private mode 25
        let resp = t.take_response();
        let resp_str = String::from_utf8_lossy(&resp);
        assert!(
            resp_str.contains(";1$y"),
            "mode 25 should be set (1), got: {}",
            resp_str
        );
    }

    #[test]
    fn t_decrqm_cursor_visible_reset() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?25l"); // Hide cursor
        feed(&mut t, b"\x1b[?25$p"); // Query mode 25
        let resp = t.take_response();
        let resp_str = String::from_utf8_lossy(&resp);
        assert!(
            resp_str.contains(";2$y"),
            "mode 25 should be reset (2), got: {}",
            resp_str
        );
    }

    #[test]
    fn t_decrqm_bracketed_paste() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?2004h"); // Enable bracketed paste
        feed(&mut t, b"\x1b[?2004$p"); // Query mode 2004
        let resp = t.take_response();
        let resp_str = String::from_utf8_lossy(&resp);
        assert!(
            resp_str.contains("2004;1$y"),
            "mode 2004 should be set (1), got: {}",
            resp_str
        );
    }

    #[test]
    fn t_decrqm_unknown_mode() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?9999$p"); // Query unknown mode
        let resp = t.take_response();
        let resp_str = String::from_utf8_lossy(&resp);
        assert!(
            resp_str.contains("9999;2$y"),
            "unknown mode should be reset (2), got: {}",
            resp_str
        );
    }

    #[test]
    fn t_osc4_color_query_single() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]4;1;?\x1b\\"); // Query color index 1 (red)
        let resp = t.take_response();
        let resp_str = String::from_utf8_lossy(&resp);
        assert!(
            resp_str.contains("4;1;rgb:cd/00/00"),
            "red palette query should return rgb:cd/00/00, got: {}",
            resp_str
        );
    }

    #[test]
    fn t_osc4_color_query_multiple() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]4;0;?;7;?\x1b\\"); // Query black and white
        let resp = t.take_response();
        let resp_str = String::from_utf8_lossy(&resp);
        assert!(
            resp_str.contains("4;0;rgb:00/00/00"),
            "should contain black color response"
        );
        assert!(
            resp_str.contains("4;7;rgb:e5/e5/e5"),
            "should contain white color response"
        );
    }

    #[test]
    fn t_osc4_color_set_and_query() {
        // Set color index 1 (red) to a custom value, then query it.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]4;1;rgb:ff/00/ff\x1b\\"); // Set index 1 to magenta
        let _ = t.take_response(); // Clear any pending response

        // Query the modified color
        feed(&mut t, b"\x1b]4;1;?\x1b\\");
        let resp = t.take_response();
        let resp_str = String::from_utf8_lossy(&resp);
        assert!(
            resp_str.contains("4;1;rgb:ff/00/ff"),
            "after SET, query should return new color, got: {}",
            resp_str
        );
    }

    #[test]
    fn t_osc4_color_set_affects_resolve() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]4;2;rgb:01/02/03\x1b\\"); // Set index 2
        assert_eq!(
            t.resolve_palette_color(2),
            (1, 2, 3),
            "resolve_palette_color should return overridden value"
        );
        // Other indices should still return built-in colors
        assert_eq!(t.resolve_palette_color(1), (205, 0, 0));
    }

    #[test]
    fn t_osc104_reset_specific_palette_entry() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]4;1;rgb:ff/00/ff\x1b\\"); // Override index 1
        assert_eq!(t.resolve_palette_color(1), (255, 0, 255));

        // Reset index 1 only
        feed(&mut t, b"\x1b]104;1\x1b\\");
        assert_eq!(
            t.resolve_palette_color(1),
            (205, 0, 0),
            "should revert to built-in red after OSC 104 reset"
        );
    }

    #[test]
    fn t_osc104_reset_all_palette() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]4;1;rgb:ff/00/ff\x1b\\"); // Override index 1
        feed(&mut t, b"\x1b]4;5;rgb:01/02/03\x1b\\"); // Override index 5

        // Reset ALL palette entries
        feed(&mut t, b"\x1b]104\x1b\\");
        assert_eq!(t.resolve_palette_color(1), (205, 0, 0), "index 1 reverted");
        assert_eq!(
            t.resolve_palette_color(5),
            (205, 0, 205),
            "index 5 reverted"
        );
        assert!(t.palette_overrides().is_empty(), "all overrides cleared");
    }

    #[test]
    fn t_decset12_cursor_blink_default() {
        let t = Terminal::new(80, 24);
        assert!(
            t.cursor_blink_enabled(),
            "cursor blink should be enabled by default"
        );
    }

    #[test]
    fn t_decset12_cursor_blink_off() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?12l"); // Disable cursor blink
        assert!(!t.cursor_blink_enabled());
    }

    #[test]
    fn t_decset12_cursor_blink_on() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?12l"); // Disable
        assert!(!t.cursor_blink_enabled());
        feed(&mut t, b"\x1b[?12h"); // Enable
        assert!(t.cursor_blink_enabled());
    }

    #[test]
    fn t_decrqm_cursor_blink() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?12l"); // Disable
        feed(&mut t, b"\x1b[?12$p"); // Query mode 12
        let resp = t.take_response();
        let resp_str = String::from_utf8_lossy(&resp);
        assert!(
            resp_str.contains("12;2$y"),
            "mode 12 should be reset (2), got: {}",
            resp_str
        );
    }

    #[test]
    fn t_decset5_reverse_video_default() {
        let t = Terminal::new(80, 24);
        assert!(!t.reverse_video(), "reverse video should be off by default");
    }

    #[test]
    fn t_decset5_reverse_video_on() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?5h"); // Enable reverse video
        assert!(t.reverse_video());
    }

    #[test]
    fn t_decset5_reverse_video_off() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?5h"); // Enable
        feed(&mut t, b"\x1b[?5l"); // Disable
        assert!(!t.reverse_video());
    }

    #[test]
    fn t_decrqm_reverse_video() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?5h"); // Enable
        feed(&mut t, b"\x1b[?5$p"); // Query mode 5
        let resp = t.take_response();
        let resp_str = String::from_utf8_lossy(&resp);
        assert!(
            resp_str.contains("5;1$y"),
            "mode 5 should be set (1), got: {}",
            resp_str
        );
    }

    #[test]
    fn t_sgr58_underline_color_rgb() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[58;2;100;150;200m");
        assert_eq!(t.underline_color, Color::Rgb(100, 150, 200));
    }

    #[test]
    fn t_sgr58_underline_color_indexed() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[58;5;42m");
        assert_eq!(t.underline_color, Color::Indexed(42));
    }

    #[test]
    fn t_sgr59_default_underline_color() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[58;5;42m");
        assert_eq!(t.underline_color, Color::Indexed(42));
        feed(&mut t, b"\x1b[59m");
        assert_eq!(t.underline_color, Color::Default);
    }

    #[test]
    fn t_sgr0_resets_underline_color() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[58;2;10;20;30m");
        feed(&mut t, b"\x1b[0m");
        assert_eq!(t.underline_color, Color::Default);
    }

    #[test]
    fn t_dcs_sequence_not_printed() {
        // DCS sequences (ESC P ... ST) must be consumed and NOT printed
        // to the screen. Programs like tmux send DCS for capability queries.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"AB\x1bP1;2;3qSOME DCS DATA\x1b\\CD");
        // A, B should be at columns 0-1, C, D at columns 2-3
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'C');
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'D');
    }

    #[test]
    fn t_dcs_bel_terminated() {
        // Some implementations use BEL instead of ST to terminate DCS.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"X\x1bP1qdata\x07Y");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'Y');
    }

    #[test]
    fn t_sos_pm_apc_consumed() {
        // ESC X (SOS), ESC ^ (PM), ESC _ (APC) must be consumed like DCS.
        let mut t = Terminal::new(80, 24);
        // SOS
        feed(&mut t, b"A\x1bXsome text\x1b\\B");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B');
        // PM
        feed(&mut t, b"\x1b^private\x07C");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'C');
        // APC
        feed(&mut t, b"\x1b_apc data\x1b\\D");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'D');
    }

    #[test]
    fn t_enq_answerback() {
        // ENQ (0x05) should trigger an answerback response.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x05");
        let resp = t.take_response();
        assert_eq!(resp, b"ggterm");
    }

    // ── DECRQM extended mode tests ─────────────────────────────

    #[test]
    fn t_decrqm_focus_event_mode() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?1004h"); // Enable focus events
        feed(&mut t, b"\x1b[?1004$p"); // Query mode 1004
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("1004;1$y"),
            "focus event should be set, got: {s}"
        );
    }

    #[test]
    fn t_decrqm_autowrap_mode() {
        let mut t = Terminal::new(80, 24);
        // Autowrap is on by default.
        feed(&mut t, b"\x1b[7$p"); // Query ANSI mode 7 (DECAWM)
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("7;1$y"),
            "autowrap should be set by default, got: {s}"
        );
        feed(&mut t, b"\x1b[?7l"); // Disable autowrap
        feed(&mut t, b"\x1b[7$p"); // Query again
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(s.contains("7;2$y"), "autowrap should be reset, got: {s}");
    }

    #[test]
    fn t_decrqm_synchronized_output() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?2026$p"); // Query mode 2026 (should be off)
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("2026;2$y"),
            "sync output should be reset, got: {s}"
        );
    }

    #[test]
    fn t_decrqm_reflow_default() {
        let t = Terminal::new(80, 24);
        assert!(t.reflow_enabled()); // Default on
    }

    #[test]
    fn t_decrqm_mouse_sgr() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?1006h"); // Enable SGR mouse
        feed(&mut t, b"\x1b[?1006$p"); // Query mode 1006
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(s.contains("1006;1$y"), "SGR mouse should be set, got: {s}");
    }

    #[test]
    fn t_decrqm_mouse_sgr_pixel() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?1016h"); // Enable SGR pixel mouse
        feed(&mut t, b"\x1b[?1016$p"); // Query mode 1016
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("1016;1$y"),
            "SGR pixel mouse should be set, got: {s}"
        );
    }

    #[test]
    fn t_decrqm_origin_mode() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?6h"); // Enable origin mode
        feed(&mut t, b"\x1b[?6$p"); // Query mode 6
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(s.contains("6;1$y"), "origin mode should be set, got: {s}");
    }

    #[test]
    fn t_decrqm_auto_wrap_default() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?7$p"); // Query mode 7
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("7;1$y"),
            "auto_wrap should be set by default, got: {s}"
        );
    }

    #[test]
    fn t_decrqm_ansi_irm() {
        // ANSI mode 4 (IRM) — insert mode
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[4h"); // Enable insert mode
        feed(&mut t, b"\x1b[4$p"); // Query ANSI mode 4
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(s.contains("4;1$y"), "IRM should be set, got: {s}");
    }

    #[test]
    fn t_decrqm_ansi_auto_repeat() {
        // ANSI mode 8 (ARM) — auto-repeat, should be permanently set (3)
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[8$p");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("8;3$y"),
            "Auto-repeat should be permanently set, got: {s}"
        );
    }

    #[test]
    fn t_decrqm_x10_mouse() {
        // Private mode 9 (X10 mouse)
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?9$p");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("?9;2$y"),
            "X10 mouse should be reset by default, got: {s}"
        );
        // Enable it
        feed(&mut t, b"\x1b[?9h");
        feed(&mut t, b"\x1b[?9$p");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("?9;1$y"),
            "X10 mouse should be set after enable, got: {s}"
        );
    }

    #[test]
    fn t_title_push_pop() {
        // Set title → push → change → pop → restore
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]2;My Shell\x07"); // OSC 2: set title
        assert_eq!(t.title(), "My Shell");

        feed(&mut t, b"\x1b[22;2t"); // Push title
        feed(&mut t, b"\x1b]2;vim\x07"); // Change title
        assert_eq!(t.title(), "vim");

        feed(&mut t, b"\x1b[23;2t"); // Pop title
        assert_eq!(t.title(), "My Shell");
    }

    #[test]
    fn t_title_push_pop_multiple() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]0;A\x07"); // Set title A
        feed(&mut t, b"\x1b[22t"); // Push
        feed(&mut t, b"\x1b]0;B\x07"); // Set title B
        feed(&mut t, b"\x1b[22t"); // Push
        feed(&mut t, b"\x1b]0;C\x07"); // Set title C
        assert_eq!(t.title(), "C");

        feed(&mut t, b"\x1b[23t"); // Pop → B
        assert_eq!(t.title(), "B");

        feed(&mut t, b"\x1b[23t"); // Pop → A
        assert_eq!(t.title(), "A");
    }

    #[test]
    fn t_csi_18t_size_report() {
        let mut t = Terminal::new(120, 40);
        feed(&mut t, b"\x1b[18t");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        // Should report CSI 8 ; 40 ; 120 t (rows=40, cols=120)
        assert!(s.contains("8;40;120t"), "size report wrong, got: {s}");
    }

    #[test]
    fn t_csi_11t_window_state() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[11t");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        // Should respond CSI 1 t (not iconified)
        assert!(s.contains("\x1b[1t"), "window state report wrong, got: {s}");
    }

    #[test]
    fn t_decreqtparm_response() {
        // CSI x (DECREQTPARM) — programs use this during terminal init.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[x");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("\x1b[2;1;0;0;0;0x"),
            "DECREQTPARM response wrong, got: {s}"
        );
    }

    #[test]
    fn t_csi_21t_title_query() {
        // CSI 21t — report window title (tmux uses this).
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]2;My Title\x1b\\"); // Set title via OSC 2
        feed(&mut t, b"\x1b[21t"); // Query title
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("\x1b]lMy Title\x1b\\"),
            "title report wrong, got: {s}"
        );
    }

    #[test]
    fn t_color_palette_16_colors() {
        assert_eq!(color_for_index(0), (0, 0, 0));
        assert_eq!(color_for_index(7), (229, 229, 229));
        assert_eq!(color_for_index(15), (255, 255, 255));
    }

    #[test]
    fn t_color_palette_cube() {
        // Index 16 = (0, 0, 0) — start of cube
        assert_eq!(color_for_index(16), (0, 0, 0));
        // Index 21 = (0, 0, 255) — blue max
        assert_eq!(color_for_index(21), (0, 0, 255));
        // Index 196 = (255, 0, 0) — red max
        assert_eq!(color_for_index(196), (255, 0, 0));
        // Index 231 = (255, 255, 255) — white max
        assert_eq!(color_for_index(231), (255, 255, 255));
    }

    #[test]
    fn t_color_palette_grayscale() {
        // Index 232 = darkest gray (8)
        assert_eq!(color_for_index(232), (8, 8, 8));
        // Index 255 = lightest gray (238)
        let v = 8 + (255 - 232) * 10;
        assert_eq!(color_for_index(255), (v, v, v));
        // Middle of ramp
        assert_eq!(
            color_for_index(243),
            (8 + 11 * 10, 8 + 11 * 10, 8 + 11 * 10)
        );
    }

    #[test]
    fn t_osc4_query_256_color() {
        // Querying palette index 196 (red) should return correct RGB
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]4;196;?\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("4;196;rgb:ff/00/00"),
            "OSC 4 query for 196 should be rgb:ff/00/00, got: {s}"
        );
    }

    // ================================================================
    //  OSC 1337 — iTerm2 shell integration (4 tests)
    // ================================================================

    #[test]
    fn t_osc1337_current_dir() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]1337;CurrentDir=/home/user/projects\x1b\\");
        assert_eq!(t.cwd().unwrap().to_str().unwrap(), "/home/user/projects");
    }

    #[test]
    fn t_osc1337_remote_host() {
        let mut t = Terminal::new(80, 24);
        feed(
            &mut t,
            b"\x1b]1337;RemoteHost=root@server.example.com\x1b\\",
        );
        assert_eq!(t.remote_host().unwrap(), "root@server.example.com");
    }

    #[test]
    fn t_osc1337_set_mark() {
        let mut t = Terminal::new(80, 24);
        // Move cursor to row 5
        feed(&mut t, b"\x1b[6;1H");
        feed(&mut t, b"\x1b]1337;SetMark\x1b\\");
        assert_eq!(t.mark_row(), Some(5));
    }

    #[test]
    fn t_osc1337_clear_scrollback() {
        let mut t = Terminal::new(10, 3);
        // Fill content and scroll to generate scrollback
        feed(&mut t, b"AAAA\nBBBB\nCCCC\nDDDD");
        assert!(t.grid().scrollback_len() > 0);
        // Clear scrollback
        feed(&mut t, b"\x1b]1337;ClearScrollback\x1b\\");
        assert_eq!(t.grid().scrollback_len(), 0);
    }

    #[test]
    fn t_osc1337_set_user_var() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]1337;SetUserVar=git_branch=main\x1b\\");
        assert_eq!(t.user_var("git_branch"), Some("main"));
        // Overwrite
        feed(
            &mut t,
            b"\x1b]1337;SetUserVar=git_branch=feature/test\x1b\\",
        );
        assert_eq!(t.user_var("git_branch"), Some("feature/test"));
        // Multiple vars
        feed(&mut t, b"\x1b]1337;SetUserVar=project_name=ggterm\x1b\\");
        assert_eq!(t.user_var("project_name"), Some("ggterm"));
        assert_eq!(t.user_var("git_branch"), Some("feature/test"));
    }

    // ================================================================
    //  OSC 104 / 110 / 111 / 112 — Reset dynamic colors
    // ================================================================

    #[test]
    fn t_osc110_reset_dynamic_fg() {
        let mut t = Terminal::new(80, 24);
        // Set dynamic fg via OSC 10
        feed(&mut t, b"\x1b]10;rgb:ff/00/00\x1b\\");
        assert!(t.dynamic_fg().is_some());
        // Reset via OSC 110
        feed(&mut t, b"\x1b]110\x1b\\");
        assert!(t.dynamic_fg().is_none());
    }

    #[test]
    fn t_osc111_reset_dynamic_bg() {
        let mut t = Terminal::new(80, 24);
        // Set dynamic bg via OSC 11
        feed(&mut t, b"\x1b]11;rgb:00/ff/00\x1b\\");
        assert!(t.dynamic_bg().is_some());
        // Reset via OSC 111
        feed(&mut t, b"\x1b]111\x1b\\");
        assert!(t.dynamic_bg().is_none());
    }

    #[test]
    fn t_osc112_reset_dynamic_cursor() {
        let mut t = Terminal::new(80, 24);
        // Set dynamic cursor via OSC 12
        feed(&mut t, b"\x1b]12;rgb:00/00/ff\x1b\\");
        assert!(t.dynamic_cursor().is_some());
        // Reset via OSC 112
        feed(&mut t, b"\x1b]112\x1b\\");
        assert!(t.dynamic_cursor().is_none());
    }

    #[test]
    fn t_osc104_reset_palette_consumed() {
        // OSC 104 should be consumed without error or panic.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]104\x1b\\");
        feed(&mut t, b"\x1b]104;0;1;2\x1b\\");
        // No crash = success. Response buffer should be empty.
        assert!(t.take_response().is_empty());
    }

    // ================================================================
    // DECSC / DECRC — full state save/restore (ESC 7 / ESC 8)
    // ================================================================

    #[test]
    fn t_decsc_restores_cursor_position() {
        let mut t = Terminal::new(80, 24);
        // Move cursor to row 5, col 10
        feed(&mut t, b"\x1b[6;11H");
        // Save state (ESC 7)
        feed(&mut t, b"\x1b7");
        // Move cursor away
        feed(&mut t, b"\x1b[1;1H");
        assert_eq!((t.cursor().0, t.cursor().1), (0, 0));
        // Restore (ESC 8)
        feed(&mut t, b"\x1b8");
        assert_eq!((t.cursor().0, t.cursor().1), (10, 5));
    }

    #[test]
    fn t_decsc_restores_sgr_attributes() {
        let mut t = Terminal::new(80, 24);
        // Set bold + red foreground
        feed(&mut t, b"\x1b[1;31m");
        // Save state
        feed(&mut t, b"\x1b7");
        // Clear attributes
        feed(&mut t, b"\x1b[0m");
        assert!(!t.flags.contains(CellFlags::BOLD));
        // Restore
        feed(&mut t, b"\x1b8");
        assert!(t.flags.contains(CellFlags::BOLD));
    }

    #[test]
    fn t_decsc_restores_autowrap_mode() {
        let mut t = Terminal::new(80, 24);
        // Disable autowrap
        feed(&mut t, b"\x1b[?7l");
        assert!(!t.modes.auto_wrap);
        // Save
        feed(&mut t, b"\x1b7");
        // Re-enable autowrap
        feed(&mut t, b"\x1b[?7h");
        assert!(t.modes.auto_wrap);
        // Restore — should be disabled again
        feed(&mut t, b"\x1b8");
        assert!(!t.modes.auto_wrap);
    }

    #[test]
    fn t_decsc_no_saved_state_restores_home() {
        // DECRC without prior DECSC should restore cursor to (0,0) and
        // reset SGR attributes to defaults (VT220 spec).
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[10;10H");
        feed(&mut t, b"\x1b[1;31m"); // bold + red
        feed(&mut t, b"\x1b[?6h"); // origin mode on
        feed(&mut t, b"\x1b8");
        assert_eq!((t.cursor().0, t.cursor().1), (0, 0));
        assert_eq!(t.fg, Color::Default);
        assert!(!t.flags.contains(CellFlags::BOLD));
        assert!(!t.modes.origin);
        assert!(t.modes.auto_wrap);
    }

    #[test]
    fn t_decsc_restores_origin_mode() {
        let mut t = Terminal::new(80, 24);
        // Enable origin mode
        feed(&mut t, b"\x1b[?6h");
        assert!(t.modes.origin);
        // Save
        feed(&mut t, b"\x1b7");
        // Disable origin mode
        feed(&mut t, b"\x1b[?6l");
        assert!(!t.modes.origin);
        // Restore — should be enabled again
        feed(&mut t, b"\x1b8");
        assert!(t.modes.origin);
    }

    #[test]
    fn t_decsc_restores_protected_attr() {
        let mut t = Terminal::new(80, 24);
        // Enable protected attribute (DECSCA 1) — CSI 1 " q
        feed(&mut t, b"\x1b[1\"q");
        assert!(t.protected_attr);
        // Save
        feed(&mut t, b"\x1b7");
        // Disable protected attribute (DECSCA 2) — CSI 2 " q
        feed(&mut t, b"\x1b[2\"q");
        assert!(!t.protected_attr);
        // Restore — should be enabled again
        feed(&mut t, b"\x1b8");
        assert!(t.protected_attr);
    }

    // ===== Robustness / edge case tests =====

    #[test]
    fn t_empty_terminal_feed() {
        // Feeding zero bytes should not panic.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"");
        assert_eq!(t.cursor(), (0, 0));
    }

    #[test]
    fn t_partial_escape_sequence() {
        // Partial ESC sequence at end of input should not panic.
        // The VTE parser maintains state across feed() calls.
        let mut t = Terminal::new(80, 24);
        let mut p = crate::vte::Parser::new();
        p.feed(b"hello\x1b[3", &mut t);
        p.feed(b"1m", &mut t);
        // Should have processed the SGR 31 (red foreground)
        assert_eq!(t.fg, Color::Indexed(1));
    }

    #[test]
    fn t_nul_byte_ignored() {
        // NUL bytes should be silently ignored.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"AB\x00CD");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'C');
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'D');
    }

    #[test]
    fn t_resize_to_minimum() {
        // Resizing to 1x1 should not panic.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"Hello World");
        t.resize(1, 1);
        assert_eq!(t.grid().width(), 1);
        assert_eq!(t.grid().height(), 1);
    }

    #[test]
    fn t_grow_terminal() {
        // Growing terminal should not lose content.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABC");
        t.resize(20, 5);
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'C');
    }

    #[test]
    fn test_osc_with_invalid_utf8() {
        // OSC with invalid UTF-8 should not panic.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]0;\xff\xfe\x1b\\");
        // Should not crash; title may contain replacement chars
    }

    #[test]
    fn t_multiple_reset() {
        // Multiple RIS resets should be safe.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[31mHELLO\x1b[2J\x1b[H");
        t.ris();
        t.ris();
        t.ris();
        assert_eq!(t.cursor(), (0, 0));
        assert_eq!(t.fg, Color::Default);
    }

    #[test]
    fn test_csi_with_many_params() {
        // CSI with many parameters should not panic.
        let mut t = Terminal::new(80, 24);
        let params: String = (0..50).map(|i| format!("{};", i)).collect();
        let seq = format!("\x1b[{}m", params); // SGR with 50 params
        feed(&mut t, seq.as_bytes());
        // Should not crash
    }

    #[test]
    fn test_extract_row_text_simple() {
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"Hello World");
        let text = t.extract_row_text(0);
        assert_eq!(text, "Hello World");
    }

    #[test]
    fn test_extract_row_text_empty() {
        let t = Terminal::new(10, 5);
        assert_eq!(t.extract_row_text(0), "");
    }

    #[test]
    fn test_extract_row_text_trims_trailing() {
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"ab   ");
        // Trailing spaces should be trimmed.
        assert_eq!(t.extract_row_text(0), "ab");
    }

    #[test]
    fn test_extract_row_text_wide_char() {
        let mut t = Terminal::new(20, 5);
        // Feed a CJK wide character (U+4E2D = 中).
        feed(&mut t, "中".as_bytes());
        // The wide char occupies 2 cells; the spacer should be skipped.
        let text = t.extract_row_text(0);
        assert_eq!(text, "中");
    }

    #[test]
    fn test_extract_row_text_multiple_rows() {
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"Line1\r\nLine2");
        assert_eq!(t.extract_row_text(0), "Line1");
        assert_eq!(t.extract_row_text(1), "Line2");
    }

    #[test]
    fn test_extract_row_text_combining_char() {
        let mut t = Terminal::new(20, 5);
        // Feed "e" followed by U+0301 (combining acute accent → é).
        feed(&mut t, "e\u{0301}".as_bytes());
        let text = t.extract_row_text(0);
        // Should include both the base char and the combining mark.
        assert_eq!(text, "e\u{0301}");
    }

    #[test]
    fn test_last_output_time_set_on_print() {
        let mut t = Terminal::new(10, 3);
        assert!(t.last_output_time().is_none());
        feed(&mut t, b"hi");
        assert!(t.last_output_time().is_some());
    }

    #[test]
    fn test_last_output_time_not_set_by_escape() {
        let mut t = Terminal::new(10, 3);
        // Escape sequences should not update last_output_time.
        feed(&mut t, b"\x1b[31m");
        // print() is not called for escape sequences.
        assert!(t.last_output_time().is_none());
    }
}
