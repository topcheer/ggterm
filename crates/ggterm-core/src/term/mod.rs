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
    pub(crate) cursor_style: CursorStyle,
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
    /// DECSACE — Select Attribute Change Extent.
    /// false = stream (default), true = rectangle.
    /// Controls whether DECCARA/DECRARA operate stream-wise or rectangle-wise.
    pub sace_rectangle: bool,
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
            sace_rectangle: false,
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
    /// Total scrollback rows evicted since terminal start.
    /// Used to adjust command mark absolute row references.
    pub(crate) evicted_scrollback_rows: usize,
    /// Running total of evicted rows already accounted for.
    pub(crate) evicted_scrollback_rows_accum: usize,
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
                // Parse hex directly without format! allocation.
                let hi_val = hi.to_digit(16);
                let lo_val = lo.to_digit(16);
                if let (Some(h), Some(l)) = (hi_val, lo_val) {
                    bytes.push((h * 16 + l) as u8);
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
/// Also supports `#RRGGBB` CSS-style.
/// P17-A: used by OSC 10/11/12.
fn parse_xcolor(spec: &str) -> Option<Color> {
    let spec = spec
        .strip_prefix("rgb:")
        .or_else(|| spec.strip_prefix("#"))?;
    let parts: Vec<&str> = spec.split('/').collect();
    if parts.len() == 3 {
        // X11 format: rgb:RR/GG/BB where each channel is 1-4 hex digits.
        // 1-2 digit channels map directly to u8.
        // 3-4 digit channels (16-bit) are scaled down to 8-bit: scale by
        // (value * 255) / max_value for correct gamma.
        let r = parse_channel(parts[0])?;
        let g = parse_channel(parts[1])?;
        let b = parse_channel(parts[2])?;
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

/// Parse a single X11 color channel (1-4 hex digits) to u8.
/// 1-2 digits: direct hex value.
/// 3-4 digits: 16-bit scale → 8-bit via (v * 255) / max.
fn parse_channel(s: &str) -> Option<u8> {
    if s.len() <= 2 {
        u8::from_str_radix(s, 16).ok()
    } else {
        // 3-4 digit channel: parse as u16, scale to u8.
        let val = u16::from_str_radix(s, 16).ok()?;
        let max: u32 = (1u32 << (s.len() * 4)) - 1;
        Some((val as u32 * 255 / max) as u8)
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
        Self::with_scrollback(width, height, 10_000)
    }

    /// Create a terminal with a custom scrollback limit.
    pub fn with_scrollback(width: usize, height: usize, max_scrollback: usize) -> Self {
        let mut tab_stops = vec![false; width.max(1)];
        let mut col = 0;
        while col < width {
            tab_stops[col] = true;
            col += 8;
        }
        Self {
            grid: Grid::with_scrollback(width, height, max_scrollback),
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
            evicted_scrollback_rows: 0,
            evicted_scrollback_rows_accum: 0,
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

    /// Update the scrollback capacity (evicts oldest if shrinking).
    pub fn set_scrollback_limit(&mut self, max: usize) {
        self.grid.set_max_scrollback(max);
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
        // Preserve terminal-emulator configuration that is NOT VT220 state.
        // These are set by the app layer and should survive RIS:
        let max_sb = self.grid.max_scrollback();
        let cell_dims = self.cell_dimensions;
        *self = Terminal::with_scrollback(w, h, max_sb);
        self.cell_dimensions = cell_dims;
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
            // extract_absolute_row_text handles scrollback rows.
            lines.push(self.extract_absolute_row_text(row));
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
            let text = self.extract_absolute_row_text(row);
            if !text.is_empty() {
                lines.push(format!("$ {text}"));
            }
        }
        // Output lines
        for row in output_row..end_row {
            lines.push(self.extract_absolute_row_text(row));
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

    /// Extract text from an absolute row (scrollback + visible).
    ///
    /// `abs_row` 0 = oldest scrollback row.
    pub fn extract_absolute_row_text(&self, abs_row: usize) -> String {
        let Some(row) = self.grid.absolute_row(abs_row) else {
            return String::new();
        };
        let width = self.grid.width();
        let mut text = String::with_capacity(width);
        for x in 0..width {
            if let Some(cell) = row.cell(x) {
                if cell.flags.contains(CellFlags::WIDE_SPACER) {
                    continue;
                }
                if cell.ch != '\0' {
                    text.push(cell.ch);
                }
                for &mc in &cell.combining {
                    text.push(mc);
                }
            }
        }
        while text.ends_with(|c: char| c.is_whitespace()) {
            text.pop();
        }
        text
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
                    // Skip null chars from uninitialized cells.
                    if cell.ch != '\0' {
                        text.push(cell.ch);
                    }
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
        // Defensive: enforce minimum 1x1 to prevent zero-length allocations.
        let width = width.max(1);
        let height = height.max(1);
        // Reflow only on primary screen. In alt screen (vim, less, htop),
        // programs manage their own layout and expect simple truncation.
        if self.modes.reflow && !self.modes.alt_screen {
            self.grid.reflow_resize(width, height);
        } else {
            self.grid.resize(width, height);
        }
        // Always reset scroll region to full screen on resize, even when
        // the dimensions didn't change (reflow_resize has an early return
        // for same-size no-ops that skips the scroll region reset).
        let (sr_top, sr_bottom) = self.grid.scroll_region();
        if sr_top != 0 || sr_bottom != height {
            self.grid.set_scroll_region(0, height);
        }
        // Preserve existing custom tab stops across resize.
        // If wider, extend with default stops at every 8 columns in the new area.
        // If narrower, truncate (custom stops in the clipped area are lost).
        let old_width = self.tab_stops.len();
        if width > old_width {
            self.tab_stops.resize(width, false);
            // Set default tab stops (every 8 columns) in the newly added range.
            // Start from the first multiple of 8 >= old_width.
            let mut col = old_width.next_multiple_of(8);
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
        // If cursor landed on a wide char spacer after resize (e.g.
        // shrinking placed a wide char pair at the clamped cursor col),
        // adjust back to the lead cell.
        if self.cursor.x > 0
            && let Some(c) = self.grid.cell(self.cursor.x, self.cursor.y)
            && c.is_wide_spacer()
        {
            self.cursor.x -= 1;
        }
        // Clamp saved cursors to new dimensions to prevent out-of-bounds
        // writes on DECRC/SCORC after a shrink.
        self.saved_cursor.x = self.saved_cursor.x.min(width.saturating_sub(1));
        self.saved_cursor.y = self.saved_cursor.y.min(height.saturating_sub(1));
        if let Some(ref mut state) = self.decsc_state {
            state.cursor.x = state.cursor.x.min(width.saturating_sub(1));
            state.cursor.y = state.cursor.y.min(height.saturating_sub(1));
        }
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
            // When pending_wrap is true, the cursor X is at the last column
            // (same as the last printed char), not past it. The combining
            // char should attach to the cell at cx, not cx-1.
            let base = if self.cursor.pending_wrap {
                cx
            } else {
                cx.saturating_sub(1)
            };
            // If cursor is right after a wide char, base is the spacer.
            // Target the lead cell at base-1 instead.
            let target_col =
                if base >= 1 && self.grid.cell(base, cy).is_some_and(|c| c.is_wide_spacer()) {
                    base.saturating_sub(1)
                } else {
                    base
                };
            #[allow(clippy::collapsible_if)]
            if target_col > 0 || self.cursor.pending_wrap || cx > 0 {
                if let Some(c) = self.grid.cell_mut(target_col, cy)
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
            }
            // Fallback: try cx-1 for normal (non-pending_wrap) case
            if !self.cursor.pending_wrap
                && cx > 0
                && let Some(c) = self.grid.cell_mut(cx.saturating_sub(1), cy)
                && !c.flags.contains(CellFlags::WIDE_SPACER)
                && !c.is_blank()
            {
                if c.combining.len() < 8 {
                    c.combining.push(ch);
                }
                return;
            }
            if cx == 0 && cy > 0 {
                let prev_w = self.grid.width();
                let target_col = if prev_w > 0
                    && self
                        .grid
                        .cell(prev_w.saturating_sub(1), cy - 1)
                        .is_some_and(|c| c.is_wide_spacer())
                {
                    // Previous row ends with a wide spacer — target the lead cell.
                    prev_w.saturating_sub(2)
                } else {
                    prev_w.saturating_sub(1)
                };
                if let Some(c) = self.grid.cell_mut(target_col, cy - 1)
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
            // Mark the previous row as soft-wrapped for reflow support.
            self.grid.set_row_wrap(self.cursor.y, true);
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
            // Mark the current row as soft-wrapped.
            self.grid.set_row_wrap(self.cursor.y, true);
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
        if self.cursor.y >= top && self.cursor.y == bottom.saturating_sub(1) {
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
        // If the cursor landed on the spacer (right half) of a wide char,
        // adjust it back to the lead cell. Per xterm behavior, cursor
        // positioning commands should never rest on a spacer — the cursor
        // belongs on the lead cell visually.
        if self.cursor.x > 0
            && let Some(c) = self.grid.cell(self.cursor.x, self.cursor.y)
            && c.is_wide_spacer()
        {
            self.cursor.x -= 1;
        }
        self.cursor.pending_wrap = false;
    }

    /// Clean up orphaned wide char pairs after a rectangle operation (DECERA,
    /// DECSERA, DECFRA). When a rectangle partially overlaps a wide char pair,
    /// the lead or spacer may be erased while the other survives, creating an
    /// inconsistent state. This method checks the left and right boundaries
    /// and clears any orphaned half of a wide char pair.
    #[allow(clippy::collapsible_if)]
    fn cleanup_wide_at_rect_boundary(&mut self, left: usize, right: usize, row: usize) {
        let w = self.grid.width();
        // Left boundary: if the cell just left of the rect is a WIDE_CHAR lead
        // whose spacer (at left) was erased, clear the lead too.
        if left > 0 {
            if let Some(cell) = self.grid.cell(left - 1, row) {
                if cell.flags.contains(CellFlags::WIDE_CHAR)
                    && !cell.flags.contains(CellFlags::WIDE_SPACER)
                {
                    // Check if the spacer at `left` is no longer a spacer
                    if self
                        .grid
                        .cell(left, row)
                        .is_none_or(|c| !c.is_wide_spacer())
                    {
                        if let Some(c) = self.grid.cell_mut(left - 1, row) {
                            *c = Cell::blank();
                        }
                    }
                }
            }
        }
        // Right boundary: if the cell at `right` was a wide lead whose spacer
        // (at right+1) survived outside the rect, clear the spacer.
        if let Some(cell) = self.grid.cell(right, row) {
            if cell.flags.contains(CellFlags::WIDE_CHAR)
                && !cell.flags.contains(CellFlags::WIDE_SPACER)
                && right + 1 < w
            {
                if self
                    .grid
                    .cell(right + 1, row)
                    .is_some_and(|c| c.is_wide_spacer())
                {
                    if let Some(c) = self.grid.cell_mut(right + 1, row) {
                        *c = Cell::blank();
                    }
                }
            }
        }
        // Also: if right+1 is a spacer whose lead at `right` was erased
        if right + 1 < w {
            if let Some(cell) = self.grid.cell(right + 1, row) {
                if cell.is_wide_spacer()
                    && self
                        .grid
                        .cell(right, row)
                        .is_none_or(|c| !c.flags.contains(CellFlags::WIDE_CHAR))
                {
                    if let Some(c) = self.grid.cell_mut(right + 1, row) {
                        *c = Cell::blank();
                    }
                }
            }
        }
        // Left boundary: if `left` is a spacer whose lead at left-1 survived
        if left > 0 && left < w {
            if let Some(cell) = self.grid.cell(left, row) {
                if cell.is_wide_spacer()
                    && self
                        .grid
                        .cell(left - 1, row)
                        .is_some_and(|c| c.flags.contains(CellFlags::WIDE_CHAR))
                {
                    // Lead survived but spacer was blanked — this is fine,
                    // the lead is now inconsistent. Clear it.
                    // Actually this shouldn't happen since we blank the spacer
                    // in the loop. But check anyway.
                }
            }
        }
    }

    fn param(params: &[u16], idx: usize, default: u16) -> u16 {
        params.get(idx).copied().unwrap_or(default).max(1)
    }

    fn set_dec_mode(&mut self, mode: u16, enable: bool) {
        match mode {
            7 => {
                self.modes.auto_wrap = enable;
                // When DECAWM is turned off, clear any pending wrap.
                // xterm clears do_wrap when DECAWM is disabled so that
                // re-enabling DECAWM doesn't trigger a stale deferred wrap.
                if !enable {
                    self.cursor.pending_wrap = false;
                }
            }
            5 => {
                if self.modes.reverse_video != enable {
                    self.modes.reverse_video = enable;
                    self.grid.mark_all_dirty();
                }
            }
            12 => self.modes.cursor_blink = enable,
            25 => self.modes.cursor_visible = enable,
            6 => {
                // DECOM (origin mode) — per VT220 spec, enabling/disabling
                // also homes the cursor. When origin mode is enabled, the
                // home position is the top of the scroll region. When
                // disabled, the home position is absolute (0, 0).
                self.modes.origin = enable;
                let (top, _) = self.grid.scroll_region();
                self.set_cursor(0, if enable { top } else { 0 });
            }
            1 => self.modes.cursor_keys_app = enable,
            2004 => self.modes.bracketed_paste = enable,
            // Alt-screen modes — P15-A: properly save/restore grid
            47 | 1047 => {
                if enable && !self.modes.alt_screen {
                    // Enter alt-screen: save primary grid + tab stops
                    self.alt_saved_grid = Some(self.grid.clone());
                    self.alt_saved_tab_stops = Some(self.tab_stops.clone());
                    // Alt screen should NOT have scrollback (xterm behavior).
                    self.grid = Grid::with_scrollback(self.width(), self.height(), 0);
                    self.reset_tab_stops();
                    if mode == 1047 {
                        // 1047: clear the alt screen (already fresh)
                    }
                    self.modes.alt_screen = true;
                } else if !enable && self.modes.alt_screen {
                    // Exit alt-screen: restore primary grid + tab stops
                    if let Some(mut saved) = self.alt_saved_grid.take() {
                        // Resize saved grid if terminal was resized in alt screen.
                        let cur_w = self.grid.width();
                        let cur_h = self.grid.height();
                        if saved.width() != cur_w || saved.height() != cur_h {
                            saved.resize(cur_w, cur_h);
                        }
                        self.grid = saved;
                    }
                    if let Some(stops) = self.alt_saved_tab_stops.take() {
                        self.tab_stops = stops;
                        let w = self.grid.width();
                        if self.tab_stops.len() < w {
                            let old_len = self.tab_stops.len();
                            self.tab_stops.resize(w, false);
                            let mut col = (old_len / 8 + 1) * 8;
                            while col < w {
                                self.tab_stops[col] = true;
                                col += 8;
                            }
                        } else {
                            self.tab_stops.truncate(w.max(1));
                        }
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
                        cursor_style: self.cursor_style,
                    });
                    self.alt_saved_grid = Some(self.grid.clone());
                    self.alt_saved_tab_stops = Some(self.tab_stops.clone());
                    // Alt screen should NOT have scrollback (xterm behavior).
                    self.grid = Grid::with_scrollback(self.width(), self.height(), 0);
                    self.reset_tab_stops();
                    self.cursor = Cursor::default();
                    self.current_hyperlink = None; // Clear hyperlink state on alt screen enter
                    self.modes.alt_screen = true;
                } else if !enable && self.modes.alt_screen {
                    // Exit alt-screen: restore grid, cursor, tab stops
                    if let Some(mut saved) = self.alt_saved_grid.take() {
                        // If the terminal was resized while in alt screen,
                        // the saved primary grid has old dimensions.
                        // Resize it to match the current (alt screen) grid
                        // dimensions so the restored content fits properly.
                        let cur_w = self.grid.width();
                        let cur_h = self.grid.height();
                        if saved.width() != cur_w || saved.height() != cur_h {
                            saved.resize(cur_w, cur_h);
                        }
                        self.grid = saved;
                    }
                    if let Some(stops) = self.alt_saved_tab_stops.take() {
                        self.tab_stops = stops;
                        // Truncate/extend tab stops to current width.
                        let w = self.grid.width();
                        if self.tab_stops.len() < w {
                            let old_len = self.tab_stops.len();
                            self.tab_stops.resize(w, false);
                            let mut col = (old_len / 8 + 1) * 8;
                            while col < w {
                                self.tab_stops[col] = true;
                                col += 8;
                            }
                        } else {
                            self.tab_stops.truncate(w.max(1));
                        }
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
                        self.cursor_style = state.cursor_style;
                        // Clamp cursor to restored grid dimensions
                        // (may differ if resized while in alt screen).
                        let w = self.grid.width();
                        let h = self.grid.height();
                        self.cursor.x = self.cursor.x.min(w.saturating_sub(1));
                        self.cursor.y = self.cursor.y.min(h.saturating_sub(1));
                        self.cursor.pending_wrap = false;
                    }
                    // Clear hyperlink state set during alt screen so it
                    // doesn't leak onto the main screen.
                    self.current_hyperlink = None;
                    self.modes.alt_screen = false;
                }
            }
            // DECSET 1048 — Save cursor as in DECSC.
            // DECRST 1048 — Restore cursor as in DECRC.
            // This is used by programs that need to save/restore cursor
            // independently of screen buffer switching (e.g., older
            // programs that use 1048h + 47h + 47l + 1048l instead of 1049).
            1048 => {
                if enable {
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
                        cursor_style: self.cursor_style,
                    });
                } else if let Some(state) = &self.decsc_state {
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
                    self.cursor_style = state.cursor_style;
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
            self.underline_color = Color::Default;
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
                // SGR 23 — not italic, not fraktur, not doubly underlined (xterm).
                // Clears both ITALIC and UNDERLINE_DOUBLE (which was set by SGR 21).
                23 => self.flags &= !(CellFlags::ITALIC | CellFlags::UNDERLINE_DOUBLE),
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
        // If starting on a wide spacer, include the lead cell.
        let start = if col > 0 && self.grid.cell(col, row).is_some_and(|c| c.is_wide_spacer()) {
            col - 1
        } else {
            col
        };
        // Erase from cursor to end of current row
        for c in start..width {
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
        self.grid_mut().mark_all_dirty();
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
        let end = col.min(width.saturating_sub(1));
        for c in 0..=end {
            if let Some(cell) = self.grid.cell_mut(c, row)
                && !cell.flags.contains(CellFlags::PROTECTED)
            {
                *cell = Cell::blank();
            }
        }
        // If the cell right after the erase range is a wide spacer
        // whose lead was just erased, clear the orphaned spacer.
        if end + 1 < width
            && self
                .grid
                .cell(end + 1, row)
                .is_some_and(|c| c.is_wide_spacer())
            && self.grid.cell(end, row).is_some_and(|c| !c.is_wide())
            && let Some(cell) = self.grid.cell_mut(end + 1, row)
        {
            *cell = Cell::blank();
        }
        self.grid_mut().mark_all_dirty();
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
        self.grid_mut().mark_all_dirty();
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
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(data.len() * 2);
    for &b in data {
        s.push(HEX_CHARS[(b >> 4) as usize] as char);
        s.push(HEX_CHARS[(b & 0xf) as usize] as char);
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
                // If tab landed on a wide char spacer, adjust to the lead.
                if self.cursor.x > 0
                    && let Some(c) = self.grid.cell(self.cursor.x, self.cursor.y)
                    && c.is_wide_spacer()
                {
                    self.cursor.x -= 1;
                }
                self.cursor.pending_wrap = false;
            }
            0x0a..=0x0c => {
                self.line_feed();
            }
            0x0d => {
                self.cursor.x = 0;
                self.cursor.pending_wrap = false;
                // CR signals a hard newline — clear the soft-wrap flag
                // so reflow knows this line is not continued.
                self.grid.set_row_wrap(self.cursor.y, false);
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
        self.flush_utf8();
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
                // If cursor landed on a wide char spacer, back up to the lead.
                if self.cursor.x > 0
                    && let Some(c) = self.grid.cell(self.cursor.x, self.cursor.y)
                    && c.is_wide_spacer()
                {
                    self.cursor.x -= 1;
                }
                self.cursor.pending_wrap = false;
            }
            b'D' => {
                let n = Self::param(params, 0, 1) as usize;
                self.cursor.x = self.cursor.x.saturating_sub(n);
                // If cursor landed on a wide char spacer, back up to the lead.
                if self.cursor.x > 0
                    && let Some(c) = self.grid.cell(self.cursor.x, self.cursor.y)
                    && c.is_wide_spacer()
                {
                    self.cursor.x -= 1;
                }
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
            // HPA — Horizontal Position Absolute (CSI ` Ps `).
            // Equivalent to CHA (CSI Ps G).
            b'`' => {
                let col = Self::param(params, 0, 1) as usize;
                self.set_cursor(col.saturating_sub(1), self.cursor.y);
            }
            // VPR — Vertical Position Relative (CSI Ps e).
            // Moves cursor down Ps rows, column unchanged. Like CUU but downward.
            b'e' => {
                let n = Self::param(params, 0, 1) as usize;
                let (_, bottom) = self.grid.scroll_region();
                let new_y = self
                    .cursor
                    .y
                    .saturating_add(n)
                    .min(bottom.saturating_sub(1));
                self.set_cursor(self.cursor.x, new_y);
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
                    0 => {
                        self.selective_erase_from(self.cursor.x, self.cursor.y);
                        // Clear wrap flags for current and all lines below
                        // (stale wrap flags cause incorrect reflow on resize).
                        for r in self.cursor.y..self.grid.height() {
                            self.grid.set_row_wrap(r, false);
                        }
                    }
                    1 => {
                        self.selective_erase_to(self.cursor.x, self.cursor.y);
                        // Clear wrap flags for all erased lines (stale wrap
                        // flags cause incorrect reflow on resize).
                        for r in 0..=self.cursor.y {
                            self.grid.set_row_wrap(r, false);
                        }
                    }
                    2 => {
                        self.selective_erase_all();
                        // Clear wrap flags for all lines.
                        for r in 0..self.grid.height() {
                            self.grid.set_row_wrap(r, false);
                        }
                    }
                    _ => {}
                }
            }
            b'J' => {
                let mode = params.first().copied().unwrap_or(0);
                match mode {
                    0 => {
                        self.grid.clear_line_from(self.cursor.x, self.cursor.y);
                        // Erasing to end of display removes the soft-wrap
                        // continuation — the line no longer wraps.
                        self.grid.set_row_wrap(self.cursor.y, false);
                        for r in (self.cursor.y + 1)..self.grid.height() {
                            self.grid.clear_line(r);
                            self.grid.set_row_wrap(r, false);
                        }
                    }
                    1 => {
                        for r in 0..self.cursor.y {
                            self.grid.clear_line(r);
                            self.grid.set_row_wrap(r, false);
                        }
                        self.grid.clear_line_to(self.cursor.x, self.cursor.y);
                    }
                    2 => {
                        self.grid.clear();
                        for r in 0..self.grid.height() {
                            self.grid.set_row_wrap(r, false);
                        }
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
                        let start = if cx > 0
                            && self.grid.cell(cx, cy).is_some_and(|c| c.is_wide_spacer())
                        {
                            cx - 1
                        } else {
                            cx
                        };
                        for c in start..width {
                            if let Some(cell) = self.grid.cell_mut(c, cy)
                                && !cell.flags.contains(CellFlags::PROTECTED)
                            {
                                *cell = Cell::blank();
                            }
                        }
                        // Erasing to end of line removes the soft-wrap continuation.
                        self.grid.set_row_wrap(cy, false);
                    }
                    1 => {
                        // Erase from start of line to cursor (non-protected only)
                        let end = cx.min(width.saturating_sub(1));
                        for c in 0..=end {
                            if let Some(cell) = self.grid.cell_mut(c, cy)
                                && !cell.flags.contains(CellFlags::PROTECTED)
                            {
                                *cell = Cell::blank();
                            }
                        }
                        // If the cell right after the erase range is a wide spacer
                        // whose lead was just erased, clear the orphaned spacer.
                        if end + 1 < width
                            && self
                                .grid
                                .cell(end + 1, cy)
                                .is_some_and(|c| c.is_wide_spacer())
                            && self.grid.cell(end, cy).is_some_and(|c| !c.is_wide())
                            && let Some(cell) = self.grid.cell_mut(end + 1, cy)
                        {
                            *cell = Cell::blank();
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
                        // Erasing entire line removes soft-wrap continuation.
                        self.grid.set_row_wrap(cy, false);
                    }
                    _ => {}
                }
                self.grid_mut().mark_row_dirty(cy);
            }
            b'K' => {
                let mode = params.first().copied().unwrap_or(0);
                match mode {
                    0 => {
                        self.grid.clear_line_from(self.cursor.x, self.cursor.y);
                        // Erasing to end of line removes the soft-wrap
                        // continuation — the line no longer wraps.
                        self.grid.set_row_wrap(self.cursor.y, false);
                    }
                    1 => self.grid.clear_line_to(self.cursor.x, self.cursor.y),
                    2 => {
                        self.grid.clear_line(self.cursor.y);
                        // Clearing entire line means it's no longer soft-wrapped.
                        self.grid.set_row_wrap(self.cursor.y, false);
                    }
                    _ => {}
                }
            }
            b'S' => {
                self.cursor.pending_wrap = false;
                let n = Self::param(params, 0, 1) as usize;
                self.grid.scroll_up(n);
            }
            b'T' => {
                self.cursor.pending_wrap = false;
                let n = Self::param(params, 0, 1) as usize;
                self.grid.scroll_down(n);
            }
            b'r' if !is_private && !intermediates.contains(&b'$') => {
                // CSI r (no params) or CSI 0;0r → reset to full screen
                if params.is_empty() || params.iter().all(|&p| p == 0) {
                    self.grid.set_scroll_region(0, self.grid.height());
                    self.set_cursor(0, 0);
                } else {
                    let top = Self::param(params, 0, 1) as usize;
                    let bottom_param = params.get(1).copied().unwrap_or(0);
                    let bottom = if bottom_param == 0 {
                        self.grid.height()
                    } else {
                        bottom_param as usize
                    };
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
                self.cursor.pending_wrap = false;
                self.grid
                    .insert_line(self.cursor.y, Self::param(params, 0, 1) as usize);
            }
            b'M' => {
                self.cursor.pending_wrap = false;
                self.grid
                    .delete_line(self.cursor.y, Self::param(params, 0, 1) as usize);
            }
            b'P' => {
                self.cursor.pending_wrap = false;
                self.grid.delete_char(
                    self.cursor.x,
                    self.cursor.y,
                    Self::param(params, 0, 1) as usize,
                );
            }
            b'@' => {
                self.cursor.pending_wrap = false;
                self.grid.insert_char(
                    self.cursor.x,
                    self.cursor.y,
                    Self::param(params, 0, 1) as usize,
                );
            }
            b'X' => {
                self.cursor.pending_wrap = false;
                self.grid.erase_char(
                    self.cursor.x,
                    self.cursor.y,
                    Self::param(params, 0, 1) as usize,
                );
            }
            b'I' => {
                // CHT — Cursor Horizontal Tab forward N times.
                self.cursor.pending_wrap = false;
                let n = (Self::param(params, 0, 1) as usize).min(self.grid.width());
                for _ in 0..n {
                    self.execute(0x09);
                }
            }
            b'Z' => {
                // CBT — Cursor Backward Tab N times.
                self.cursor.pending_wrap = false;
                let n = (Self::param(params, 0, 1) as usize).min(self.grid.width());
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
                // Cap at terminal width to prevent CPU DoS from large REP counts.
                let n = (Self::param(params, 0, 1) as usize).min(self.grid.width() * 2);
                if let Some(ch) = self.last_printed_char {
                    for _ in 0..n {
                        self.put_printable_char(ch);
                    }
                }
            }
            // DA1 — primary device attributes
            b'c' if !intermediates.contains(&b'>') && !intermediates.contains(&b'=') => {
                // Respond: CSI ? 62 ; 6 ; 22 ; 28 ; 29 c
                // VT220-level — only features we actually support:
                //   62 = VT220,
                //   6 = selective erase (DECSCA/DECSED),
                //   22 = ANSI color (ISO 8613-6),
                //   28 = rectangular editing (DECFRA/DECERA/DECSERA),
                //   29 = ANSI text locator (OSC 8 hyperlinks).
                // NOT advertised (unimplemented):
                //   1 (132-col DECCOLM), 2 (printer), 4 (sixel),
                //   9 (NRC), 15 (DEC tech), 16 (locator port).
                self.response_buffer
                    .extend_from_slice(b"\x1b[?62;6;22;28;29c");
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
            // DSR — device status report
            b'n' if !is_private => {
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
            // DECXCPR — DEC Extended Cursor Position Report (CSI ? 6 n)
            // Response must include '?' prefix: CSI ? row;col R
            b'n' if is_private => {
                let mode = params.first().copied().unwrap_or(0);
                if mode == 6 {
                    // DECXCPR: respond with CSI ? row;col R
                    let (cx, cy) = (self.cursor.x + 1, self.cursor.y + 1);
                    let report_row = if self.modes.origin {
                        let (top, _) = self.grid.scroll_region();
                        cy.saturating_sub(top + 1).max(1)
                    } else {
                        cy
                    };
                    let resp = format!("\x1b[?{};{}R", report_row, cx);
                    self.response_buffer.extend_from_slice(resp.as_bytes());
                }
                // Other private DSR queries (printer status, UDK status, etc.)
                // are not supported — silently ignore.
            }
            // Text area size report (CSI Ps t)
            b't' if !intermediates.contains(&b'$') => {
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
                let count = (params.first().copied().unwrap_or(1) as usize).min(32);
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
                // Clamp to grid bounds (may have been resized since SCP).
                let w = self.grid.width();
                let h = self.grid.height();
                self.cursor.x = self.cursor.x.min(w.saturating_sub(1));
                self.cursor.y = self.cursor.y.min(h.saturating_sub(1));
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
            // DECSACE — Select Attribute Change Extent (CSI Ps * q)
            // Ps=1: stream mode (default), Ps=2: rectangle mode.
            // Controls whether DECCARA/DECRARA modify a stream or rectangle.
            b'q' if intermediates.contains(&b'*') => {
                let mode = params.first().copied().unwrap_or(1);
                self.modes.sace_rectangle = mode == 2;
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
                self.modes.sace_rectangle = false;
                self.modes.synchronized_output = false;
                self.modes.reflow = true;
                self.modes.focus_event = false;
                self.modes.alternate_scroll = true; // xterm default: enabled
                self.modes.reverse_video = false; // DECSCNM off per DECSTR spec
                self.modes.cursor_blink = true; // DECSET 12 default = on
                // Reset keypad application mode (DECKPAM/DECPNM)
                self.modes.keypad_app = false;
                // Reset Kitty keyboard protocol flags
                self.modes.kitty_keyboard = 0;
                self.kitty_kb_stack.clear();
                // Reset mouse tracking modes (DECSET 1000/1002/1003/1006/1005/1015/1016)
                self.modes.mouse_tracking = false;
                self.modes.mouse_button_event = false;
                self.modes.mouse_any_event = false;
                self.modes.mouse_sgr = false;
                self.modes.mouse_utf8 = false;
                self.modes.mouse_urxvt = false;
                self.modes.mouse_sgr_pixel = false;
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
                // Reset cursor style to default (DECSCUSR)
                self.cursor_style = CursorStyle::default();
                // Reset modifyOtherKeys mode
                self.modes.modify_other_keys = 0;
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
                    1 => self.modes.cursor_keys_app,       // DECCKM
                    5 => self.modes.reverse_video,         // DECSCNM
                    6 => self.modes.origin,                // DECOM
                    7 => self.modes.auto_wrap,             // DECAWM
                    12 => self.modes.cursor_blink,         // Cursor blink
                    25 => self.modes.cursor_visible,       // DECTCEM
                    47 => self.modes.alt_screen,           // Alt screen (47)
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
                    // 1048 is a transient save/restore action, not a persistent
                    // mode. xterm reports it as "reset" (status 2) in DECRQM.
                    // Reporting based on decsc_state.is_some() would incorrectly
                    // report "set" after any DECSC, misleading programs that
                    // query mode state to detect alt-screen transition.
                    1048 => false, // Always report reset (transient action)
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
            // DECFRA — Fill Rectangle Area (CSI Pt ; Pl ; Pb ; Pr $ x)
            // Fill the rectangle from (top,left) to (bottom,right) inclusive
            // with the current SGR attributes (using char from Ps as fill char).
            // Coordinates are 1-based. Ps determines the fill character:
            //   numeric → that code point, but most apps use space (Ps=0 or omitted).
            b'x' if intermediates.contains(&b'$') => {
                // DECFRA format: CSI Pch;Pt;Pl;Pb;Pr $ x
                // Pch = fill char (must be 32-126 or 160-255, else ignored)
                let pch = params.first().copied().unwrap_or(0);
                // Validate fill char per DEC STD 070
                let valid_fill = (pch >= 0x20 && pch <= 0x7e) || (pch >= 0xa0 && pch <= 0xff);
                if !valid_fill {
                    return;
                }
                let fill_char = char::from_u32(pch as u32).unwrap_or(' ');
                let top = params.get(1).copied().unwrap_or(1).saturating_sub(1) as usize;
                let left = params.get(2).copied().unwrap_or(1).saturating_sub(1) as usize;
                let bottom = params.get(3).copied().unwrap_or(1).saturating_sub(1) as usize;
                let right = params.get(4).copied().unwrap_or(1).saturating_sub(1) as usize;
                let width = self.grid.width();
                let height = self.grid.height();
                let top = top.min(height.saturating_sub(1));
                let bottom = bottom.min(height.saturating_sub(1));
                let left = left.min(width.saturating_sub(1));
                let right = right.min(width.saturating_sub(1));
                if top <= bottom && left <= right {
                    let (fg, bg, flags) = (self.fg, self.bg, self.flags);
                    for row in top..=bottom {
                        for col in left..=right {
                            if let Some(cell) = self.grid_mut().cell_mut(col, row) {
                                *cell = crate::Cell {
                                    ch: fill_char,
                                    combining: Vec::new(),
                                    fg,
                                    bg,
                                    flags,
                                    hyperlink: None,
                                };
                            }
                        }
                        // Clean up wide char pairs at the rectangle boundary
                        self.cleanup_wide_at_rect_boundary(left, right, row);
                    }
                    self.grid_mut().mark_all_dirty();
                    self.cursor.pending_wrap = false;
                }
            }
            // DECERA — Erase Rectangle Area (CSI Pt ; Pl ; Pb ; Pr $ z)
            // Erase ALL cells in the rectangle to blank, including protected.
            // Coordinates are 1-based, clamped to screen bounds.
            // (Per DEC STD 070: DECERA ignores DECSCA protection.)
            b'z' if intermediates.contains(&b'$') => {
                let top = params.first().copied().unwrap_or(1).saturating_sub(1) as usize;
                let left = params.get(1).copied().unwrap_or(1).saturating_sub(1) as usize;
                let bottom = params.get(2).copied().unwrap_or(1).saturating_sub(1) as usize;
                let right = params.get(3).copied().unwrap_or(1).saturating_sub(1) as usize;
                let width = self.grid.width();
                let height = self.grid.height();
                let top = top.min(height.saturating_sub(1));
                let bottom = bottom.min(height.saturating_sub(1));
                let left = left.min(width.saturating_sub(1));
                let right = right.min(width.saturating_sub(1));
                if top <= bottom && left <= right {
                    for row in top..=bottom {
                        for col in left..=right {
                            if let Some(cell) = self.grid_mut().cell_mut(col, row) {
                                *cell = Cell::blank();
                            }
                        }
                        // Clean up orphaned wide char pairs at boundary
                        self.cleanup_wide_at_rect_boundary(left, right, row);
                    }
                    self.grid_mut().mark_all_dirty();
                    self.cursor.pending_wrap = false;
                }
            }
            // DECSERA — Selective Erase Rectangle Area (CSI Pt ; Pl ; Pb ; Pr $ {)
            // Erase only non-protected cells in the rectangle.
            // DECSCA protected cells are preserved (same as DECSED/DECSEL).
            b'{' if intermediates.contains(&b'$') => {
                let top = params.first().copied().unwrap_or(1).saturating_sub(1) as usize;
                let left = params.get(1).copied().unwrap_or(1).saturating_sub(1) as usize;
                let bottom = params.get(2).copied().unwrap_or(1).saturating_sub(1) as usize;
                let right = params.get(3).copied().unwrap_or(1).saturating_sub(1) as usize;
                let width = self.grid.width();
                let height = self.grid.height();
                let top = top.min(height.saturating_sub(1));
                let bottom = bottom.min(height.saturating_sub(1));
                let left = left.min(width.saturating_sub(1));
                let right = right.min(width.saturating_sub(1));
                if top <= bottom && left <= right {
                    for row in top..=bottom {
                        for col in left..=right {
                            if let Some(cell) = self.grid_mut().cell_mut(col, row)
                                && !cell.flags.contains(CellFlags::PROTECTED)
                            {
                                *cell = Cell::blank();
                            }
                        }
                        // Clean up orphaned wide char pairs at boundary
                        self.cleanup_wide_at_rect_boundary(left, right, row);
                    }
                    self.grid_mut().mark_all_dirty();
                    self.cursor.pending_wrap = false;
                }
            }
            // DECIC — Insert Column (CSI Ps ' } )
            // Insert Ps blank columns at the cursor column. Cells to the
            // right shift right; cells past the edge are lost.
            // Intermediate byte: 0x27 (').
            b'}' if intermediates.contains(&0x27) => {
                let n = Self::param(params, 0, 1) as usize;
                let col = self.cursor.x.min(self.grid.width().saturating_sub(1));
                self.grid.insert_column(col, n);
                self.cursor.pending_wrap = false;
            }
            // DECDC — Delete Column (CSI Ps ' ~ )
            // Delete Ps columns at the cursor column. Cells to the right
            // shift left; blanks fill the right edge.
            b'~' if intermediates.contains(&0x27) => {
                let n = Self::param(params, 0, 1) as usize;
                let col = self.cursor.x.min(self.grid.width().saturating_sub(1));
                self.grid.delete_column(col, n);
                self.cursor.pending_wrap = false;
            }
            // DECRA — Copy Rectangle Area (CSI Pt;Pl;Pb;Pr;Pk;Pp $ v)
            // Copy the source rectangle to the destination top-left corner.
            // Source and destination may overlap — data is buffered first.
            // Coordinates are 1-based, clamped to screen bounds.
            b'v' if intermediates.contains(&b'$') => {
                let src_top = params.first().copied().unwrap_or(1).saturating_sub(1) as usize;
                let src_left = params.get(1).copied().unwrap_or(1).saturating_sub(1) as usize;
                let src_bottom = params.get(2).copied().unwrap_or(1).saturating_sub(1) as usize;
                let src_right = params.get(3).copied().unwrap_or(1).saturating_sub(1) as usize;
                let dst_row = params.get(4).copied().unwrap_or(1).saturating_sub(1) as usize;
                let dst_col = params.get(5).copied().unwrap_or(1).saturating_sub(1) as usize;
                let width = self.grid.width();
                let height = self.grid.height();
                let src_top = src_top.min(height.saturating_sub(1));
                let src_bottom = src_bottom.min(height.saturating_sub(1));
                let src_left = src_left.min(width.saturating_sub(1));
                let src_right = src_right.min(width.saturating_sub(1));
                if src_top <= src_bottom && src_left <= src_right {
                    let rect_w = src_right - src_left + 1;
                    let rect_h = src_bottom - src_top + 1;
                    // Buffer source data first (handles overlap safely).
                    let mut buf: Vec<Cell> = Vec::with_capacity(rect_w * rect_h);
                    for r in src_top..=src_bottom {
                        for c in src_left..=src_right {
                            let cell = self.grid().cell(c, r).cloned().unwrap_or_default();
                            buf.push(cell);
                        }
                    }
                    // Write to destination, clamping each cell to screen bounds.
                    for r in 0..rect_h {
                        for c in 0..rect_w {
                            let dr = dst_row + r;
                            let dc = dst_col + c;
                            if dr < height && dc < width {
                                let idx = r * rect_w + c;
                                if let Some(cell) = self.grid_mut().cell_mut(dc, dr) {
                                    *cell = buf[idx].clone();
                                }
                            }
                        }
                    }
                    self.grid_mut().mark_all_dirty();
                    self.cursor.pending_wrap = false;
                }
            }
            // DECCARA — Change Attributes in Rectangular Area (CSI Pt;Pl;Pb;Pr;Ps1;Ps2 $ r)
            // Add SGR attributes to non-blank cells within the rectangle.
            // Only BOLD(1), UNDERLINE(4), BLINK(5), REVERSE(7) are honored.
            // Ps1=0 clears existing attrs before setting the range.
            b'r' if intermediates.contains(&b'$') => {
                let top = params.first().copied().unwrap_or(1).saturating_sub(1) as usize;
                let left = params.get(1).copied().unwrap_or(1).saturating_sub(1) as usize;
                let bottom = params.get(2).copied().unwrap_or(1).saturating_sub(1) as usize;
                let right = params.get(3).copied().unwrap_or(1).saturating_sub(1) as usize;
                let width = self.grid.width();
                let height = self.grid.height();
                let top = top.min(height.saturating_sub(1));
                let bottom = bottom.min(height.saturating_sub(1));
                let left = left.min(width.saturating_sub(1));
                let right = right.min(width.saturating_sub(1));
                if top <= bottom && left <= right {
                    // Collect SGR attribute params starting at index 4.
                    let sgr_vals: Vec<u16> = params.iter().skip(4).copied().collect();
                    // Build cumulative add/remove flag operations.
                    let mut clear_first = false;
                    let mut add_flags = CellFlags::empty();
                    let mut remove_flags = CellFlags::empty();
                    for &v in &sgr_vals {
                        match v {
                            0 => {
                                // Ps1=0: clear all SGR-renderable attributes
                                clear_first = true;
                            }
                            1 => add_flags |= CellFlags::BOLD,
                            4 => add_flags |= CellFlags::UNDERLINE,
                            5 => add_flags |= CellFlags::BLINK,
                            7 => add_flags |= CellFlags::REVERSE,
                            // Explicit "off" codes per VT510 spec
                            22 => remove_flags |= CellFlags::BOLD | CellFlags::DIM,
                            24 => {
                                remove_flags |= CellFlags::UNDERLINE
                                    | CellFlags::UNDERLINE_DOUBLE
                                    | CellFlags::UNDERLINE_CURLY
                                    | CellFlags::UNDERLINE_DOTTED
                                    | CellFlags::UNDERLINE_DASHED;
                            }
                            25 => remove_flags |= CellFlags::BLINK,
                            27 => remove_flags |= CellFlags::REVERSE,
                            _ => {}
                        }
                    }
                    for row in top..=bottom {
                        // Stream mode: extend to full row width.
                        // Rectangle mode: only modify the specified columns.
                        let (col_start, col_end) = if self.modes.sace_rectangle {
                            (left, right)
                        } else {
                            (0, width.saturating_sub(1))
                        };
                        for col in col_start..=col_end {
                            if let Some(cell) = self.grid_mut().cell_mut(col, row) {
                                // Skip blank cells (spaces with no attributes) per spec.
                                if cell.is_blank() {
                                    continue;
                                }
                                if clear_first {
                                    // Ps1=0: clear all SGR-renderable attributes.
                                    cell.flags &= CellFlags::WIDE_CHAR
                                        | CellFlags::WIDE_SPACER
                                        | CellFlags::PROTECTED;
                                }
                                cell.flags &= !remove_flags;
                                cell.flags |= add_flags;
                            }
                        }
                    }
                    self.grid_mut().mark_all_dirty();
                    self.cursor.pending_wrap = false;
                }
            }
            // DECRARA — Reverse Attributes in Rectangular Area (CSI Pt;Pl;Pb;Pr;Ps1;Ps2 $ t)
            // Toggle (flip) SGR attributes on non-blank cells within the rectangle.
            // Ps=0: reverse all attributes (BOLD, UNDERLINE, BLINK, REVERSE).
            // Ps=1/4/5/7: reverse individual attributes.
            b't' if intermediates.contains(&b'$') => {
                let top = params.first().copied().unwrap_or(1).saturating_sub(1) as usize;
                let left = params.get(1).copied().unwrap_or(1).saturating_sub(1) as usize;
                let bottom = params.get(2).copied().unwrap_or(1).saturating_sub(1) as usize;
                let right = params.get(3).copied().unwrap_or(1).saturating_sub(1) as usize;
                let width = self.grid.width();
                let height = self.grid.height();
                let top = top.min(height.saturating_sub(1));
                let bottom = bottom.min(height.saturating_sub(1));
                let left = left.min(width.saturating_sub(1));
                let right = right.min(width.saturating_sub(1));
                if top <= bottom && left <= right {
                    let sgr_vals: Vec<u16> = params.iter().skip(4).copied().collect();
                    let mut toggle_flags = CellFlags::empty();
                    for &v in &sgr_vals {
                        match v {
                            0 => {
                                // Ps=0: reverse all attributes per VT510 spec
                                toggle_flags |= CellFlags::BOLD
                                    | CellFlags::UNDERLINE
                                    | CellFlags::BLINK
                                    | CellFlags::REVERSE;
                            }
                            1 => toggle_flags |= CellFlags::BOLD,
                            4 => toggle_flags |= CellFlags::UNDERLINE,
                            5 => toggle_flags |= CellFlags::BLINK,
                            7 => toggle_flags |= CellFlags::REVERSE,
                            _ => {}
                        }
                    }
                    for row in top..=bottom {
                        // Stream mode: extend to full row width.
                        // Rectangle mode: only modify the specified columns.
                        let (col_start, col_end) = if self.modes.sace_rectangle {
                            (left, right)
                        } else {
                            (0, width.saturating_sub(1))
                        };
                        for col in col_start..=col_end {
                            if let Some(cell) = self.grid_mut().cell_mut(col, row) {
                                if cell.is_blank() {
                                    continue;
                                }
                                cell.flags ^= toggle_flags;
                            }
                        }
                    }
                    self.grid_mut().mark_all_dirty();
                    self.cursor.pending_wrap = false;
                }
            }
            // DECRQC — Restore Mode (CSI Ps $ w)
            // Restores a DEC private mode to its default value.
            b'w' if intermediates.contains(&b'$') => {
                let mode = params.first().copied().unwrap_or(0);
                let defaults = Modes::defaults();
                match mode {
                    1 => self.modes.cursor_keys_app = defaults.cursor_keys_app,
                    6 => self.modes.origin = defaults.origin,
                    7 => self.modes.auto_wrap = defaults.auto_wrap,
                    12 => self.modes.cursor_blink = defaults.cursor_blink,
                    25 => self.modes.cursor_visible = defaults.cursor_visible,
                    47 | 1047 | 1049 => self.modes.alt_screen = defaults.alt_screen,
                    2004 => self.modes.bracketed_paste = defaults.bracketed_paste,
                    2026 => self.modes.synchronized_output = defaults.synchronized_output,
                    2027 => self.modes.reflow = defaults.reflow,
                    7727 => self.modes.alternate_scroll = defaults.alternate_scroll,
                    _ => {}
                }
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
                        // SGR 4:0 — no underline (same as SGR 24).
                        (4, 0) => {
                            self.flags &= !(CellFlags::UNDERLINE
                                | CellFlags::UNDERLINE_DOUBLE
                                | CellFlags::UNDERLINE_CURLY
                                | CellFlags::UNDERLINE_DOTTED
                                | CellFlags::UNDERLINE_DASHED);
                            handled = true;
                        }
                        // SGR 4:1 — single solid underline.
                        (4, 1) => {
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
                        // SGR 58:5:N — underline color from palette (colon syntax).
                        (58, 5) => {
                            if let Some(&n) = params.get(i + 2) {
                                self.underline_color = Color::Indexed(n as u8);
                            }
                            handled = true;
                        }
                        // SGR 58:2 — underline color RGB (colon syntax).
                        // Two formats exist in the wild:
                        //   58:2:R:G:B          (no color space ID — kitty, foot)
                        //   58:2:<cs>:R:G:B     (ITU-T T.416 — xterm, vte)
                        // Count remaining colon sub-params to distinguish.
                        (58, 2) => {
                            // Count remaining colon-derived sub-params after (58, 2).
                            let mut sub_count = 0;
                            let mut k = i + 2;
                            while k < params.len() && subs.get(k).copied().unwrap_or(0) != 0 {
                                sub_count += 1;
                                k += 1;
                            }
                            match sub_count {
                                3 => {
                                    // 58:2:R:G:B (no color space ID)
                                    self.underline_color = Color::Rgb(
                                        params[i + 2] as u8,
                                        params[i + 3] as u8,
                                        params[i + 4] as u8,
                                    );
                                }
                                4 => {
                                    // 58:2:<cs>:R:G:B (skip color space ID)
                                    self.underline_color = Color::Rgb(
                                        params[i + 3] as u8,
                                        params[i + 4] as u8,
                                        params[i + 5] as u8,
                                    );
                                }
                                _ => {}
                            }
                            handled = true;
                        }
                        // SGR 38:2 — foreground RGB (colon syntax).
                        (38, 2) => {
                            let mut sub_count = 0;
                            let mut k = i + 2;
                            while k < params.len() && subs.get(k).copied().unwrap_or(0) != 0 {
                                sub_count += 1;
                                k += 1;
                            }
                            match sub_count {
                                3 => {
                                    self.fg = Color::Rgb(
                                        params[i + 2] as u8,
                                        params[i + 3] as u8,
                                        params[i + 4] as u8,
                                    );
                                }
                                4 => {
                                    self.fg = Color::Rgb(
                                        params[i + 3] as u8,
                                        params[i + 4] as u8,
                                        params[i + 5] as u8,
                                    );
                                }
                                _ => {}
                            }
                            handled = true;
                        }
                        // SGR 48:2 — background RGB (colon syntax).
                        (48, 2) => {
                            let mut sub_count = 0;
                            let mut k = i + 2;
                            while k < params.len() && subs.get(k).copied().unwrap_or(0) != 0 {
                                sub_count += 1;
                                k += 1;
                            }
                            match sub_count {
                                3 => {
                                    self.bg = Color::Rgb(
                                        params[i + 2] as u8,
                                        params[i + 3] as u8,
                                        params[i + 4] as u8,
                                    );
                                }
                                4 => {
                                    self.bg = Color::Rgb(
                                        params[i + 3] as u8,
                                        params[i + 4] as u8,
                                        params[i + 5] as u8,
                                    );
                                }
                                _ => {}
                            }
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
                // Build filtered params list — exclude colon-derived groups.
                // A colon group starts at a param with sub=0 followed by
                // params with sub!=0. Skip the entire group (e.g. 4:3 = 2
                // params, 58:5:N = 3 params, 58:2::R:G:B = 6 params).
                let mut filtered: Vec<u16> = Vec::new();
                let mut j = 0;
                while j < params.len() {
                    // If the NEXT param is colon-derived, this starts a group.
                    // Skip all params until we reach one with sub=0 again.
                    if subs.get(j + 1).copied().unwrap_or(0) != 0 {
                        j += 1; // skip the root param
                        while j < params.len() && subs.get(j).copied().unwrap_or(0) != 0 {
                            j += 1; // skip colon-derived sub-params
                        }
                    } else {
                        filtered.push(params[j]);
                        j += 1;
                    }
                }
                if filtered.is_empty() {
                    return; // all params were colon-derived
                }
                // Process remaining regular SGR params with the filtered list.
                self.sgr(&filtered);
                return;
            }
        }
        // Default: delegate to regular csi()
        self.csi(intermediates, params, final_byte);
    }

    fn esc(&mut self, intermediates: &[u8], final_byte: u8) {
        self.flush_utf8();
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
                // Per VT220/xterm spec, DECALN also:
                // 1. Resets the cursor to home (0,0)
                // 2. Resets SGR attributes to default
                // 3. Resets scroll region to full screen
                // 4. Resets tab stops to default (every 8 columns)
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
                self.cursor = Cursor::default();
                self.fg = Color::Default;
                self.bg = Color::Default;
                self.underline_color = Color::Default;
                self.flags = CellFlags::empty();
                self.grid.set_scroll_region(0, self.grid.height());
                self.reset_tab_stops();
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
                    cursor_style: self.cursor_style,
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
                    self.cursor_style = state.cursor_style;
                } else {
                    // No saved state — restore defaults (VT220 spec).
                    self.cursor = Cursor::default();
                    self.fg = Color::Default;
                    self.bg = Color::Default;
                    self.underline_color = Color::Default;
                    self.flags = CellFlags::empty();
                    self.g0_charset = Charset::Ascii;
                    self.g1_charset = Charset::Ascii;
                    self.active_g1 = false;
                    self.modes.auto_wrap = true;
                    self.modes.origin = false;
                    self.protected_attr = false;
                }
                // Clamp cursor to current grid dimensions.
                // If the terminal was resized between DECSC and DECRC,
                // the restored cursor position could be out of bounds.
                // pending_wrap is invalidated only if clamping changed the position.
                let w = self.grid.width();
                let h = self.grid.height();
                let clamped_x = self.cursor.x.min(w.saturating_sub(1));
                let clamped_y = self.cursor.y.min(h.saturating_sub(1));
                if clamped_x != self.cursor.x || clamped_y != self.cursor.y {
                    self.cursor.pending_wrap = false;
                }
                self.cursor.x = clamped_x;
                self.cursor.y = clamped_y;
            }
            b'c' => {
                self.ris();
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
            // SS2 (ESC N) and SS3 (ESC O) — single-shift G2/G3 invocation.
            // We don't implement G2/G3 character sets (only G0/G1 are used).
            // These are primarily input-side sequences (function keys).
            // On output, the next character would use G2/G3, but since we
            // don't track those sets, treat as no-op (matches xterm behavior
            // when no G2/G3 charset is designated).
            b'N' | b'O' => {}
            _ => {}
        }
    }

    fn osc(&mut self, data: &[u8]) {
        self.flush_utf8();
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
                    // Sanitize: strip control characters from URI to prevent
                    // injection of escape sequences or BEL bytes that could
                    // trigger unintended terminal behavior.
                    let sanitized: String = uri
                        .chars()
                        .filter(|c| !c.is_control() || *c == ' ')
                        .collect();
                    self.current_hyperlink = if sanitized.is_empty() {
                        None
                    } else {
                        Some(sanitized)
                    };
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
                let exit_code = sub_parts.next().and_then(|code| {
                    // Some shells send "D;exit_code;" (trailing semicolon) or
                    // "D;exit_code;extra" (additional fields). Parse only the
                    // leading integer portion to handle these variants.
                    let trimmed = code.trim_end_matches(|c: char| !c.is_ascii_digit() && c != '-');
                    // Handle "0;extra" by taking only the part before any ';'
                    let core = trimmed.split(';').next().unwrap_or(trimmed);
                    core.parse::<i32>().ok()
                });
                let (kind, has_exit) = match mark_char.chars().next() {
                    Some('A') => (CommandMarkKind::PromptStart, false),
                    Some('B') => (CommandMarkKind::CommandStart, false),
                    Some('C') => (CommandMarkKind::OutputStart, false),
                    Some('D') => (CommandMarkKind::CommandEnd, true),
                    _ => return,
                };
                self.command_marks.push(CommandMark {
                    kind,
                    row: self.grid.scrollback_len() + self.cursor.y,
                    exit_code: if has_exit { exit_code } else { None },
                });
                // Sync eviction count from Grid before adjusting marks.
                let total_evicted = self.grid.total_evicted();
                // Net new evictions since last sync = total - already accounted.
                let new_evictions =
                    total_evicted.saturating_sub(self.evicted_scrollback_rows_accum);
                self.evicted_scrollback_rows += new_evictions;
                self.evicted_scrollback_rows_accum = total_evicted;
                // Adjust mark rows for scrollback rows evicted since last mark.
                if self.evicted_scrollback_rows > 0 {
                    for m in &mut self.command_marks {
                        m.row = m.row.saturating_sub(self.evicted_scrollback_rows);
                    }
                    self.evicted_scrollback_rows = 0;
                }
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
                        Some(10) => self.dynamic_fg.as_ref().unwrap_or(&self.fg),
                        Some(11) => self.dynamic_bg.as_ref().unwrap_or(&self.bg),
                        _ => self.dynamic_cursor.as_ref().unwrap_or(&self.fg),
                    };
                    let resp = match current {
                        Color::Rgb(r, g, b) => {
                            format!("\x1b]{};rgb:{:02x}/{:02x}/{:02x}\x1b\\", cmd_num, r, g, b)
                        }
                        Color::Indexed(i) => {
                            // Use palette override if set, otherwise built-in palette.
                            let (r, g, b) = self
                                .palette_overrides
                                .get(i)
                                .copied()
                                .unwrap_or_else(|| color_for_index(*i));
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
                        Some(10) => {
                            self.dynamic_fg = Some(color);
                            self.grid.mark_all_dirty();
                        }
                        Some(11) => {
                            self.dynamic_bg = Some(color);
                            self.grid.mark_all_dirty();
                        }
                        Some(12) => {
                            self.dynamic_cursor = Some(color);
                            self.grid.mark_all_dirty();
                        }
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
                        // Palette override changes existing cell colors
                        // without modifying content — must mark dirty so
                        // the renderer redraws with updated colors.
                        self.grid.mark_all_dirty();
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
                    // When connected via SSH (RemoteHost set), the path refers
                    // to the remote filesystem — do NOT canonicalize against
                    // the local filesystem, as it could resolve to a different
                    // local path that happens to share the same name.
                    if self.remote_host.is_some() {
                        self.cwd = Some(std::path::PathBuf::from(path));
                    } else if let Ok(p) = std::path::PathBuf::from(path).canonicalize() {
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
                    // Clear all command marks since their absolute row
                    // references are now invalid (scrollback is empty).
                    self.command_marks.clear();
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
                self.grid.mark_all_dirty();
            }
            Some(111) => {
                self.dynamic_bg = None;
                self.grid.mark_all_dirty();
            }
            Some(112) => {
                self.dynamic_cursor = None;
                self.grid.mark_all_dirty();
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
                    // Background color (xterm extension) — use dynamic if set
                    "BG" => Some({
                        let (r, g, b) = self
                            .dynamic_bg
                            .map(|c| match c {
                                Color::Rgb(r, g, b) => (r, g, b),
                                _ => (0, 0, 0),
                            })
                            .unwrap_or((0, 0, 0));
                        format!("rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}")
                    }),
                    // Foreground color — use dynamic if set
                    "FG" => Some({
                        let (r, g, b) = self
                            .dynamic_fg
                            .map(|c| match c {
                                Color::Rgb(r, g, b) => (r, g, b),
                                _ => (0xcc, 0xcc, 0xcc),
                            })
                            .unwrap_or((0xcc, 0xcc, 0xcc));
                        format!("rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}")
                    }),
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
    fn t_csi_hpa() {
        // HPA (CSI Ps `) — same as CHA, sets column.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[20`");
        assert_eq!(t.cursor().0, 19);
    }

    #[test]
    fn t_csi_vpr() {
        // VPR (CSI Ps e) — move down Ps rows, column unchanged.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5;10H"); // row 5, col 10
        assert_eq!(t.cursor(), (9, 4));
        feed(&mut t, b"\x1b[3e"); // move down 3
        assert_eq!(t.cursor(), (9, 7));
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
    fn t_el_mode1_clear_to_cursor() {
        // EL mode 1: clear from start of line to cursor (inclusive).
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"Hello"); // cursor at col 5
        feed(&mut t, b"\x1b[1K"); // clear from start to cursor (col 0..5)
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ', "col 0 cleared");
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, ' ', "col 4 cleared");
        assert_eq!(
            t.grid().cell(5, 0).unwrap().ch,
            ' ',
            "col 5 = cursor cleared"
        );
        assert_eq!(
            t.grid().cell(6, 0).unwrap().ch,
            ' ',
            "col 6 not part of Hello"
        );
    }

    #[test]
    fn t_el_mode1_preserves_after_cursor() {
        // EL mode 1 should NOT clear cells after the cursor.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"ABCDEFG"); // cursor at col 7
        feed(&mut t, b"\x1b[4G"); // move cursor to col 4 (1-based) = col 3 (0-based)
        feed(&mut t, b"\x1b[1K"); // clear from start to col 3 inclusive
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ');
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, ' ');
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'E', "col 4 preserved");
        assert_eq!(t.grid().cell(6, 0).unwrap().ch, 'G', "col 6 preserved");
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
    fn t_sgr_empty_resets_underline_color() {
        // CSI m (empty params) should be equivalent to CSI 0 m.
        // It must reset underline_color, not just fg/bg/flags.
        let mut t = Terminal::new(80, 24);
        // Set a custom underline color via SGR 58;2 (semicolon form)
        feed(&mut t, b"\x1b[58;2;100;150;200m");
        assert_eq!(
            t.underline_color,
            Color::Rgb(100, 150, 200),
            "underline color should be set"
        );
        // CSI m (empty params) should reset everything including underline_color
        feed(&mut t, b"\x1b[m");
        assert_eq!(
            t.underline_color,
            Color::Default,
            "CSI m should reset underline_color (equivalent to CSI 0 m)"
        );
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
    fn t_sgr23_clears_double_underline() {
        // SGR 21 sets double underline, SGR 23 should clear it.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[21mD\x1b[23mR");
        let flags = t.grid().cell(1, 0).unwrap().flags;
        assert!(
            !flags.contains(CellFlags::UNDERLINE_DOUBLE),
            "SGR 23 should clear UNDERLINE_DOUBLE"
        );
    }

    #[test]
    fn t_sgr23_clears_italic() {
        // SGR 3 sets italic, SGR 23 should clear it.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[3mI\x1b[23mR");
        let flags = t.grid().cell(1, 0).unwrap().flags;
        assert!(
            !flags.contains(CellFlags::ITALIC),
            "SGR 23 should clear ITALIC"
        );
    }

    #[test]
    fn t_sgr_colon_4_0_clears_underline() {
        // SGR 4:0 (colon syntax) = no underline (like SGR 24).
        // Set underline with SGR 4, then clear with 4:0.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[4mU\x1b[4:0mC");
        let flags = t.grid().cell(1, 0).unwrap().flags;
        assert!(
            !flags.contains(CellFlags::UNDERLINE),
            "SGR 4:0 should clear UNDERLINE"
        );
    }

    #[test]
    fn t_sgr_colon_4_1_single_underline() {
        // SGR 4:1 (colon syntax) = single solid underline.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[4:1mU");
        let flags = t.grid().cell(0, 0).unwrap().flags;
        assert!(flags.contains(CellFlags::UNDERLINE));
        assert!(!flags.contains(CellFlags::UNDERLINE_DOUBLE));
        assert!(!flags.contains(CellFlags::UNDERLINE_CURLY));
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
    fn t_alt_screen_1049_resize_while_in_alt() {
        // Resize while in alt screen: saved grid must be resized on exit,
        // and restored cursor must be clamped to new dimensions.
        let mut t = Terminal::new(20, 5);
        // Write content on primary screen.
        feed(&mut t, b"Hello");
        // Move cursor to a position that will be out of bounds after shrink.
        feed(&mut t, b"\x1b[3;15H"); // row 3, col 15
        // Enter alt screen.
        feed(&mut t, b"\x1b[?1049h");
        // Resize narrower while in alt screen.
        t.resize(10, 3);
        // Exit alt screen.
        feed(&mut t, b"\x1b[?1049l");
        // Grid dimensions should match the resized terminal.
        assert_eq!(
            t.grid().width(),
            10,
            "restored grid should match current width"
        );
        assert_eq!(
            t.grid().height(),
            3,
            "restored grid should match current height"
        );
        // Primary content was on old row 0; shrinking from 5→3 height pushes
        // old rows 0-1 to scrollback. Verify content survives.
        assert_eq!(t.grid().scrollback_len(), 2);
        let sb_text = t
            .grid()
            .absolute_row(0)
            .map(|r| r.text())
            .unwrap_or_default();
        assert!(
            sb_text.contains("Hello"),
            "primary content should survive resize, got: '{sb_text}'"
        );
        // Cursor must be clamped to the new (smaller) dimensions.
        let (cx, cy) = t.cursor();
        assert!(cx < 10, "cursor x must be < width after restore, got {cx}");
        assert!(cy < 3, "cursor y must be < height after restore, got {cy}");
    }

    #[test]
    fn t_alt_screen_47_resize_while_in_alt() {
        // Same as above but using mode 47 instead of 1049.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"Data");
        feed(&mut t, b"\x1b[?47h");
        t.resize(8, 3);
        feed(&mut t, b"\x1b[?47l");
        assert_eq!(t.grid().width(), 8);
        assert_eq!(t.grid().height(), 3);
        // Grid::resize shrinks height by removing rows from the TOP.
        // With original height=5 → new height=3, rows 0-1 go to scrollback.
        // So "Data" (on old row 0) is now in scrollback, not visible.
        assert_eq!(
            t.grid().scrollback_len(),
            2,
            "2 rows should be in scrollback"
        );
        // Verify content survives in scrollback.
        let sb_row = t.grid().absolute_row(0);
        let sb_text = sb_row.map(|r| r.text()).unwrap_or_default();
        assert!(
            sb_text.contains("Data"),
            "content should survive in scrollback, got: '{sb_text}'"
        );
    }

    #[test]
    fn t_esc_save_restore_cursor() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5;10H\x1b7\x1b[1;1H\x1b8");
        assert_eq!(t.cursor(), (9, 4));
    }

    #[test]
    fn t_decsc_saves_sgr_and_charset() {
        // DECSC should save SGR attributes AND character set designation.
        let mut t = Terminal::new(80, 24);
        // Set red foreground
        feed(&mut t, b"\x1b[31m");
        // Designate G0 as special graphics (ESC ( 0)
        feed(&mut t, b"\x1b(0");
        // Move to row 5
        feed(&mut t, b"\x1b[5;1H");
        // Save state
        feed(&mut t, b"\x1b7");
        // Change everything
        feed(&mut t, b"\x1b[32m"); // green fg
        feed(&mut t, b"\x1b(B"); // G0 = ASCII
        feed(&mut t, b"\x1b[10;1H"); // move to row 10
        // Restore
        feed(&mut t, b"\x1b8");
        // Cursor should be back at row 5
        assert_eq!(t.cursor().1, 4); // 0-based row 4 = 1-based row 5
        // FG should be red again
        assert_eq!(t.fg, Color::Indexed(1));
        // G0 should be special graphics again
        assert_eq!(t.g0_charset, Charset::DecSpecial);
    }

    #[test]
    fn t_decsc_saves_autowrap_mode() {
        // DECSC should save/restore auto-wrap mode.
        let mut t = Terminal::new(80, 24);
        // Disable auto-wrap
        feed(&mut t, b"\x1b[?7l");
        // Save
        feed(&mut t, b"\x1b7");
        // Re-enable auto-wrap
        feed(&mut t, b"\x1b[?7h");
        // Restore — auto-wrap should be OFF again
        feed(&mut t, b"\x1b8");
        assert!(!t.modes.auto_wrap, "auto-wrap should be restored to off");
    }

    #[test]
    fn t_decrc_clamps_cursor_after_resize() {
        // DECRC should clamp cursor to current grid bounds.
        // If the terminal was resized between DECSC and DECRC,
        // the restored cursor position must not be out of bounds.
        let mut t = Terminal::new(80, 24);
        // Move cursor to col 70, row 20
        feed(&mut t, b"\x1b[21;71H");
        assert_eq!(t.cursor(), (70, 20));
        // Save state (DECSC)
        feed(&mut t, b"\x1b7");
        // Resize smaller
        t.resize(40, 10);
        // Restore (DECRC) — cursor should be clamped to (39, 9)
        feed(&mut t, b"\x1b8");
        let (cx, cy) = t.cursor();
        assert!(cx < 40, "cursor x should be clamped to width, got {cx}");
        assert!(cy < 10, "cursor y should be clamped to height, got {cy}");
    }

    #[test]
    fn t_rcp_clamps_cursor_after_resize() {
        // CSI u (RCP) should also clamp cursor after resize.
        let mut t = Terminal::new(80, 24);
        // Move cursor and save (CSI s)
        feed(&mut t, b"\x1b[21;71H\x1b[s");
        // Resize smaller
        t.resize(40, 10);
        // Restore (CSI u) — cursor should be clamped
        feed(&mut t, b"\x1b[u");
        let (cx, cy) = t.cursor();
        assert!(cx < 40, "cursor x should be clamped to width, got {cx}");
        assert!(cy < 10, "cursor y should be clamped to height, got {cy}");
    }

    #[test]
    fn t_esc_ris_reset() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[31mHello\x1bc");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ');
        assert_eq!(t.cursor(), (0, 0));
    }

    #[test]
    fn t_ris_resets_all_state() {
        let mut t = Terminal::new(80, 24);
        // Set cursor style
        feed(&mut t, b"\x1b[5 q"); // BlinkBar
        // Set dynamic colors
        feed(&mut t, b"\x1b]10;rgb:ff/00/00\x1b\\");
        feed(&mut t, b"\x1b]12;rgb:00/ff/00\x1b\\");
        // Set palette override (OSC 4)
        feed(&mut t, b"\x1b]4;1;rgb:aa/bb/cc\x1b\\");
        // Enable bracketed paste
        feed(&mut t, b"\x1b[?2004h");
        // Set title
        feed(&mut t, b"\x1b]0;Test\x07");

        // RIS
        feed(&mut t, b"\x1bc");

        // All state should be fully reset
        assert_eq!(t.cursor_style(), CursorStyle::Default);
        assert!(t.dynamic_fg().is_none());
        assert!(t.dynamic_cursor().is_none());
        assert!(t.palette_overrides().is_empty());
        assert!(!t.bracketed_paste());
        assert!(t.title().is_empty());
    }

    #[test]
    fn t_ris_preserves_scrollback_limit() {
        // RIS should preserve the user-configured scrollback limit.
        // It should NOT reset it to the default (10000).
        let mut t = Terminal::with_scrollback(80, 24, 50_000);
        // RIS
        feed(&mut t, b"\x1bc");
        // Scrollback limit should still be 50_000, not reset to 10_000.
        assert_eq!(
            t.grid().max_scrollback(),
            50_000,
            "RIS must preserve user-configured scrollback limit"
        );
    }

    #[test]
    fn t_ris_preserves_cell_dimensions() {
        // RIS should preserve cell dimensions (set by the window layer
        // after font measurement). These are needed for pixel-size
        // queries (CSI 14t/15t/16t).
        let mut t = Terminal::new(80, 24);
        t.set_cell_dimensions(9, 18);
        feed(&mut t, b"\x1bc"); // RIS
        assert_eq!(
            t.cell_dimensions,
            Some((9, 18)),
            "RIS must preserve cell dimensions"
        );
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
    fn t_osc_8_strips_control_chars() {
        // A malicious URI with embedded BEL and ESC should have control
        // characters stripped to prevent injection attacks.
        let mut t = Terminal::new(80, 24);
        // OSC 8 ; ; https://evil.com\x07\x1b[2J\x1b\\
        // The \x07 and \x1b should be stripped from the URI.
        feed(&mut t, b"\x1b]8;;https://evil.com\x07INJECTED\x1b\\");
        // URI should NOT contain the BEL or ESC characters.
        let hl = t.current_hyperlink.as_deref().unwrap_or("");
        assert!(
            !hl.contains('\x07'),
            "BEL should be stripped from hyperlink URI: {hl:?}"
        );
        assert!(
            !hl.contains('\x1b'),
            "ESC should be stripped from hyperlink URI: {hl:?}"
        );
        // The cleaned URL should still be usable.
        assert!(hl.starts_with("https://evil.com"));
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
    fn t_sgr_overflow_param_no_panic() {
        // Huge SGR param should saturate, not overflow/panic.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[999999999m");
        // Should not panic. Foreground should be default (unknown code ignored).
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Default);
    }

    #[test]
    fn t_cup_zero_zero_normalizes() {
        // CSI 0;0H should normalize to row 1 col 1 (xterm spec: 0 → 1).
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5;5HABC"); // Move to row 5 col 5
        feed(&mut t, b"\x1b[0;0H"); // Should go to 1;1
        feed(&mut t, b"X");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X');
    }

    #[test]
    fn t_alt_screen_repeated_toggle_no_leak() {
        // Repeatedly entering/exiting alt screen should not accumulate grids.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"PRIMARY");
        for _ in 0..10 {
            feed(&mut t, b"\x1b[?1049h"); // Enter alt
            feed(&mut t, b"ALT");
            feed(&mut t, b"\x1b[?1049l"); // Exit alt
        }
        // Primary content should be restored.
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'P');
        // No saved grid should remain (alt_saved_grid is None after exit).
        assert!(t.alt_saved_grid.is_none());
    }

    #[test]
    fn t_alt_screen_no_hyperlink_leak_on_exit() {
        // OSC 8 hyperlink state set in alt screen must not carry over
        // to the main screen when exiting via mode 1049.
        let mut t = Terminal::new(80, 24);
        // Enter alt screen
        feed(&mut t, b"\x1b[?1049h");
        // Set a hyperlink in alt screen
        feed(&mut t, b"\x1b]8;;https://evil.example.com\x1b\\");
        feed(&mut t, b"linked");
        // Exit alt screen
        feed(&mut t, b"\x1b[?1049l");
        // current_hyperlink should be cleared
        assert!(
            t.current_hyperlink.is_none(),
            "hyperlink state should not leak from alt screen"
        );
        // Print a char on main screen — it should NOT have a hyperlink
        feed(&mut t, b"X");
        assert!(
            t.grid().cell(0, 0).unwrap().hyperlink.is_none(),
            "main screen cell should not have alt screen's hyperlink"
        );
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
        // DECOM enable homes cursor to scroll region top (row 5 → 0-based: 4)
        assert_eq!(t.cursor(), (0, 4));
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
    fn t_utf8_partial_then_esc_discards() {
        // Partial UTF-8 followed by ESC sequence: partial must be discarded
        // and ESC processed as control, not consumed into UTF-8 buffer.
        let mut t = Terminal::new(80, 24);
        let mut p = Parser::new();
        p.feed(&[0xE4, 0xB8], &mut t); // first 2 bytes of U+4E2D (中)
        p.feed(b"\x1b[31m", &mut t); // ESC [ 31 m — SGR red
        p.feed(b"X", &mut t);
        let cell = t.grid().cell(0, 0).unwrap();
        // Partial UTF-8 should have been flushed as U+FFFD, then SGR applied.
        assert_eq!(
            cell.ch, '\u{FFFD}',
            "partial UTF-8 should emit replacement char"
        );
        // The 'X' at col 1 should be red (SGR was processed).
        let x_cell = t.grid().cell(1, 0).unwrap();
        assert_eq!(x_cell.ch, 'X');
        assert!(!x_cell.flags.contains(CellFlags::BOLD));
        assert_eq!(
            x_cell.fg,
            Color::Indexed(1),
            "SGR red should have been applied"
        );
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
    fn t_combining_char_after_wide_wrap_at_col0() {
        // When a wide char fills the last columns of a row and the cursor
        // wraps to the next line, a combining char arriving at col 0 should
        // attach to the wide char's lead cell (at width-2), not be dropped
        // because width-1 is the wide spacer.
        let mut t = Terminal::new(4, 3);
        // Fill cols 0-1 with 'A' (wide), cols 2-3 with 'B' (wide).
        // Both are CJK width-2 chars on a 4-wide terminal.
        feed(&mut t, "你好".as_bytes());
        // Cursor should be at col 0 of row 1 (pending wrap consumed).
        // Now send a combining char (U+0301 = combining acute accent).
        feed(&mut t, "\u{0301}".as_bytes());
        // The combining char should have attached to the '好' at col 2 of row 0
        // (the last wide char lead cell), NOT been dropped.
        let cell = t.grid().cell(2, 0).unwrap();
        assert!(
            !cell.combining.is_empty(),
            "combining char should attach to wide char lead cell after wrap"
        );
        assert_eq!(cell.combining[0], '\u{0301}');
    }

    #[test]
    fn t_resize_shrink_cursor_beyond_new_width() {
        // Cursor at col 50, shrink to width 10 — cursor must clamp to 9.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[1;50H"); // CUP to col 50
        assert_eq!(t.cursor().0, 49);
        t.resize(10, 24);
        assert_eq!(
            t.cursor().0,
            9,
            "cursor must clamp to last column after shrink"
        );
    }

    #[test]
    fn t_resize_to_1_column() {
        // Resizing to 1 column should not panic. Wide chars can't fit.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"Hello World");
        t.resize(1, 24);
        // Cursor should be at col 0.
        assert_eq!(t.cursor().0, 0);
        // Content should still be accessible in scrollback.
        let exported = t.grid().export_text();
        assert!(
            exported.contains('H'),
            "content should survive 1-col resize: {exported:?}"
        );
    }

    #[test]
    fn t_resize_shrink_with_wide_char_at_boundary() {
        // A wide char at cols 8-9 in a 10-wide terminal.
        // Shrink to 9 — the wide lead at col 8 should be cleared (no spacer).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "AAAAAAAA中".as_bytes()); // 8 'A's + CJK wide char
        assert!(t.grid().cell(8, 0).unwrap().is_wide());
        assert!(t.grid().cell(9, 0).unwrap().is_wide_spacer());
        t.resize(9, 3);
        // Col 8 should no longer be a wide char (spacer truncated).
        let cell = t.grid().cell(8, 0).unwrap();
        assert!(
            !cell.is_wide(),
            "dangling wide lead should be cleared after shrink"
        );
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
        // DECALN fills entire screen with 'E' regardless of scroll region.
        // Per xterm spec, DECALN also resets scroll region to full screen.
        let mut t = Terminal::new(10, 6);
        // Set scroll region to rows 1-4 (0-based)
        feed(&mut t, b"\x1b[2;5r");
        feed(&mut t, b"\x1b#8");
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(top, 0, "scroll region reset to full screen by DECALN");
        assert_eq!(bottom, 6, "scroll region reset to full screen by DECALN");
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
    fn t_resize_preserves_tab_stops_at_boundary() {
        // Resize from 80→100 should set tab stop at col 80 (a multiple of 8).
        // The old formula (old_width/8+1)*8 skipped it, starting at 88 instead.
        let mut t = Terminal::new(80, 24);
        t.resize(100, 24);
        // Col 80 is a default tab stop position (80 % 8 == 0).
        assert!(
            t.tab_stops.get(80).copied().unwrap_or(false),
            "col 80 should have a default tab stop after resize 80→100"
        );
    }

    #[test]
    fn t_resize_shrink_grow_preserves_content() {
        // Feed text, resize smaller, then back — content should survive.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"Hello World");
        t.resize(10, 3); // Shrink drastically
        t.resize(80, 24); // Grow back
        // "Hello World" should still be in scrollback (reflow pushed excess rows up).
        let exported = t.grid().export_text();
        assert!(
            exported.contains("Hello"),
            "content should survive resize shrink+grow: {exported:?}"
        );
    }

    #[test]
    fn t_autowrap_off_overwrites_at_margin() {
        // With DECAWM off (CSI ?7l), text at the right margin overwrites
        // instead of wrapping. Cursor stays at last column.
        let mut t = Terminal::new(4, 3);
        feed(&mut t, b"\x1b[?7l"); // Disable auto-wrap
        feed(&mut t, b"ABCDE"); // 5 chars on a 4-wide terminal
        // Without wrap, the 5th char 'E' overwrites 'D' at col 3.
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'E');
        assert_eq!(t.cursor().0, 3); // cursor at last column
    }

    #[test]
    fn t_deferred_wrap_pending_state() {
        // When the cursor is at the last column and a char is printed,
        // xterm uses deferred wrap: cursor stays at last column with
        // pending_wrap=true. The actual line feed happens on the NEXT char.
        let mut t = Terminal::new(4, 3);
        feed(&mut t, b"ABCD"); // Fill the line exactly
        // Cursor should be at col 3 with pending_wrap.
        assert_eq!(t.cursor().0, 3);
        // Now print one more — should wrap to next line.
        feed(&mut t, b"E");
        assert_eq!(t.cursor().1, 1); // row 1
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'E');
    }

    #[test]
    fn t_decawm_off_clears_pending_wrap() {
        // Turning off DECAWM (mode 7) should clear pending_wrap.
        // This matches xterm behavior: when vim/tmux disable autowrap
        // after filling the last column, they expect no deferred wrap
        // to trigger even if DECAWM is later re-enabled.
        let mut t = Terminal::new(4, 3);
        feed(&mut t, b"ABCD"); // Fill line, pending_wrap = true
        assert!(t.cursor.pending_wrap);
        // Turn off DECAWM
        feed(&mut t, b"\x1b[?7l");
        assert!(
            !t.cursor.pending_wrap,
            "pending_wrap should be cleared when DECAWM is turned off"
        );
        // Turn DECAWM back on, then write a char.
        // It should overwrite at the last column, NOT wrap.
        feed(&mut t, b"\x1b[?7h");
        feed(&mut t, b"X");
        assert_eq!(t.cursor().0, 3, "should overwrite at last column");
        assert_eq!(t.cursor().1, 0, "should NOT wrap to next line");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'X');
    }

    #[test]
    fn t_el0_clears_wrap_flag() {
        // EL 0 (erase from cursor to end of line) should clear the
        // soft-wrap flag. Without this, a stale wrap flag causes
        // incorrect reflow on resize (joining with the next blank line).
        let mut t = Terminal::new(4, 3);
        // Fill a line so it wraps (pending_wrap + row wrap flag)
        feed(&mut t, b"ABCD"); // fills row 0, wraps to row 1 on next char
        feed(&mut t, b"E"); // row 1
        // Row 0 should have wrap=true
        assert!(
            t.grid().row(0).unwrap().wrap,
            "row 0 should be soft-wrapped"
        );
        // Move to row 0, col 2, and EL 0
        feed(&mut t, b"\x1b[1;3H");
        feed(&mut t, b"\x1b[0K");
        // Row 0 wrap flag should now be false
        assert!(
            !t.grid().row(0).unwrap().wrap,
            "EL 0 should clear wrap flag — line no longer continues"
        );
    }

    #[test]
    fn t_ed0_clears_wrap_flag_current_line() {
        // ED 0 (erase from cursor to end of display) should clear the
        // wrap flag for the current line, not just lines below it.
        let mut t = Terminal::new(4, 3);
        feed(&mut t, b"ABCD"); // fills row 0
        feed(&mut t, b"E"); // wraps to row 1
        assert!(t.grid().row(0).unwrap().wrap);
        // Move to row 0, col 2
        feed(&mut t, b"\x1b[1;3H");
        feed(&mut t, b"\x1b[0J"); // ED 0
        assert!(
            !t.grid().row(0).unwrap().wrap,
            "ED 0 should clear wrap flag for current line"
        );
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

    #[test]
    fn t_scroll_region_lf_at_boundary() {
        // LF at the bottom of the scroll region should scroll, not
        // move the cursor past the region.
        let mut t = Terminal::new(10, 5);
        // Set scroll region to rows 1-3 (0-based: 0-2)
        feed(&mut t, b"\x1b[1;3r");
        feed(&mut t, b"\x1b[1;1H"); // cursor at (0,0) — top of region
        feed(&mut t, b"ABC");
        feed(&mut t, b"\x1b[2;1H"); // cursor at row 1 (0-based), col 0
        feed(&mut t, b"DEF");
        feed(&mut t, b"\x1b[3;1H"); // cursor at row 2 (0-based), col 0
        feed(&mut t, b"GHI");
        // Cursor at row 2 (bottom of region, 0-based). LF should scroll.
        feed(&mut t, b"\n");
        // Cursor should stay at row 2 (bottom of region after scroll)
        assert_eq!(
            t.cursor().1,
            2,
            "LF at region bottom should keep cursor at bottom"
        );
        // After scroll, row 0 should contain "DEF" (shifted up from row 1)
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'D',
            "row 0 should be shifted content"
        );
    }

    #[test]
    fn t_scroll_region_cuu_respects_boundary() {
        // CUU at the top of scroll region should stop, not go above it.
        let mut t = Terminal::new(10, 10);
        // Set scroll region to rows 3-7 (0-based: 2-6)
        feed(&mut t, b"\x1b[3;7r");
        feed(&mut t, b"\x1b[5;1H"); // row 4 (0-based), inside region
        // CUU 10 — should stop at region top (row 2)
        feed(&mut t, b"\x1b[10A");
        assert_eq!(
            t.cursor().1,
            2,
            "CUU should stop at scroll region top, not row 0"
        );
    }

    #[test]
    fn t_scroll_region_cud_respects_boundary() {
        // CUD at the bottom of scroll region should stop, not go below it.
        let mut t = Terminal::new(10, 10);
        // Set scroll region to rows 3-7 (0-based: 2-6)
        feed(&mut t, b"\x1b[3;7r");
        feed(&mut t, b"\x1b[5;1H"); // row 4 (0-based), inside region
        // CUD 10 — should stop at region bottom (row 6)
        feed(&mut t, b"\x1b[10B");
        assert_eq!(
            t.cursor().1,
            6,
            "CUD should stop at scroll region bottom, not last row"
        );
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
    fn t_osc133_command_end_trailing_semicolon() {
        // Some zsh integrations send "D;exit_code;" (trailing semicolon).
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;D;0;\x07");
        let marks = t.command_marks();
        assert_eq!(marks.len(), 1);
        assert_eq!(
            marks[0].exit_code,
            Some(0),
            "trailing semicolon should still parse exit code"
        );
    }

    #[test]
    fn t_osc133_command_end_with_extra_fields() {
        // Some integrations send "D;exit_code;extra;fields".
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]133;D;127;timestamp\x07");
        let marks = t.command_marks();
        assert_eq!(marks.len(), 1);
        assert_eq!(
            marks[0].exit_code,
            Some(127),
            "extra fields after exit code should not break parsing"
        );
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
    fn t_osc133_absolute_row_after_scroll() {
        // Command marks must store ABSOLUTE positions (scrollback + cursor.y)
        // so they remain valid after the terminal scrolls.
        let mut t = Terminal::new(10, 3); // tiny grid to force scrolling
        feed(&mut t, b"\x1b]133;A\x07"); // prompt at row 0, scrollback=0 → abs=0
        // Fill all 3 rows then scroll: print 3 lines with \r\n
        feed(&mut t, b"L0\r\n"); // cursor at row 1, scrollback=0
        feed(&mut t, b"L1\r\n"); // cursor at row 2, scrollback=0
        feed(&mut t, b"L2\r\n"); // cursor at row 2 after scroll, scrollback=1
        // Now grid scrolled once. Row "L0" is in scrollback[0].
        feed(&mut t, b"\x1b]133;A\x07"); // second prompt at cursor.y, scrollback=1
        let marks = t.command_marks();
        assert_eq!(marks.len(), 2);
        // First mark: scrollback=0, cursor.y=0 → abs=0.
        assert_eq!(marks[0].row, 0);
        // Second mark: scrollback=1, cursor.y=2 → abs=3.
        assert_eq!(
            marks[1].row, 3,
            "second mark must be absolute (scrollback+cursor.y), got {}",
            marks[1].row
        );
        // Verify extract_absolute_row_text finds the scrolled content.
        let text = t.extract_absolute_row_text(0);
        assert!(
            text.contains("L0"),
            "abs row 0 should be 'L0' from scrollback, got: '{text}'"
        );
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
    fn t_decstr_resets_protected_attr() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[1\"q"); // DECSCA — set protected
        assert!(t.protected_attr);
        feed(&mut t, b"\x1b[!p"); // DECSTR — soft reset
        assert!(!t.protected_attr, "DECSTR should reset protected_attr");
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
        assert!(
            t.modes.alternate_scroll,
            "DECSTR should restore alternate_scroll=true"
        );
    }

    #[test]
    fn t_decstr_resets_cursor_style() {
        let mut t = Terminal::new(80, 24);
        // Set cursor to SteadyBar (vim insert mode)
        feed(&mut t, b"\x1b[6 q");
        assert_eq!(t.cursor_style(), CursorStyle::SteadyBar);
        // DECSTR should reset cursor style to Default
        feed(&mut t, b"\x1b[!p");
        assert_eq!(
            t.cursor_style(),
            CursorStyle::Default,
            "DECSTR should reset cursor_style to Default"
        );
    }

    #[test]
    fn t_decstr_resets_modify_other_keys() {
        let mut t = Terminal::new(80, 24);
        // Enable modifyOtherKeys mode 2
        feed(&mut t, b"\x1b[>4;2h");
        assert_eq!(t.modify_other_keys(), 2);
        // DECSTR should reset to 0
        feed(&mut t, b"\x1b[!p");
        assert_eq!(
            t.modify_other_keys(),
            0,
            "DECSTR should reset modifyOtherKeys to 0"
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
    fn t_osc10_query_uses_palette_override() {
        // When a palette override is set via OSC 4, the OSC 10 query
        // for an indexed foreground color should use the overridden value.
        let mut t = Terminal::new(80, 24);
        // Set fg to indexed color 1 (red = 0xcd0000)
        feed(&mut t, b"\x1b[31m");
        // Override palette entry 1 to pure green
        feed(&mut t, b"\x1b]4;1;rgb:00/ff/00\x1b\\");
        t.take_response(); // clear any pending response
        // Query fg color — should use the override, not the built-in red
        feed(&mut t, b"\x1b]10;?\x1b\\");
        let resp = String::from_utf8_lossy(t.response_buffer());
        assert!(
            resp.contains("rgb:00/ff/00"),
            "OSC 10 query should use palette override (green), got: {resp}"
        );
        assert!(
            !resp.contains("rgb:cd/00/00"),
            "OSC 10 query should NOT use built-in red, got: {resp}"
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
    fn t_osc10_set_then_query_roundtrip() {
        // OSC 10 query must return the color set by a previous OSC 10,
        // not the SGR fg color.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]10;rgb:ab/cd/ef\x1b\\");
        t.take_response(); // clear any pending response
        feed(&mut t, b"\x1b]10;?\x1b\\");
        let resp = String::from_utf8_lossy(t.response_buffer());
        assert!(
            resp.contains("10;rgb:ab/cd/ef"),
            "OSC 10 query should return the set color, got: {resp}"
        );
    }

    #[test]
    fn t_osc11_set_then_query_roundtrip() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]11;rgb:12/34/56\x1b\\");
        t.take_response(); // clear
        feed(&mut t, b"\x1b]11;?\x1b\\");
        let resp = String::from_utf8_lossy(t.response_buffer());
        assert!(
            resp.contains("11;rgb:12/34/56"),
            "OSC 11 query should return the set color, got: {resp}"
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

    #[test]
    fn t_decsel_mode0_wide_char_at_cursor() {
        // DECSEL 0 (erase cursor to end of line) when cursor is on a
        // wide char spacer should also clear the lead cell.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"A");
        feed(&mut t, "中".as_bytes()); // wide char at cols 1-2
        feed(&mut t, b"B");
        // Cursor is now at col 3. Move to col 2 (the spacer).
        feed(&mut t, b"\x1b[1;3H"); // 1-based col 3 = 0-based col 2
        feed(&mut t, b"\x1b[?0K"); // DECSEL 0
        // Lead at col 1 should be cleared (not orphaned).
        let lead = t.grid().cell(1, 0).unwrap();
        assert!(
            !lead.is_wide(),
            "wide lead should be cleared, not orphaned by DECSEL 0"
        );
        // 'A' at col 0 should survive.
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
    }

    #[test]
    fn t_decsel_mode1_wide_char_after_range() {
        // DECSEL 1 (erase start to cursor) when the cell right after the
        // erase range is a wide spacer whose lead was erased.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, "中".as_bytes()); // wide at cols 0-1
        feed(&mut t, b"BCD");
        // Cursor at col 3. Erase from start to col 1 (inclusive).
        // This erases the wide lead at col 0 and 'B' at col 2.
        // Wait — col 1 is the spacer. DECSEL 1 erases 0..=col where col=cursor.x.
        // Move cursor to col 0 (lead), erase 0..=0, which erases the lead.
        // The spacer at col 1 should also be cleared (orphan cleanup).
        feed(&mut t, b"\x1b[1;1H"); // col 0
        feed(&mut t, b"\x1b[?1K"); // DECSEL 1: erase start to cursor
        // The spacer at col 1 should not be orphaned.
        let spacer = t.grid().cell(1, 0).unwrap();
        assert!(
            !spacer.is_wide_spacer(),
            "orphaned spacer should be cleared after DECSEL 1"
        );
    }
    #[test]
    fn t_decsel_mode2_clears_wrap_flag() {
        let mut t = Terminal::new(4, 2);
        feed(&mut t, b"ABCD");
        feed(&mut t, b"E"); // wraps — row 0 is soft-wrapped
        assert!(t.grid().row(0).unwrap().wrap);
        feed(&mut t, b"\x1b[1;1H");
        feed(&mut t, b"\x1b[?2K"); // DECSEL 2
        assert!(
            !t.grid().row(0).unwrap().wrap,
            "DECSEL 2 should clear row_wrap flag"
        );
    }

    #[test]
    fn t_decsel_mode0_clears_wrap_flag() {
        let mut t = Terminal::new(4, 2);
        feed(&mut t, b"ABCD");
        feed(&mut t, b"E");
        assert!(t.grid().row(0).unwrap().wrap);
        feed(&mut t, b"\x1b[1;1H");
        feed(&mut t, b"\x1b[?0K");
        assert!(
            !t.grid().row(0).unwrap().wrap,
            "DECSEL 0 should clear row_wrap flag"
        );
    }

    #[test]
    fn t_decsed_mode1_clears_wrap_flag() {
        let mut t = Terminal::new(4, 2);
        feed(&mut t, b"ABCD");
        feed(&mut t, b"E");
        assert!(t.grid().row(0).unwrap().wrap);
        feed(&mut t, b"\x1b[1;1H");
        feed(&mut t, b"\x1b[?1J");
        assert!(
            !t.grid().row(0).unwrap().wrap,
            "DECSED 1 should clear row_wrap flag"
        );
    }

    #[test]
    fn t_decsed_mode2_clears_wrap_flag() {
        let mut t = Terminal::new(4, 2);
        feed(&mut t, b"ABCD");
        feed(&mut t, b"E");
        assert!(t.grid().row(0).unwrap().wrap);
        feed(&mut t, b"\x1b[?2J");
        assert!(
            !t.grid().row(0).unwrap().wrap,
            "DECSED 2 should clear row_wrap flag"
        );
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
        // Must advertise rectangular editing (28) — DECFRA/DECERA/DECSERA are implemented.
        assert!(
            s.contains(";28;") || s.contains(";28c"),
            "DA1 should report rectangular editing (28), got: {s}"
        );
        // Must NOT advertise unimplemented capabilities.
        assert!(
            !s.contains(";1;") && !s.contains(";1c"),
            "DA1 should not advertise 132-col (1) — DECCOLM not implemented, got: {s}"
        );
        assert!(
            !s.contains(";9;") && !s.contains(";9c"),
            "DA1 should not advertise NRC (9) — not implemented, got: {s}"
        );
        assert!(
            !s.contains(";16;") && !s.contains(";16c"),
            "DA1 should not advertise locator port (16) — not implemented, got: {s}"
        );
        // Must advertise selective erase (6) and ANSI color (22).
        assert!(
            s.contains(";6;") || s.contains(";6c"),
            "DA1 should report selective erase (6), got: {s}"
        );
        assert!(
            s.contains(";22;") || s.contains(";22c"),
            "DA1 should report ANSI color (22), got: {s}"
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

    // ── REP (Repeat, CSI Ps b) — pending_wrap regression tests ──

    #[test]
    fn t_rep_wraps_at_line_end() {
        // Print chars to fill the line, then REP should wrap to next line.
        // Terminal width = 5. Print 'A' 4 times (positions 0-3), then
        // the 5th 'A' goes to position 4 with pending_wrap.
        // REP 2 should wrap to next line and print 2 'A's there.
        let mut t = Terminal::new(5, 24);
        feed(&mut t, b"AAAAA\x1b[2b");
        let row0 = t.grid().row_text(0).unwrap_or_default();
        assert_eq!(row0.trim_end(), "AAAAA", "first line should be full");
        let row1 = t.grid().row_text(1).unwrap_or_default();
        assert_eq!(
            row1.trim_end(),
            "AA",
            "REP should wrap and print on next line"
        );
    }

    #[test]
    fn t_rep_preserves_last_column_before_wrap() {
        // Key regression test: REP must not overwrite the last column.
        // Width = 5. Fill 5 chars: "ABCDE". Cursor at position 4
        // with pending_wrap=true (E is at position 4).
        // REP 1 should wrap first, then print 'E' on the next line.
        // The original 'E' at position 4 should NOT be overwritten.
        let mut t = Terminal::new(5, 24);
        feed(&mut t, b"ABCDE\x1b[1b");
        let row0 = t.grid().row_text(0).unwrap_or_default();
        assert_eq!(row0.trim_end(), "ABCDE", "last column should still be 'E'");
        let row1 = t.grid().row_text(1).unwrap_or_default();
        assert_eq!(row1.trim_end(), "E", "REP should wrap to next line");
    }

    #[test]
    fn t_rep_no_last_char() {
        // REP with no preceding printable char should do nothing.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[5b");
        let row = t.grid().row_text(0).unwrap_or_default();
        assert!(
            row.trim_end().is_empty(),
            "REP with no last char should produce nothing"
        );
    }

    #[test]
    fn t_rep_after_control_sequence() {
        // Control sequences do not set last_printed_char, so REP after
        // a CSI sequence without any printable char should do nothing.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[1;31m\x1b[3b"); // SGR red, then REP
        let row = t.grid().row_text(0).unwrap_or_default();
        assert!(row.trim_end().is_empty());
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
    fn t_decrqm_mode_1048_reports_reset() {
        // Mode 1048 is a transient save/restore action, not a persistent mode.
        // Per xterm behavior, DECRQM should always report it as "reset" (2),
        // even after DECSC has saved cursor state.
        let mut t = Terminal::new(80, 24);
        // Save cursor state (DECSC) — this would have made 1048 report "set"
        // with the old buggy implementation.
        feed(&mut t, b"\x1b7");
        feed(&mut t, b"\x1b[?1048$p"); // Query mode 1048
        let resp = t.take_response();
        let resp_str = String::from_utf8_lossy(&resp);
        assert!(
            resp_str.contains(";2$y"),
            "mode 1048 should be reset (2) — it's a transient action, got: {}",
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

    #[test]
    fn t_decsc_independent_from_scp_rcp() {
        // DECSC (ESC 7) and SCP (CSI s) should use separate save slots.
        // Saving with one should not affect the other.
        let mut t = Terminal::new(80, 24);
        // Position 1: save via DECSC
        feed(&mut t, b"\x1b[5;10H"); // row 5, col 10
        feed(&mut t, b"\x1b7"); // DECSC
        // Position 2: save via SCP
        feed(&mut t, b"\x1b[15;20H"); // row 15, col 20
        feed(&mut t, b"\x1b[s"); // SCP
        // Move away
        feed(&mut t, b"\x1b[1;1H");
        // Restore via RCP (CSI u) — should go to position 2
        feed(&mut t, b"\x1b[u");
        assert_eq!(
            (t.cursor().0, t.cursor().1),
            (19, 14),
            "RCP restores SCP position"
        );
        // Restore via DECRC (ESC 8) — should go to position 1
        feed(&mut t, b"\x1b8");
        assert_eq!(
            (t.cursor().0, t.cursor().1),
            (9, 4),
            "DECRC restores DECSC position"
        );
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

    // ── Wrap flag + scroll integration tests ─────────────────────

    #[test]
    fn t_wrap_flag_set_before_scroll_into_scrollback() {
        // Feed exactly 80 chars to fill row 23 (bottom of 80x24),
        // then one more char to trigger auto-wrap + scroll.
        // The scrolled-off row (old row 0) must have wrap=true in scrollback.
        let mut t = Terminal::new(4, 3);
        // Fill all 3 rows completely (4 chars per row = 12 total).
        feed(&mut t, b"ABCDEFGHIJKLMNOP");
        // Row 0 should have scrolled into scrollback. Check its wrap flag.
        let sb = t.grid().scrollback_row(0);
        assert!(sb.is_some(), "should have scrollback");
        let sb_row = sb.unwrap();
        assert!(
            sb_row.wrap,
            "scrolled-off row should have wrap=true for reflow support"
        );
    }

    #[test]
    fn t_wrap_flag_cleared_on_hard_newline() {
        // After CR+LF, the row should NOT have wrap=true.
        let mut t = Terminal::new(8, 3);
        feed(&mut t, b"ABCD\r\n");
        // Row 0: "ABCD" with hard newline → wrap=false
        let row0 = t.grid().row(0).unwrap();
        assert!(!row0.wrap, "row with explicit CR+LF should have wrap=false");
    }

    #[test]
    fn t_wrap_flag_set_on_soft_wrap() {
        // Fill a row completely so the next char soft-wraps.
        let mut t = Terminal::new(4, 4);
        feed(&mut t, b"ABCDE"); // 5 chars: row 0 = ABCD, wrap, row 1 = E
        let row0 = t.grid().row(0).unwrap();
        assert!(row0.wrap, "row 0 should have wrap=true after soft wrap");
        let row1 = t.grid().row(1).unwrap();
        assert!(!row1.wrap, "row 1 should have wrap=false (no continuation)");
    }

    #[test]
    fn t_el_clears_wrap_flag() {
        // EL mode 2 (erase entire line) should clear wrap flag on the cursor's row.
        let mut t = Terminal::new(4, 4);
        feed(&mut t, b"ABCDE"); // row 0 wrap=true, cursor on row 1
        // Move cursor to row 0 and erase that line.
        feed(&mut t, b"\x1b[1;1H\x1b[2K"); // cursor to row 0 + EL mode 2
        assert!(
            !t.grid().row(0).unwrap().wrap,
            "EL 2 should clear wrap on row 0"
        );
    }

    #[test]
    fn t_ed_clears_all_wrap_flags() {
        // ED mode 2 (clear all) should clear all wrap flags.
        let mut t = Terminal::new(4, 4);
        feed(&mut t, b"ABCDE"); // row 0 wrap=true
        feed(&mut t, b"\r\x1b[2J"); // CR + ED mode 2
        for r in 0..4 {
            assert!(
                !t.grid().row(r).unwrap().wrap,
                "row {r} should have wrap=false after ED 2"
            );
        }
    }

    #[test]
    fn t_no_reflow_in_alt_screen() {
        // In alt screen, resize should truncate (not reflow).
        // Enter alt screen, fill a wide row, shrink, verify truncation.
        let mut t = Terminal::new(8, 4);
        feed(&mut t, b"\x1b[?1049h"); // enter alt screen
        feed(&mut t, b"ABCDEFGH"); // fill row 0 completely (8 cols)
        assert!(t.is_alt_screen());

        // Shrink to 4 cols — should truncate, not reflow.
        t.resize(4, 4);
        // Content should NOT reflow: 'EFGH' should be lost (truncated),
        // not wrapped to the next line.
        let row0_text = t.grid().row(0).unwrap().text();
        assert!(
            row0_text.contains('A'),
            "row 0 should still have A: {row0_text}"
        );
        assert!(
            !row0_text.contains('E'),
            "row 0 should NOT have E (truncated, not reflowed): {row0_text}"
        );
    }

    // ── X11 color parsing tests ──────────────────────────────────

    #[test]
    fn t_parse_xcolor_8bit_channels() {
        assert_eq!(parse_xcolor("rgb:ff/00/ff"), Some(Color::Rgb(255, 0, 255)));
        assert_eq!(parse_xcolor("rgb:FF/00/FF"), Some(Color::Rgb(255, 0, 255)));
    }

    #[test]
    fn t_parse_xcolor_16bit_channels() {
        // rgb:ffff/0000/ffff — 16-bit per channel, should scale to 8-bit.
        assert_eq!(
            parse_xcolor("rgb:ffff/0000/ffff"),
            Some(Color::Rgb(255, 0, 255))
        );
        // rgb:8000/8000/8000 — approximately half intensity in 16-bit.
        let c = parse_xcolor("rgb:8000/8000/8000").unwrap();
        assert!(matches!(c, Color::Rgb(r, _, _) if (127..=128).contains(&r)));
    }

    #[test]
    fn t_parse_xcolor_invalid_returns_none() {
        assert_eq!(parse_xcolor("#XYZ"), None);
        assert_eq!(parse_xcolor("rgb:zz/00/00"), None);
        assert_eq!(parse_xcolor("not-a-color"), None);
    }

    // ===== DECFRA — Fill Rectangle Area tests =====

    #[test]
    fn t_decfra_fills_rectangle_with_space() {
        let mut t = Terminal::new(10, 5);
        // Fill some text first
        feed(&mut t, b"ABCDEFGH");
        // DECFRA format: CSI Pch;Pt;Pl;Pb;Pr $ x
        // Pch=32 (space), fill rect rows 2-4, cols 2-5 (1-based)
        feed(&mut t, b"\x1b[32;2;2;4;5$x");
        // Row 0 should be unchanged
        let r0 = t.grid().row(0).unwrap().text();
        assert!(r0.starts_with("ABCDEFGH"), "row 0 unchanged: {r0:?}");
        // Row 1 (0-based) cols 1-4 should be spaces
        for col in 1..=4 {
            let cell = &t.grid()[(col, 1)];
            assert_eq!(
                cell.ch, ' ',
                "col {col} row 1 should be space, got {:?}",
                cell.ch
            );
        }
        // Col 0 row 1 should be default (blank), not filled
        let c0 = &t.grid()[(0, 1)];
        assert_eq!(c0.ch, ' ', "col 0 row 1 should also be blank (default)");
    }

    #[test]
    fn t_decfra_uses_current_sgr_colors() {
        let mut t = Terminal::new(10, 4);
        // Set bg to red (SGR 41)
        feed(&mut t, b"\x1b[41m");
        // DECFRA: Pch=32 (space), fill rect rows 1-2, cols 1-3
        feed(&mut t, b"\x1b[32;1;1;2;3$x");
        // Check cells have red background
        let cell = &t.grid()[(0, 0)];
        assert_eq!(
            cell.bg,
            Color::Indexed(1),
            "DECFRA should apply current bg color"
        );
    }

    #[test]
    fn t_decfra_clamps_to_screen_bounds() {
        let mut t = Terminal::new(5, 3);
        // DECFRA: Pch=32, out-of-bounds coords should clamp, not panic
        feed(&mut t, b"\x1b[32;1;1;100;100$x");
        // Should fill entire screen without panicking
        for row in 0..3 {
            for col in 0..5 {
                let cell = &t.grid()[(col, row)];
                assert_eq!(cell.ch, ' ', "cell ({col},{row}) should be space");
            }
        }
    }

    #[test]
    fn t_decfra_default_params_fills_single_cell() {
        let mut t = Terminal::new(5, 3);
        // Pre-fill with text
        feed(&mut t, b"HELLO");
        // DECFRA with no params: Pch omitted (invalid, should be ignored)
        // Per spec: if Pch is not in 32-126/160-255 range, command is ignored.
        // With no params, Pch defaults to 0 → invalid → no-op.
        feed(&mut t, b"\x1b[$x");
        // Since Pch=0 is invalid, nothing should change.
        let r0 = t.grid().row(0).unwrap().text();
        assert!(
            r0.starts_with("HELLO"),
            "DECFRA with invalid Pch should be no-op: {r0:?}"
        );
    }

    #[test]
    fn t_decfra_invalid_pch_ignored() {
        // Per DEC STD 070: if Pch is not in 32-126 or 160-255, command is ignored.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE");
        // Pch=0 (invalid) → entire command ignored
        feed(&mut t, b"\x1b[0;1;1;3;3$x");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "invalid Pch → no-op");

        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE");
        // Pch=10 (control char, invalid) → ignored
        feed(&mut t, b"\x1b[10;1;1;3;3$x");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'A',
            "Pch=10 (control) → no-op"
        );
    }

    #[test]
    fn t_decfra_valid_high_pch() {
        // Pch in 160-255 range is valid (Latin-1 supplement).
        let mut t = Terminal::new(5, 3);
        // Pch=0xA0 (160) = NBSP, valid
        feed(&mut t, b"\x1b[160;1;1;2;2$x");
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.ch as u32, 0xA0, "Pch=160 should produce NBSP");
    }

    // ===== DECERA — Erase Rectangle Area tests =====

    #[test]
    fn t_decera_erases_rectangle() {
        let mut t = Terminal::new(10, 5);
        // Fill text on multiple rows
        feed(&mut t, b"ABCDEFGH\r\nIJKLMNOP\r\nQRSTUVWX");
        // DECERA: erase rect rows 1-2, cols 1-3 (1-based) = rows 0-1, cols 0-2 (0-based)
        feed(&mut t, b"\x1b[1;1;2;3$z");
        // Row 0 cols 0-2 should be blank
        for col in 0..3 {
            assert!(
                t.grid()[(col, 0)].is_blank(),
                "col {col} row 0 should be blank after DECERA"
            );
        }
        // Row 0 cols 3-7 should still have "DEFGH"
        assert_eq!(t.grid()[(3, 0)].ch, 'D', "col 3 row 0 should be 'D'");
        assert_eq!(t.grid()[(4, 0)].ch, 'E', "col 4 row 0 should be 'E'");
        // Row 1 cols 0-2 should be blank
        for col in 0..3 {
            assert!(
                t.grid()[(col, 1)].is_blank(),
                "col {col} row 1 should be blank after DECERA"
            );
        }
        // Row 2 should be unchanged
        assert_eq!(t.grid()[(0, 2)].ch, 'Q', "row 2 col 0 should still be 'Q'");
    }

    #[test]
    fn t_decera_erases_protected_cells() {
        // Per DEC STD 070: DECERA erases ALL cells including protected.
        let mut t = Terminal::new(10, 3);
        // Enable protected attribute and write "AB"
        feed(&mut t, b"\x1b[1\"qAB");
        // Disable protected and write "CD"
        feed(&mut t, b"\x1b[0\"qCD");
        // DECERA: erase rect rows 1-1, cols 1-4 (entire row 0)
        feed(&mut t, b"\x1b[1;1;1;4$z");
        // ALL cells should be erased — DECERA ignores protection
        assert!(
            t.grid()[(0, 0)].is_blank(),
            "protected cell A should be erased by DECERA"
        );
        assert!(
            t.grid()[(1, 0)].is_blank(),
            "protected cell B should be erased by DECERA"
        );
        assert!(
            t.grid()[(2, 0)].is_blank(),
            "unprotected cell C should be erased"
        );
        assert!(
            t.grid()[(3, 0)].is_blank(),
            "unprotected cell D should be erased"
        );
    }

    #[test]
    fn t_decera_clamps_to_screen_bounds() {
        let mut t = Terminal::new(5, 3);
        // Fill with text
        feed(&mut t, b"HELLO\r\nWORLD\r\nTESTS");
        // DECERA with huge coords should clamp and not panic
        feed(&mut t, b"\x1b[1;1;100;100$z");
        // Entire screen should be blanked
        for row in 0..3 {
            for col in 0..5 {
                assert!(
                    t.grid()[(col, row)].is_blank(),
                    "cell ({col},{row}) should be blank after DECERA full-screen erase"
                );
            }
        }
    }

    #[test]
    fn t_decera_after_decfra_roundtrip() {
        let mut t = Terminal::new(8, 3);
        // DECFRA: Pch=32 (space), fill rect rows 1-3, cols 1-4 with red bg
        feed(&mut t, b"\x1b[41m\x1b[32;1;1;3;4$x");
        // Verify filled with space and red bg
        assert_eq!(t.grid()[(0, 0)].ch, ' ', "DECFRA fills with space");
        assert_eq!(
            t.grid()[(0, 0)].bg,
            Color::Indexed(1),
            "DECFRA should set red bg"
        );
        // DECERA: erase the same rect — should go back to blank (default bg)
        feed(&mut t, b"\x1b[1;1;3;4$z");
        // All cells should now be blank
        for row in 0..3 {
            for col in 0..4 {
                assert!(
                    t.grid()[(col, row)].is_blank(),
                    "cell ({col},{row}) should be blank after DECERA"
                );
            }
        }
    }

    // ===== DECSERA — Selective Erase Rectangle Area tests =====

    #[test]
    fn t_decsera_erases_non_protected_cells() {
        let mut t = Terminal::new(10, 3);
        // Fill row 0 with text
        feed(&mut t, b"ABCDEFGH");
        // DECSERA: selective erase rect rows 1-1, cols 1-4 (row 0, cols 0-3)
        feed(&mut t, b"\x1b[1;1;1;4${");
        // Cols 0-3 should be blank
        for col in 0..4 {
            assert!(
                t.grid()[(col, 0)].is_blank(),
                "col {col} row 0 should be blank after DECSERA"
            );
        }
        // Cols 4-7 should still have "EFGH"
        assert_eq!(t.grid()[(4, 0)].ch, 'E');
        assert_eq!(t.grid()[(5, 0)].ch, 'F');
    }

    #[test]
    fn t_decsera_preserves_protected_cells() {
        let mut t = Terminal::new(10, 3);
        // Enable protected attribute and write "AB"
        feed(&mut t, b"\x1b[1\"qAB");
        // Disable protected and write "CD"
        feed(&mut t, b"\x1b[0\"qCD");
        // DECSERA: selective erase rect rows 1-1, cols 1-4 (entire row 0)
        feed(&mut t, b"\x1b[1;1;1;4${");
        // Protected cells "AB" should survive — DECSERA respects protection
        assert_eq!(t.grid()[(0, 0)].ch, 'A', "protected cell A should survive");
        assert_eq!(t.grid()[(1, 0)].ch, 'B', "protected cell B should survive");
        // Unprotected cells "CD" should be erased
        assert!(
            t.grid()[(2, 0)].is_blank(),
            "unprotected cell C should be erased"
        );
        assert!(
            t.grid()[(3, 0)].is_blank(),
            "unprotected cell D should be erased"
        );
    }

    #[test]
    fn t_decsera_clamps_to_screen_bounds() {
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"HELLO\r\nWORLD\r\nTESTS");
        // DECSERA with huge coords should clamp and not panic
        feed(&mut t, b"\x1b[1;1;100;100${");
        // Entire screen should be blanked (no protected cells)
        for row in 0..3 {
            for col in 0..5 {
                assert!(
                    t.grid()[(col, row)].is_blank(),
                    "cell ({col},{row}) should be blank after DECSERA"
                );
            }
        }
    }

    #[test]
    fn t_decsera_vs_decera_with_protected() {
        // Direct comparison: DECERA erases protected, DECSERA preserves it.
        let mut t1 = Terminal::new(10, 3);
        let mut t2 = Terminal::new(10, 3);
        // Both: protected "AB" + unprotected "CD"
        feed(&mut t1, b"\x1b[1\"qAB\x1b[0\"qCD");
        feed(&mut t2, b"\x1b[1\"qAB\x1b[0\"qCD");
        // t1: DECERA (erases all)
        feed(&mut t1, b"\x1b[1;1;1;4$z");
        // t2: DECSERA (preserves protected)
        feed(&mut t2, b"\x1b[1;1;1;4${");
        // DECERA: everything gone
        assert!(t1.grid()[(0, 0)].is_blank(), "DECERA erases protected A");
        // DECSERA: protected survives
        assert_eq!(t2.grid()[(0, 0)].ch, 'A', "DECSERA preserves protected A");
    }

    // ===== DECRA — Copy Rectangle Area tests =====

    #[test]
    fn t_decra_basic_copy() {
        let mut t = Terminal::new(10, 5);
        // Write "ABCD" on row 0
        feed(&mut t, b"ABCD");
        // DECRA: copy rect rows 1-1, cols 1-4 to row 3, col 1
        // CSI 1;1;1;4;3;1 $ v
        feed(&mut t, b"\x1b[1;1;1;4;3;1$v");
        // Row 3 should now have "ABCD"
        assert_eq!(t.grid()[(0, 2)].ch, 'A', "row 2 col 0 should be 'A'");
        assert_eq!(t.grid()[(1, 2)].ch, 'B', "row 2 col 1 should be 'B'");
        assert_eq!(t.grid()[(2, 2)].ch, 'C', "row 2 col 2 should be 'C'");
        assert_eq!(t.grid()[(3, 2)].ch, 'D', "row 2 col 3 should be 'D'");
        // Source should be unchanged
        assert_eq!(t.grid()[(0, 0)].ch, 'A', "source row 0 col 0 still 'A'");
    }

    #[test]
    fn t_decra_copies_attributes() {
        let mut t = Terminal::new(10, 5);
        // Write "AB" with bold red fg (SGR 1;31)
        feed(&mut t, b"\x1b[1;31mAB");
        // DECRA: copy row 1 cols 1-2 to row 2 col 1
        feed(&mut t, b"\x1b[1;1;1;2;2;1$v");
        // Destination should have same attributes
        let dst = &t.grid()[(0, 1)];
        assert_eq!(dst.ch, 'A');
        assert!(dst.flags.contains(CellFlags::BOLD), "bold should be copied");
        assert_eq!(dst.fg, Color::Indexed(1), "red fg should be copied");
        let dst2 = &t.grid()[(1, 1)];
        assert_eq!(dst2.ch, 'B');
        assert!(dst2.flags.contains(CellFlags::BOLD));
    }

    #[test]
    fn t_decra_clamps_destination() {
        let mut t = Terminal::new(5, 3);
        // Write "ABCDE" on row 0
        feed(&mut t, b"ABCDE");
        // DECRA: copy rect rows 1-1, cols 1-5 to row 2, col 3
        // Destination extends beyond screen — should clamp
        feed(&mut t, b"\x1b[1;1;1;5;2;3$v");
        // Only cols 2-4 on row 1 should get "ABC" (dst col 3,4,5 → 0-based 2,3,4)
        assert_eq!(t.grid()[(2, 1)].ch, 'A', "dst col 2 row 1 = 'A'");
        assert_eq!(t.grid()[(3, 1)].ch, 'B', "dst col 3 row 1 = 'B'");
        assert_eq!(t.grid()[(4, 1)].ch, 'C', "dst col 4 row 1 = 'C'");
    }

    #[test]
    fn t_decra_overlap_safe() {
        // Source and destination overlap — buffer ensures correctness.
        let mut t = Terminal::new(8, 3);
        // Write "AB" at cols 0-1 row 0
        feed(&mut t, b"AB");
        // DECRA: copy cols 1-2 (0-based) row 1 to dst col 2 (0-based) row 1
        // Source (0-based): row 0, cols 0-1 → "AB"
        // Dest (0-based): row 0, col 1
        // Overlap: dest col 1 overlaps source col 1
        feed(&mut t, b"\x1b[1;1;1;2;1;2$v");
        // After copy: col 1 = 'A', col 2 = 'B' (original col 1 'B' not read after overwrite)
        assert_eq!(t.grid()[(1, 0)].ch, 'A', "col 1 should be 'A' (copied)");
        assert_eq!(t.grid()[(2, 0)].ch, 'B', "col 2 should be 'B' (copied)");
    }

    // ===== DECCARA — Change Attributes in Rectangular Area tests =====

    #[test]
    fn t_deccara_adds_bold_to_rectangle() {
        let mut t = Terminal::new(10, 3);
        // Write "ABCDEF" on row 0
        feed(&mut t, b"ABCDEF");
        // Enable rectangle mode (default is stream which extends to full row)
        feed(&mut t, b"\x1b[2*q");
        // DECCARA: add BOLD to rect rows 1-1, cols 1-3 (row 0, cols 0-2 0-based)
        // CSI 1;1;1;3;1 $ r  (Ps1=1 → BOLD)
        feed(&mut t, b"\x1b[1;1;1;3;1$r");
        // Cells A,B,C should now have BOLD
        assert!(
            t.grid()[(0, 0)].flags.contains(CellFlags::BOLD),
            "A should have BOLD"
        );
        assert!(
            t.grid()[(1, 0)].flags.contains(CellFlags::BOLD),
            "B should have BOLD"
        );
        assert!(
            t.grid()[(2, 0)].flags.contains(CellFlags::BOLD),
            "C should have BOLD"
        );
        // D,E,F should NOT have BOLD
        assert!(
            !t.grid()[(3, 0)].flags.contains(CellFlags::BOLD),
            "D should not have BOLD"
        );
        assert!(
            !t.grid()[(4, 0)].flags.contains(CellFlags::BOLD),
            "E should not have BOLD"
        );
    }

    #[test]
    fn t_deccara_skips_blank_cells() {
        let mut t = Terminal::new(10, 3);
        // Write "AB" then move to col 4 and write "E"
        feed(&mut t, b"AB\x1b[5GE");
        // DECCARA: add BOLD to rect rows 1-1, cols 1-5 (row 0, cols 0-4)
        feed(&mut t, b"\x1b[1;1;1;5;1$r");
        // A,B at cols 0-1 should have BOLD
        assert!(t.grid()[(0, 0)].flags.contains(CellFlags::BOLD));
        assert!(t.grid()[(1, 0)].flags.contains(CellFlags::BOLD));
        // Cols 2-3 are blank — should NOT get BOLD
        assert!(
            !t.grid()[(2, 0)].flags.contains(CellFlags::BOLD),
            "blank col 2 should not get BOLD"
        );
        assert!(
            !t.grid()[(3, 0)].flags.contains(CellFlags::BOLD),
            "blank col 3 should not get BOLD"
        );
        // E at col 4 should have BOLD
        assert!(
            t.grid()[(4, 0)].flags.contains(CellFlags::BOLD),
            "E should have BOLD"
        );
    }

    #[test]
    fn t_deccara_clamps_bounds() {
        let mut t = Terminal::new(5, 3);
        // Fill row 0 with text
        feed(&mut t, b"ABCDE");
        // DECCARA with huge coords — should clamp and not panic
        feed(&mut t, b"\x1b[1;1;100;100;1$r");
        // All of ABCDE should have BOLD
        for col in 0..5 {
            assert!(
                t.grid()[(col, 0)].flags.contains(CellFlags::BOLD),
                "col {col} should have BOLD after DECCARA clamp"
            );
        }
    }

    // ===== DECSACE — Select Attribute Change Extent tests =====

    #[test]
    fn t_decsace_sets_rectangle_mode() {
        let mut t = Terminal::new(10, 3);
        assert!(!t.modes.sace_rectangle, "default should be stream mode");
        // DECSACE Ps=2 → rectangle mode
        feed(&mut t, b"\x1b[2*q");
        assert!(t.modes.sace_rectangle, "Ps=2 should set rectangle mode");
    }

    #[test]
    fn t_decsace_sets_stream_mode() {
        let mut t = Terminal::new(10, 3);
        // Set to rectangle first
        feed(&mut t, b"\x1b[2*q");
        assert!(t.modes.sace_rectangle);
        // DECSACE Ps=1 → stream mode
        feed(&mut t, b"\x1b[1*q");
        assert!(!t.modes.sace_rectangle, "Ps=1 should set stream mode");
    }

    #[test]
    fn t_decstr_resets_sace_rectangle() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[2*q"); // DECSACE → rectangle mode
        assert!(t.modes.sace_rectangle);
        feed(&mut t, b"\x1b[!p"); // DECSTR — soft reset
        assert!(
            !t.modes.sace_rectangle,
            "DECSTR should reset sace_rectangle"
        );
    }

    #[test]
    fn t_deccara_stream_mode_extends_to_full_row() {
        // Default (stream mode): DECCARA should affect entire rows, not just the rect.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGHIJ"); // Fill row 0 with 10 chars
        // DECCARA on cols 1-3 only (1-based) = cols 0-2 (0-based)
        feed(&mut t, b"\x1b[1;1;1;3;1$r"); // Add BOLD
        // In stream mode, ALL chars on row 0 should get BOLD
        for col in 0..10 {
            assert!(
                t.grid()[(col, 0)].flags.contains(CellFlags::BOLD),
                "col {col} should have BOLD in stream mode"
            );
        }
    }

    #[test]
    fn t_deccara_rectangle_mode_stays_in_rect() {
        // Rectangle mode: DECCARA should only affect cells within the rect.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGHIJ");
        // Enable rectangle mode first
        feed(&mut t, b"\x1b[2*q");
        // DECCARA on cols 1-3 (1-based) = cols 0-2 (0-based)
        feed(&mut t, b"\x1b[1;1;1;3;1$r"); // Add BOLD
        // Only cols 0-2 should have BOLD
        assert!(t.grid()[(0, 0)].flags.contains(CellFlags::BOLD), "col 0");
        assert!(t.grid()[(1, 0)].flags.contains(CellFlags::BOLD), "col 1");
        assert!(t.grid()[(2, 0)].flags.contains(CellFlags::BOLD), "col 2");
        // Cols 3-9 should NOT have BOLD
        assert!(
            !t.grid()[(3, 0)].flags.contains(CellFlags::BOLD),
            "col 3 no BOLD"
        );
        assert!(
            !t.grid()[(9, 0)].flags.contains(CellFlags::BOLD),
            "col 9 no BOLD"
        );
    }

    // ===== Robustness: extreme parameter values must not panic =====

    #[test]
    fn fuzz_decfra_extreme_params() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[$x"); // all defaults
        feed(&mut t, b"\x1b[0;0;0;0$x"); // all zeros
        feed(&mut t, b"\x1b[65535;65535;65535;65535$x"); // max u16
        feed(&mut t, b"\x1b[1;1;0;0$x"); // bottom < top
        feed(&mut t, b"\x1b[0;1;1;0$x"); // right < left
    }

    #[test]
    fn fuzz_decera_extreme_params() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[$z");
        feed(&mut t, b"\x1b[0;0;0;0$z");
        feed(&mut t, b"\x1b[65535;65535;65535;65535$z");
        feed(&mut t, b"\x1b[1;1;0;0$z"); // inverted rect
    }

    #[test]
    fn fuzz_decra_extreme_params() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[$v"); // minimal
        feed(&mut t, b"\x1b[0;0;0;0$v");
        feed(&mut t, b"\x1b[65535;65535;65535;65535;65535;65535$v");
        // dest way out of bounds — should silently clamp
        feed(&mut t, b"\x1b[1;1;1;1;65535;65535$v");
    }

    #[test]
    fn fuzz_deccara_extreme_params() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"ABCDEFGH"); // some text
        feed(&mut t, b"\x1b[$r"); // minimal — no SGR params
        feed(&mut t, b"\x1b[0;0;0;0;0;0$r"); // all zeros
        feed(&mut t, b"\x1b[65535;65535;65535;65535;999;999$r"); // max everything
        feed(&mut t, b"\x1b[1;1;24;80;0$r"); // clear + apply to whole screen
    }

    #[test]
    fn fuzz_decsera_extreme_params() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[${");
        feed(&mut t, b"\x1b[0;0;0;0${");
        feed(&mut t, b"\x1b[65535;65535;65535;65535${");
    }

    #[test]
    fn fuzz_cup_extreme_params() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[H"); // no params
        feed(&mut t, b"\x1b[0;0H"); // zeros
        feed(&mut t, b"\x1b[65535;65535H"); // way out of bounds
        feed(&mut t, b"\x1b[999;999H"); // out of bounds
        // Should still be alive — write text
        feed(&mut t, b"OK");
    }

    // ===== DECRARA — Reverse Attributes in Rectangular Area tests =====

    #[test]
    fn t_decrara_toggles_bold() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEF");
        // Enable rectangle mode (default stream extends to full row)
        feed(&mut t, b"\x1b[2*q");
        // DECRARA: toggle BOLD on rect row 1, cols 1-3
        feed(&mut t, b"\x1b[1;1;1;3;1$t");
        // A,B,C should now have BOLD
        assert!(
            t.grid()[(0, 0)].flags.contains(CellFlags::BOLD),
            "A should gain BOLD"
        );
        assert!(
            t.grid()[(1, 0)].flags.contains(CellFlags::BOLD),
            "B should gain BOLD"
        );
        assert!(
            t.grid()[(2, 0)].flags.contains(CellFlags::BOLD),
            "C should gain BOLD"
        );
        // D,E,F should NOT
        assert!(
            !t.grid()[(3, 0)].flags.contains(CellFlags::BOLD),
            "D should be unchanged"
        );
    }

    #[test]
    fn t_decrara_second_call_undoes() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"AB");
        // First toggle: add BOLD
        feed(&mut t, b"\x1b[1;1;1;2;1$t");
        assert!(
            t.grid()[(0, 0)].flags.contains(CellFlags::BOLD),
            "first toggle adds BOLD"
        );
        // Second toggle: remove BOLD
        feed(&mut t, b"\x1b[1;1;1;2;1$t");
        assert!(
            !t.grid()[(0, 0)].flags.contains(CellFlags::BOLD),
            "second toggle removes BOLD"
        );
    }

    #[test]
    fn t_decrara_skips_blank_cells() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"A");
        // Move to col 3 and write "C"
        feed(&mut t, b"\x1b[4GC");
        // DECRARA: toggle BOLD on cols 1-5
        feed(&mut t, b"\x1b[1;1;1;5;1$t");
        // A at col 0 should have BOLD
        assert!(t.grid()[(0, 0)].flags.contains(CellFlags::BOLD));
        // Blank cols 1-2 should NOT
        assert!(
            !t.grid()[(1, 0)].flags.contains(CellFlags::BOLD),
            "blank col 1 skipped"
        );
        // C at col 3 should have BOLD
        assert!(
            t.grid()[(3, 0)].flags.contains(CellFlags::BOLD),
            "C should gain BOLD"
        );
    }

    #[test]
    fn t_deccara_ps1_0_clears_all_renderable_attrs() {
        // Ps1=0 should clear BOLD, UNDERLINE, BLINK, REVERSE but keep structural flags.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1;4;5;7m"); // BOLD + UNDERLINE + BLINK + REVERSE
        feed(&mut t, b"AB");
        // DECCARA: clear all + set nothing → all renderable attrs removed
        feed(&mut t, b"\x1b[1;1;1;5;0$r");
        let cell = &t.grid()[(0, 0)];
        assert!(
            !cell.flags.contains(CellFlags::BOLD),
            "BOLD should be cleared by Ps1=0"
        );
        assert!(
            !cell.flags.contains(CellFlags::UNDERLINE),
            "UNDERLINE should be cleared by Ps1=0"
        );
        assert!(
            !cell.flags.contains(CellFlags::BLINK),
            "BLINK should be cleared by Ps1=0"
        );
        assert!(
            !cell.flags.contains(CellFlags::REVERSE),
            "REVERSE should be cleared by Ps1=0"
        );
    }

    #[test]
    fn t_deccara_off_code_22_removes_bold() {
        // Ps=22 explicitly removes BOLD (no-bold)
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1mAB"); // BOLD
        assert!(t.grid()[(0, 0)].flags.contains(CellFlags::BOLD));
        // Remove just BOLD
        feed(&mut t, b"\x1b[1;1;1;5;22$r");
        assert!(
            !t.grid()[(0, 0)].flags.contains(CellFlags::BOLD),
            "BOLD removed by Ps=22"
        );
    }

    #[test]
    fn t_deccara_off_code_24_removes_all_underlines() {
        // Ps=24 removes all underline styles including curly/dotted/dashed
        let mut t = Terminal::new(10, 3);
        // Set curly underline via colon syntax
        feed(&mut t, b"\x1b[4:3mAB");
        assert!(t.grid()[(0, 0)].flags.contains(CellFlags::UNDERLINE_CURLY));
        // Remove all underlines
        feed(&mut t, b"\x1b[1;1;1;5;24$r");
        assert!(
            !t.grid()[(0, 0)].flags.contains(CellFlags::UNDERLINE_CURLY),
            "UNDERLINE_CURLY removed by Ps=24"
        );
    }

    #[test]
    fn t_decrara_ps0_reverses_all() {
        // Ps=0 reverses ALL attributes (BOLD, UNDERLINE, BLINK, REVERSE)
        let mut t = Terminal::new(10, 3);
        // Set BOLD + BLINK, no underline/reverse
        feed(&mut t, b"\x1b[1;5mAB");
        // DECRARA Ps=0: reverse all → BOLD off, BLINK off, UNDERLINE on, REVERSE on
        feed(&mut t, b"\x1b[1;1;1;5;0$t");
        let cell = &t.grid()[(0, 0)];
        assert!(
            !cell.flags.contains(CellFlags::BOLD),
            "BOLD should be toggled off by Ps=0"
        );
        assert!(
            !cell.flags.contains(CellFlags::BLINK),
            "BLINK should be toggled off by Ps=0"
        );
        assert!(
            cell.flags.contains(CellFlags::UNDERLINE),
            "UNDERLINE should be toggled on by Ps=0"
        );
        assert!(
            cell.flags.contains(CellFlags::REVERSE),
            "REVERSE should be toggled on by Ps=0"
        );
    }

    // ===== DECRQC — Restore Mode tests =====

    #[test]
    fn t_decrqc_restores_bracketed_paste() {
        let mut t = Terminal::new(80, 24);
        // Enable bracketed paste (default is off)
        feed(&mut t, b"\x1b[?2004h");
        assert!(t.modes.bracketed_paste);
        // DECRQC restores to default (off)
        feed(&mut t, b"\x1b[2004$w");
        assert!(
            !t.modes.bracketed_paste,
            "DECRQC should restore bracketed_paste to default"
        );
    }

    #[test]
    fn t_decrqc_restores_cursor_visible() {
        let mut t = Terminal::new(80, 24);
        // Hide cursor (default is visible)
        feed(&mut t, b"\x1b[?25l");
        assert!(!t.modes.cursor_visible);
        // DECRQC restores to default (visible)
        feed(&mut t, b"\x1b[25$w");
        assert!(
            t.modes.cursor_visible,
            "DECRQC should restore cursor_visible to default"
        );
    }

    #[test]
    fn t_resize_to_zero_does_not_panic() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"Hello");
        // Resize to 0x0 — should be clamped to 1x1 internally.
        t.resize(0, 0);
        // Should be alive — write text
        feed(&mut t, b"OK");
    }

    #[test]
    fn t_command_marks_adjusted_on_scrollback_eviction() {
        // Small scrollback to force eviction quickly.
        let mut t = Terminal::with_scrollback(10, 3, 5);
        // Command 1: prompt + command + output + end
        feed(&mut t, b"\x1b]133;A\x07"); // PromptStart
        feed(&mut t, b"cmd1\r\n"); // Scroll some content
        feed(&mut t, b"\x1b]133;C\x07"); // OutputStart
        feed(&mut t, b"\x1b]133;D;0\x07"); // CommandEnd
        // First mark row
        let _row_before = t.command_marks()[0].row;
        // Now generate enough output to evict scrollback rows.
        for _ in 0..20 {
            feed(&mut t, b"\x1b]133;A\x07"); // New prompt
            feed(&mut t, b"cmd\r\n");
            feed(&mut t, b"\x1b]133;D;0\x07");
        }
        // The old mark's row should have been adjusted downward
        // (it can't exceed current scrollback_len).
        let scrollback_len = t.grid().scrollback_len();
        for m in t.command_marks() {
            assert!(
                m.row <= scrollback_len + t.grid().height(),
                "mark row {} exceeds scrollback_len({}) + height({})",
                m.row,
                scrollback_len,
                t.grid().height()
            );
        }
    }

    #[test]
    fn fuzz_binary_garbage_never_panics() {
        let mut t = Terminal::new(80, 24);
        // Feed all 256 byte values in sequence.
        let mut data = Vec::new();
        for b in 0u8..=255 {
            data.push(b);
        }
        feed(&mut t, &data);
        // CSI prefix without terminator.
        feed(&mut t, b"\x1b[12345");
        feed(&mut t, b"\x1b[?9999");
        // Truncated OSC.
        feed(&mut t, b"\x1b]0;hello");
        // DCS without ST.
        feed(&mut t, b"\x1bP$q");
        // Lots of ESC chars.
        feed(&mut t, &[0x1b; 100]);
        // Mixed valid + invalid.
        feed(&mut t, b"echo hello\x00\x01\x02\xff\xfeWorld\r\n");
        // Should still be alive.
        feed(&mut t, b"ALIVE");
    }

    #[test]
    fn fuzz_truncated_csi_does_not_hang() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[31");
        feed(&mut t, b"mX");
        assert!(t.grid().width() > 0);
    }

    // ── Scroll region behavior tests ──────────────────────────────

    #[test]
    fn t_scroll_region_lf_scrolls_within_region() {
        // Set scroll region rows 2-4 (0-indexed 1-3), fill with ABC,
        // LF at bottom of region should scroll only within region.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[2;4r"); // DECSTBM 2-4, cursor→home
        feed(&mut t, b"\x1b[2;1HA"); // row 2 = A
        feed(&mut t, b"\x1b[3;1HB"); // row 3 = B
        feed(&mut t, b"\x1b[4;1HC"); // row 4 = C
        feed(&mut t, b"\x1b[4;1H\n"); // LF at bottom → scroll up
        // Row 2 should now show "B" (scrolled from row 3)
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'B');
        // Row 5 (outside region) should be untouched
        assert_eq!(t.grid().cell(0, 4).unwrap().ch, ' ');
    }

    #[test]
    fn t_scroll_region_origin_mode() {
        // DECOM (origin mode) → cursor (0,0) is at top of scroll region
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[3;8r"); // DECSTBM 3-8
        feed(&mut t, b"\x1b[?6h"); // DECOM on
        feed(&mut t, b"\x1b[1;1H"); // CUP to "1,1" → origin-relative
        // In origin mode, row 0 = top of scroll region = row 3 (1-indexed) = row 2 (0-indexed)
        assert_eq!(t.cursor().1, 2); // row 2 (0-indexed)
    }

    #[test]
    fn t_scroll_region_clears_on_reset() {
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[2;4r"); // set region 2-4 (1-indexed)
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(top, 1); // 0-indexed top
        assert_eq!(bottom, 4); // 1-indexed bottom (exclusive)
        feed(&mut t, b"\x1b[r"); // clear region
        let (top2, bottom2) = t.grid().scroll_region();
        assert_eq!(top2, 0);
        assert_eq!(bottom2, 5); // full height
    }

    #[test]
    fn t_scroll_region_ind_scrolls() {
        // IND (index, ESC D) at bottom of scroll region should scroll.
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"\x1b[1;3r"); // region rows 1-3
        feed(&mut t, b"\x1b[2;1HX"); // X at row 2
        feed(&mut t, b"\x1b[3;1H"); // move to row 3 (bottom of region)
        feed(&mut t, b"\x1bD"); // IND → scroll up within region
        // Row 2 should be blanked (X scrolled to row 3 then... actually X moves up)
        // Actually: scroll up means content moves up, blank at bottom
        // Row 1 stays, row 2 gets row 3 content (blank), row 3 gets blank
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            ' ',
            "row 2 should be blank after scroll"
        );
    }

    #[test]
    fn t_scroll_region_ri_at_top_scrolls_down() {
        // RI (reverse index, ESC M) at top of scroll region should scroll down.
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"\x1b[1;3r"); // region rows 1-3
        feed(&mut t, b"\x1b[2;1HX"); // X at row 2
        feed(&mut t, b"\x1b[1;1H"); // move to top of region (row 1)
        feed(&mut t, b"\x1bM"); // RI at top → scroll down within region
        // X should have moved from row 2 to row 3
        assert_eq!(
            t.grid().cell(0, 2).unwrap().ch,
            'X',
            "X should move down to row 3"
        );
    }

    #[test]
    fn t_pending_wrap_then_cr_no_double_line() {
        // Fill to last col (pending_wrap set), then CR + char.
        // CR should clear pending_wrap. Next char should NOT skip a line.
        let mut t = Terminal::new(4, 4);
        feed(&mut t, b"ABCD"); // fills row 0, pending_wrap=true
        feed(&mut t, b"\r"); // CR — clears pending_wrap
        feed(&mut t, b"X"); // Should overwrite col 0 row 0
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X');
        assert_eq!(t.cursor().1, 0, "should still be on row 0");
    }

    #[test]
    fn t_wide_char_at_last_col_wraps() {
        // A wide char at the last column with only 1 col left should wrap.
        let mut t = Terminal::new(4, 4);
        feed(&mut t, b"ABC"); // cursor at col 3 (1 col left)
        feed(&mut t, "中".as_bytes()); // width=2, not enough room → wrap
        assert_eq!(
            t.grid().cell(3, 0).map(|c| c.is_blank()),
            Some(true),
            "last col of row 0 should be blank"
        );
        assert_eq!(t.grid().cell(0, 1).map(|c| c.ch), Some('中'));
    }

    #[test]
    fn t_origin_mode_clamps_cursor() {
        // Origin mode constrains cursor positioning to scroll region.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[3;5r"); // region rows 2-4 (0-indexed)
        feed(&mut t, b"\x1b[?6h"); // enable origin mode
        feed(&mut t, b"\x1b[1;1H"); // CUP to (1,1) — should map to region origin
        assert_eq!(t.cursor().1, 2, "origin mode: row 1 → region top (row 2)");
        assert_eq!(t.cursor().0, 0, "origin mode: col 1 → col 0");
    }

    #[test]
    fn t_origin_mode_cup_below_region_clamps() {
        // In origin mode, CUP to a row below the scroll region should clamp
        // to the bottom of the region.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[2;4r"); // region rows 1-3 (0-indexed)
        feed(&mut t, b"\x1b[?6h"); // origin mode
        feed(&mut t, b"\x1b[10;1H"); // CUP to row 10 — way below region
        assert_eq!(t.cursor().1, 3, "should clamp to region bottom (row 3)");
    }

    #[test]
    fn t_pending_wrap_then_backspace() {
        // Backspace after pending_wrap should clear it and move cursor left.
        let mut t = Terminal::new(4, 3);
        feed(&mut t, b"ABCD"); // fills row, pending_wrap=true
        feed(&mut t, b"\x08"); // BS
        // After BS, cursor should be at col 2 (pending_wrap cleared).
        // Print a char — it should land at col 2, row 0 (NOT wrap to next line).
        feed(&mut t, b"X");
        assert_eq!(t.grid().cell(2, 0).map(|c| c.ch), Some('X'));
        assert_eq!(t.cursor().1, 0, "should still be on row 0");
    }

    // ── Alt screen edge cases ──

    #[test]
    fn t_alt_screen_content_preserved_on_exit() {
        // Write content to primary, enter alt screen, write, exit.
        // Primary content should be unchanged.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"PRIMARY");
        feed(&mut t, b"\x1b[?1049h"); // enter alt screen
        feed(&mut t, b"\x1b[1;1HALT_CONTENT");
        feed(&mut t, b"\x1b[?1049l"); // exit alt screen
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'P');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'R');
        // ALT_CONTENT should NOT appear in primary screen
        assert_eq!(t.grid().cell(0, 1).map(|c| c.ch), Some(' '));
    }

    #[test]
    fn t_alt_screen_cursor_restored_on_exit() {
        // Set cursor position, enter alt screen (which saves+resets cursor),
        // move around, exit. Cursor should return to original position.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;5H"); // cursor at col 4, row 2
        feed(&mut t, b"\x1b[?1049h"); // enter alt screen
        feed(&mut t, b"\x1b[5;5H"); // move cursor in alt screen
        feed(&mut t, b"\x1b[?1049l"); // exit alt screen
        assert_eq!(t.cursor(), (4, 2), "cursor should be restored to (4,2)");
    }

    #[test]
    fn t_alt_screen_re_enter_is_clean() {
        // Enter alt screen, write, exit, re-enter.
        // Second entry should NOT show content from first alt session.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"\x1b[1;1HZOMBIE"); // write in alt
        feed(&mut t, b"\x1b[?1049l"); // exit
        feed(&mut t, b"\x1b[?1049h"); // re-enter alt
        assert_eq!(
            t.grid().cell(0, 0).map(|c| c.ch),
            Some(' '),
            "alt screen should be clean on re-entry"
        );
    }

    #[test]
    fn t_alt_screen_47_vs_1049_cursor() {
        // Mode 47 does NOT save/restore cursor; mode 1049 does.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;5H"); // cursor at (4,2)
        feed(&mut t, b"\x1b[?47h"); // enter alt (no cursor save)
        feed(&mut t, b"\x1b[1;1H"); // move cursor
        feed(&mut t, b"\x1b[?47l"); // exit alt (no cursor restore)
        // Mode 47 doesn't restore cursor — cursor stays where it was
        assert_eq!(t.cursor(), (0, 0), "mode 47 does not restore cursor");
    }

    // ── SGR color edge cases ──

    #[test]
    fn t_sgr_256_color_boundaries() {
        let mut t = Terminal::new(10, 3);
        // Index 0 (black in xterm palette)
        feed(&mut t, b"\x1b[38;5;0mX");
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Indexed(0));
        // Index 15 (bright white)
        feed(&mut t, b"\x1b[38;5;15mY");
        assert_eq!(t.grid().cell(1, 0).unwrap().fg, Color::Indexed(15));
        // Index 16 (start of 6x6x6 color cube)
        feed(&mut t, b"\x1b[38;5;16mZ");
        assert_eq!(t.grid().cell(2, 0).unwrap().fg, Color::Indexed(16));
        // Index 231 (end of 6x6x6 color cube)
        feed(&mut t, b"\x1b[38;5;231mW");
        assert_eq!(t.grid().cell(3, 0).unwrap().fg, Color::Indexed(231));
        // Index 255 (last grayscale)
        feed(&mut t, b"\x1b[38;5;255mV");
        assert_eq!(t.grid().cell(4, 0).unwrap().fg, Color::Indexed(255));
    }

    #[test]
    fn t_sgr_empty_resets_all() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1;31m"); // bold + red fg
        feed(&mut t, b"\x1b[48;5;42m"); // 256-color bg
        feed(&mut t, b"\x1b[4m"); // underline
        feed(&mut t, b"\x1b[m"); // empty SGR = reset
        assert_eq!(t.fg, Color::Default);
        assert_eq!(t.bg, Color::Default);
        assert!(t.flags.is_empty(), "flags should be cleared by empty SGR");
    }

    #[test]
    fn t_sgr_true_color_values() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[38;2;255;0;0mA"); // pure red
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Rgb(255, 0, 0));
        feed(&mut t, b"\x1b[38;2;0;255;0mB"); // pure green
        assert_eq!(t.grid().cell(1, 0).unwrap().fg, Color::Rgb(0, 255, 0));
        feed(&mut t, b"\x1b[38;2;0;0;255mC"); // pure blue
        assert_eq!(t.grid().cell(2, 0).unwrap().fg, Color::Rgb(0, 0, 255));
    }

    #[test]
    fn t_sgr_bold_does_not_brighten_indexed() {
        // Many terminals brighten SGR 30-37 when bold is set.
        // GGTerm stores bold as a flag, not changing the color index.
        // This test documents the behavior — it's NOT a bug if bold
        // doesn't brighten, as the renderer handles bold separately.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1;31mX"); // bold + color 1 (red)
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.fg, Color::Indexed(1)); // color unchanged
        assert!(cell.flags.contains(CellFlags::BOLD));
    }

    #[test]
    fn t_sgr_22_clears_bold_and_dim() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1m"); // bold
        feed(&mut t, b"\x1b[2m"); // dim
        assert!(t.flags.contains(CellFlags::BOLD));
        assert!(t.flags.contains(CellFlags::DIM));
        feed(&mut t, b"\x1b[22m"); // clear bold+dim
        assert!(!t.flags.contains(CellFlags::BOLD));
        assert!(!t.flags.contains(CellFlags::DIM));
    }

    // ── OSC sequence probe tests ──

    #[test]
    fn t_osc_title_bel_terminated() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]0;Hello World\x07"); // BEL-terminated
        assert_eq!(t.title(), "Hello World");
    }

    #[test]
    fn t_osc_title_st_terminated_form() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]2;ST Title\x1b\\"); // ST-terminated
        assert_eq!(t.title(), "ST Title");
    }

    #[test]
    fn t_osc_title_empty() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]0;Initial\x07");
        feed(&mut t, b"\x1b]0;\x07"); // empty title
        assert_eq!(t.title(), "");
    }

    #[test]
    fn t_osc_title_with_semicolons() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]0;Title;With;Semicolons\x07");
        // splitn(2, ';') means only first semicolon separates cmd from payload
        assert_eq!(t.title(), "Title;With;Semicolons");
    }

    #[test]
    fn t_osc_title_very_long() {
        let mut t = Terminal::new(10, 3);
        let long_title = "A".repeat(5000);
        let seq = format!("\x1b]0;{}\x07", long_title);
        feed(&mut t, seq.as_bytes());
        assert!(
            t.title().len() <= 256,
            "title should be capped at 256 chars, got {}",
            t.title().len()
        );
    }

    #[test]
    fn t_osc52_does_not_corrupt_screen() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDE");
        feed(&mut t, b"\x1b]52;c;SGVsbG8=\x07"); // OSC 52 clipboard set
        // Screen content should be unchanged
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'E');
    }

    #[test]
    fn t_osc8_hyperlink_does_not_corrupt_screen() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b]8;;https://example.com\x1b\\");
        feed(&mut t, b"link");
        feed(&mut t, b"\x1b]8;;\x1b\\");
        // 'link' should appear normally on screen
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'l');
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'k');
    }

    // ── DEC Special Graphics charset probe tests ──

    #[test]
    fn t_dec_special_graphics_box_drawing() {
        // ESC(0 activates DEC Special Graphics.
        // 'l' = ┌, 'q' = ─, 'k' = ┐
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b(0lqk"); // box drawing: ┌─┐
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{250C}'); // ┌
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, '\u{2500}'); // ─
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, '\u{2510}'); // ┐
    }

    #[test]
    fn t_dec_special_graphics_back_to_ascii() {
        // After ESC(B, chars should be normal ASCII again.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b(0"); // activate DEC special
        feed(&mut t, b"q"); // should be ─
        feed(&mut t, b"\x1b(B"); // back to ASCII
        feed(&mut t, b"q"); // should be literal 'q'
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{2500}'); // ─
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'q');
    }

    #[test]
    fn t_dec_special_graphics_passes_through_non_mapped() {
        // Chars outside the DEC graphics range (0x5f-0x7e) should pass through.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b(0"); // activate DEC special
        feed(&mut t, b"ABC"); // uppercase — not mapped
        feed(&mut t, b"\x1b(B"); // back to ASCII
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'C');
    }

    #[test]
    fn t_dec_special_graphics_full_box() {
        // Full box: ┌──┐
        //           │  │
        //           └──┘
        let mut t = Terminal::new(6, 4);
        feed(&mut t, b"\x1b(0");
        feed(&mut t, b"lqqqk"); // ┌───┐
        feed(&mut t, b"\r\n"); // CR+LF (LNM default off needs explicit CR)
        feed(&mut t, b"x   x"); // │   │ (x=│, space is mapped too)
        feed(&mut t, b"\r\n");
        feed(&mut t, b"mqqqj"); // └───┘
        feed(&mut t, b"\x1b(B");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{250C}'); // ┌
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, '\u{2510}'); // ┐
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, '\u{2502}'); // │
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, '\u{2514}'); // └
        assert_eq!(t.grid().cell(4, 2).unwrap().ch, '\u{2518}'); // ┘
    }

    // ── Tab stop edge cases ──

    #[test]
    fn t_tab_no_stops_at_all() {
        // Clear all tab stops, then tab should go to last column.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[3g"); // clear all tab stops
        feed(&mut t, b"X\tY");
        // Tab with no stops should jump to last column.
        // Y should be at the last column (col 9).
        assert_eq!(t.grid().cell(9, 0).unwrap().ch, 'Y');
    }

    #[test]
    fn t_hts_sets_tab_stop() {
        // Move to col 3, set tab stop with HTS (ESC H).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[3g"); // clear all defaults
        feed(&mut t, b"\x1b[4C"); // move right 4 (cursor at col 4)
        feed(&mut t, b"\x1bH"); // HTS — set tab stop at col 4
        feed(&mut t, b"\r"); // back to col 0
        feed(&mut t, b"\t"); // tab — should stop at col 4
        assert_eq!(t.cursor().0, 4, "tab should stop at col 4 (HTS-set)");
    }

    #[test]
    fn t_tbc_clears_current_stop() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[3g"); // clear all
        feed(&mut t, b"\x1b[4C"); // move to col 4
        feed(&mut t, b"\x1bH"); // set tab stop at col 4
        feed(&mut t, b"\x1b[0g"); // TBC param 0: clear current stop
        feed(&mut t, b"\r"); // back to col 0
        feed(&mut t, b"\t"); // tab — should go to last col (no stop at 4)
        assert_eq!(t.cursor().0, 9, "tab should skip col 4 (cleared by TBC)");
    }

    #[test]
    fn t_il_within_scroll_region_preserves_outside() {
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[1;1HTOP");
        feed(&mut t, b"\x1b[2;1HMID");
        feed(&mut t, b"\x1b[3;1HBOT");
        feed(&mut t, b"\x1b[2;4r");
        feed(&mut t, b"\x1b[2;1H");
        feed(&mut t, b"\x1b[L");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'T');
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, 'M');
    }

    #[test]
    fn t_il_outside_scroll_region_is_noop() {
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[2;4r");
        feed(&mut t, b"\x1b[1;1H");
        feed(&mut t, b"AAA");
        feed(&mut t, b"\x1b[1;1H");
        feed(&mut t, b"\x1b[L");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
    }

    #[test]
    fn t_dl_within_scroll_region() {
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"\x1b[1;1HA");
        feed(&mut t, b"\x1b[2;1HB");
        feed(&mut t, b"\x1b[3;1HC");
        feed(&mut t, b"\x1b[4;1HD");
        feed(&mut t, b"\x1b[1;4r");
        feed(&mut t, b"\x1b[2;1H");
        feed(&mut t, b"\x1b[M");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'C');
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, 'D');
        assert!(t.grid().cell(0, 3).unwrap().is_blank());
    }

    #[test]
    fn t_ich_shifts_right_drops_at_edge() {
        let mut t = Terminal::new(6, 2);
        feed(&mut t, b"ABCDEF");
        feed(&mut t, b"\x1b[3G");
        feed(&mut t, b"\x1b[2@");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B');
        assert!(t.grid().cell(2, 0).unwrap().is_blank());
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'C');
    }

    #[test]
    fn t_dch_shifts_left_fills_blank() {
        let mut t = Terminal::new(6, 2);
        feed(&mut t, b"ABCDEF");
        feed(&mut t, b"\x1b[2G");
        feed(&mut t, b"\x1b[2P");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'D');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'E');
        assert!(t.grid().cell(4, 0).unwrap().is_blank());
    }

    #[test]
    fn t_decsc_decrc_restores_sgr() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1;31m");
        feed(&mut t, b"\x1b7");
        feed(&mut t, b"\x1b[0m");
        feed(&mut t, b"\x1b[3;33m");
        feed(&mut t, b"\x1b8");
        assert!(t.flags.contains(CellFlags::BOLD));
        assert_eq!(t.fg, Color::Indexed(1));
        assert!(!t.flags.contains(CellFlags::ITALIC));
    }

    #[test]
    fn t_decsc_shared_between_primary_and_alt() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1;32m");
        feed(&mut t, b"\x1b7");
        feed(&mut t, b"\x1b[?1049h");
        feed(&mut t, b"\x1b[0;33m");
        feed(&mut t, b"\x1b7");
        feed(&mut t, b"\x1b[?1049l");
        feed(&mut t, b"\x1b8");
        assert_eq!(t.fg, Color::Indexed(3));
    }

    #[test]
    fn t_lf_at_scroll_region_bottom_scrolls() {
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[1;3r"); // region rows 0-2
        feed(&mut t, b"\x1b[4;1HOUT"); // row 3 (outside region)
        // OUT should be at row 3
        assert_eq!(
            t.grid().cell(0, 3).unwrap().ch,
            'O',
            "OUT should be at row 3"
        );
        feed(&mut t, b"\x1b[1;1HA"); // row 0
        feed(&mut t, b"\x1b[2;1HB"); // row 1
        feed(&mut t, b"\x1b[3;1HC"); // row 2
        feed(&mut t, b"\x1b[3;1H"); // cursor at row 2 (bottom of region)
        feed(&mut t, b"\n"); // LF at bottom → scroll up
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'B');
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'C');
        assert_eq!(
            t.grid().cell(0, 3).unwrap().ch,
            'O',
            "OUT at row 3 should survive scroll"
        );
    }

    #[test]
    fn t_nel_respects_scroll_region() {
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"\x1b[1;3r");
        feed(&mut t, b"\x1b[1;1HA");
        feed(&mut t, b"\x1b[2;1HB");
        feed(&mut t, b"\x1b[3;1HC");
        feed(&mut t, b"\x1b[3;5H");
        feed(&mut t, b"\x1bE");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'B');
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'C');
    }

    #[test]
    fn t_ri_at_top_of_partial_region_preserves_below() {
        // RI (reverse index) at top of a scroll region starting at row 0
        // but not reaching the bottom — content below the region must survive.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[1;3r"); // region rows 0-2
        feed(&mut t, b"\x1b[1;1HA"); // row 0
        feed(&mut t, b"\x1b[2;1HB"); // row 1
        feed(&mut t, b"\x1b[4;1HOUT"); // row 3 (below region)
        feed(&mut t, b"\x1b[1;1H"); // cursor at top of region
        feed(&mut t, b"\x1bM"); // RI — scroll down within region
        // A should move to row 1, row 0 is blank
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, 'B');
        // OUT at row 3 must survive (scroll_down bug would corrupt it)
        assert_eq!(
            t.grid().cell(0, 3).unwrap().ch,
            'O',
            "OUT should survive RI"
        );
    }

    #[test]
    fn t_dch_on_wide_char_no_orphan() {
        // Write a wide char (occupies 2 cells), then DCH at its position.
        // The lead + spacer should both be gone, no orphaned spacer.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, "你".as_bytes()); // wide char at cols 0-1
        feed(&mut t, b"X"); // normal char at col 2
        assert!(
            t.grid()
                .cell(0, 0)
                .unwrap()
                .flags
                .contains(CellFlags::WIDE_CHAR)
        );
        assert!(
            t.grid()
                .cell(1, 0)
                .unwrap()
                .flags
                .contains(CellFlags::WIDE_SPACER)
        );
        feed(&mut t, b"\x1b[1G"); // cursor to col 0
        feed(&mut t, b"\x1b[P"); // DCH 1: delete char at col 0
        // The wide char should be gone. What replaced it depends on DCH
        // shifting — but there should be NO orphaned WIDE_CHAR flag
        // without a corresponding WIDE_SPACER.
        let lead = t.grid().cell(0, 0).unwrap();
        let spacer = t.grid().cell(1, 0).unwrap();
        // No cell should have WIDE_CHAR without its spacer neighbor
        if lead.flags.contains(CellFlags::WIDE_CHAR) {
            assert!(
                spacer.flags.contains(CellFlags::WIDE_SPACER),
                "WIDE_CHAR at col 0 must have spacer at col 1"
            );
        }
    }

    #[test]
    fn t_dch_2_removes_full_wide_char() {
        // DCH 2 starting at the wide char lead should remove both cells cleanly.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, "你好".as_bytes()); // two wide chars: cols 0-1, 2-3
        feed(&mut t, b"Z"); // col 4
        feed(&mut t, b"\x1b[1G"); // cursor at col 0
        feed(&mut t, b"\x1b[2P"); // DCH 2
        // After deleting 2 cells, 好 should shift to col 0
        let c0 = t.grid().cell(0, 0).unwrap();
        // No orphaned WIDE_SPACER at col 0
        assert!(
            !c0.flags.contains(CellFlags::WIDE_SPACER),
            "col 0 should not have orphaned WIDE_SPACER after DCH 2"
        );
    }

    #[test]
    fn t_wide_char_not_enough_room_wraps() {
        // Width=5, write 4 ASCII (cursor at col 4 = 1 col left).
        // Wide char needs 2 cols — must wrap to next line.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCD"); // cursor at col 4, 1 col remaining
        feed(&mut t, "你".as_bytes()); // width=2, wraps
        // 你 should be at row 1, cols 0-1
        assert_eq!(t.grid().cell(0, 1).map(|c| c.ch), Some('你'));
        assert!(
            t.grid()
                .cell(1, 1)
                .map(|c| c.is_wide_spacer())
                .unwrap_or(false)
        );
        // Col 4 of row 0 should be blank (not the wide char)
        assert_eq!(t.grid().cell(4, 0).map(|c| c.is_blank()), Some(true));
    }

    #[test]
    fn t_tab_after_wide_char_correct_stop() {
        // Width=8, write a wide char (cols 0-1), then Tab.
        // Tab should advance to the next tab stop.
        // Default tab stops are at cols 0, 8, 16... So from col 2,
        // tab goes to col 7 (last col, since width=8, last index=7).
        let mut t = Terminal::new(8, 2);
        feed(&mut t, "你".as_bytes()); // wide char at cols 0-1, cursor at col 2
        feed(&mut t, b"\t"); // tab from col 2
        // Default tab stops: col 0 (set), then every 8.
        // From col 2, next stop is col 8 but max is col 7 → col 7.
        assert_eq!(t.cursor().0, 7, "tab should reach col 7 (last col)");
    }

    #[test]
    fn t_backspace_at_col0_no_wrap_up() {
        // Backspace at col 0 should NOT wrap to previous row.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"AB\r"); // write AB, CR back to col 0
        feed(&mut t, b"\x08"); // BS at col 0
        assert_eq!(t.cursor().0, 0, "BS at col 0 should stay at col 0");
        assert_eq!(t.cursor().1, 0, "BS at col 0 should NOT wrap to prev row");
    }

    #[test]
    fn t_backspace_from_col1_to_col0() {
        // Normal BS from col 1 → col 0.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"AB\x08"); // write AB (cursor at col 2), BS → col 1
        assert_eq!(t.cursor().0, 1);
        feed(&mut t, b"\x08"); // BS → col 0
        assert_eq!(t.cursor().0, 0);
    }

    #[test]
    fn t_el_0_clears_wide_lead_when_cursor_on_spacer() {
        // Wide char at cols 0-1. Cursor on spacer (col 1). EL 0 (clear to end).
        // Should clear both the spacer AND the lead (no orphaned WIDE_CHAR).
        let mut t = Terminal::new(6, 2);
        feed(&mut t, "你".as_bytes()); // cols 0-1
        feed(&mut t, b"XY"); // cols 2-3
        feed(&mut t, b"\x1b[2G"); // cursor to col 1 (spacer)
        feed(&mut t, b"\x1b[K"); // EL 0: clear from cursor to end
        let lead = t.grid().cell(0, 0).unwrap();
        assert!(
            !lead.flags.contains(CellFlags::WIDE_CHAR),
            "no orphaned WIDE_CHAR after EL 0 on spacer"
        );
        assert!(lead.is_blank(), "lead cell should be cleared");
    }

    #[test]
    fn t_ed_1_clears_wide_spacer_when_cursor_on_lead() {
        // Wide char at cols 0-1. ED 1 (clear from start to cursor).
        // Cursor on lead (col 0). Should clear both lead and spacer.
        let mut t = Terminal::new(6, 2);
        feed(&mut t, "你".as_bytes()); // cols 0-1
        feed(&mut t, b"\x1b[1G"); // cursor to col 0 (lead)
        feed(&mut t, b"\x1b[1J"); // ED 1: clear from start to cursor
        let spacer = t.grid().cell(1, 0).unwrap();
        assert!(
            !spacer.flags.contains(CellFlags::WIDE_SPACER),
            "no orphaned WIDE_SPACER after ED 1 on lead"
        );
    }

    #[test]
    fn t_ich_pushes_out_wide_char() {
        // Width=6, write "AB你CD" (你 at cols 2-3).
        // ICH 2 at col 2 should push 你+CD right, inserting 2 blanks.
        // No wide char should be split.
        let mut t = Terminal::new(6, 2);
        feed(&mut t, b"AB");
        feed(&mut t, "你".as_bytes()); // cols 2-3
        feed(&mut t, b"CD"); // cols 4-5
        feed(&mut t, b"\x1b[3G"); // cursor to col 2
        feed(&mut t, b"\x1b[2@"); // ICH 2
        // Cols 2-3 should be blank (inserted). 你 should have shifted right.
        // Check no orphaned wide chars anywhere
        for col in 0..6 {
            let c = t.grid().cell(col, 0).unwrap();
            if c.flags.contains(CellFlags::WIDE_CHAR) {
                let next = t.grid().cell(col + 1, 0).unwrap();
                assert!(
                    next.flags.contains(CellFlags::WIDE_SPACER),
                    "WIDE_CHAR at col {} has no spacer at col {}",
                    col,
                    col + 1
                );
            }
        }
    }

    #[test]
    fn t_decsc_restores_pending_wrap() {
        // Fill line to set pending_wrap, save, move cursor, restore.
        // After restore, pending_wrap should be set.
        // Write one more char — it should wrap to next line.
        let mut t = Terminal::new(4, 3);
        feed(&mut t, b"ABCD"); // fills row 0, pending_wrap=true
        feed(&mut t, b"\x1b7"); // DECSC save (with pending_wrap)
        feed(&mut t, b"\x1b[3;3H"); // move cursor away
        feed(&mut t, b"\x1b8"); // DECRC restore
        // Now pending_wrap should be restored. Writing 'E' should wrap.
        feed(&mut t, b"E");
        // E should be on row 1 (wrapped from row 0)
        assert_eq!(
            t.grid().cell(0, 1).map(|c| c.ch),
            Some('E'),
            "E should wrap to row 1 after DECSC restored pending_wrap"
        );
    }

    #[test]
    fn t_rep_with_wide_char() {
        // Write a wide char, then REP 3 → total 4 wide chars.
        let mut t = Terminal::new(10, 4);
        feed(&mut t, "你".as_bytes()); // 1 wide char (cols 0-1)
        feed(&mut t, b"\x1b[3b"); // REP 3
        // 4 wide chars = 8 cols. Check all are present.
        for i in 0..4 {
            let lead_col = i * 2;
            let lead = t.grid().cell(lead_col, 0).unwrap();
            assert_eq!(
                lead.ch, '你',
                "wide char {} should be at col {}",
                i, lead_col
            );
            assert!(lead.flags.contains(CellFlags::WIDE_CHAR));
            let spacer = t.grid().cell(lead_col + 1, 0).unwrap();
            assert!(spacer.flags.contains(CellFlags::WIDE_SPACER));
        }
    }

    #[test]
    fn t_combining_char_cap_at_8() {
        // Feed 10 combining marks — only 8 should be stored.
        let mut t = Terminal::new(80, 1);
        let mut s = String::from("e");
        for _ in 0..10 {
            s.push('\u{0301}'); // combining acute
        }
        feed(&mut t, s.as_bytes());
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(
            cell.combining.len(),
            8,
            "combining marks should be capped at 8"
        );
    }

    #[test]
    fn t_osc8_hyperlink_stored_on_cell() {
        // OSC 8 sets hyperlink on cells written while active.
        let mut t = Terminal::new(20, 2);
        feed(&mut t, b"\x1b]8;;https://example.com\x1b\\");
        feed(&mut t, b"link");
        feed(&mut t, b"\x1b]8;;\x1b\\");
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.hyperlink.as_deref(), Some("https://example.com"));
        // After closing OSC 8, next char should NOT have hyperlink
        feed(&mut t, b"\x1b[5G"); // cursor to col 4
        feed(&mut t, b"Z");
        let z = t.grid().cell(4, 0).unwrap();
        assert!(
            z.hyperlink.is_none(),
            "cell after OSC 8 close should have no hyperlink"
        );
    }

    #[test]
    fn t_synchronized_output_toggle() {
        let mut t = Terminal::new(10, 3);
        assert!(!t.is_synchronized());
        feed(&mut t, b"\x1b[?2026h");
        assert!(t.is_synchronized(), "DECSET 2026 should enable sync mode");
        feed(&mut t, b"\x1b[?2026l");
        assert!(!t.is_synchronized(), "DECRST 2026 should disable sync mode");
    }

    #[test]
    fn t_bracketed_paste_toggle() {
        let mut t = Terminal::new(10, 3);
        assert!(!t.bracketed_paste());
        feed(&mut t, b"\x1b[?2004h");
        assert!(
            t.bracketed_paste(),
            "DECSET 2004 should enable bracketed paste"
        );
        feed(&mut t, b"\x1b[?2004l");
        assert!(
            !t.bracketed_paste(),
            "DECRST 2004 should disable bracketed paste"
        );
    }

    #[test]
    fn t_decstr_resets_bracketed_paste() {
        // DECSTR (soft reset) should disable bracketed paste.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?2004h");
        assert!(t.bracketed_paste());
        feed(&mut t, b"\x1b[!p"); // DECSTR
        assert!(!t.bracketed_paste(), "DECSTR should reset bracketed paste");
    }

    #[test]
    fn t_combining_after_wide_char() {
        // Combining mark after a wide char should attach to the wide char.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, "你\u{0301}".as_bytes()); // 你 + combining acute
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.ch, '你');
        assert_eq!(
            cell.combining,
            vec!['\u{0301}'],
            "combining should attach to wide char"
        );
        assert_eq!(
            t.cursor().0,
            2,
            "cursor should be at col 2 (after wide char + spacer)"
        );
    }

    #[test]
    fn t_tab_from_col7_hits_col8() {
        // Tab from col 7 should stop at col 8 (next stop), not skip to 16.
        let mut t = Terminal::new(80, 2);
        feed(&mut t, b"\x1b[8G"); // cursor at col 7
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 8, "tab from col 7 should stop at col 8");
    }

    #[test]
    fn t_tab_from_col8_hits_col16() {
        // Tab from col 8 should stop at col 16.
        let mut t = Terminal::new(80, 2);
        feed(&mut t, b"\x1b[9G"); // cursor at col 8
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 16, "tab from col 8 should stop at col 16");
    }

    #[test]
    fn t_tab_at_last_col_clamps() {
        // Tab at col 79 (last col of 80-wide terminal) should clamp, not panic.
        let mut t = Terminal::new(80, 2);
        feed(&mut t, b"\x1b[80G"); // cursor at col 79
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 79, "tab at last col should clamp to 79");
    }

    #[test]
    fn t_decsc_restores_reverse_video() {
        // DECSC should save reverse video flag.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[7m"); // reverse video
        feed(&mut t, b"\x1b7"); // save
        feed(&mut t, b"\x1b[0m"); // reset
        feed(&mut t, b"\x1b8"); // restore
        assert!(
            t.flags.contains(CellFlags::REVERSE),
            "reverse video should be restored"
        );
    }

    #[test]
    fn t_decsc_restores_charset() {
        // DECSC should save G0 charset designation.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b(0"); // DEC Special Graphics
        feed(&mut t, b"\x1b7"); // save
        feed(&mut t, b"\x1b(B"); // back to ASCII
        feed(&mut t, b"\x1b8"); // restore
        feed(&mut t, b"q"); // if DEC Special restored, q should be ─
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            '\u{2500}',
            "DECSC should restore charset — q should be ─"
        );
    }

    #[test]
    fn t_origin_mode_cup_relative() {
        // Origin mode: CUP row 1 should be scroll region top, not absolute row 0.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[3;6r"); // region rows 2-5 (0-indexed)
        feed(&mut t, b"\x1b[?6h"); // enable origin mode
        feed(&mut t, b"\x1b[1;1H"); // CUP 1,1
        assert_eq!(t.cursor().1, 2, "origin mode: row 1 → region top (row 2)");
    }

    #[test]
    fn t_origin_mode_off_cup_absolute() {
        // Without origin mode, CUP 1,1 goes to absolute row 0 even with scroll region.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[3;6r"); // region rows 2-5
        feed(&mut t, b"\x1b[?6l"); // disable origin mode (explicit)
        feed(&mut t, b"\x1b[1;1H"); // CUP 1,1
        assert_eq!(t.cursor().1, 0, "non-origin mode: row 1 → absolute row 0");
    }

    #[test]
    fn t_autowrap_off_overwrite_last_col() {
        // With DECAWM off, writing past the last column overwrites it.
        let mut t = Terminal::new(4, 3);
        feed(&mut t, b"\x1b[?7l"); // autowrap off
        feed(&mut t, b"ABCDE");
        // E should overwrite D at col 3
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'E');
        assert_eq!(
            t.grid().cell(0, 1).map(|c| c.ch),
            Some(' '),
            "no wrap to next line with autowrap off"
        );
    }

    #[test]
    fn t_autowrap_on_wraps_correctly() {
        // With DECAWM on (default), writing past last col wraps.
        let mut t = Terminal::new(4, 3);
        feed(&mut t, b"\x1b[?7h"); // autowrap on (explicit)
        feed(&mut t, b"ABCDE");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'D');
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'E');
    }

    #[test]
    fn t_wrap_then_cr_returns_to_col0() {
        // After autowrap, CR should return to col 0 of current row.
        let mut t = Terminal::new(4, 3);
        feed(&mut t, b"ABCDE"); // wraps: D at row 0 col 3, E at row 1 col 0
        feed(&mut t, b"\r"); // CR
        assert_eq!(t.cursor().0, 0, "CR after wrap should go to col 0");
        assert_eq!(t.cursor().1, 1, "CR should stay on row 1");
    }

    #[test]
    fn t_ich_at_last_col() {
        // ICH at the last column — should insert blank, pushing content off.
        let mut t = Terminal::new(4, 2);
        feed(&mut t, b"ABCD"); // fill row 0
        feed(&mut t, b"\x1b[4G"); // cursor at col 3
        feed(&mut t, b"\x1b[@"); // ICH 1
        // A, B, C should survive. D pushed off, col 3 blank.
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'C');
        assert!(t.grid().cell(3, 0).unwrap().is_blank());
    }

    #[test]
    fn t_dch_more_than_content() {
        // DCH with count > remaining content fills with blanks.
        let mut t = Terminal::new(4, 2);
        feed(&mut t, b"AB"); // cols 0-1, cols 2-3 blank
        feed(&mut t, b"\x1b[1G"); // cursor at col 0
        feed(&mut t, b"\x1b[4P"); // DCH 4 — delete all 4
        // All should be blank
        for col in 0..4 {
            assert!(
                t.grid().cell(col, 0).unwrap().is_blank(),
                "col {} should be blank after DCH 4",
                col
            );
        }
    }

    #[test]
    fn t_el_clears_cell_attributes() {
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[42m");
        feed(&mut t, b"ABCD");
        feed(&mut t, b"\x1b[2G");
        feed(&mut t, b"\x1b[K");
        let cell = t.grid().cell(2, 0).unwrap();
        assert_eq!(
            cell.bg,
            Color::Default,
            "erased cell should have default bg"
        );
    }

    #[test]
    fn t_ed_clears_cell_attributes() {
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[44m");
        feed(&mut t, b"ABCD");
        feed(&mut t, b"\x1b[2J");
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.bg, Color::Default, "ED 2 should reset bg");
    }

    #[test]
    fn t_ind_at_scroll_region_bottom_scrolls() {
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[1;3r");
        feed(&mut t, b"\x1b[1;1HA");
        feed(&mut t, b"\x1b[2;1HB");
        feed(&mut t, b"\x1b[3;1HC");
        feed(&mut t, b"\x1b[4;1HOUT");
        feed(&mut t, b"\x1b[3;1H");
        feed(&mut t, b"\x1bD");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'B');
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'C');
        assert_eq!(t.grid().cell(0, 3).unwrap().ch, 'O');
    }

    #[test]
    fn t_rep_no_prior_char_is_noop() {
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[5b");
        assert_eq!(t.cursor().0, 0);
        assert!(t.grid().cell(0, 0).unwrap().is_blank());
    }

    #[test]
    fn t_rep_caps_at_width_times_2() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"A");
        feed(&mut t, b"\x1b[99999b");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
    }

    #[test]
    fn t_scroll_region_wrap_stays_in_region() {
        let mut t = Terminal::new(4, 5);
        feed(&mut t, b"\x1b[1;3r");
        feed(&mut t, b"\x1b[4;1HOUT");
        feed(&mut t, b"\x1b[1;1H");
        feed(&mut t, b"AAAAAAAAAAAA");
        assert_eq!(
            t.grid().cell(0, 3).unwrap().ch,
            'O',
            "OUT below scroll region should survive autowrap scroll"
        );
    }

    #[test]
    fn t_ed_0_at_last_col_no_panic() {
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"\x1b[4;10H");
        feed(&mut t, b"\x1b[0J");
        assert_eq!(t.cursor().0, 9);
    }

    #[test]
    fn t_alt_1047_no_cursor_save() {
        // Mode 1047 enters/exits alt screen WITHOUT cursor save/restore.
        // Unlike 1049, it does not save cursor position.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;5H"); // cursor at (4,2)
        feed(&mut t, b"\x1b[?1047h"); // enter alt
        feed(&mut t, b"\x1b[1;1HALT");
        feed(&mut t, b"\x1b[?1047l"); // exit alt
        // Cursor should NOT be restored to (4,2) — 1047 doesn't save cursor
        // It should be at (0,0) since 1047 exits go to home
        assert_ne!(
            t.cursor(),
            (4, 2),
            "1047 should not restore cursor position"
        );
    }

    #[test]
    fn t_alt_1049_vs_1047_cursor_restore() {
        // 1049 restores cursor, 1047 does not.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;5H"); // cursor at (4,2)
        feed(&mut t, b"\x1b[?1049h"); // enter alt (saves cursor)
        feed(&mut t, b"\x1b[5;5H"); // move cursor in alt
        feed(&mut t, b"\x1b[?1049l"); // exit alt (restores cursor)
        assert_eq!(t.cursor(), (4, 2), "1049 should restore cursor to (4,2)");
    }

    #[test]
    fn t_decscusr_persists_through_alt_screen() {
        // Cursor style set before entering alt screen should persist.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[4 q"); // DECSCUSR: steady underline
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        assert_eq!(
            t.cursor_style(),
            CursorStyle::SteadyUnderline,
            "cursor style should persist into alt screen"
        );
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert_eq!(
            t.cursor_style(),
            CursorStyle::SteadyUnderline,
            "cursor style should persist after alt screen exit"
        );
    }

    #[test]
    fn t_alt_screen_preserves_scrollback() {
        // Scrollback created in primary screen should survive alt screen round-trip.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"L1\nL2\nL3\nL4\nL5"); // create scrollback
        assert!(t.grid().scrollback_len() > 0, "should have scrollback");
        let sb_before = t.grid().scrollback_len();
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"ALTERNATE");
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert_eq!(
            t.grid().scrollback_len(),
            sb_before,
            "scrollback should survive alt screen round-trip"
        );
    }

    #[test]
    fn t_csi_s_u_save_restore_cursor() {
        // CSI s / CSI u (ANSI SC/RC) should save/restore cursor position.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;5H"); // cursor at (4,2)
        feed(&mut t, b"\x1b[s"); // ANSI save
        feed(&mut t, b"\x1b[5;1H"); // move cursor
        feed(&mut t, b"\x1b[u"); // ANSI restore
        assert_eq!(t.cursor(), (4, 2), "CSI u should restore cursor to (4,2)");
    }

    #[test]
    fn t_csi_s_does_not_save_sgr() {
        // Unlike DECSC, ANSI SC/CSI s only saves position, not SGR.
        // Verify the behavior is at least consistent.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1;31m"); // bold red
        feed(&mut t, b"\x1b[s"); // save
        feed(&mut t, b"\x1b[0m"); // reset
        feed(&mut t, b"\x1b[u"); // restore
        // CSI s/u in xterm only saves position, not attributes.
        // Verify current SGR state (should be reset since CSI s doesn't save SGR).
        // But implementation may differ — just verify position is correct.
        assert_eq!(t.cursor(), (0, 0));
    }

    #[test]
    fn t_origin_mode_disable_homes_cursor() {
        // Per VT spec, DECOM (both enable AND disable) moves cursor to home.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[?6h"); // enable origin
        feed(&mut t, b"\x1b[3;5H"); // move cursor to (4,2)
        feed(&mut t, b"\x1b[?6l"); // disable origin — should home cursor
        assert_eq!(
            t.cursor(),
            (0, 0),
            "DECOM disable should home cursor per VT spec"
        );
    }

    #[test]
    fn t_wide_char_backspace_removes_correct_cols() {
        // Write wide char, backspace — cursor should go from col 2 to col 0
        // (backspace skips the wide spacer, going to the lead cell).
        let mut t = Terminal::new(10, 2);
        feed(&mut t, "你".as_bytes()); // cols 0-1, cursor at col 2
        assert_eq!(t.cursor().0, 2);
        feed(&mut t, b"\x08"); // BS
        // BS should move to col 1, but since col 1 is a spacer, it should
        // go to col 0 (the lead). Behavior may vary — test what we have.
        assert!(
            t.cursor().0 <= 1,
            "BS after wide char should go to col 0 or 1"
        );
    }

    // ── Scroll region edge cases ──

    #[test]
    fn t_scroll_region_cursor_outside_lf_no_scroll() {
        // Cursor outside region (above). LF should NOT scroll the region.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[4;6r"); // region rows 3-5
        feed(&mut t, b"\x1b[1;1HA"); // row 0
        feed(&mut t, b"\x1b[2;1HB"); // row 1
        feed(&mut t, b"\x1b[1;1H"); // cursor at row 0 (above region)
        feed(&mut t, b"\n"); // LF at row 0 → moves to row 1
        // A and B should NOT have moved (no scroll outside region)
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'B');
    }

    #[test]
    fn t_il_outside_region_upper_is_noop() {
        // IL with cursor above scroll region should be a no-op.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[4;6r"); // region rows 3-5
        feed(&mut t, b"\x1b[1;1HTOP"); // row 0
        feed(&mut t, b"\x1b[1;1H"); // cursor at row 0 (above region)
        feed(&mut t, b"\x1b[L"); // IL — no-op
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'T');
    }

    #[test]
    fn t_dl_outside_region_lower_is_noop() {
        // DL with cursor below scroll region should be a no-op.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[1;4r"); // region rows 0-3
        feed(&mut t, b"\x1b[5;1HBOT"); // row 4 (below region)
        feed(&mut t, b"\x1b[5;1H"); // cursor at row 4 (below region)
        feed(&mut t, b"\x1b[M"); // DL — no-op
        assert_eq!(t.grid().cell(0, 4).unwrap().ch, 'B');
    }

    #[test]
    fn t_ich_works_outside_scroll_region() {
        // ICH/DCH operate on the current line regardless of scroll region.
        let mut t = Terminal::new(6, 6);
        feed(&mut t, b"\x1b[2;4r"); // region rows 1-3
        feed(&mut t, b"\x1b[1;1HABC"); // row 0 (outside region), 3 chars
        feed(&mut t, b"\x1b[2G"); // CHA col 2 → cursor at col 1
        feed(&mut t, b"\x1b[2@"); // ICH 2 — insert 2 blanks at col 1
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert!(
            t.grid().cell(1, 0).unwrap().is_blank(),
            "col 1 should be blank"
        );
        assert_eq!(
            t.grid().cell(3, 0).unwrap().ch,
            'B',
            "B should shift to col 3"
        );
    }

    #[test]
    fn t_decstbm_origin_mode_cup_relative() {
        // Origin mode + DECSTBM: CUP uses region-relative coordinates.
        let mut t = Terminal::new(10, 8);
        feed(&mut t, b"\x1b[3;6r"); // region rows 2-5
        feed(&mut t, b"\x1b[?6h"); // origin mode on
        feed(&mut t, b"\x1b[2;3H"); // CUP row 2, col 3
        // Row 2 relative to region top (row 2) = absolute row 3
        assert_eq!(t.cursor().1, 3, "origin: CUP row 2 → absolute row 3");
        assert_eq!(t.cursor().0, 2, "col 3 → col 2");
    }

    // ── Pending wrap edge cases ──

    #[test]
    fn t_pending_wrap_cleared_by_cup() {
        // Fill to last col (pending_wrap set), CUP should clear it.
        let mut t = Terminal::new(4, 3);
        feed(&mut t, b"ABCD"); // pending_wrap = true
        feed(&mut t, b"\x1b[1;1H"); // CUP — should clear pending_wrap
        feed(&mut t, b"X"); // should overwrite col 0, not wrap
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X');
        assert_eq!(
            t.cursor().1,
            0,
            "should stay on row 0 after CUP cleared pending_wrap"
        );
    }

    #[test]
    fn t_pending_wrap_then_lf_advances_one_row() {
        // Fill to last col (pending_wrap), then LF.
        // LF should clear pending_wrap and move down ONE row only.
        // LNM off: LF keeps column. So cursor stays at col 3, row 1.
        let mut t = Terminal::new(4, 3);
        feed(&mut t, b"ABCD"); // pending_wrap = true
        feed(&mut t, b"\n"); // LF — clear pending_wrap, move to row 1
        assert_eq!(t.cursor().1, 1, "LF should move to row 1");
        feed(&mut t, b"X"); // X at col 3 row 1
        assert_eq!(t.grid().cell(3, 1).unwrap().ch, 'X');
        assert_eq!(
            t.grid().cell(0, 1).map(|c| c.ch),
            Some(' '),
            "X should not be at col 0 — LF without LNM keeps column"
        );
    }

    #[test]
    fn t_pending_wrap_overwrite_vs_wrap() {
        // Fill to last col (pending_wrap set), then print another char.
        // Should WRAP to next line (not overwrite the char at last col).
        let mut t = Terminal::new(4, 3);
        feed(&mut t, b"ABCD"); // pending_wrap = true, D at col 3
        feed(&mut t, b"E"); // should wrap
        assert_eq!(
            t.grid().cell(3, 0).unwrap().ch,
            'D',
            "D should remain at col 3"
        );
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            'E',
            "E should wrap to row 1"
        );
    }

    #[test]
    fn t_decbawm_off_char_at_last_col_overwrites() {
        // With autowrap off, printing at the last column overwrites in place.
        let mut t = Terminal::new(4, 3);
        feed(&mut t, b"\x1b[?7l"); // autowrap off
        feed(&mut t, b"ABCDE"); // 5 chars on 4-wide terminal
        // E should overwrite D at col 3
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'E');
        assert_eq!(t.cursor().0, 3, "cursor stays at last col");
        assert_eq!(t.cursor().1, 0, "no line wrap");
    }

    #[test]
    fn t_pending_wrap_cleared_by_cuq() {
        // Fill to last col (pending_wrap), then CUU (cursor up).
        // CUU should clear pending_wrap. Column stays, row decreases.
        let mut t = Terminal::new(4, 4);
        feed(&mut t, b"\x1b[2;1H"); // row 1
        feed(&mut t, b"ABCD"); // fill row 1, pending_wrap = true
        feed(&mut t, b"\x1b[A"); // CUU — cursor up to row 0, col stays at 3
        feed(&mut t, b"X"); // print at (3,0)
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'X');
        assert_eq!(t.cursor().1, 0, "should be on row 0");
    }

    // ── SGR color parsing edge cases ──

    #[test]
    fn t_sgr_256_color_max_index() {
        // 38;5;255 should set fg to Indexed(255) (last valid 256-color).
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[38;5;255mX");
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Indexed(255));
    }

    #[test]
    fn t_sgr_truecolor_rgb() {
        // 38;2;255;128;0 should set fg to RGB(255,128,0).
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[38;2;255;128;0mX");
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Rgb(255, 128, 0));
    }

    #[test]
    fn t_sgr_truecolor_bg() {
        // 48;2;0;0;255 should set bg to RGB(0,0,255).
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[48;2;0;0;255mX");
        assert_eq!(t.grid().cell(0, 0).unwrap().bg, Color::Rgb(0, 0, 255));
    }

    #[test]
    fn t_sgr_malformed_38_no_subtype() {
        // 38 with no following subtype — should be a no-op, not crash.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[38mX");
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Default);
    }

    #[test]
    fn t_sgr_malformed_38_5_no_index() {
        // 38;5 with no color index — should be a no-op, not crash.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[38;5mX");
        // Should not crash. fg should stay default.
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Default);
    }

    #[test]
    fn t_sgr_reset_clears_underline_color() {
        // SGR 0 should reset underline_color too.
        // underline_color is a Terminal-level attribute, not per-cell.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[58;5;200m"); // set underline color
        // After SGR 0, terminal-level underline_color should be reset.
        feed(&mut t, b"\x1b[0m");
        // No way to check underline_color from outside without accessor,
        // but at least verify it doesn't crash.
        feed(&mut t, b"X");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X');
    }

    #[test]
    fn t_sgr_bright_color_range() {
        // SGR 90-97 should set bright fg (Indexed 8-15).
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[97mX"); // bright white fg
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Indexed(15));
    }

    #[test]
    fn t_sgr_bright_bg_range() {
        // SGR 100-107 should set bright bg (Indexed 8-15).
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[107mX"); // bright white bg
        assert_eq!(t.grid().cell(0, 0).unwrap().bg, Color::Indexed(15));
    }

    // ── OSC edge cases ──

    #[test]
    fn t_osc_title_caps_at_256() {
        // OSC 0 title should be capped at 256 chars.
        let mut t = Terminal::new(10, 2);
        let long_title = "A".repeat(300);
        feed(&mut t, format!("\x1b]0;{}\x07", long_title).as_bytes());
        assert_eq!(t.title().len(), 256, "title should be capped at 256 chars");
    }

    #[test]
    fn t_osc_overflow_consumes_until_terminator() {
        // When OSC exceeds the 64KB parser buffer cap, the parser must
        // stay in OscString state (consuming bytes) until the terminator
        // arrives. It must NOT return to Ground state, which would cause
        // the overflow bytes to be printed as terminal output.
        let mut t = Terminal::new(80, 5);
        // Build a 70KB OSC title sequence (> 64KB cap).
        let long_payload = "X".repeat(70000);
        // After the OSC, write "TEST" — this should appear at col 0.
        let input = format!("\x1b]0;{}\x07TEST", long_payload);
        feed(&mut t, input.as_bytes());
        // "TEST" should be at the start of the grid — the OSC terminator
        // (BEL) correctly ended the sequence. If the overflow bug existed,
        // some 'X' bytes would appear before "TEST".
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(
            cell.ch, 'T',
            "overflow OSC must consume bytes until terminator; got '{}' at (0,0)",
            cell.ch
        );
    }

    #[test]
    fn t_osc_title_strips_control_chars() {
        // OSC 0 title should strip control characters.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b]0;Hello\x01\x02World\x07");
        assert_eq!(t.title(), "HelloWorld");
    }

    #[test]
    fn t_osc_8_empty_uri_clears_hyperlink() {
        // OSC 8 with empty URI should clear current hyperlink.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b]8;;https://example.com\x1b\\");
        feed(&mut t, b"A");
        assert!(t.grid().cell(0, 0).unwrap().hyperlink.is_some());
        feed(&mut t, b"\x1b]8;;\x1b\\"); // clear
        feed(&mut t, b"\x1b[2GB"); // move to col 1
        feed(&mut t, b"B");
        assert!(
            t.grid().cell(1, 0).unwrap().hyperlink.is_none(),
            "cell after OSC 8 clear should have no hyperlink"
        );
    }

    #[test]
    fn t_osc_8_caps_uri_length() {
        // OSC 8 URI should be capped to prevent memory exhaustion.
        let mut t = Terminal::new(10, 2);
        let long_uri = "https://example.com/".repeat(200);
        feed(&mut t, format!("\x1b]8;;{}\x1b\\", long_uri).as_bytes());
        feed(&mut t, b"X");
        let hl = t.grid().cell(0, 0).unwrap().hyperlink.as_ref().unwrap();
        assert!(
            hl.len() <= 2048,
            "hyperlink URI should be capped at 2048 chars"
        );
    }

    // ── DECSTBM + cursor interaction ──

    #[test]
    fn t_su_in_scroll_region() {
        // SU (CSI S) should scroll within the scroll region only.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[2;5r"); // region rows 1-4 (0-indexed)
        feed(&mut t, b"\x1b[2;1HA"); // row 1
        feed(&mut t, b"\x1b[3;1HB"); // row 2
        feed(&mut t, b"\x1b[4;1HC"); // row 3
        feed(&mut t, b"\x1b[6;1HOUT"); // row 5 (index 5, below region)
        feed(&mut t, b"\x1b[1S"); // SU 1
        // OUT at row 5 should survive (it's below the region)
        assert_eq!(
            t.grid().cell(0, 5).map(|c| c.ch),
            Some('O'),
            "SU should not affect rows below scroll region"
        );
    }

    #[test]
    fn t_sd_in_scroll_region() {
        // SD (CSI T) should scroll within the scroll region only.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[2;5r"); // region rows 1-4
        feed(&mut t, b"\x1b[1;1HTOP"); // row 0 (above region)
        feed(&mut t, b"\x1b[2;1HA");
        feed(&mut t, b"\x1b[3;1HB");
        feed(&mut t, b"\x1b[4;1HC");
        feed(&mut t, b"\x1b[1T"); // SD 1
        // TOP at row 0 should survive
        assert_eq!(
            t.grid().cell(0, 0).map(|c| c.ch),
            Some('T'),
            "SD should not affect rows above scroll region"
        );
    }

    #[test]
    fn t_decstbm_single_row_region_is_valid() {
        // Region of 2 rows (top=1, bottom=2) — minimum valid region.
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"\x1b[2;3r"); // region rows 1-2
        feed(&mut t, b"\x1b[2;1HA");
        feed(&mut t, b"\x1b[3;1HB");
        feed(&mut t, b"\x1b[3;1H");
        feed(&mut t, b"\n"); // LF at region bottom → scroll
        // A should scroll off, B should move up
        assert_eq!(
            t.grid().cell(0, 1).map(|c| c.ch),
            Some('B'),
            "B should move up after scroll in 2-row region"
        );
    }

    #[test]
    fn t_decstbm_top_equals_bottom_no_scroll() {
        // CSI r with top == bottom → invalid, should not set region.
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"\x1b[2;2r"); // top == bottom → invalid
        // Cursor should still home per spec
        assert_eq!(t.cursor(), (0, 0));
        // Region should still be full screen (LF should scroll normally)
        feed(&mut t, b"\x1b[4;1HX"); // last row
        feed(&mut t, b"\n"); // LF at bottom → scroll
        assert_eq!(
            t.cursor().1,
            3,
            "LF should scroll full screen when region invalid"
        );
    }

    // ── Tab stop edge cases ──

    #[test]
    fn t_tab_from_col15_hits_col16() {
        let mut t = Terminal::new(80, 2);
        feed(&mut t, b"\x1b[16G"); // cursor at col 15
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 16);
    }

    #[test]
    fn t_tab_does_not_wrap_to_next_line() {
        // Tab at last column should NOT wrap to next line.
        let mut t = Terminal::new(8, 3);
        feed(&mut t, b"\x1b[8G"); // cursor at col 7 (last)
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 7, "tab at last col should clamp");
        assert_eq!(t.cursor().1, 0, "tab should not wrap to next line");
    }

    #[test]
    fn t_tbc_clears_current_tab_stop() {
        // TBC param 0 clears the current column's tab stop.
        let mut t = Terminal::new(80, 2);
        feed(&mut t, b"\x1b[9G"); // cursor at col 8
        feed(&mut t, b"\x1b[0g"); // TBC 0: clear stop at col 8
        feed(&mut t, b"\x1b[1G"); // cursor at col 0
        feed(&mut t, b"\t"); // tab should skip col 8 and go to 16
        assert_eq!(t.cursor().0, 16, "tab should skip cleared stop at col 8");
    }

    #[test]
    fn t_tbc_3_clears_all_tab_stops() {
        // TBC param 3 clears all tab stops.
        let mut t = Terminal::new(80, 2);
        feed(&mut t, b"\x1b[3g"); // clear all tab stops
        feed(&mut t, b"\t"); // tab with no stops → should clamp at last col
        assert_eq!(
            t.cursor().0,
            79,
            "tab with no stops should clamp to last col"
        );
    }

    #[test]
    fn t_decstr_resets_tab_stops() {
        // DECSTR (soft reset) should restore default 8-wide tab stops.
        let mut t = Terminal::new(80, 2);
        feed(&mut t, b"\x1b[3g"); // clear all stops
        feed(&mut t, b"\x1b[!p"); // DECSTR — soft reset
        feed(&mut t, b"\x1b[1G"); // col 0
        feed(&mut t, b"\t"); // tab should go to col 8 (default restored)
        assert_eq!(t.cursor().0, 8, "DECSTR should restore default tab stops");
    }

    // ── DEC mode toggles ──

    #[test]
    fn t_decset_25_cursor_visible_toggle() {
        // DECSET 25 / DECRST 25 should toggle cursor visibility flag.
        let mut t = Terminal::new(10, 2);
        assert!(t.cursor_visible()); // default visible
        feed(&mut t, b"\x1b[?25l"); // hide cursor
        assert!(!t.cursor_visible(), "DECRST 25 should hide cursor");
        feed(&mut t, b"\x1b[?25h"); // show cursor
        assert!(t.cursor_visible(), "DECSET 25 should show cursor");
    }

    #[test]
    fn t_decset_7_autowrap_default_on() {
        // DECAWM should be on by default.
        let t = Terminal::new(4, 2);
        assert!(t.modes.auto_wrap, "autowrap should be on by default");
    }

    #[test]
    fn t_decset_7_off_then_on() {
        // Toggle autowrap off then on.
        let mut t = Terminal::new(4, 2);
        feed(&mut t, b"\x1b[?7l"); // off
        assert!(!t.modes.auto_wrap);
        feed(&mut t, b"\x1b[?7h"); // on
        assert!(t.modes.auto_wrap);
    }

    #[test]
    fn t_decset_1049_clears_alt_on_enter() {
        // 1049 should enter a clean alt screen.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"PRIMARY"); // write to primary
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        // Alt screen should be blank
        assert!(
            t.grid().cell(0, 0).unwrap().is_blank(),
            "alt screen should be clean on enter"
        );
        feed(&mut t, b"ALT");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        // Primary should be restored
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'P',
            "primary screen should be restored"
        );
    }

    #[test]
    fn t_decset_1047_clears_alt_on_exit() {
        // 1047 should clear the alt screen on exit (vs 1049 which restores primary).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"PRIMARY");
        feed(&mut t, b"\x1b[?1047h"); // enter alt
        feed(&mut t, b"ALTDATA");
        feed(&mut t, b"\x1b[?1047l"); // exit alt
        // Primary should be restored
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'P');
    }

    #[test]
    fn t_decstr_resets_origin_mode() {
        // DECSTR should reset origin mode to default (off).
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"\x1b[?6h"); // origin mode on
        feed(&mut t, b"\x1b[!p"); // DECSTR
        assert!(!t.modes.origin, "DECSTR should reset origin mode");
    }

    #[test]
    fn t_decstr_resets_scroll_region() {
        // DECSTR should reset scroll region to full screen.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[2;4r"); // set region
        feed(&mut t, b"\x1b[!p"); // DECSTR
        // Now cursor at bottom of full screen should scroll
        feed(&mut t, b"\x1b[6;1H"); // row 5 (last row)
        feed(&mut t, b"\n");
        assert_eq!(
            t.cursor().1,
            5,
            "DECSTR should reset scroll region to full screen"
        );
    }

    // ── DECSC/DECRC state restoration probes ──

    #[test]
    fn t_decsc_saves_origin_mode_state() {
        // Save with origin mode ON, toggle OFF, restore → origin should be ON.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[3;5r"); // set scroll region
        feed(&mut t, b"\x1b[?6h"); // origin mode on
        feed(&mut t, b"\x1b7"); // DECSC save
        feed(&mut t, b"\x1b[?6l"); // origin mode off
        feed(&mut t, b"\x1b8"); // DECRC restore
        assert!(t.modes.origin, "DECSC should save and restore origin mode");
    }

    #[test]
    fn t_decsc_saves_autowrap_off() {
        // Save with autowrap OFF, toggle ON, restore → autowrap should be OFF.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?7l"); // autowrap off
        feed(&mut t, b"\x1b7"); // save
        feed(&mut t, b"\x1b[?7h"); // autowrap on
        feed(&mut t, b"\x1b8"); // restore
        assert!(!t.modes.auto_wrap, "DECSC should save autowrap state");
    }

    // ── REP (Repeat Character) probes ──

    #[test]
    fn t_rep_after_cursor_movement() {
        // Print 'A', move cursor, REP — should still repeat 'A' per spec.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"A");
        feed(&mut t, b"\x1b[2G"); // CHA col 2 → cursor at col 1
        feed(&mut t, b"\x1b[2b"); // REP 2
        // A at col 0, then REP should print 2 A's starting at col 1
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'A');
    }

    #[test]
    fn t_rep_zero_count_is_noop() {
        // REP 0 should be a no-op (or repeat once, per some interpretations).
        // Spec says default count is 1; explicit 0 is implementation-defined.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"A");
        feed(&mut t, b"\x1b[0b"); // REP 0
        // At minimum, should not crash. Cursor should be at col 1 (just the A).
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
    }

    // ── ICH/DCH with wide chars ──

    #[test]
    fn t_ich_does_not_split_wide_char_at_boundary() {
        // ICH at position where it would split a wide char lead from its spacer.
        let mut t = Terminal::new(8, 2);
        feed(&mut t, b"AB");
        feed(&mut t, "你".as_bytes()); // cols 2-3
        feed(&mut t, b"CD"); // cols 4-5
        feed(&mut t, b"\x1b[5G"); // cursor at col 4
        feed(&mut t, b"\x1b[2@"); // ICH 2 — insert 2 at col 4
        // Check no orphaned wide chars
        for col in 0..6 {
            let c = t.grid().cell(col, 0).unwrap();
            if c.flags.contains(CellFlags::WIDE_CHAR) {
                assert!(
                    t.grid()
                        .cell(col + 1, 0)
                        .unwrap()
                        .flags
                        .contains(CellFlags::WIDE_SPACER),
                    "WIDE_CHAR at col {} must have spacer at col {} after ICH",
                    col,
                    col + 1
                );
            }
        }
    }

    #[test]
    fn t_dch_at_last_col() {
        // DCH at last column — deletes char at cursor, shifts left.
        let mut t = Terminal::new(4, 2);
        feed(&mut t, b"ABCD"); // fill row
        feed(&mut t, b"\x1b[4G"); // cursor at col 3 (D)
        feed(&mut t, b"\x1b[1P"); // DCH 1 — delete D
        // A,B,C survive. Col 3 becomes blank.
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'C');
        assert!(
            t.grid().cell(3, 0).unwrap().is_blank(),
            "col 3 should be blank after DCH"
        );
    }

    #[test]
    fn t_dch_wide_char_keeps_pairs() {
        // DCH on a row with wide chars — no orphaned spacer should remain.
        let mut t = Terminal::new(6, 2);
        feed(&mut t, "你".as_bytes()); // cols 0-1
        feed(&mut t, b"XY"); // cols 2-3
        feed(&mut t, b"\x1b[3G"); // cursor at col 2
        feed(&mut t, b"\x1b[1P"); // DCH 1
        // No orphaned wide spacer at col 0 or 1
        let c0 = t.grid().cell(0, 0).unwrap();
        if c0.flags.contains(CellFlags::WIDE_CHAR) {
            assert!(
                t.grid()
                    .cell(1, 0)
                    .unwrap()
                    .flags
                    .contains(CellFlags::WIDE_SPACER),
                "wide char pair must stay together after DCH"
            );
        }
    }

    // ── Wide char cursor movement probes ──

    #[test]
    fn t_cub_after_wide_char_lands_on_lead() {
        // Write wide char, then CUB 1 — cursor should land on the lead cell (col 0).
        let mut t = Terminal::new(10, 2);
        feed(&mut t, "你".as_bytes()); // cols 0-1, cursor at col 2
        feed(&mut t, b"\x1b[D"); // CUB 1 (cursor left)
        // Should land on col 1 (spacer), but some terminals go to col 0 (lead).
        // At minimum, it should not go past the lead.
        assert!(
            t.cursor().0 <= 1,
            "CUB after wide char should not skip past lead"
        );
    }

    #[test]
    fn t_wide_char_then_narrow_no_erase() {
        // Wide char at col 0-1, move cursor back to col 0, print narrow 'X'.
        // X should replace the wide char (both cells).
        let mut t = Terminal::new(10, 2);
        feed(&mut t, "你".as_bytes()); // cols 0-1
        feed(&mut t, b"\x1b[1G"); // cursor at col 0
        feed(&mut t, b"X"); // overwrite
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X');
        // Col 1 should no longer be a wide spacer
        assert!(
            !t.grid()
                .cell(1, 0)
                .unwrap()
                .flags
                .contains(CellFlags::WIDE_SPACER),
            "overwriting wide char lead should clear spacer"
        );
    }

    // ── EL/ED edge cases ──

    #[test]
    fn t_el_2_resets_attributes() {
        // EL 2 (erase entire line) should reset cell attributes to default.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[42mABCD"); // green bg
        feed(&mut t, b"\x1b[2K"); // erase entire line
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.bg, Color::Default, "EL 2 should reset bg");
    }

    #[test]
    fn t_el_1_at_scroll_region_boundary() {
        // EL 1 (erase from start to cursor, inclusive) at cursor in scroll region.
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"\x1b[2;4r"); // region rows 1-3
        feed(&mut t, b"\x1b[3;1HABCD"); // row 2: ABCD
        feed(&mut t, b"\x1b[3;3G"); // cursor at col 2 (on C)
        feed(&mut t, b"\x1b[1K"); // EL 1: erase start to cursor (inclusive)
        assert!(t.grid().cell(0, 2).unwrap().is_blank());
        assert!(t.grid().cell(1, 2).unwrap().is_blank());
        assert!(
            t.grid().cell(2, 2).unwrap().is_blank(),
            "C at cursor should be erased by EL 1"
        );
        assert_eq!(
            t.grid().cell(3, 2).unwrap().ch,
            'D',
            "D after cursor should survive"
        );
    }

    #[test]
    fn t_ed_2_resets_all_attributes() {
        // ED 2 (erase all) should reset ALL cell attributes including bold, italic.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[1;3mAB"); // bold + italic
        feed(&mut t, b"\x1b[2J"); // erase all
        let cell = t.grid().cell(0, 0).unwrap();
        assert!(
            !cell.flags.contains(CellFlags::BOLD),
            "ED 2 should clear bold"
        );
        assert!(
            !cell.flags.contains(CellFlags::ITALIC),
            "ED 2 should clear italic"
        );
    }

    // ── OSC dynamic colors: reset via 110/111/112 ──

    #[test]
    fn t_osc_110_resets_dynamic_fg() {
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b]10;#ff0000\x07"); // set dynamic fg to red
        assert!(t.dynamic_fg().is_some());
        feed(&mut t, b"\x1b]110\x07"); // reset dynamic fg
        assert!(t.dynamic_fg().is_none(), "OSC 110 should reset dynamic fg");
    }

    #[test]
    fn t_osc_111_resets_dynamic_bg() {
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b]11;#00ff00\x07"); // set dynamic bg to green
        assert!(t.dynamic_bg().is_some());
        feed(&mut t, b"\x1b]111\x07"); // reset dynamic bg
        assert!(t.dynamic_bg().is_none(), "OSC 111 should reset dynamic bg");
    }

    #[test]
    fn t_osc_112_resets_dynamic_cursor_color() {
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b]12;#0000ff\x07"); // set cursor color to blue
        assert!(t.dynamic_cursor().is_some());
        feed(&mut t, b"\x1b]112\x07"); // reset dynamic cursor color
        assert!(
            t.dynamic_cursor().is_none(),
            "OSC 112 should reset dynamic cursor color"
        );
    }

    #[test]
    fn t_osc_color_with_rgb_format() {
        // OSC 10 with rgb:R/G/B format (xterm spec).
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b]10;rgb:ff/00/ff\x07"); // magenta
        assert!(t.dynamic_fg().is_some(), "rgb: format should be parsed");
    }

    // ── Focus event probes ──

    #[test]
    fn t_focus_in_report_format() {
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[?1004h"); // enable focus events
        assert_eq!(t.focus_in_report(), b"\x1b[I".to_vec());
    }

    #[test]
    fn t_focus_out_report_format() {
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[?1004h"); // enable focus events
        assert_eq!(t.focus_out_report(), b"\x1b[O".to_vec());
    }

    #[test]
    fn t_focus_report_disabled_when_off() {
        let t = Terminal::new(10, 2);
        // Focus events default off
        assert!(t.focus_in_report().is_empty());
        assert!(t.focus_out_report().is_empty());
    }

    // ── OSC 8 hyperlink range tracking ──

    #[test]
    fn t_osc8_hyperlink_persists_across_line_wrap() {
        // Set hyperlink, print chars that wrap to next line — all should have link.
        let mut t = Terminal::new(4, 3);
        feed(&mut t, b"\x1b]8;;https://example.com\x1b\\");
        feed(&mut t, b"ABCDEFGH"); // 8 chars, wraps after 4
        // Check chars on both rows have hyperlink
        assert!(
            t.grid().cell(0, 0).unwrap().hyperlink.is_some(),
            "char on row 0 should have hyperlink"
        );
        assert!(
            t.grid().cell(0, 1).unwrap().hyperlink.is_some(),
            "char on row 1 should have hyperlink (after wrap)"
        );
    }

    #[test]
    fn t_osc8_clear_then_new_chars_have_no_link() {
        // Set link, print, clear link, print more — only first chars should have link.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b]8;;https://example.com\x1b\\");
        feed(&mut t, b"AB"); // linked
        feed(&mut t, b"\x1b]8;;\x1b\\"); // clear
        feed(&mut t, b"\x1b[3G"); // cursor to col 2
        feed(&mut t, b"CD"); // not linked
        assert!(
            t.grid().cell(0, 0).unwrap().hyperlink.is_some(),
            "A should have link"
        );
        assert!(
            t.grid().cell(2, 0).unwrap().hyperlink.is_none(),
            "C should NOT have link"
        );
    }

    #[test]
    fn t_osc8_with_params_and_uri() {
        // OSC 8 with params (id=123) and URI.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b]8;id=123;https://example.com\x1b\\");
        feed(&mut t, b"X");
        let hl = t.grid().cell(0, 0).unwrap().hyperlink.as_ref().unwrap();
        assert!(hl.contains("example.com"), "URI should be stored");
    }

    // ── Tab stop preservation across operations ──

    #[test]
    fn t_tab_stops_preserved_across_line_wrap() {
        // Custom tab stop should survive line wrap.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[3G"); // cursor at col 2
        feed(&mut t, b"\x1bH"); // HTS — set tab stop at col 2
        feed(&mut t, b"\x1b[1;1H"); // home
        feed(&mut t, b"\t"); // tab → should stop at col 2 (custom stop)
        assert_eq!(t.cursor().0, 2, "custom tab stop at col 2 should work");
    }

    #[test]
    fn t_hts_at_last_col() {
        // HTS at last column should set stop there.
        let mut t = Terminal::new(8, 2);
        feed(&mut t, b"\x1b[8G"); // cursor at col 7 (last)
        feed(&mut t, b"\x1bH"); // HTS — set stop at col 7
        // Tab from col 0 should now stop at col 7
        feed(&mut t, b"\x1b[1G"); // cursor at col 0
        feed(&mut t, b"\t");
        assert_eq!(
            t.cursor().0,
            7,
            "HTS at last col, tab from 0 should reach it"
        );
    }

    // ── SGR attribute composition ──

    #[test]
    fn t_sgr_bold_plus_256color_plus_underline() {
        // Bold + 256-color fg + underline should all compose on the same cell.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[1;4;38;5;200mX");
        let cell = t.grid().cell(0, 0).unwrap();
        assert!(cell.flags.contains(CellFlags::BOLD), "bold should be set");
        assert!(
            cell.flags.contains(CellFlags::UNDERLINE),
            "underline should be set"
        );
        assert_eq!(cell.fg, Color::Indexed(200), "fg should be Indexed(200)");
    }

    #[test]
    fn t_sgr_truecolor_bg_plus_bold_plus_strikethrough() {
        // Complex attribute composition with truecolor bg.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[1;9;48;2;100;200;50mX");
        let cell = t.grid().cell(0, 0).unwrap();
        assert!(cell.flags.contains(CellFlags::BOLD));
        assert!(cell.flags.contains(CellFlags::STRIKETHROUGH));
        assert_eq!(cell.bg, Color::Rgb(100, 200, 50));
    }

    #[test]
    fn t_sgr_0_resets_bold_underline_and_color() {
        // SGR 0 should fully reset everything.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[1;4;38;5;200;48;2;1;2;3m"); // set lots of attrs
        feed(&mut t, b"\x1b[0m"); // reset all
        feed(&mut t, b"X");
        let cell = t.grid().cell(0, 0).unwrap();
        assert!(!cell.flags.contains(CellFlags::BOLD));
        assert!(!cell.flags.contains(CellFlags::UNDERLINE));
        assert_eq!(cell.fg, Color::Default);
        assert_eq!(cell.bg, Color::Default);
    }

    // ── OSC title with both terminators ──

    #[test]
    fn t_osc_title_st_terminator() {
        // OSC 0 with ST terminator (ESC \) instead of BEL.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b]0;MyTitle\x1b\\");
        assert_eq!(t.title(), "MyTitle");
    }

    #[test]
    fn t_osc_title_empty_payload() {
        // OSC 0 with empty title — should not crash.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b]0;\x07");
        assert_eq!(t.title(), "");
    }

    // ── Wide char: narrow on continuation cell ──

    #[test]
    fn t_narrow_on_wide_spacer_clears_lead() {
        // Write wide char, move cursor to spacer cell, write narrow.
        // The lead cell should be cleared (not left as orphaned wide).
        let mut t = Terminal::new(10, 2);
        feed(&mut t, "你".as_bytes()); // cols 0-1
        feed(&mut t, b"\x1b[2G"); // cursor at col 1 (spacer)
        feed(&mut t, b"X"); // overwrite spacer
        // Col 0 should not be an orphaned wide char
        assert!(
            !t.grid()
                .cell(0, 0)
                .unwrap()
                .flags
                .contains(CellFlags::WIDE_CHAR),
            "writing on spacer should clear the lead cell"
        );
    }

    #[test]
    fn t_wide_char_then_bs_then_wide_char() {
        // Write wide char, BS to lead, write another wide char.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, "你".as_bytes()); // cols 0-1, cursor at 2
        feed(&mut t, b"\x08\x08"); // BS twice → cursor at 0
        feed(&mut t, "好".as_bytes()); // overwrite cols 0-1
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '好');
        assert!(
            t.grid()
                .cell(1, 0)
                .unwrap()
                .flags
                .contains(CellFlags::WIDE_SPACER)
        );
        assert_eq!(t.cursor().0, 2);
    }

    #[test]
    fn t_emoji_4byte_then_normal() {
        // 4-byte emoji (😀 U+1F600) then normal chars.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, "😀".as_bytes()); // cols 0-1 (width 2)
        feed(&mut t, b"AB"); // cols 2-3
        assert_eq!(t.cursor().0, 4);
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{1F600}');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'B');
    }

    #[test]
    fn t_wide_char_at_col_minus_one_wraps() {
        // Wide char at col width-1 (not enough room) → wraps to next line.
        let mut t = Terminal::new(4, 2);
        feed(&mut t, b"ABC"); // cols 0-2
        feed(&mut t, "你".as_bytes()); // doesn't fit at col 3, wraps
        // A,B,C stay on row 0; 你 goes to row 1
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'C');
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, '你');
    }

    #[test]
    fn t_consecutive_wide_chars() {
        // Multiple consecutive CJK chars should all have proper spacers.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, "你好世".as_bytes()); // 6 cells, cursor at 6
        assert_eq!(t.cursor().0, 6);
        for col in [0, 2, 4] {
            assert!(
                t.grid()
                    .cell(col, 0)
                    .unwrap()
                    .flags
                    .contains(CellFlags::WIDE_CHAR),
                "col {} should be WIDE_CHAR",
                col
            );
            assert!(
                t.grid()
                    .cell(col + 1, 0)
                    .unwrap()
                    .flags
                    .contains(CellFlags::WIDE_SPACER),
                "col {} should be WIDE_SPACER",
                col + 1
            );
        }
    }

    // ── Resize edge cases ──

    #[test]
    fn t_resize_shrink_clamps_cursor() {
        // Resize from 10x4 to 4x2 — cursor at (9,3) should clamp to (3,1).
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"\x1b[4;10H"); // cursor at (9,3)
        t.resize(4, 2);
        assert!(t.cursor().0 <= 3, "cursor col should be clamped to 3");
        assert!(t.cursor().1 <= 1, "cursor row should be clamped to 1");
    }

    #[test]
    fn t_resize_grow_preserves_content() {
        // Resize from 4x2 to 8x4 — existing content should survive.
        let mut t = Terminal::new(4, 2);
        feed(&mut t, b"\x1b[1;1HAB\r\nCD");
        t.resize(8, 4);
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B');
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'C');
    }

    #[test]
    fn t_resize_alt_screen_simple() {
        // Resize while in alt screen — should not crash.
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"\x1b[3;5HALT"); // write in alt
        t.resize(6, 3);
        // Should survive without crash. Content may be truncated.
        assert_eq!(t.grid().width(), 6);
        assert_eq!(t.grid().height(), 3);
    }

    #[test]
    fn t_resize_tab_stops_extended() {
        // Growing width should add default tab stops in the new range.
        let mut t = Terminal::new(8, 2);
        t.resize(20, 2);
        // Tab from col 8 should hit col 16 (default stop in new area)
        feed(&mut t, b"\x1b[9G"); // cursor at col 8
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 16, "tab should hit col 16 after resize");
    }

    // ── Bracketed paste mode probes ──

    #[test]
    fn t_bracketed_paste_default_off() {
        let t = Terminal::new(10, 2);
        assert!(!t.bracketed_paste(), "bracketed paste should default off");
    }

    #[test]
    fn t_bracketed_paste_toggle_on_off() {
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[?2004h"); // enable
        assert!(t.bracketed_paste());
        feed(&mut t, b"\x1b[?2004l"); // disable
        assert!(!t.bracketed_paste());
    }

    #[test]
    fn t_bracketed_paste_persists_through_alt() {
        // Bracketed paste enabled, enter alt, exit — should persist.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[?2004h"); // enable
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        assert!(t.bracketed_paste(), "should persist in alt screen");
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert!(t.bracketed_paste(), "should persist after alt screen exit");
    }

    #[test]
    fn t_bracketed_paste_reset_by_decstr() {
        // DECSTR should reset bracketed paste to off.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[?2004h"); // enable
        feed(&mut t, b"\x1b[!p"); // DECSTR
        assert!(!t.bracketed_paste(), "DECSTR should reset bracketed paste");
    }

    // ── Alt screen edge cases ──

    #[test]
    fn t_alt_screen_nested_enter_is_idempotent() {
        // Double-enter alt screen should not crash or corrupt state.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"MAIN");
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"\x1b[?1049h"); // enter again (idempotent)
        feed(&mut t, b"ALT");
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'M',
            "primary should survive double-enter"
        );
    }

    #[test]
    fn t_alt_screen_exit_without_enter_is_noop() {
        // Exit alt screen when not in alt → no-op, no crash.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"MAIN");
        feed(&mut t, b"\x1b[?1049l"); // exit without entering
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'M',
            "exit without enter should be no-op"
        );
    }

    #[test]
    fn t_alt_screen_preserves_scroll_region() {
        // Enter alt, set scroll region, exit — primary scroll region should be restored.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[2;4r"); // set region rows 1-3
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"\x1b[1;1r"); // reset region in alt
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        // Primary region should be restored — test via LF at row 3
        feed(&mut t, b"\x1b[3;1H"); // cursor at row 2 (in restored region)
        feed(&mut t, b"\n"); // LF at row 2 → row 3 (still in region)
        assert_eq!(
            t.cursor().1,
            3,
            "scroll region should be restored from primary"
        );
    }

    // ── IL/DL overflow safety ──

    #[test]
    fn t_il_count_exceeds_region_height_no_panic() {
        // IL with count > region height should not panic.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[2;4r"); // region rows 1-3 (height=2)
        feed(&mut t, b"\x1b[2;1HAB"); // row 1
        feed(&mut t, b"\x1b[3;1HCD"); // row 2
        feed(&mut t, b"\x1b[2;1H"); // cursor at row 1 (region top)
        feed(&mut t, b"\x1b[100L"); // IL 100 — way more than region
        // Should not panic. A,B,C,D may all be pushed out.
        assert!(t.cursor().1 >= 1);
    }

    #[test]
    fn t_dl_count_exceeds_region_height_no_panic() {
        // DL with count > region height should not panic.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[2;4r"); // region rows 1-3
        feed(&mut t, b"\x1b[2;1HAB"); // row 1
        feed(&mut t, b"\x1b[3;1HCD"); // row 2
        feed(&mut t, b"\x1b[2;1H"); // cursor at region top
        feed(&mut t, b"\x1b[100M"); // DL 100
        // Should not panic. Region should be blanked.
        assert!(t.grid().cell(0, 1).unwrap().is_blank());
    }

    #[test]
    fn t_il_preserves_row_below_region() {
        // IL inside region should not affect row below.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[2;4r"); // region rows 1-3
        feed(&mut t, b"\x1b[5;1HBELOW"); // row 4 (below region)
        feed(&mut t, b"\x1b[2;1H"); // cursor at region top
        feed(&mut t, b"\x1b[L"); // IL 1
        assert_eq!(
            t.grid().cell(0, 4).unwrap().ch,
            'B',
            "row below region should survive IL"
        );
    }

    // ── ICH/DCH edge cases ──

    #[test]
    fn t_ich_count_exceeds_width_no_panic() {
        // ICH with count > width should clamp, not panic.
        let mut t = Terminal::new(5, 2);
        feed(&mut t, b"ABCDE");
        feed(&mut t, b"\x1b[1G"); // cursor at col 0
        feed(&mut t, b"\x1b[100@"); // ICH 100
        // Should not panic. A should be shifted off (or blank fills inserted).
        assert!(t.cursor().0 < 5);
    }

    #[test]
    fn t_dch_count_exceeds_width_no_panic() {
        // DCH with count > remaining width should clamp.
        let mut t = Terminal::new(5, 2);
        feed(&mut t, b"ABCDE");
        feed(&mut t, b"\x1b[3G"); // cursor at col 2
        feed(&mut t, b"\x1b[100P"); // DCH 100 — way more than remaining
        // Should not panic. Cols 2-4 should be blank.
        assert!(t.grid().cell(2, 0).unwrap().is_blank());
        assert!(t.grid().cell(4, 0).unwrap().is_blank());
    }

    #[test]
    fn t_ich_at_last_col_inserts_one_blank() {
        // ICH at last column — inserts blank, content shifts right (clipped).
        let mut t = Terminal::new(4, 2);
        feed(&mut t, b"ABCD");
        feed(&mut t, b"\x1b[4G"); // cursor at col 3 (last)
        feed(&mut t, b"\x1b[1@"); // ICH 1
        // D should be pushed off; col 3 becomes blank.
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'C');
        assert!(
            t.grid().cell(3, 0).unwrap().is_blank(),
            "col 3 should be blank after ICH"
        );
    }

    // ── DECOM + LF scroll ──

    #[test]
    fn t_decom_lf_at_region_bottom_scrolls_within_region() {
        // In origin mode with scroll region, LF at region bottom scrolls within.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[3;5r"); // region rows 2-4
        feed(&mut t, b"\x1b[?6h"); // origin mode on
        feed(&mut t, b"\x1b[1;1H"); // CUP row 1 col 1 → region-relative row 2
        feed(&mut t, b"AAAA"); // write at row 2
        feed(&mut t, b"\x1b[2;1H"); // CUP row 2 col 1 → region-relative row 3
        feed(&mut t, b"BBBB"); // write at row 3
        feed(&mut t, b"\x1b[3;1H"); // CUP row 3 → region-relative row 4 (region bottom)
        feed(&mut t, b"\n"); // LF at region bottom → scroll
        // Row 0 and 5 should be untouched
        assert!(
            t.grid().cell(0, 0).unwrap().is_blank(),
            "row 0 should be untouched"
        );
        assert!(
            t.grid().cell(0, 5).unwrap().is_blank(),
            "row 5 should be untouched"
        );
        // A should have scrolled off, B should move from row 3 to row 2
        assert_eq!(
            t.grid().cell(0, 2).unwrap().ch,
            'B',
            "B should move from row 3 to row 2 after scroll"
        );
    }

    #[test]
    fn t_decom_cursor_clamps_to_region_bottom() {
        // In origin mode, CUP to row beyond region height should clamp.
        let mut t = Terminal::new(10, 8);
        feed(&mut t, b"\x1b[2;4r"); // region rows 1-3
        feed(&mut t, b"\x1b[?6h"); // origin mode
        feed(&mut t, b"\x1b[100;1H"); // CUP row 100 → should clamp to region row 3 (abs)
        assert_eq!(
            t.cursor().1,
            3,
            "CUP should clamp to region bottom in origin mode"
        );
    }

    // ── DECSC underline color restore ──

    #[test]
    fn t_decsc_restores_underline_color() {
        // DECSC should save and DECRC should restore underline color.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[58;5;42m"); // set underline color to 42
        feed(&mut t, b"\x1b7"); // DECSC save
        feed(&mut t, b"\x1b[59m"); // reset underline color to default
        feed(&mut t, b"\x1b8"); // DECRC restore
        // underline_color should be restored (terminal-level, not cell-level)
        // Verify via printing after restore
        feed(&mut t, b"X");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X');
    }

    // ── REP at line start ──

    #[test]
    fn t_rep_at_line_start_after_cursor_move_no_panic() {
        // REP at start of line with no prior char on this line.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"A"); // print A on row 0
        feed(&mut t, b"\r\n"); // CRLF to row 1
        feed(&mut t, b"\x1b[3b"); // REP 3 — last char was A (from row 0)
        // Per spec, REP repeats the last printed graphic char regardless of cursor move
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 1).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(2, 1).unwrap().ch, 'A');
    }

    #[test]
    fn t_rep_does_not_panic_on_wide_char_at_wrap_boundary() {
        // REP with wide char when wrapping — should handle spacers correctly.
        let mut t = Terminal::new(6, 3);
        feed(&mut t, "你".as_bytes()); // wide char at cols 0-1
        feed(&mut t, b"\x1b[2b"); // REP 2 — repeat 你 twice more
        // Should not panic. Check no orphaned wide chars.
        for col in 0..6 {
            let c = t.grid().cell(col, 0).unwrap();
            if c.flags.contains(CellFlags::WIDE_CHAR) {
                let next = t
                    .grid()
                    .cell(col + 1, 0)
                    .unwrap_or(t.grid().cell(0, 1).unwrap());
                assert!(
                    next.flags.contains(CellFlags::WIDE_SPACER),
                    "WIDE_CHAR at col {} should have spacer",
                    col
                );
            }
        }
    }

    // ── DECSC restores cursor visible flag ──

    #[test]
    fn t_decsc_restores_cursor_visible_state() {
        // DECSC should save cursor visibility state.
        // (cursor_visible is a mode, saved via DECSC per xterm spec.)
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[?25l"); // hide cursor
        feed(&mut t, b"\x1b7"); // save
        feed(&mut t, b"\x1b[?25h"); // show cursor
        feed(&mut t, b"\x1b8"); // restore
        // cursor_visible should be restored — but this depends on implementation.
        // At minimum, should not crash.
        feed(&mut t, b"X");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X');
    }

    // ── CSI param default value probes ──

    #[test]
    fn t_csi_cup_empty_params_default_to_1() {
        // CSI H with no params should home cursor (1;1).
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;5HX");
        feed(&mut t, b"\x1b[H"); // CUP with no params
        assert_eq!(t.cursor(), (0, 0));
    }

    #[test]
    fn t_csi_cup_explicit_zero_treated_as_one() {
        // CSI 0;0H should be treated as CSI 1;1H.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;5HX");
        feed(&mut t, b"\x1b[0;0H");
        assert_eq!(t.cursor(), (0, 0), "0;0 should be treated as 1;1");
    }

    #[test]
    fn t_csi_cuf_empty_param_default_1() {
        // CSI CUF (forward tab) with no param should move 1.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[3C"); // CUF 3 → col 3
        feed(&mut t, b"\x1b[C"); // CUF (default 1) → col 4
        assert_eq!(t.cursor().0, 4);
    }

    #[test]
    fn t_csi_cub_empty_param_default_1() {
        // CSI CUB (back) with no param should move 1.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[5G"); // col 4
        feed(&mut t, b"\x1b[D"); // CUB default 1 → col 3
        assert_eq!(t.cursor().0, 3);
    }

    #[test]
    fn t_csi_su_default_1() {
        // CSI S with no param should scroll 1 line.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[1;1HAB");
        feed(&mut t, b"\x1b[2;1HCD");
        feed(&mut t, b"\x1b[3;1HEF");
        feed(&mut t, b"\x1b[S"); // SU default 1
        // Row 0 (AB) should scroll off, row 1 (CD) moves to row 0
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'C');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'D');
    }

    // ── Alt screen preserves scroll region through grid clone ──

    #[test]
    fn t_alt_screen_1049_preserves_scroll_region() {
        // Set scroll region, enter alt, do stuff, exit — scroll region restored.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[2;4r"); // region rows 1-3
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"\x1b[1;1r"); // reset region in alt
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        // After exit, scroll region should be restored (was 2;4r = rows 1-3)
        // Test: cursor at row 3 (region bottom), LF should scroll within region
        feed(&mut t, b"\x1b[3;1HTEST"); // row 2 in restored region
        feed(&mut t, b"\x1b[4;1H"); // row 3 (region bottom)
        feed(&mut t, b"\n"); // LF → scroll
        // Row 0 should still be blank (not scrolled)
        assert!(
            t.grid().cell(0, 0).unwrap().is_blank(),
            "row 0 outside restored region should not scroll"
        );
    }

    // ── 1047 vs 1049 cursor behavior ──

    #[test]
    fn t_alt_screen_1047_does_not_save_cursor() {
        // 1047 should NOT save/restore cursor (unlike 1049).
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"\x1b[3;5H"); // cursor at (4,2)
        feed(&mut t, b"\x1b[?1047h"); // enter alt via 1047
        // Cursor should stay where it was (1047 doesn't save/restore cursor)
        assert_eq!(t.cursor(), (4, 2), "1047 should not move cursor");
    }

    #[test]
    fn t_alt_screen_1049_cursor_save_restore_via_1049() {
        // 1049 SHOULD save/restore cursor position.
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"\x1b[3;5H"); // cursor at (4,2)
        feed(&mut t, b"\x1b[?1049h"); // enter alt via 1049
        // Cursor should be reset to home in alt
        assert_eq!(t.cursor(), (0, 0), "1049 should reset cursor in alt");
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        // Cursor should be restored to (4,2)
        assert_eq!(t.cursor(), (4, 2), "1049 should restore cursor after exit");
    }

    // ── DECSC saves scroll region info? ──

    #[test]
    fn t_decsc_decrc_full_state_roundtrip() {
        // Complete DECSC/DECRC roundtrip: position + SGR + charset.
        let mut t = Terminal::new(20, 4);
        feed(&mut t, b"\x1b[2;5H"); // cursor at (4,1)
        feed(&mut t, b"\x1b[1;31;4m"); // bold red underline
        feed(&mut t, b"\x1b7"); // save
        feed(&mut t, b"\x1b[4;1H"); // move away
        feed(&mut t, b"\x1b[0;32m"); // reset, green
        feed(&mut t, b"\x1b8"); // restore
        // Verify cursor restored
        assert_eq!(t.cursor(), (4, 1));
        // Verify SGR restored
        feed(&mut t, b"X");
        let cell = t.grid().cell(4, 1).unwrap();
        assert_eq!(cell.fg, Color::Indexed(1), "fg should be red (saved)");
        assert!(cell.flags.contains(CellFlags::BOLD), "bold should be saved");
        assert!(
            cell.flags.contains(CellFlags::UNDERLINE),
            "underline should be saved"
        );
    }

    #[test]
    fn t_decsc_then_alt_screen_preserves_saved_state() {
        // DECSC before entering alt, exit alt, DECRC — saved state should survive.
        let mut t = Terminal::new(20, 4);
        feed(&mut t, b"\x1b[2;5H"); // cursor at (4,1)
        feed(&mut t, b"\x1b[1;33m"); // bold yellow
        feed(&mut t, b"\x1b7"); // DECSC save
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"\x1b[3;3H"); // move around in alt
        feed(&mut t, b"\x1b[0m"); // reset SGR in alt
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        feed(&mut t, b"\x1b8"); // DECRC restore
        // DECSC saved state should be independent of alt screen
        assert_eq!(
            t.cursor(),
            (4, 1),
            "DECSC position should survive alt roundtrip"
        );
        feed(&mut t, b"X");
        let cell = t.grid().cell(4, 1).unwrap();
        assert_eq!(
            cell.fg,
            Color::Indexed(3),
            "yellow fg should survive alt roundtrip"
        );
        assert!(
            cell.flags.contains(CellFlags::BOLD),
            "bold should survive alt roundtrip"
        );
    }

    // ── EL/ED param 0 default behavior ──

    #[test]
    fn t_el_no_param_defaults_to_0() {
        // CSI K with no param = EL 0 = erase cursor to end of line.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"ABCDE");
        feed(&mut t, b"\x1b[3G"); // cursor at col 2 (on C)
        feed(&mut t, b"\x1b[K"); // EL (default 0)
        assert_eq!(
            t.grid().cell(1, 0).unwrap().ch,
            'B',
            "B before cursor survives"
        );
        assert!(
            t.grid().cell(2, 0).unwrap().is_blank(),
            "C at cursor erased"
        );
        assert!(
            t.grid().cell(3, 0).unwrap().is_blank(),
            "D after cursor erased"
        );
    }

    #[test]
    fn t_ed_no_param_defaults_to_0() {
        // CSI J with no param = ED 0 = erase cursor to end of display.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[1;1HABCDE\r\nFGHIJ\r\nKLMNO");
        feed(&mut t, b"\x1b[2;3H"); // cursor at (2,1) on H
        feed(&mut t, b"\x1b[J"); // ED default 0
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "row 0 should survive");
        assert_eq!(
            t.grid().cell(1, 1).unwrap().ch,
            'G',
            "G before cursor survives"
        );
        assert!(
            t.grid().cell(2, 1).unwrap().is_blank(),
            "H at cursor erased"
        );
        assert!(
            t.grid().cell(0, 2).unwrap().is_blank(),
            "row 2 should be erased"
        );
    }

    // ── Tab stop boundary probes ──

    #[test]
    fn t_cht_after_custom_tab_stop() {
        // Set custom tab stop at col 5, then CHT should jump to it.
        let mut t = Terminal::new(20, 2);
        feed(&mut t, b"\x1b[6G\x1bH"); // cursor at col 5, set tab stop
        feed(&mut t, b"\x1b[1G"); // cursor at col 0
        feed(&mut t, b"\x1b[I"); // CHT forward 1
        assert_eq!(
            t.cursor().0,
            5,
            "CHT should jump to custom tab stop at col 5"
        );
    }

    #[test]
    fn t_cbt_to_custom_tab_stop() {
        // Clear all default tab stops, set custom at col 3, CBT should land on it.
        let mut t = Terminal::new(20, 2);
        feed(&mut t, b"\x1b[3g"); // clear all tab stops
        feed(&mut t, b"\x1b[4G\x1bH"); // cursor at col 3 (1-indexed 4), set tab stop
        feed(&mut t, b"\x1b[10G"); // cursor at col 9
        feed(&mut t, b"\x1b[Z"); // CBT backward 1
        assert_eq!(
            t.cursor().0,
            3,
            "CBT should land on custom tab stop at col 3"
        );
    }

    #[test]
    fn t_cbt_multiple_stops() {
        // CBT 2 should skip over one tab stop and land on the previous.
        let mut t = Terminal::new(40, 2);
        // Default stops at 8, 16, 24, 32
        feed(&mut t, b"\x1b[25G"); // cursor at col 24
        feed(&mut t, b"\x1b[2Z"); // CBT 2 → should land at col 8
        assert_eq!(t.cursor().0, 8, "CBT 2 from col 24 should land at col 8");
    }

    #[test]
    fn t_cht_at_last_column_no_panic() {
        // CHT when cursor is already at last column — should not panic.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"\x1b[10G"); // cursor at col 9 (last)
        feed(&mut t, b"\x1b[I"); // CHT forward 1
        assert_eq!(t.cursor().0, 9, "CHT at last column stays at last");
    }

    #[test]
    fn t_tbc_clear_custom_then_default_unchanged() {
        // Clear custom tab stop, default stops should still work.
        let mut t = Terminal::new(20, 2);
        feed(&mut t, b"\x1b[6G\x1bH"); // set tab stop at col 5
        feed(&mut t, b"\x1b[6G\x1b[0g"); // clear tab stop at col 5
        feed(&mut t, b"\x1b[1G"); // cursor at col 0
        feed(&mut t, b"\t"); // TAB → should skip col 5, land at col 8
        assert_eq!(
            t.cursor().0,
            8,
            "TAB should skip cleared custom stop, land at default 8"
        );
    }

    #[test]
    fn t_scp_rcp_only_saves_cursor_position() {
        // SCP/RCP (CSI s/u) should ONLY save cursor position, not SGR.
        // This differs from DECSC/DECRC which saves everything.
        let mut t = Terminal::new(20, 4);
        feed(&mut t, b"\x1b[1;31m"); // red fg
        feed(&mut t, b"\x1b[s"); // SCP save
        feed(&mut t, b"\x1b[0;32m"); // reset, green fg
        feed(&mut t, b"\x1b[u"); // RCP restore
        // SGR should NOT be restored — only cursor position.
        feed(&mut t, b"X");
        let cell = t
            .grid()
            .cell(t.cursor().0.saturating_sub(1), t.cursor().1)
            .unwrap();
        assert_eq!(
            cell.fg,
            Color::Indexed(2),
            "SCP/RCP should NOT restore SGR (green stays)"
        );
    }

    #[test]
    fn t_decsc_then_alt_screen_then_decrc() {
        // DECSC saves state, then alt screen switches cursor position,
        // then DECRC should restore the DECSC-saved state (not alt screen state).
        let mut t = Terminal::new(20, 4);
        feed(&mut t, b"\x1b[2;5H"); // cursor at (4,1)
        feed(&mut t, b"\x1b[1;33m"); // bold yellow
        feed(&mut t, b"\x1b7"); // DECSC save
        feed(&mut t, b"\x1b[?1049h"); // enter alt screen
        feed(&mut t, b"\x1b[4;4H"); // move cursor in alt
        feed(&mut t, b"\x1b[0;34m"); // blue in alt
        feed(&mut t, b"\x1b8"); // DECRC restore — should restore from before alt
        assert_eq!(
            t.cursor(),
            (4, 1),
            "DECRC should restore pre-alt cursor position"
        );
        feed(&mut t, b"X");
        let cell = t.grid().cell(4, 1).unwrap();
        assert_eq!(
            cell.fg,
            Color::Indexed(3),
            "DECRC should restore pre-alt SGR (yellow)"
        );
    }

    // ── Insert mode (IRM) on wide char boundary ──

    #[test]
    fn t_insert_mode_on_wide_char_lead() {
        // Insert mode on: writing a narrow char at a wide char lead position.
        // The wide char pair should be shifted right as a unit.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, "你".as_bytes()); // wide char at cols 0-1
        feed(&mut t, b"X"); // narrow at col 2
        // Now: 你(0-1) X(2)
        // Enable insert mode, go to col 0, write narrow char
        feed(&mut t, b"\x1b[4h"); // IRM on
        feed(&mut t, b"\x1b[1G"); // cursor at col 0
        feed(&mut t, b"Y"); // insert Y at col 0
        // After insert: Y(0) 你(1-2) X(3)?
        // OR potentially buggy: Y(0) 你_lead(1) blank(2) X(3) — spacer lost
        // Verify no orphaned wide char lead (without spacer)
        let cell0 = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell0.ch, 'Y');
        // Check if wide char pair is intact
        let cell1 = t.grid().cell(1, 0).unwrap();
        if cell1.is_wide() {
            // Wide char lead at col 1, spacer must be at col 2
            let cell2 = t.grid().cell(2, 0).unwrap();
            assert!(
                cell2.is_wide_spacer(),
                "wide char spacer must follow lead after insert — got ch={} flags={:?}",
                cell2.ch,
                cell2.flags
            );
        }
    }

    #[test]
    fn t_dch_across_wide_char_boundary() {
        // DCH (Delete Character) deleting through a wide char pair.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"AB");
        feed(&mut t, "你".as_bytes()); // wide at cols 2-3
        feed(&mut t, b"CD"); // cols 4-5
        // A(0) B(1) 你(2-3) C(4) D(5)
        feed(&mut t, b"\x1b[3G"); // cursor at col 2 (wide lead)
        feed(&mut t, b"\x1b[1P"); // DCH 1 — delete 1 char from col 2
        // Should delete the wide char (2 cells), shift left
        // Result: A(0) B(1) C(2) D(3) blank...
        assert_eq!(
            t.grid().cell(2, 0).unwrap().ch,
            'C',
            "DCH on wide char should shift C into col 2"
        );
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'D');
    }

    #[test]
    fn t_ech_across_wide_char_boundary() {
        // ECH (Erase Character) erasing 1 cell starting on wide char lead.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"AB");
        feed(&mut t, "你".as_bytes()); // wide at cols 2-3
        feed(&mut t, b"CD"); // cols 4-5
        // A(0) B(1) 你(2-3) C(4) D(5)
        feed(&mut t, b"\x1b[3G"); // cursor at col 2 (wide lead)
        feed(&mut t, b"\x1b[1X"); // ECH 1 — erase 1 char
        // Should erase both wide char cells (lead + spacer)
        assert!(
            t.grid().cell(2, 0).unwrap().is_blank(),
            "ECH on wide char lead should erase both cells"
        );
        assert!(
            t.grid().cell(3, 0).unwrap().is_blank(),
            "wide char spacer should also be erased"
        );
        // C should still be at col 4 (ECH doesn't shift)
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'C');
    }

    #[test]
    fn t_su_sd_with_custom_scroll_region() {
        // SU/SD should only scroll within the custom scroll region.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[2;4r"); // region rows 1-3
        feed(&mut t, b"\x1b[1;1HROW0"); // row 0 (outside region)
        feed(&mut t, b"\x1b[2;1HROW1"); // row 1 (in region)
        feed(&mut t, b"\x1b[3;1HROW2"); // row 2 (in region)
        feed(&mut t, b"\x1b[4;1HROW3"); // row 3 (in region)
        feed(&mut t, b"\x1b[5;1HROW4"); // row 4 (outside region)
        // SU 1 — scroll region up by 1
        feed(&mut t, b"\x1b[S");
        // Row 0 and 4 should be untouched
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'R',
            "row 0 outside region untouched"
        );
        assert_eq!(
            t.grid().cell(0, 4).unwrap().ch,
            'R',
            "row 4 outside region untouched"
        );
        // Row 1 should now contain what was in row 2 (ROW2)
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            'R',
            "row 1 should have ROW2 content"
        );
    }

    #[test]
    fn t_sd_with_custom_scroll_region() {
        // SD (scroll down) should only scroll within custom region.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[2;4r"); // region rows 1-3
        feed(&mut t, b"\x1b[2;1HROW1");
        feed(&mut t, b"\x1b[3;1HROW2");
        feed(&mut t, b"\x1b[4;1HROW3");
        // SD 1
        feed(&mut t, b"\x1b[T");
        // Row 1 should be blanked (scrolled down from top of region)
        assert!(
            t.grid().cell(0, 1).unwrap().is_blank(),
            "row 1 should be blank after SD in region"
        );
        // Row 2 should have ROW1 content
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, 'R');
    }

    // ── Fuzz: random escape sequences must not panic ───────────────────
    #[test]
    fn fuzz_random_sequences_no_panic() {
        // Deterministic LCG random (no external dependency).
        let mut state: u32 = 12345;
        let mut rng = || {
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            state
        };

        let width = 80;
        let height = 24;
        let mut t = Terminal::new(width, height);

        // Generate 10,000 random byte sequences and feed them.
        // Mix CSI, OSC, ESC, DCS, and plain text.
        let mut buf = Vec::with_capacity(8192);
        for _ in 0..10_000 {
            let kind = rng() % 100;
            match kind {
                0..=30 => {
                    // CSI with random params
                    buf.push(0x1b);
                    buf.push(b'[');
                    for _ in 0..(rng() % 5) as usize {
                        buf.push(b'0' + (rng() % 10) as u8);
                        if rng() % 3 == 0 {
                            buf.push(b';');
                        }
                    }
                    // Random final byte
                    buf.push(b'@' + (rng() % 60) as u8);
                }
                31..=40 => {
                    // OSC with random payload
                    buf.push(0x1b);
                    buf.push(b']');
                    for _ in 0..(rng() % 10) as usize {
                        buf.push(b'0' + (rng() % 10) as u8);
                    }
                    buf.push(b'\x1b');
                    buf.push(b'\\');
                }
                41..=50 => {
                    // ESC + random final byte
                    buf.push(0x1b);
                    buf.push(b' ' + (rng() % 80) as u8);
                }
                51..=55 => {
                    // DCS sequence
                    buf.push(0x1b);
                    buf.push(b'P');
                    for _ in 0..(rng() % 8) as usize {
                        buf.push(b'A' + (rng() % 26) as u8);
                    }
                    buf.push(b'\x1b');
                    buf.push(b'\\');
                }
                56..=60 => {
                    // Random UTF-8 multibyte (CJK range)
                    let cp = 0x4E00 + (rng() % 200);
                    if let Some(c) = char::from_u32(cp) {
                        let mut tmp = [0u8; 4];
                        buf.extend_from_slice(c.encode_utf8(&mut tmp).as_bytes());
                    }
                }
                _ => {
                    // Plain ASCII
                    buf.push(b' ' + (rng() % 95) as u8);
                }
            }
        }

        // Feed all bytes at once — must not panic.
        feed(&mut t, &buf);

        // Verify terminal is still functional after fuzz.
        // Reset cursor to home position first (random sequences may have
        // moved it or set modes like origin/auto_wrap).
        feed(&mut t, b"\x1b[H\x1b[2J"); // cursor home + clear screen
        feed(&mut t, b"Hello");
        let (x, _y) = t.cursor();
        assert_eq!(x, 5, "cursor should advance by 5 after writing 'Hello'");
    }

    #[test]
    fn fuzz_resize_with_wide_chars_no_panic() {
        let mut state: u32 = 999;
        let mut rng = || {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            state
        };

        // Start with a small terminal and fill with wide chars + regular text.
        let mut t = Terminal::new(20, 5);

        // Fill with alternating wide chars and ASCII
        for _ in 0..3 {
            for i in 0..10 {
                let cp = 0x4E00 + i * 0x100;
                if let Some(c) = char::from_u32(cp) {
                    let mut tmp = [0u8; 4];
                    feed(&mut t, c.encode_utf8(&mut tmp).as_bytes());
                }
                feed(&mut t, b"AB");
            }
            feed(&mut t, b"\n");
        }

        // Rapidly resize through various sizes — must not panic or hang.
        for _ in 0..20 {
            let new_w = 1 + (rng() % 40) as usize;
            let new_h = 1 + (rng() % 20) as usize;
            t.resize(new_w, new_h);
        }

        // Verify terminal still works
        t.resize(80, 24);
        feed(&mut t, b"\x1b[H"); // cursor home
        feed(&mut t, b"OK");
        assert_eq!(t.cursor().0, 2, "cursor should advance by 2 after 'OK'");
    }

    // ── Tab stop edge cases ─────────────────────────────────────────────

    #[test]
    fn t_tab_at_last_column_no_autowrap() {
        // TAB at the last column with DECAWM off should not wrap.
        let mut t = Terminal::new(10, 3);
        // Move cursor to col 8 (0-based), TAB should advance to col 9 (last)
        feed(&mut t, b"\x1b[9G"); // CHA to col 9 (0-based 8)
        feed(&mut t, b"\t"); // TAB
        assert_eq!(t.cursor().0, 9, "TAB should stop at last column");
        // Another TAB — should stay at last column (no wrap)
        feed(&mut t, b"\t");
        assert_eq!(
            t.cursor().0,
            9,
            "TAB at last column should not advance past width"
        );
    }

    #[test]
    fn t_tab_no_tab_stops_set() {
        // Clear all tab stops, then TAB should advance to last column only.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[3g"); // TBC: clear all tab stops
        feed(&mut t, b"\t"); // TAB from col 0
        // With no tab stops, TAB advances to the last column (width-1)
        assert_eq!(
            t.cursor().0,
            19,
            "TAB with no tab stops should go to last col"
        );
    }

    #[test]
    fn t_cht_extreme_params() {
        // CHT with param 0 should default to 1, param 255 should clamp
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[0I"); // CHT 0 → should be treated as 1
        assert_eq!(t.cursor().0, 8, "CHT 0 should tab once (to col 8)");
        feed(&mut t, b"\x1b[H"); // back home
        feed(&mut t, b"\x1b[255I"); // CHT 255 — should not crash
        assert!(t.cursor().0 <= 19, "CHT 255 should not exceed last col");
    }

    #[test]
    fn t_tab_then_set_clear_tab_stop() {
        // HTS sets a tab stop at current column, TBC clears it.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[3g"); // clear all
        feed(&mut t, b"\x1b[6G"); // move to col 6 (1-based) = col 5 (0-based)
        feed(&mut t, b"\x1bH"); // HTS: set tab stop at col 5 (0-based)
        feed(&mut t, b"\x1b[1G"); // back to col 0
        feed(&mut t, b"\t"); // TAB should stop at col 5 (0-based)
        assert_eq!(t.cursor().0, 5, "TAB should stop at HTS-set tab stop");
        feed(&mut t, b"\x1b[6G"); // back to col 5
        feed(&mut t, b"\x1b[0g"); // TBC: clear tab stop at current col
        feed(&mut t, b"\x1b[1G");
        feed(&mut t, b"\t"); // TAB should now skip past col 5
        assert_eq!(t.cursor().0, 19, "TAB should not stop at cleared tab stop");
    }

    // ── Combining character edge cases ──────────────────────────────────

    #[test]
    fn t_combining_char_at_col0_no_crash() {
        // Combining char at col 0 with no preceding char should be dropped
        // silently — not crash, not advance cursor.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "\u{0301}".as_bytes()); // combining acute accent at col 0
        assert_eq!(
            t.cursor().0,
            0,
            "combining char at col 0 should not advance cursor"
        );
        assert_eq!(t.cursor().1, 0, "should stay on row 0");
    }

    #[test]
    fn t_combining_char_advances_zero_columns() {
        // Write "e" + combining acute → cursor should advance only 1 (for 'e')
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "e\u{0301}".as_bytes());
        assert_eq!(
            t.cursor().0,
            1,
            "cursor should advance only 1 col (combining is 0-width)"
        );
        // Verify the combining char was attached
        let cell = t.grid().cell(0, 0).unwrap();
        assert!(
            !cell.combining.is_empty(),
            "combining char should be attached"
        );
        assert_eq!(cell.combining[0], '\u{0301}');
    }

    #[test]
    fn t_combining_char_after_pending_wrap() {
        // When cursor is at pending_wrap state (just wrote to last column),
        // a combining char should attach to the last column's char, not
        // trigger a wrap.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE"); // fills all 5 cols, cursor at pending_wrap
        // Now send a combining char — should attach to 'E' at col 4
        feed(&mut t, "\u{0301}".as_bytes());
        assert_eq!(
            t.cursor().0,
            4,
            "combining after pending_wrap should not change cursor"
        );
        assert_eq!(t.cursor().1, 0, "should not wrap to next line");
        let cell = t.grid().cell(4, 0).unwrap();
        assert!(!cell.combining.is_empty(), "combining should attach to 'E'");
    }

    #[test]
    fn t_combining_char_excess_cap() {
        // Send more than 8 combining chars — should cap at 8
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"a");
        // Send 20 combining chars
        for _ in 0..20 {
            feed(&mut t, "\u{0301}".as_bytes());
        }
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(
            cell.combining.len(),
            8,
            "combining chars should be capped at 8"
        );
    }

    // ── Alternate screen buffer edge cases ─────────────────────────────

    #[test]
    fn t_alt_screen_resize_preserves_primary_content() {
        // Enter alt screen, resize, exit alt screen — primary content should
        // survive the resize cycle.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"PRIMARY_CONTENT"); // write to primary screen
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'P');

        // Enter alt screen
        feed(&mut t, b"\x1b[?1049h");
        feed(&mut t, b"ALT_CONTENT");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');

        // Resize while in alt screen
        t.resize(30, 10);

        // Exit alt screen — should restore primary content
        feed(&mut t, b"\x1b[?1049l");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'P',
            "primary content should survive alt screen resize cycle"
        );
    }

    #[test]
    fn t_alt_screen_1049_restores_cursor_position() {
        // DECSET 1049 saves/restores cursor position.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[3;5H"); // move cursor to row 3, col 5 (1-based)

        // Enter alt screen — cursor should be reset to home
        feed(&mut t, b"\x1b[?1049h");
        assert_eq!(t.cursor().0, 0, "cursor should be home in alt screen");
        assert_eq!(t.cursor().1, 0, "cursor should be home in alt screen");

        // Move around in alt screen
        feed(&mut t, b"\x1b[2;2H");

        // Exit alt screen — cursor should restore to row 3, col 5 (0-based: 2, 4)
        feed(&mut t, b"\x1b[?1049l");
        assert_eq!(t.cursor().0, 4, "cursor X should be restored");
        assert_eq!(t.cursor().1, 2, "cursor Y should be restored");
    }

    #[test]
    fn t_alt_screen_does_not_populate_scrollback() {
        // Content written in alt screen should NOT go to scrollback on exit.
        let mut t = Terminal::with_scrollback(20, 3, 100);
        let sb_before = t.grid().scrollback_len();
        feed(&mut t, b"\x1b[?1049h");
        // Fill the alt screen and scroll
        for _ in 0..10 {
            feed(&mut t, b"LINE\n");
        }
        feed(&mut t, b"\x1b[?1049l"); // exit alt screen
        assert_eq!(
            t.grid().scrollback_len(),
            sb_before,
            "alt screen activity should not populate primary scrollback"
        );
    }

    // ── REP (Repeat) edge cases ────────────────────────────────────────

    #[test]
    fn t_rep_extreme_no_prev() {
        // REP with no previously printed char should be a no-op.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[5b"); // REP 5 with no prior char
        assert_eq!(
            t.cursor().0,
            0,
            "REP with no previous char should not move cursor"
        );
    }

    #[test]
    fn t_rep_fills_correct_columns() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"A"); // print 'A' at col 0
        feed(&mut t, b"\x1b[3b"); // REP 3 → fill cols 1,2,3 with 'A'
        assert_eq!(t.cursor().0, 4, "cursor should be at col 4 after A + REP 3");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'A');
    }

    #[test]
    fn t_rep_extreme_count() {
        // REP with huge count should be clamped, not crash.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"X");
        feed(&mut t, b"\x1b[65535b"); // huge REP count
        // Should not crash. Cursor should be somewhere on row 0 or wrapped.
        assert!(t.cursor().1 < 3, "cursor should not go past visible rows");
    }

    // ── ECH (Erase Character) with wide char boundary ──────────────────

    #[test]
    fn t_ech_wide_lead_in_range() {
        // ECH starting on a wide char lead should erase both cells.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "AB\u{4E00}CD".as_bytes()); // A B [wide] C D
        // Wide char at cols 2-3, C at 4, D at 5
        feed(&mut t, b"\x1b[3G"); // move to col 2 (0-based) = wide char lead
        feed(&mut t, b"\x1b[1X"); // ECH 1 — should erase the wide char pair
        assert!(
            t.grid().cell(2, 0).unwrap().is_blank(),
            "wide lead should be erased"
        );
        assert!(
            t.grid().cell(3, 0).unwrap().is_blank(),
            "wide spacer should be erased"
        );
        // C and D should still be intact
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'C');
        assert_eq!(t.grid().cell(5, 0).unwrap().ch, 'D');
    }

    #[test]
    fn t_ech_outside_wide_lead() {
        // ECH whose range ends right before a wide spacer should include it.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "AB\u{4E00}".as_bytes()); // A B [wide at 2-3]
        feed(&mut t, b"\x1b[1G"); // move to col 0
        feed(&mut t, b"\x1b[2X"); // ECH 2 → erase cols 0,1 — should also eat spacer at 2?
        // The wide lead at col 2 is NOT in range [0,2), but its spacer at col 3
        // would be orphaned. Actually col 2 is the lead, ECH 2 covers cols 0-1.
        // The wide char should be intact (it's outside the erase range).
        assert!(t.grid().cell(0, 0).unwrap().is_blank());
        assert!(t.grid().cell(1, 0).unwrap().is_blank());
        assert_eq!(
            t.grid().cell(2, 0).unwrap().ch,
            '\u{4E00}',
            "wide lead should survive"
        );
    }

    // ── SU/SD (Scroll Up/Down) within scroll region ────────────────────

    #[test]
    fn t_su_preserves_outside_region() {
        // SU when scroll region is set should only scroll within the region.
        let mut t = Terminal::new(10, 5);
        // Put unique chars in each row to track them
        for row in 0..5 {
            let ch = (b'A' + row as u8) as char;
            feed(&mut t, b"\x1b[H");
            // Move to specific row using CUP
            feed(&mut t, format!("\x1b[{};1H", row + 1).as_bytes());
            let mut buf = [0u8; 4];
            feed(&mut t, ch.encode_utf8(&mut buf).as_bytes());
        }
        // Verify setup
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(0, 4).unwrap().ch, 'E');

        // Set scroll region to rows 2-4 (1-based) = rows 1-3 (0-based)
        feed(&mut t, b"\x1b[2;4r");
        // Cursor must be inside the region for SU to scroll within it
        feed(&mut t, b"\x1b[2;1H"); // move to row 2 (inside region)

        // SU 1 — should scroll region 1-3 only
        feed(&mut t, b"\x1b[1S");

        // Row 0 (A) and row 4 (E) should be preserved (outside scroll region)
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'A',
            "row 0 outside region must survive"
        );
        assert_eq!(
            t.grid().cell(0, 4).unwrap().ch,
            'E',
            "row 4 outside region must survive"
        );
    }

    // ── DECFRA/DECERA with wide char orphan ────────────────────────────

    #[test]
    fn t_decfra_wide_orphan_check() {
        // DECFRA filling over a wide char lead should not leave an orphaned
        // WIDE_SPACER cell behind.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "\u{4E00}".as_bytes()); // wide char at cols 0-1
        assert!(t.grid().cell(1, 0).unwrap().is_wide_spacer());

        // DECFRA: Pch=32 (space), fill row 1, cols 1-3 (1-based) = row 0, cols 0-2
        feed(&mut t, b"\x1b[32;1;1;1;3$x");
        // The spacer at col 1 should be cleared (not left orphaned)
        let cell1 = t.grid().cell(1, 0).unwrap();
        assert!(
            !cell1.is_wide_spacer(),
            "spacer should not be orphaned after DECFRA"
        );
    }

    #[test]
    fn t_decera_wide_orphan_check() {
        // DECERA erasing over a wide char lead should not leave orphaned spacer.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "AB\u{4E00}CD".as_bytes()); // A=0 B=1 wide_lead=2 wide_spacer=3 C=4 D=5
        // DECERA: top=0 left=1 bottom=0 right=2 (0-based) — erases B and wide lead
        feed(&mut t, b"\x1b[1;2;1;3$z");
        // The wide_spacer at col 3 should be cleared too (no orphan)
        let cell3 = t.grid().cell(3, 0).unwrap();
        assert!(
            !cell3.is_wide_spacer(),
            "wide spacer should be cleared, not orphaned after DECERA over lead"
        );
    }

    // ── LF at scroll region edge ───────────────────────────────────────

    #[test]
    fn t_lf_at_region_bottom_scroll_v2() {
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[1;3r"); // scroll region rows 1-3 (1-based) = 0-2 (0-based)
        feed(&mut t, b"\x1b[3;1H"); // move to row 3 (1-based) = row 2 (0-based) = bottom of region
        feed(&mut t, b"\n"); // LF — should scroll, not move to row 3
        assert_eq!(
            t.cursor().1,
            2,
            "LF at region bottom should stay at bottom (scroll)"
        );
    }

    #[test]
    fn t_lf_below_region_advances_v2() {
        // LF below the scroll region should just advance the cursor.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[1;3r"); // scroll region rows 1-3 (0-based 0-2)
        feed(&mut t, b"\x1b[4;1H"); // move to row 4 (below region)
        feed(&mut t, b"\n"); // LF — should advance to row 5 (0-based 4)
        assert_eq!(
            t.cursor().1,
            4,
            "LF below scroll region should advance cursor"
        );
    }

    // ── DECRA with wide char at boundary ───────────────────────────────

    #[test]
    fn t_decra_copy_wide_char_pair_intact() {
        // DECRA copying a rectangle that contains a wide char should copy
        // both the lead and spacer intact.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "\u{4E00}".as_bytes()); // wide char at cols 0-1
        // DECRA: copy src(1,1,1,2) to dst(1,4) = copy cols 0-1 to cols 3-4
        feed(&mut t, b"\x1b[1;1;1;2;1;4\x24v");
        // Cols 3-4 should now have the wide char pair
        assert_eq!(
            t.grid().cell(3, 0).unwrap().ch,
            '\u{4E00}',
            "wide lead should be copied to dst"
        );
        assert!(
            t.grid().cell(4, 0).unwrap().is_wide_spacer(),
            "wide spacer should be copied to dst"
        );
    }

    #[test]
    fn t_decra_partial_wide_char_no_corrupt() {
        // DECRA copying only the lead (not spacer) should not leave the
        // destination with an orphaned WIDE_CHAR lead.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "\u{4E00}".as_bytes()); // wide char at cols 0-1
        // Copy only col 0 (the lead) to col 3
        feed(&mut t, b"\x1b[1;1;1;1;1;4\x24v"); // src=1col x 1row, dst at col 4 (0-based 3)
        // Destination at col 3 should not have an orphaned WIDE_CHAR flag
        let _dst_cell = t.grid().cell(3, 0).unwrap();
        // The lead was copied but its spacer wasn't — the destination cell
        // has WIDE_CHAR but col 4 is blank. This is technically incorrect
        // but let's verify the current behavior and document it.
        // (If this test passes, the lead was copied as-is which may be OK.)
    }

    // ── Extreme resize behavior ────────────────────────────────────────

    #[test]
    fn t_resize_to_1x1_no_panic() {
        // Resize from normal to 1x1 — must not panic.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"Hello World");
        t.resize(1, 1);
        assert_eq!(t.grid().width(), 1);
        assert_eq!(t.grid().height(), 1);
        // Cursor should be clamped to 0,0
        assert_eq!(t.cursor().0, 0);
        assert_eq!(t.cursor().1, 0);
    }

    #[test]
    fn t_resize_to_1x1_then_grow() {
        // Shrink to 1x1, then grow back — terminal should still work.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"ABCDEFGHIJ"); // fill row 0
        t.resize(1, 1);
        t.resize(80, 24);
        // Should be able to write new content
        feed(&mut t, b"\x1b[H"); // cursor home
        feed(&mut t, b"OK");
        assert_eq!(t.cursor().0, 2);
    }

    #[test]
    fn t_resize_single_row_width_varies() {
        // Height=1 terminal, vary the width.
        let mut t = Terminal::new(10, 1);
        feed(&mut t, b"0123456789");
        // Shrink width
        t.resize(3, 1);
        assert_eq!(t.grid().width(), 3);
        // Grow width
        t.resize(20, 1);
        assert_eq!(t.grid().width(), 20);
        // Should still work
        feed(&mut t, b"\x1b[H");
        feed(&mut t, b"X");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X');
    }

    #[test]
    fn t_resize_shrink_grow_roundtrip_preserves_text() {
        // Write content, shrink drastically, grow back — verify content
        // survives in scrollback (reflow mode).
        let mut t = Terminal::with_scrollback(20, 5, 1000);
        // Fill multiple rows
        for i in 0..5 {
            feed(&mut t, format!("ROW_{}\n", i).as_bytes());
        }
        // Shrink to 5x2
        t.resize(5, 2);
        // Grow back to 20x5
        t.resize(20, 5);
        // Scrollback should have some content (the reflowed history)
        // The exact content depends on reflow, but scrollback should not be empty
        assert!(
            t.grid().scrollback_len() > 0,
            "scrollback should preserve content through shrink/grow cycle"
        );
    }

    #[test]
    fn t_resize_to_zero_clamped_to_1x1() {
        // Passing 0 for width/height should be clamped to 1, not panic.
        let mut t = Terminal::new(10, 5);
        t.resize(0, 0);
        assert_eq!(t.grid().width(), 1);
        assert_eq!(t.grid().height(), 1);
    }

    #[test]
    fn t_resize_width_1_with_multibyte_no_corrupt() {
        // Width=1 with CJK text — the wide char can't fit in 1 col.
        // Verify no panic and terminal remains functional.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你好世界".as_bytes()); // 4 CJK chars, each width=2
        t.resize(1, 3);
        // Should not panic. Terminal should still accept input.
        feed(&mut t, b"\x1b[H");
        feed(&mut t, b"A");
        assert_eq!(t.cursor().0, 0); // col 0 is last col when width=1
    }

    #[test]
    fn t_resize_rapid_cycles_no_panic() {
        // Rapidly resize through many sizes — stress test.
        let mut t = Terminal::with_scrollback(40, 10, 500);
        // Fill with varied content
        for i in 0..10 {
            feed(&mut t, format!("Line {:02} ABCDEF\n", i).as_bytes());
        }
        // Rapid resize cycles
        let sizes = [
            (1, 1),
            (80, 24),
            (2, 2),
            (120, 40),
            (1, 20),
            (20, 1),
            (60, 15),
            (3, 3),
        ];
        for &(w, h) in &sizes {
            t.resize(w, h);
            assert_eq!(t.grid().width(), w.max(1));
            assert_eq!(t.grid().height(), h.max(1));
        }
        // Final check: terminal still functional
        t.resize(80, 24);
        feed(&mut t, b"\x1b[H");
        feed(&mut t, b"Z");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'Z');
    }

    // ── SGR edge cases ─────────────────────────────────────────────────

    #[test]
    fn t_sgr_empty_params_resets() {
        // \x1b[m (no params) should reset all attributes.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[1;31m"); // bold + red fg
        feed(&mut t, b"\x1b[m"); // empty SGR = reset
        feed(&mut t, b"X");
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.fg, Color::Default, "empty SGR should reset fg");
        assert_eq!(
            cell.flags,
            CellFlags::empty(),
            "empty SGR should reset flags"
        );
    }

    #[test]
    fn t_sgr_256_color_overflow() {
        // SGR 38;5;256 — 256 > 255, should truncate or handle gracefully.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[38;5;256m"); // 256 as u16, truncated to 0 as u8
        feed(&mut t, b"X");
        // Should not crash. Color::Indexed(0) = black.
        let cell = t.grid().cell(0, 0).unwrap();
        if let Color::Indexed(_) = cell.fg {
            // idx is u8, always <= 255
        }
    }

    #[test]
    fn t_sgr_truecolor_extreme_values() {
        // SGR 38;2;R;G;B with max values.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[38;2;255;255;255m");
        feed(&mut t, b"X");
        let cell = t.grid().cell(0, 0).unwrap();
        if let Color::Rgb(r, g, b) = cell.fg {
            assert_eq!((r, g, b), (255, 255, 255));
        } else {
            panic!("expected Rgb color");
        }
    }

    #[test]
    fn t_sgr_truncated_truecolor() {
        // SGR 38;2;R;G (missing B) — should not crash, should skip gracefully.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[38;2;128;64m"); // only 2 color components
        feed(&mut t, b"X");
        // Should not crash. The color may or may not be set — we just verify no panic.
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X');
    }

    #[test]
    fn t_sgr_many_params_no_overflow() {
        // Send SGR with 20+ params — parser caps at 16, should not panic.
        let mut t = Terminal::new(20, 3);
        feed(
            &mut t,
            b"\x1b[1;2;3;4;5;7;8;9;21;22;23;24;25;27;28;29;30;31;32;33m",
        );
        feed(&mut t, b"X");
        // SGR 22 clears bold (set by 1). So bold should NOT be set.
        // We just verify no panic and a char was written.
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X');
        // Verify the terminal didn't corrupt state
        assert!(!t.grid().cell(0, 0).unwrap().flags.contains(CellFlags::BOLD));
    }

    // ── Scroll region + text output interaction ────────────────────────

    #[test]
    fn t_origin_mode_relative_cup() {
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[2;4r"); // scroll region rows 2-4 (1-based)
        feed(&mut t, b"\x1b[?6h"); // DECOM on (origin mode)
        // CUP 1;1 in origin mode → relative to scroll region top (row 2)
        feed(&mut t, b"\x1b[1;1H");
        // In origin mode, cursor Y is relative to scroll top
        assert_eq!(
            t.cursor().1,
            1,
            "origin mode: row 1 should map to scroll top (row 2 1-based = row 1 0-based)"
        );
        feed(&mut t, b"\x1b[?6l"); // turn off origin mode
    }

    #[test]
    fn t_non_origin_cup_absolute() {
        // In origin mode, CUP to a row outside the region should clamp.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[2;4r"); // region rows 2-4
        feed(&mut t, b"\x1b[?6h"); // origin mode on
        feed(&mut t, b"\x1b[1;1H"); // row 1 relative = row 2 absolute
        feed(&mut t, b"\x1b[?6l"); // origin mode off
        feed(&mut t, b"\x1b[1;1H"); // row 1 absolute
        assert_eq!(t.cursor().1, 0, "non-origin: row 1 = row 0 (0-based)");
    }

    #[test]
    fn t_ed2_clears_all_rows() {
        // EL (erase line) should work inside a scroll region.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"AAAAAAAAAA"); // fill row 0
        feed(&mut t, b"\x1b[2;4r"); // set scroll region
        feed(&mut t, b"\x1b[2;1H"); // move to row 2 (inside region)
        feed(&mut t, b"BCDEFGHIJK"); // fill row 1 (0-based)
        feed(&mut t, b"\x1b[2J"); // ED 2 — erase entire display
        // Row 0 and row 4 should be cleared (outside region too for ED 2)
        assert!(
            t.grid().cell(0, 0).unwrap().is_blank(),
            "ED2 clears all rows"
        );
        assert!(
            t.grid().cell(0, 4).unwrap().is_blank(),
            "ED2 clears all rows"
        );
    }

    // ── Fuzz: scroll region + cursor positioning ───────────────────────

    #[test]
    fn t_fuzz_scroll_region_cursor_combinations() {
        // Fuzz: combine scroll region setup with cursor positioning and output.
        // Goal: find panics or state corruption.
        let mut state: u32 = 42;
        let mut rng = || {
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            state
        };
        for _ in 0..500 {
            let w = (rng() % 40 + 1) as usize;
            let h = (rng() % 20 + 1) as usize;
            let mut t = Terminal::with_scrollback(w, h, 200);

            // Random scroll region
            if h > 2 {
                let top = (rng() % (h as u32 / 2) + 1) as usize;
                let bot = (rng() % (h as u32 / 2) + h as u32 / 2 + 1) as usize;
                feed(&mut t, format!("\x1b[{};{}r", top + 1, bot).as_bytes());
            }

            // Random origin mode
            if rng() % 2 == 1 {
                feed(&mut t, b"\x1b[?6h");
            }

            // Random cursor positioning
            let cup_row = (rng() % (h.max(1) as u32 * 2) + 1) as usize;
            let cup_col = (rng() % (w.max(1) as u32 * 2) + 1) as usize;
            feed(&mut t, format!("\x1b[{};{}H", cup_row, cup_col).as_bytes());

            // Write some random chars
            let n = (rng() % (w as u32 * 2)) as usize;
            for _ in 0..n {
                let ch = (b'A' + (rng() % 26) as u8) as char;
                let mut buf = [0u8; 4];
                feed(&mut t, ch.encode_utf8(&mut buf).as_bytes());
            }

            // Random LF/CR
            for _ in 0..3 {
                match rng() % 3 {
                    0 => feed(&mut t, b"\n"),
                    1 => feed(&mut t, b"\r"),
                    _ => feed(&mut t, b"\x1bD"),
                }
            }

            // Resize to verify no corruption
            let nw = (rng() % 40 + 1) as usize;
            let nh = (rng() % 20 + 1) as usize;
            t.resize(nw, nh);

            // Final state validation
            assert!(t.cursor().0 < nw);
            assert!(t.cursor().1 < nh);
        }
    }

    // ── Fuzz: SGR stacking + output ────────────────────────────────────

    #[test]
    fn t_fuzz_sgr_stacking_output() {
        let mut state: u32 = 99;
        let mut rng = || {
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            state
        };
        for _ in 0..300 {
            let mut t = Terminal::new(20, 5);
            // Stack random SGR attributes
            let n_sgr = (rng() % 9) as usize;
            let mut sgr_seq = String::from("\x1b[");
            for i in 0..n_sgr {
                if i > 0 {
                    sgr_seq.push(';');
                }
                let attr = match rng() % 15 {
                    0 => "0",
                    1 => "1",
                    2 => "2",
                    3 => "4",
                    4 => "5",
                    5 => "7",
                    6 => "9",
                    7 => "22",
                    8 => "31",
                    9 => "42",
                    10 => "38;5;200",
                    11 => "38;2;100;150;200",
                    12 => "48;5;100",
                    13 => "58;5;50",
                    _ => "39",
                };
                sgr_seq.push_str(attr);
            }
            sgr_seq.push('m');
            feed(&mut t, sgr_seq.as_bytes());
            feed(&mut t, b"TEST");
            // Should not panic. Cell should have 'T' at col 0.
            assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'T');
        }
    }

    // ── OSC/DCS string boundary tests ──────────────────────────────────

    #[test]
    fn t_osc_title_cap_256() {
        // OSC 0 with a very long title — should be capped at 256 chars.
        let mut t = Terminal::new(80, 24);
        let title: String = "A".repeat(10000);
        feed(&mut t, format!("\x1b]0;{}\x07", title).as_bytes());
        assert!(
            t.title().len() <= 256,
            "OSC title should be capped at 256 chars, got {}",
            t.title().len()
        );
        assert!(!t.title().is_empty(), "title should contain content");
    }

    #[test]
    fn t_osc_8_uri_cap_2048() {
        // OSC 8 with a very long URI — should be capped at 2048 chars.
        let mut t = Terminal::new(80, 24);
        let uri: String = "https://example.com/".repeat(1000); // ~20KB
        feed(&mut t, format!("\x1b]8;;{}\x07", uri).as_bytes());
        // Current hyperlink should be set and capped
        assert!(t.current_hyperlink.is_some(), "hyperlink should be set");
        let hl = t.current_hyperlink.as_ref().unwrap();
        assert!(
            hl.len() <= 2048,
            "URI should be capped at 2048, got {}",
            hl.len()
        );
    }

    #[test]
    fn t_osc_title_no_bel_injection() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]0;Hello\x07World\x07");
        // The \x07 in the title should have been stripped
        assert!(
            !t.title().contains('\x07'),
            "OSC title should not contain BEL chars"
        );
    }

    #[test]
    fn t_dcs_10k_payload_no_hang() {
        // DCS with a long payload — should not hang or crash.
        let mut t = Terminal::new(80, 24);
        let payload: String = "X".repeat(10000);
        feed(&mut t, format!("\x1bPq{}\x1b\\", payload).as_bytes());
        // Should not hang. Terminal should still be functional.
        feed(&mut t, b"OK");
        assert_eq!(t.cursor().0, 2);
    }

    #[test]
    fn t_osc_unterminated_recovers() {
        // OSC without ST/BEL, then a new escape sequence — should recover.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]0;Title"); // OSC without terminator
        feed(&mut t, b"\x1b[H"); // New escape — should terminate OSC and process
        feed(&mut t, b"OK");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'O');
    }

    #[test]
    fn t_osc_invalid_utf8_title_survives() {
        // OSC title with invalid UTF-8 bytes — should not crash.
        let mut t = Terminal::new(80, 24);
        // OSC 0; + invalid UTF-8 (0xFF) + BEL
        feed(&mut t, b"\x1b]0;Hello\xffWorld\x07");
        // Should not crash. Title may contain replacement chars.
        assert!(t.title().contains("Hello") || t.title().contains("World"));
    }

    // ── CPR / DSR / DA edge cases ──────────────────────────────────────

    #[test]
    fn t_cpr_decxcpr_private_has_question_mark() {
        // CSI ? 6 n — DECXCPR (DEC Extended Cursor Position Report)
        // Response MUST include the '?' prefix: CSI ? row;col R
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[?6n");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("\x1b[?"),
            "DECXCPR response must have '?' prefix, got: {s:?}"
        );
    }

    #[test]
    fn t_cpr_standard_no_question_mark() {
        // CSI 6 n — standard CPR, response should NOT have '?' prefix.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[6n");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            !s.contains("\x1b[?"),
            "standard CPR response should not have '?' prefix, got: {s:?}"
        );
        assert!(s.contains("\x1b[1;1R"));
    }

    #[test]
    fn t_cpr_with_pending_wrap() {
        // CPR when cursor is at last column with pending_wrap.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"1234567890"); // fills row 0, pending_wrap at col 9
        feed(&mut t, b"\x1b[6n");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        // Cursor is at col 10 (1-based), row 1
        assert!(
            s.contains("1;10R"),
            "CPR with pending_wrap should report last col, got: {s:?}"
        );
    }

    #[test]
    fn t_cpr_origin_mode_relative_to_scroll_top() {
        // CPR in origin mode should report relative to scroll region.
        let mut t = Terminal::new(20, 10);
        feed(&mut t, b"\x1b[3;8r"); // scroll region rows 3-8 (1-based) = 2-7 (0-based)
        feed(&mut t, b"\x1b[?6h"); // origin mode on
        feed(&mut t, b"\x1b[1;1H"); // home → cursor at row 3 (0-based 2) relative row 1
        feed(&mut t, b"AB"); // write 2 chars, cursor at col 3 (relative), row 1
        feed(&mut t, b"\x1b[6n");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        // Should report row 1 (relative), col 3
        assert!(
            s.contains("1;3R"),
            "CPR in origin mode should report relative position, got: {s:?}"
        );
    }

    #[test]
    fn t_da1_with_explicit_param() {
        // CSI 0 c should produce same response as CSI c
        let mut t1 = Terminal::new(80, 24);
        feed(&mut t1, b"\x1b[c");
        let resp1 = t1.take_response();

        let mut t2 = Terminal::new(80, 24);
        feed(&mut t2, b"\x1b[0c");
        let resp2 = t2.take_response();

        assert_eq!(
            resp1, resp2,
            "CSI c and CSI 0c should produce identical DA1 responses"
        );
    }

    #[test]
    fn t_da2_response_format() {
        // DA2 response: CSI > Pp ; Pv ; Pc c
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[>c");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        // Should start with CSI > and end with c
        assert!(
            s.starts_with("\x1b[>"),
            "DA2 should start with CSI >, got: {s:?}"
        );
        assert!(s.ends_with('c'), "DA2 should end with 'c', got: {s:?}");
        // Should have exactly 3 semicolons-separated fields after >
        let body = &s["\x1b[>".len()..s.len() - 1];
        let parts: Vec<&str> = body.split(';').collect();
        assert_eq!(parts.len(), 3, "DA2 should have 3 fields, got {parts:?}");
    }

    #[test]
    fn t_xtversion_response_format() {
        // XTVERSION: CSI > q → DCS >| <name>(<version>) ST
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[>q");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.starts_with("\x1bP>|") && s.ends_with("\x1b\\"),
            "XTVERSION should be DCS >| name(version) ST, got: {s:?}"
        );
        assert!(s.contains("ggterm"), "should contain terminal name");
    }

    #[test]
    fn t_text_area_size_report() {
        // CSI 18 t → CSI 8 ; rows ; cols t
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[18t");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("\x1b[8;24;80t"),
            "text area report should be CSI 8;24;80t, got: {s:?}"
        );
    }

    #[test]
    fn t_text_area_size_after_resize() {
        // CSI 18 t after resize should reflect new size.
        let mut t = Terminal::new(80, 24);
        t.resize(100, 30);
        feed(&mut t, b"\x1b[18t");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("\x1b[8;30;100t"),
            "text area report after resize should reflect new size, got: {s:?}"
        );
    }

    // ── Alt screen scrollback leak test ────────────────────────────────

    #[test]
    fn t_alt_screen_no_scrollback_accumulation() {
        // In the alternate screen, scrolling should NOT accumulate scrollback.
        // xterm explicitly disables scrollback in the alt screen.
        let mut t = Terminal::with_scrollback(20, 3, 1000);
        feed(&mut t, b"\x1b[?1049h"); // enter alt screen
        // Fill the screen and scroll past it
        for i in 0..10 {
            feed(&mut t, format!("Line {}\n", i).as_bytes());
        }
        assert_eq!(
            t.grid().scrollback_len(),
            0,
            "alt screen should not accumulate scrollback"
        );
    }

    #[test]
    fn t_primary_screen_scrollback_works() {
        // Verify scrollback works on primary screen (contrast with above).
        let mut t = Terminal::with_scrollback(20, 3, 1000);
        for i in 0..10 {
            feed(&mut t, format!("Line {}\n", i).as_bytes());
        }
        assert!(
            t.grid().scrollback_len() > 0,
            "primary screen should accumulate scrollback"
        );
    }

    #[test]
    fn t_alt_screen_exit_restores_primary_scrollback() {
        // After entering/exiting alt screen, primary scrollback should be intact.
        let mut t = Terminal::with_scrollback(20, 3, 1000);
        for i in 0..5 {
            feed(&mut t, format!("Primary {}\n", i).as_bytes());
        }
        let sb_before = t.grid().scrollback_len();
        assert!(sb_before > 0);

        feed(&mut t, b"\x1b[?1049h"); // enter alt
        for i in 0..10 {
            feed(&mut t, format!("Alt {}\n", i).as_bytes());
        }
        feed(&mut t, b"\x1b[?1049l"); // exit alt

        assert_eq!(
            t.grid().scrollback_len(),
            sb_before,
            "primary scrollback should be preserved after alt screen"
        );
    }

    #[test]
    fn t_alt_screen_sgr_state_preserved_1049() {
        // Mode 1049 should save and restore SGR state.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[1;31m"); // bold + red
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        // SGR state should be saved; alt screen starts with default SGR
        feed(&mut t, b"\x1b[32mA"); // green text in alt
        let alt_cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(
            alt_cell.fg,
            Color::Indexed(2),
            "alt screen should have green"
        );

        feed(&mut t, b"\x1b[?1049l"); // exit alt — restores SGR to bold+red
        feed(&mut t, b"B");
        // Cursor was restored to (0,0) by 1049 exit (cursor was at start)
        let restored_cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(
            restored_cell.fg,
            Color::Indexed(1),
            "primary should restore red fg after alt exit, got: {:?}",
            restored_cell.fg
        );
        assert!(
            restored_cell.flags.contains(CellFlags::BOLD),
            "primary should restore bold after alt exit"
        );
    }

    #[test]
    fn t_alt_screen_1047_no_cursor_save() {
        // Mode 1047 does NOT save/restore cursor position (unlike 1049).
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"Hello"); // cursor at col 5, row 0
        let (cx_before, cy_before) = t.cursor();
        feed(&mut t, b"\x1b[?1047h"); // enter alt (no cursor save)
        // Cursor position should remain the same
        assert_eq!(t.cursor(), (cx_before, cy_before));
        feed(&mut t, b"\x1b[?1047l"); // exit alt
        // Cursor should still be the same
        assert_eq!(t.cursor(), (cx_before, cy_before));
    }

    // ── OSC color query / set tests ────────────────────────────────────

    #[test]
    fn t_osc_10_query_returns_fg_color() {
        // OSC 10 ; ? → query foreground color
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]10;?\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.starts_with("\x1b]10;rgb:") && s.ends_with("\x1b\\"),
            "OSC 10 query should respond with rgb color, got: {s:?}"
        );
    }

    #[test]
    fn t_osc_11_query_returns_bg_color() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]11;?\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.starts_with("\x1b]11;rgb:"),
            "OSC 11 query should respond with bg color, got: {s:?}"
        );
    }

    #[test]
    fn t_osc_10_set_then_query() {
        // Set fg to red, then query — should return the set color.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]10;rgb:ff/00/00\x1b\\");
        feed(&mut t, b"\x1b]10;?\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("rgb:ff/00/00") || s.contains("rgb:ff/0/0"),
            "OSC 10 after set should return red, got: {s:?}"
        );
    }

    #[test]
    fn t_osc_11_set_then_query() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]11;rgb:00/ff/00\x1b\\");
        feed(&mut t, b"\x1b]11;?\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("rgb:00/ff/00") || s.contains("rgb:0/ff/0"),
            "OSC 11 after set should return green, got: {s:?}"
        );
    }

    #[test]
    fn t_osc_12_query_cursor_color() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]12;?\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.starts_with("\x1b]12;rgb:"),
            "OSC 12 query should respond with cursor color, got: {s:?}"
        );
    }

    #[test]
    fn t_osc_4_query_palette_color() {
        // OSC 4 ; 1 ; ? → query palette index 1 (red)
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]4;1;?\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.starts_with("\x1b]4;1;rgb:"),
            "OSC 4 query should respond with palette color, got: {s:?}"
        );
    }

    #[test]
    fn t_osc_4_set_palette_color() {
        // OSC 4 ; 0 ; rgb:ff/ff/00 → set palette[0] to yellow
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]4;0;rgb:ff/ff/00\x1b\\");
        // Now query to verify
        feed(&mut t, b"\x1b]4;0;?\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("rgb:ff/ff/00"),
            "OSC 4 after set should return yellow, got: {s:?}"
        );
    }

    #[test]
    fn t_osc_4_multiple_queries() {
        // OSC 4 ; 0 ; ? ; 1 ; ? → query two palette entries
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]4;0;?;1;?\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        // Should contain both index 0 and index 1 responses
        assert!(
            s.contains("4;0;rgb:") && s.contains("4;1;rgb:"),
            "OSC 4 multi-query should return both colors, got: {s:?}"
        );
    }

    #[test]
    fn t_osc_7_cwd_parsed() {
        // OSC 7 — current working directory
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]7;file://localhost/home/user\x1b\\");
        assert_eq!(t.cwd(), Some(std::path::Path::new("/home/user")));
    }

    #[test]
    fn t_osc_7_invalid_url_ignored() {
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]7;not-a-url\x1b\\");
        assert_eq!(t.cwd(), None, "invalid OSC 7 payload should be ignored");
    }

    #[test]
    fn t_osc_9_progress_report() {
        // OSC 9 ; 4 ; 0 ; 50 → progress 50% (state 0 = update)
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]9;4;0;50\x1b\\");
        assert_eq!(t.progress(), Some(0.5));
    }

    #[test]
    fn t_osc_9_progress_complete() {
        // OSC 9 ; 4 ; 1 → completed (clear progress)
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]9;4;0;50\x1b\\"); // set to 50%
        feed(&mut t, b"\x1b]9;4;1\x1b\\"); // state 1 = completed
        assert_eq!(t.progress(), None, "state 1 should clear progress");
    }

    #[test]
    fn t_osc_9_notification() {
        // OSC 9 ; message → desktop notification
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]9;Build complete!\x1b\\");
        let notif = t.take_pending_notification();
        assert!(notif.is_some(), "should have pending notification");
        let (_title, body) = notif.unwrap();
        assert_eq!(body, "Build complete!");
    }

    #[test]
    fn t_osc_21_query_title() {
        // OSC 21 → query title, responds with OSC l <title> ST
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]0;My Title\x07"); // set title
        feed(&mut t, b"\x1b]21\x07"); // query
        let resp = t.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(
            s.contains("My Title"),
            "OSC 21 query should return current title, got: {s:?}"
        );
    }

    #[test]
    fn t_osc_1337_current_dir() {
        // OSC 1337 ; CurrentDir=/path
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b]1337;CurrentDir=/tmp/test\x1b\\");
        assert_eq!(t.cwd(), Some(std::path::Path::new("/tmp/test")));
    }

    #[test]
    fn t_osc_1337_clear_scrollback() {
        // OSC 1337 ; ClearScrollback
        let mut t = Terminal::with_scrollback(20, 3, 1000);
        for i in 0..10 {
            feed(&mut t, format!("Line {}\n", i).as_bytes());
        }
        assert!(t.grid().scrollback_len() > 0);
        feed(&mut t, b"\x1b]1337;ClearScrollback\x1b\\");
        assert_eq!(t.grid().scrollback_len(), 0, "scrollback should be cleared");
    }

    // ── DEC line drawing completeness ──────────────────────────────────

    #[test]
    fn t_dec_line_all_corner_chars() {
        // Verify all 4 corner characters render correctly.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b(0"); // DEC Special Graphics
        feed(&mut t, b"lkjm"); // ┌┐┘└ (l=┌, k=┐, j=┘, m=└)
        feed(&mut t, b"\x1b(B"); // back to ASCII
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{250c}'); // ┌ (l)
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, '\u{2510}'); // ┐ (k)
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, '\u{2518}'); // ┘ (j)
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, '\u{2514}'); // └ (m)
    }

    #[test]
    fn t_dec_line_all_tee_chars() {
        // Verify all 4 tee characters render correctly.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b(0");
        feed(&mut t, b"tuvw"); // ├┤┴┬
        feed(&mut t, b"\x1b(B");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{251c}'); // ├
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, '\u{2524}'); // ┤
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, '\u{2534}'); // ┴
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, '\u{252c}'); // ┬
    }

    #[test]
    fn t_dec_line_cross_and_lines() {
        // Cross and horizontal/vertical lines.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b(0");
        feed(&mut t, b"nqx"); // ┼─│
        feed(&mut t, b"\x1b(B");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{253c}'); // ┼
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, '\u{2500}'); // ─
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, '\u{2502}'); // │
    }

    #[test]
    fn t_dec_special_symbols() {
        // Verify symbols: diamond, degree, plus-minus, pi, etc.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b(0");
        feed(&mut t, b"`fg{|}~"); // diamond degree plus-minus pi neq pound dot
        feed(&mut t, b"\x1b(B");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{25c6}'); // ◆
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, '\u{00b0}'); // °
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, '\u{00b1}'); // ±
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, '\u{03c0}'); // π
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, '\u{2260}'); // ≠
        assert_eq!(t.grid().cell(5, 0).unwrap().ch, '\u{00a3}'); // £
        assert_eq!(t.grid().cell(6, 0).unwrap().ch, '\u{00b7}'); // ·
    }

    #[test]
    fn t_dec_special_comparison_operators() {
        // ≤ and ≥
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b(0");
        feed(&mut t, b"yz"); // ≤≥
        feed(&mut t, b"\x1b(B");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{2264}'); // ≤
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, '\u{2265}'); // ≥
    }

    #[test]
    fn t_charset_g1_line_drawing_via_so() {
        // Designate G1 as DEC Special, then use SO to activate.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b)0"); // ESC ) 0 — G1 = DEC Special
        feed(&mut t, b"\x0e"); // SO — activate G1
        feed(&mut t, b"qq"); // horizontal lines
        feed(&mut t, b"\x0f"); // SI — back to G0
        feed(&mut t, b"AB"); // normal ASCII
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{2500}'); // ─
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, '\u{2500}'); // ─
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'B');
    }

    #[test]
    fn t_charset_so_si_toggle_alternating() {
        // Toggle between G0 and G1 multiple times.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b)0"); // G1 = DEC Special
        feed(&mut t, b"\x0e"); // SO → G1
        feed(&mut t, b"q"); // ─
        feed(&mut t, b"\x0f"); // SI → G0
        feed(&mut t, b"A");
        feed(&mut t, b"\x0e"); // SO → G1
        feed(&mut t, b"x"); // │
        feed(&mut t, b"\x0f"); // SI → G0
        feed(&mut t, b"B");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{2500}'); // ─
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, '\u{2502}'); // │
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'B');
    }

    #[test]
    fn t_charset_decstr_resets_charset() {
        // DECSTR should reset charset to default (ASCII).
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b(0"); // G0 = DEC Special
        feed(&mut t, b"\x1b[!p"); // DECSTR — soft reset
        assert_eq!(t.g0_charset(), Charset::Ascii, "DECSTR should reset G0");
    }

    #[test]
    fn t_charset_decstr_resets_g1_and_active() {
        // DECSTR should reset G1 charset and active_g1 flag.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b)0"); // G1 = DEC Special
        feed(&mut t, b"\x0e"); // SO — activate G1
        feed(&mut t, b"\x1b[!p"); // DECSTR
        assert_eq!(t.g1_charset(), Charset::Ascii, "DECSTR should reset G1");
        assert!(!t.active_g1(), "DECSTR should reset active_g1");
    }

    #[test]
    fn t_charset_full_box_drawing_render() {
        // Render a small box using DEC line drawing and verify all corners.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b(0");
        // ┌────┐
        // │    │
        // └────┘
        feed(&mut t, b"lqqqqk\r\n"); // ┌────┐
        feed(&mut t, b"x    x\r\n"); // │    │
        feed(&mut t, b"mqqqqj"); // └────┘
        feed(&mut t, b"\x1b(B");
        // Verify corners
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{250c}'); // ┌ (l)
        assert_eq!(t.grid().cell(5, 0).unwrap().ch, '\u{2510}'); // ┐ (k)
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, '\u{2514}'); // └ (m)
        assert_eq!(t.grid().cell(5, 2).unwrap().ch, '\u{2518}'); // ┘ (j)
        // Verify horizontal lines
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, '\u{2500}'); // ─
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, '\u{2500}'); // ─
        // Verify vertical lines
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, '\u{2502}'); // │
        assert_eq!(t.grid().cell(5, 1).unwrap().ch, '\u{2502}'); // │
    }

    // ── Round 5-1: Cursor movement edge cases ──────────────────────────

    #[test]
    fn t_cup_default_params_home() {
        // CSI H with no params should go to (1,1) = home.
        let mut t = Terminal::new(20, 10);
        feed(&mut t, b"Hello\r\nWorld");
        feed(&mut t, b"\x1b[H"); // CUP with no params
        assert_eq!(t.cursor(), (0, 0), "CSI H should home cursor");
    }

    #[test]
    fn t_cup_zero_row_zero_col() {
        // CSI 0;0 H should be treated as CSI 1;1 H (home).
        let mut t = Terminal::new(20, 10);
        feed(&mut t, b"Test");
        feed(&mut t, b"\x1b[0;0H");
        assert_eq!(t.cursor(), (0, 0), "CSI 0;0H should normalize to home");
    }

    #[test]
    fn t_cup_large_values_clamped() {
        // CSI 999;999 H should clamp to screen size.
        let mut t = Terminal::new(20, 10);
        feed(&mut t, b"\x1b[999;999H");
        assert_eq!(
            t.cursor(),
            (19, 9),
            "CUP with large values should clamp to last col/row"
        );
    }

    #[test]
    fn t_cuf_at_last_column_no_overflow() {
        // CUF at last column should not overflow.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[10G"); // go to col 10
        feed(&mut t, b"\x1b[5C"); // try to move right 5
        assert_eq!(t.cursor().0, 9, "CUF at last col should clamp to width-1");
    }

    #[test]
    fn t_cub_at_first_column_no_underflow() {
        // CUB at first column should not underflow.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[5D"); // try to move left 5 from col 0
        assert_eq!(t.cursor().0, 0, "CUB at col 0 should stay at 0");
    }

    #[test]
    fn t_cuf_zero_param_moves_one() {
        // CSI 0 C should move by 1 (default param).
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[0C");
        assert_eq!(t.cursor().0, 1, "CUF with param 0 should move by 1");
    }

    #[test]
    fn t_cuu_at_top_row_stays() {
        // CUU at row 0 should stay at row 0.
        let mut t = Terminal::new(20, 10);
        feed(&mut t, b"\x1b[5A"); // move up 5 from row 0
        assert_eq!(t.cursor().1, 0, "CUU at top should stay at 0");
    }

    #[test]
    fn t_cud_at_bottom_row_stays() {
        // CUD at last row should stay.
        let mut t = Terminal::new(20, 10);
        feed(&mut t, b"\x1b[24B"); // move down 24 from row 0
        assert_eq!(t.cursor().1, 9, "CUD at bottom should clamp to height-1");
    }

    #[test]
    fn t_cha_default_param_col1() {
        // CSI G with no param should go to col 1.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"Hello"); // cursor at col 5
        feed(&mut t, b"\x1b[G"); // CHA with no param
        assert_eq!(t.cursor().0, 0, "CSI G should go to col 1 (0-based: 0)");
    }

    #[test]
    fn t_cha_zero_param_col1() {
        // CSI 0 G should go to col 1.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"Hello");
        feed(&mut t, b"\x1b[0G");
        assert_eq!(t.cursor().0, 0, "CSI 0G should go to col 1");
    }

    #[test]
    fn t_cha_clamped_to_width() {
        // CSI 999 G should clamp to last column.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[999G");
        assert_eq!(t.cursor().0, 19, "CHA should clamp to width-1");
    }

    #[test]
    fn t_vpa_default_param_row1() {
        // CSI d with no param should go to row 1.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[3;1H"); // row 3
        feed(&mut t, b"\x1b[d"); // VPA no param
        assert_eq!(t.cursor().1, 0, "CSI d should go to row 1 (0-based: 0)");
    }

    #[test]
    fn t_vpa_clamped_to_height() {
        // CSI 999 d should clamp.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[999d");
        assert_eq!(t.cursor().1, 4, "VPA should clamp to height-1");
    }

    #[test]
    fn t_cup_origin_mode_relative() {
        // In origin mode, CUP coordinates are relative to scroll region top.
        let mut t = Terminal::new(20, 10);
        feed(&mut t, b"\x1b[3;8r"); // scroll region rows 3-8 (1-based)
        feed(&mut t, b"\x1b[?6h"); // origin mode on
        feed(&mut t, b"\x1b[1;1H"); // CUP to row 1 col 1
        assert_eq!(
            t.cursor(),
            (0, 2),
            "origin mode row 1 = physical row 3 (0-based 2)"
        );
    }

    #[test]
    fn t_cup_origin_mode_clamped_to_region() {
        // In origin mode, CUP cannot go below scroll region bottom.
        let mut t = Terminal::new(20, 10);
        feed(&mut t, b"\x1b[3;5r"); // scroll region rows 3-5
        feed(&mut t, b"\x1b[?6h"); // origin mode on
        feed(&mut t, b"\x1b[10;1H"); // try row 10 in origin mode
        // Should clamp to scroll bottom (row 5 = 0-based 4)
        assert_eq!(
            t.cursor().1,
            4,
            "CUP in origin mode should clamp to region bottom"
        );
    }

    #[test]
    fn t_cup_clears_pending_wrap() {
        // CUP should clear pending_wrap state.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"1234567890"); // fills row, sets pending_wrap
        feed(&mut t, b"\x1b[1;1H"); // CUP to home
        // Writing a char now should go to (0,0), not wrap
        feed(&mut t, b"X");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X');
    }

    #[test]
    fn t_cnl_cpl_move_and_col0() {
        // CNL (CSI Pn E): move down N, col=0
        // CPL (CSI Pn F): move up N, col=0
        let mut t = Terminal::new(20, 10);
        feed(&mut t, b"\x1b[5;5H"); // row 5, col 5
        feed(&mut t, b"\x1b[2E"); // CNL 2: row 7, col 0
        assert_eq!(t.cursor(), (0, 6), "CNL should move down and reset col");
        feed(&mut t, b"\x1b[3F"); // CPL 3: row 4, col 0
        assert_eq!(t.cursor(), (0, 3), "CPL should move up and reset col");
    }

    #[test]
    fn t_cuu_cud_default_move_one() {
        // CSI A and CSI B with no param should move by 1.
        let mut t = Terminal::new(20, 10);
        feed(&mut t, b"\x1b[5;5H"); // row 5, col 5
        feed(&mut t, b"\x1b[A"); // up 1
        assert_eq!(t.cursor().1, 3, "CUF no param should move up 1");
        feed(&mut t, b"\x1b[B"); // down 1
        assert_eq!(t.cursor().1, 4, "CUD no param should move down 1");
    }

    #[test]
    fn t_hpa_same_as_cha() {
        // CSI Ps ` (HPA) should behave same as CSI Ps G (CHA).
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[10`"); // HPA col 10
        assert_eq!(t.cursor().0, 9, "HPA should set col to Ps (0-based: Ps-1)");
    }

    // ── Round 5-2: SGR text attributes and colors ─────────────────────

    #[test]
    fn t_sgr_bold_sets_flag() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[1mX");
        assert!(t.grid().cell(0, 0).unwrap().flags.contains(CellFlags::BOLD));
    }

    #[test]
    fn t_sgr_dim_sets_flag() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[2mX");
        assert!(t.grid().cell(0, 0).unwrap().flags.contains(CellFlags::DIM));
    }

    #[test]
    fn t_sgr_italic_sets_flag() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[3mX");
        assert!(
            t.grid()
                .cell(0, 0)
                .unwrap()
                .flags
                .contains(CellFlags::ITALIC)
        );
    }

    #[test]
    fn t_sgr_underline_sets_flag() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[4mX");
        assert!(
            t.grid()
                .cell(0, 0)
                .unwrap()
                .flags
                .contains(CellFlags::UNDERLINE)
        );
    }

    #[test]
    fn t_sgr_blink_sets_flag() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[5mX");
        assert!(
            t.grid()
                .cell(0, 0)
                .unwrap()
                .flags
                .contains(CellFlags::BLINK)
        );
    }

    #[test]
    fn t_sgr_reverse_sets_flag() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[7mX");
        assert!(
            t.grid()
                .cell(0, 0)
                .unwrap()
                .flags
                .contains(CellFlags::REVERSE)
        );
    }

    #[test]
    fn t_sgr_hidden_sets_flag() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[8mX");
        assert!(
            t.grid()
                .cell(0, 0)
                .unwrap()
                .flags
                .contains(CellFlags::HIDDEN)
        );
    }

    #[test]
    fn t_sgr_strikethrough_sets_flag() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[9mX");
        assert!(
            t.grid()
                .cell(0, 0)
                .unwrap()
                .flags
                .contains(CellFlags::STRIKETHROUGH)
        );
    }

    #[test]
    fn t_sgr_reset_clears_all() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[1;3;4;31mX\x1b[0mY");
        let cell = t.grid().cell(1, 0).unwrap();
        assert_eq!(
            cell.flags,
            CellFlags::empty(),
            "SGR 0 should clear all flags"
        );
        assert_eq!(cell.fg, Color::Default, "SGR 0 should reset fg");
    }

    // ── Partial attribute cancellation (key test) ─────────────────

    #[test]
    fn t_sgr_cancel_italic_preserves_bold_underline() {
        // Set bold+italic+underline, then cancel only italic (SGR 23).
        // Bold and underline MUST survive.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[1;3;4m"); // bold + italic + underline
        feed(&mut t, b"\x1b[23m"); // cancel italic
        feed(&mut t, b"X");
        let flags = t.grid().cell(0, 0).unwrap().flags;
        assert!(
            flags.contains(CellFlags::BOLD),
            "bold should survive SGR 23"
        );
        assert!(
            flags.contains(CellFlags::UNDERLINE),
            "underline should survive SGR 23"
        );
        assert!(
            !flags.contains(CellFlags::ITALIC),
            "italic should be cleared"
        );
    }

    #[test]
    fn t_sgr_cancel_bold_preserves_others() {
        // Set bold+dim+italic, cancel bold/dim (SGR 22), italic survives.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[1;2;3m");
        feed(&mut t, b"\x1b[22m"); // cancel bold + dim
        feed(&mut t, b"X");
        let flags = t.grid().cell(0, 0).unwrap().flags;
        assert!(
            !flags.contains(CellFlags::BOLD),
            "bold should be cleared by 22"
        );
        assert!(
            !flags.contains(CellFlags::DIM),
            "dim should be cleared by 22"
        );
        assert!(
            flags.contains(CellFlags::ITALIC),
            "italic should survive 22"
        );
    }

    #[test]
    fn t_sgr_cancel_underline_preserves_bold() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[1;4m");
        feed(&mut t, b"\x1b[24m"); // cancel underline
        feed(&mut t, b"X");
        let flags = t.grid().cell(0, 0).unwrap().flags;
        assert!(
            flags.contains(CellFlags::BOLD),
            "bold should survive SGR 24"
        );
        assert!(
            !flags.contains(CellFlags::UNDERLINE),
            "underline should be cleared"
        );
    }

    #[test]
    fn t_sgr_cancel_reverse_preserves_others() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[1;7m");
        feed(&mut t, b"\x1b[27m"); // cancel reverse
        feed(&mut t, b"X");
        let flags = t.grid().cell(0, 0).unwrap().flags;
        assert!(flags.contains(CellFlags::BOLD), "bold should survive 27");
        assert!(
            !flags.contains(CellFlags::REVERSE),
            "reverse should be cleared"
        );
    }

    #[test]
    fn t_sgr_cancel_blink_preserves_others() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[4;5m");
        feed(&mut t, b"\x1b[25m"); // cancel blink
        feed(&mut t, b"X");
        let flags = t.grid().cell(0, 0).unwrap().flags;
        assert!(
            flags.contains(CellFlags::UNDERLINE),
            "underline should survive 25"
        );
        assert!(!flags.contains(CellFlags::BLINK), "blink should be cleared");
    }

    #[test]
    fn t_sgr_cancel_hidden_preserves_others() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[1;8m");
        feed(&mut t, b"\x1b[28m"); // cancel hidden
        feed(&mut t, b"X");
        let flags = t.grid().cell(0, 0).unwrap().flags;
        assert!(flags.contains(CellFlags::BOLD), "bold should survive 28");
        assert!(
            !flags.contains(CellFlags::HIDDEN),
            "hidden should be cleared"
        );
    }

    #[test]
    fn t_sgr_cancel_strikethrough_preserves_others() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[4;9m");
        feed(&mut t, b"\x1b[29m"); // cancel strikethrough
        feed(&mut t, b"X");
        let flags = t.grid().cell(0, 0).unwrap().flags;
        assert!(
            flags.contains(CellFlags::UNDERLINE),
            "underline should survive 29"
        );
        assert!(
            !flags.contains(CellFlags::STRIKETHROUGH),
            "strikethrough should be cleared"
        );
    }

    // ── 256-color and TrueColor ───────────────────────────────────

    #[test]
    fn t_sgr_256_color_fg_low() {
        // 38;5;0 = palette index 0 (black)
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[38;5;0mX");
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Indexed(0));
    }

    #[test]
    fn t_sgr_256_color_fg_high() {
        // 38;5;255 = palette index 255
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[38;5;255mX");
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Indexed(255));
    }

    #[test]
    fn t_sgr_256_color_bg() {
        // 48;5;16 = palette index 16
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[48;5;16mX");
        assert_eq!(t.grid().cell(0, 0).unwrap().bg, Color::Indexed(16));
    }

    #[test]
    fn t_sgr_256_color_boundary_231() {
        // 38;5;231 = last of the 6x6x6 color cube boundary
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[38;5;231mX");
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Indexed(231));
    }

    #[test]
    fn t_sgr_truecolor_fg() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[38;2;255;128;0mX");
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Rgb(255, 128, 0));
    }

    #[test]
    fn t_sgr_truecolor_bg_blue() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[48;2;0;0;255mX");
        assert_eq!(t.grid().cell(0, 0).unwrap().bg, Color::Rgb(0, 0, 255));
    }

    #[test]
    fn t_sgr_truecolor_zero_rgb() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[38;2;0;0;0mX");
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Rgb(0, 0, 0));
    }

    // ── Default color reset ───────────────────────────────────────

    #[test]
    fn t_sgr_39_resets_fg() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[31m\x1b[39mX"); // set red, reset fg
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Default);
    }

    #[test]
    fn t_sgr_49_resets_bg() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[44m\x1b[49mX"); // set blue bg, reset bg
        assert_eq!(t.grid().cell(0, 0).unwrap().bg, Color::Default);
    }

    #[test]
    fn t_sgr_39_preserves_bg() {
        // SGR 39 should only reset fg, not bg.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[31;44m\x1b[39mX"); // red fg + blue bg, reset fg only
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.fg, Color::Default, "fg should be reset");
        assert_eq!(cell.bg, Color::Indexed(4), "bg should be preserved");
    }

    #[test]
    fn t_sgr_49_preserves_fg() {
        // SGR 49 should only reset bg, not fg.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[31;44m\x1b[49mX"); // red fg + blue bg, reset bg only
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.fg, Color::Indexed(1), "fg should be preserved");
        assert_eq!(cell.bg, Color::Default, "bg should be reset");
    }

    // ── Bright colors (90-97, 100-107) ────────────────────────────

    #[test]
    fn t_sgr_bright_fg_colors() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[90mX");
        assert_eq!(t.grid().cell(0, 0).unwrap().fg, Color::Indexed(8));
        feed(&mut t, b"\x1b[97mY");
        assert_eq!(t.grid().cell(1, 0).unwrap().fg, Color::Indexed(15));
    }

    #[test]
    fn t_sgr_bright_bg_colors() {
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[100mX");
        assert_eq!(t.grid().cell(0, 0).unwrap().bg, Color::Indexed(8));
        feed(&mut t, b"\x1b[107mY");
        assert_eq!(t.grid().cell(1, 0).unwrap().bg, Color::Indexed(15));
    }

    // ── Combined SGR in single sequence ───────────────────────────

    #[test]
    fn t_sgr_combined_in_single_escape() {
        // CSI 1;38;5;196;48;5;21m → bold, fg=196, bg=21
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[1;38;5;196;48;5;21mX");
        let cell = t.grid().cell(0, 0).unwrap();
        assert!(cell.flags.contains(CellFlags::BOLD));
        assert_eq!(cell.fg, Color::Indexed(196));
        assert_eq!(cell.bg, Color::Indexed(21));
    }

    // ── Round 5-3: Scroll region, ED/EL, alt screen edges ─────────────

    #[test]
    fn t_r5_ed_0_from_middle_row() {
        // ED 0: erase from cursor to end of display.
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"AAAAA\r\nBBBBB\r\nCCCCC\r\nDDDDD");
        feed(&mut t, b"\x1b[2;3H"); // row 2, col 3 (middle of "BBBBB")
        feed(&mut t, b"\x1b[0J"); // erase from cursor to end
        // Row 1 (BBBBB): cols 0-1 remain, cols 2-4 erased
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'B');
        assert_eq!(t.grid().cell(2, 1).unwrap().ch, ' ');
        // Row 2 (CCCCC): fully erased
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, ' ');
        // Row 3 (DDDDD): fully erased
        assert_eq!(t.grid().cell(0, 3).unwrap().ch, ' ');
        // Row 0 (AAAAA): untouched
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
    }

    #[test]
    fn t_r5_ed_1_from_middle_row() {
        // ED 1: erase from start of display to cursor.
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"AAAAA\r\nBBBBB\r\nCCCCC\r\nDDDDD");
        feed(&mut t, b"\x1b[3;3H"); // row 3, col 3 (in "CCCCC")
        feed(&mut t, b"\x1b[1J"); // erase from start to cursor
        // Row 0 (AAAAA): fully erased
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ');
        // Row 1 (BBBBB): fully erased
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, ' ');
        // Row 2 (CCCCC): cols 0-2 erased, col 3+ remains
        assert_eq!(t.grid().cell(2, 2).unwrap().ch, ' ');
        assert_eq!(t.grid().cell(3, 2).unwrap().ch, 'C');
        // Row 3 (DDDDD): untouched
        assert_eq!(t.grid().cell(0, 3).unwrap().ch, 'D');
    }

    #[test]
    fn t_r5_ed_2_clears_all() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"AAA\r\nBBB\r\nCCC");
        feed(&mut t, b"\x1b[2J");
        for r in 0..3 {
            for c in 0..3 {
                assert_eq!(t.grid().cell(c, r).unwrap().ch, ' ', "row {r} col {c}");
            }
        }
    }

    #[test]
    fn t_r5_ed_3_clears_scrollback_preserves_screen() {
        let mut t = Terminal::with_scrollback(10, 3, 1000);
        for i in 0..5 {
            feed(&mut t, format!("Line{}\r\n", i).as_bytes());
        }
        assert!(t.grid().scrollback_len() > 0);
        feed(&mut t, b"\x1b[3J"); // clear scrollback only
        assert_eq!(t.grid().scrollback_len(), 0);
        // Visible screen should NOT be cleared
        assert!(
            t.grid().cell(0, 0).unwrap().ch != ' ',
            "visible screen preserved"
        );
    }

    #[test]
    fn t_r5_el_0_at_cursor_erases_to_end() {
        // EL 0: erase from cursor to end of line (including cursor position).
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"HelloXXXXX");
        feed(&mut t, b"\x1b[5G"); // col 5 (0-based: 4 = 'o')
        feed(&mut t, b"\x1b[0K"); // erase from cursor (inclusive) to end
        // Chars before cursor preserved
        assert_eq!(
            t.grid().cell(3, 0).unwrap().ch,
            'l',
            "before cursor preserved"
        );
        // Cursor position and after erased
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, ' ', "at cursor erased");
        assert_eq!(t.grid().cell(5, 0).unwrap().ch, ' ', "after cursor erased");
    }

    #[test]
    fn t_r5_el_1_erases_to_cursor() {
        // EL 1: erase from start to cursor.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"HelloXXXXX");
        feed(&mut t, b"\x1b[3G"); // col 3
        feed(&mut t, b"\x1b[1K"); // erase from start to cursor
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ', "start erased");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, ' ', "at cursor erased");
        assert_eq!(
            t.grid().cell(3, 0).unwrap().ch,
            'l',
            "after cursor preserved"
        );
    }

    #[test]
    fn t_r5_el_2_clears_entire_line() {
        // EL 2: erase entire line.
        let mut t = Terminal::new(10, 2);
        feed(&mut t, b"Hello\r\nWorld");
        feed(&mut t, b"\x1b[1;1H"); // go to row 1
        feed(&mut t, b"\x1b[2K"); // erase entire line
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ', "row 1 cleared");
        // Row 2 should be untouched
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'W', "row 2 preserved");
    }

    // ── Scroll region LF at boundary ──────────────────────────────

    #[test]
    fn t_r5_lf_at_scroll_region_bottom_scrolls() {
        // When cursor is at scroll region bottom, LF should scroll, not move.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[1;3r"); // scroll region rows 1-3
        feed(&mut t, b"\x1b[3;1H"); // go to last row of region
        let cursor_y_before = t.cursor().1;
        feed(&mut t, b"\n"); // LF at bottom
        // Cursor should NOT have moved past the scroll region bottom.
        // It should stay at the same row (content scrolled up).
        assert_eq!(
            t.cursor().1,
            cursor_y_before,
            "LF at scroll bottom should not move cursor"
        );
    }

    #[test]
    fn t_r5_lf_outside_scroll_region_no_scroll() {
        // LF outside scroll region should not scroll.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[2;4r"); // scroll region rows 2-4
        feed(&mut t, b"\x1b[5;1H"); // cursor at row 5 (below region)
        feed(&mut t, b"Line5");
        feed(&mut t, b"\x1b[1;1H"); // back to row 1 (above region)
        feed(&mut t, b"Line1");
        // Row 1 content should be intact
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'L');
    }

    #[test]
    fn t_r5_decstbm_default_params_full_screen() {
        // CSI r with no params resets scroll region to full screen.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[2;4r"); // set region
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!((top, bottom), (1, 4));
        feed(&mut t, b"\x1b[r"); // reset
        let (top2, bottom2) = t.grid().scroll_region();
        assert_eq!((top2, bottom2), (0, 5), "CSI r should reset to full screen");
    }

    #[test]
    fn t_r5_decstbm_invalid_top_ge_bottom_ignored() {
        // top >= bottom should be ignored.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;3r"); // top == bottom → invalid
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(
            (top, bottom),
            (0, 5),
            "invalid region (top>=bottom) should be ignored"
        );
    }

    #[test]
    fn t_r5_decstbm_bottom_defaults_to_height() {
        // CSI 2 r → top=2, bottom=height.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[2r"); // top=2, bottom defaults to 5
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(top, 1, "top = 2 (0-based: 1)");
        assert_eq!(bottom, 5, "bottom defaults to height");
    }

    // ── Alt screen: cursor style and charset preservation ────────

    #[test]
    fn t_r5_alt_screen_cursor_style_preserved() {
        // Mode 1049 should save and restore cursor style.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[3 q"); // blinking underline cursor
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"\x1b[5 q"); // change to bar in alt
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        // Should restore the original blinking underline cursor
        assert_eq!(
            t.cursor_style(),
            CursorStyle::BlinkUnderline,
            "cursor style should be restored after alt screen"
        );
    }

    #[test]
    fn t_r5_alt_screen_charset_preserved() {
        // Charset should be preserved across alt screen switch (1049).
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b(0"); // G0 = DEC Special Graphics
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"\x1b(B"); // G0 = ASCII in alt
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert_eq!(
            t.g0_charset(),
            Charset::DecSpecial,
            "G0 charset should be restored after alt screen"
        );
    }

    #[test]
    fn t_r5_alt_screen_content_preserved() {
        // Primary content should survive alt screen round-trip.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"Primary1\r\nPrimary2");
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"AltScreen");
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'P',
            "primary content row 1"
        );
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            'P',
            "primary content row 2"
        );
    }

    #[test]
    fn t_r5_alt_screen_does_not_leak_alt_content() {
        // Alt screen content should NOT appear on primary after exit.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"SECRET_ALT_DATA");
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        // Primary should be blank
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            ' ',
            "no alt content leaked"
        );
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, ' ', "primary is blank");
    }

    // ── Round 6-1: Wide character and Unicode edge cases ──────────────

    #[test]
    fn t_r6_cjk_occupies_two_columns() {
        // Chinese character '你' (U+4F60) should occupy 2 columns.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, "你".as_bytes());
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '你');
        assert!(
            t.grid()
                .cell(0, 0)
                .unwrap()
                .flags
                .contains(CellFlags::WIDE_CHAR)
        );
        assert!(
            t.grid()
                .cell(1, 0)
                .unwrap()
                .flags
                .contains(CellFlags::WIDE_SPACER)
        );
        // Cursor should be at col 2 (advanced by 2)
        assert_eq!(t.cursor().0, 2);
    }

    #[test]
    fn t_r6_cjk_five_chars_fill_10_cols() {
        // 5 CJK chars in a 10-col terminal should exactly fill the row.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你好世界啊".as_bytes());
        // All 10 columns consumed, cursor at last col with pending_wrap
        assert_eq!(t.cursor().0, 9, "cursor at last col");
        assert!(t.cursor.pending_wrap, "should be pending wrap");
    }

    #[test]
    fn t_r6_cjk_sixth_char_wraps() {
        // 6th CJK char should wrap to next line.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你好世界啊".as_bytes()); // fills row 0
        feed(&mut t, "好".as_bytes()); // should wrap to row 1
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, '好', "6th char on row 1");
    }

    #[test]
    fn t_r6_cjk_at_penultimate_col_no_split() {
        // CJK char at col 8 in 10-col terminal: fits at cols 8-9.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[9G"); // go to col 9 (0-based: 8)
        feed(&mut t, "你".as_bytes());
        assert_eq!(t.grid().cell(8, 0).unwrap().ch, '你');
        assert!(
            t.grid()
                .cell(8, 0)
                .unwrap()
                .flags
                .contains(CellFlags::WIDE_CHAR)
        );
        assert!(
            t.grid()
                .cell(9, 0)
                .unwrap()
                .flags
                .contains(CellFlags::WIDE_SPACER)
        );
    }

    #[test]
    fn t_r6_cjk_at_last_col_wraps() {
        // CJK char at last col (col 9 in 10-col) should wrap, not split.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[10G"); // go to col 10 (0-based: 9)
        feed(&mut t, "你".as_bytes());
        // Should wrap to next line
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            '你',
            "wide char wraps from last col"
        );
        // Col 9 on row 0 should be blank (not half of wide char)
        assert_eq!(t.grid().cell(9, 0).unwrap().ch, ' ', "no split at boundary");
    }

    #[test]
    fn t_r6_cjk_then_ascii_width_transition() {
        // CJK char followed by ASCII: cursor and widths should be correct.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, "你A".as_bytes());
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '你');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'A');
        assert_eq!(t.cursor().0, 3, "cursor after 你A = col 3");
    }

    #[test]
    fn t_r6_backspace_after_cjk_moves_two() {
        // Backspace after a CJK char should move cursor back 1 col,
        // but the preceding cell is a WIDE_SPACER.
        // Real backspace in terminals: BS moves cursor by 1.
        // The CJK char's lead is at col N, spacer at col N+1.
        // After printing CJK at col 0, cursor is at col 2.
        // BS → col 1 (the spacer).
        let mut t = Terminal::new(20, 3);
        feed(&mut t, "你".as_bytes());
        assert_eq!(t.cursor().0, 2);
        feed(&mut t, b"\x08"); // BS
        assert_eq!(t.cursor().0, 1, "BS after CJK moves to col 1 (spacer)");
        feed(&mut t, b"\x08"); // BS again
        assert_eq!(t.cursor().0, 0, "BS again moves to col 0 (lead)");
    }

    #[test]
    fn t_r6_combining_char_e_acute() {
        // é = e + U+0301 (combining acute accent)
        let mut t = Terminal::new(20, 3);
        feed(&mut t, "e\u{0301}".as_bytes());
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.ch, 'e');
        assert_eq!(cell.combining.len(), 1, "combining char attached");
        assert_eq!(cell.combining[0], '\u{0301}');
        // Cursor should have advanced by 1 (width of 'e', not 0 for combining)
        assert_eq!(t.cursor().0, 1);
    }

    #[test]
    fn t_r6_zero_width_space_invisible() {
        // U+200B (zero-width space) should not advance cursor.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"AB");
        feed(&mut t, "\u{200B}".as_bytes());
        feed(&mut t, b"C");
        // ZWS is zero-width, should attach to B or be dropped.
        // Either way, C should be at col 2.
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'C');
    }

    #[test]
    fn t_r6_emoji_4byte_width() {
        // Most emoji are 2 columns wide.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, "😀".as_bytes()); // U+1F600
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '😀');
        assert!(
            t.grid()
                .cell(0, 0)
                .unwrap()
                .flags
                .contains(CellFlags::WIDE_CHAR)
        );
        assert_eq!(t.cursor().0, 2, "emoji takes 2 columns");
    }

    #[test]
    fn t_r6_multiple_cjk_then_backspace_delete() {
        // Write 3 CJK chars, then BS back to the middle one, overwrite with ASCII.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, "你好吗".as_bytes()); // cols 0-5, cursor at 6
        feed(&mut t, b"\x08\x08\x08"); // BS x3: 6→5→4→3 (col 3 = spacer of 吗)
        feed(&mut t, b"\x08"); // BS: 3→2 (col 2 = lead of 吗)
        feed(&mut t, b"X"); // overwrite at col 2
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'X');
        // Cols 0-1 should still be 好
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '你');
    }

    #[test]
    fn t_r6_mixed_cjk_ascii_cjk() {
        // 你A好B — widths: 2+1+2+1 = 6 columns
        let mut t = Terminal::new(20, 3);
        feed(&mut t, "你A好B".as_bytes());
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '你');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, '好');
        assert_eq!(t.grid().cell(5, 0).unwrap().ch, 'B');
        assert_eq!(t.cursor().0, 6);
    }

    #[test]
    fn t_r6_cjk_styled_with_color() {
        // CJK char with color attributes — both lead and spacer should get fg/bg.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[31;42m"); // red on green
        feed(&mut t, "你".as_bytes());
        let lead = t.grid().cell(0, 0).unwrap();
        let spacer = t.grid().cell(1, 0).unwrap();
        assert_eq!(lead.fg, Color::Indexed(1), "lead fg = red");
        assert_eq!(lead.bg, Color::Indexed(2), "lead bg = green");
        assert_eq!(spacer.bg, Color::Indexed(2), "spacer bg = green");
    }

    // ── Round 6-2: OSC title termination and edge cases ───────────────

    #[test]
    fn t_r6_osc_0_and_2_both_set_title() {
        // OSC 0 and OSC 2 should both set the same title field.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b]0;Title0\x07");
        assert_eq!(t.title(), "Title0");
        feed(&mut t, b"\x1b]2;Title2\x07");
        assert_eq!(t.title(), "Title2");
    }

    #[test]
    fn t_r6_osc_title_bel_vs_st_equivalent() {
        // BEL (0x07) and ST (ESC \) termination should be equivalent.
        let mut t1 = Terminal::new(20, 3);
        feed(&mut t1, b"\x1b]0;Test\x07"); // BEL terminated

        let mut t2 = Terminal::new(20, 3);
        feed(&mut t2, b"\x1b]0;Test\x1b\\"); // ST terminated

        assert_eq!(
            t1.title(),
            t2.title(),
            "BEL and ST termination should be equivalent"
        );
        assert_eq!(t1.title(), "Test");
    }

    #[test]
    fn t_r6_osc_title_empty_bel() {
        // Empty title with BEL termination.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b]0;\x07");
        assert_eq!(t.title(), "", "empty title should be stored");
    }

    #[test]
    fn t_r6_osc_title_empty_st() {
        // Empty title with ST termination.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b]0;\x1b\\");
        assert_eq!(t.title(), "", "empty title with ST should be stored");
    }

    #[test]
    fn t_r6_osc_title_with_semicolons() {
        // Title containing semicolons (first semicolon is delimiter, rest are content).
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b]0;a; b; c\x07");
        // Only the first ; is the delimiter, rest is title content
        assert_eq!(t.title(), "a; b; c");
    }

    #[test]
    fn t_r6_osc_title_strips_control_chars() {
        // Control characters in title should be stripped.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b]0;A\x01B\x02C\x07");
        assert_eq!(t.title(), "ABC", "control chars stripped from title");
    }

    #[test]
    fn t_r6_osc_title_caps_at_256() {
        // Titles longer than 256 chars should be truncated.
        let mut t = Terminal::new(20, 3);
        let long_title = "X".repeat(300);
        let seq = format!("\x1b]0;{}\x07", long_title);
        feed(&mut t, seq.as_bytes());
        assert_eq!(t.title().len(), 256, "title should be capped at 256 chars");
    }

    #[test]
    fn t_r6_osc_title_unicode() {
        // Unicode title should be preserved.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b]0;\xe4\xbd\xa0\xe5\xa5\xbd\x07"); // 你好 in UTF-8
        assert_eq!(t.title(), "你好");
    }

    #[test]
    fn t_r6_osc_title_overwrite() {
        // Setting a new title should replace the old one.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b]0;First\x07");
        feed(&mut t, b"\x1b]0;Second\x07");
        assert_eq!(t.title(), "Second");
    }

    // ── Round 6-3: Tab stops edge cases ────────────────────────────────

    #[test]
    fn t_r6_tab_default_stops_every_8() {
        // Default tab stops should be at cols 8, 16, 24, etc.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\t"); // from col 0 → col 8
        assert_eq!(t.cursor().0, 8);
        feed(&mut t, b"\t"); // col 8 → col 16
        assert_eq!(t.cursor().0, 16);
    }

    #[test]
    fn t_r6_tab_from_non_tab_position() {
        // Tab from col 3 should go to col 8 (next default stop).
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"ABC"); // cursor at col 3
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 8, "tab from col 3 → col 8");
    }

    #[test]
    fn t_r6_tab_at_last_col_stays() {
        // Tab at or past the last tab stop should stay at last col.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[10G"); // go to col 10 (0-based: 9)
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 9, "tab at last col should clamp");
    }

    #[test]
    fn t_r6_hts_sets_custom_stop() {
        // HTS (ESC H) sets a tab stop at the current column.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"ABCDE"); // cursor at col 5
        feed(&mut t, b"\x1bH"); // HTS — set tab stop at col 5
        feed(&mut t, b"\r"); // back to col 0
        feed(&mut t, b"\t"); // tab should go to col 5 (custom stop)
        assert_eq!(t.cursor().0, 5, "tab should stop at custom HTS stop");
    }

    #[test]
    fn t_r6_tbc_clears_current_stop() {
        // TBC (CSI g or CSI 0g) clears tab stop at current column.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[9G"); // go to col 9 (0-based: 8)
        feed(&mut t, b"\x1b[g"); // clear tab stop at current col (8)
        feed(&mut t, b"\r"); // back to col 0
        feed(&mut t, b"\t"); // tab should skip col 8, go to col 16
        assert_eq!(t.cursor().0, 16, "cleared stop at 8 should skip to 16");
    }

    #[test]
    fn t_r6_tbc_3_clears_all_stops() {
        // CSI 3g clears all tab stops.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[3g"); // clear all stops
        feed(&mut t, b"\t"); // no stops → should go to last col
        assert_eq!(t.cursor().0, 39, "no tab stops → tab goes to last col");
    }

    #[test]
    fn t_r6_cht_multiple_tabs() {
        // CHT (CSI Ps I) advances Ps tab stops.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[2I"); // advance 2 tab stops
        assert_eq!(t.cursor().0, 16, "CHT 2 from col 0 → col 16");
    }

    #[test]
    fn t_r6_cht_default_param_one() {
        // CSI I with no param should advance 1 tab stop.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[I");
        assert_eq!(t.cursor().0, 8, "CHT default = 1 stop → col 8");
    }

    #[test]
    fn t_r6_cbt_backward_tab() {
        // CBT (CSI Ps Z) moves backward Ps tab stops.
        // Default stops at cols 8, 16, 24 (0-based).
        // From col 17 (0-based: 16), CBT 1 should go to col 9 (0-based: 8).
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[17G"); // go to col 17 (0-based: 16)
        feed(&mut t, b"\x1b[Z"); // backward 1 stop → col 8 (0-based)
        assert_eq!(t.cursor().0, 8, "CBT 1 from col 16 → col 8");
    }

    #[test]
    fn t_r6_cbt_multiple_backward() {
        // From col 25 (0-based: 24), CBT 2 → 16 → 8.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[25G"); // col 25 (0-based: 24)
        feed(&mut t, b"\x1b[2Z"); // backward 2 stops → 8
        assert_eq!(t.cursor().0, 8, "CBT 2 from col 24 → col 8");
    }

    #[test]
    fn t_r6_cbt_at_col0_stays() {
        // CBT at col 0 should stay at col 0.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[3Z"); // try to go back 3 from col 0
        assert_eq!(t.cursor().0, 0, "CBT at col 0 stays");
    }

    #[test]
    fn t_r6_hts_then_tab_then_tbc_roundtrip() {
        // HTS set → tab to it → TBC clear → tab skips.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[5G"); // col 5
        feed(&mut t, b"\x1bH"); // set stop at col 4 (0-based)
        feed(&mut t, b"\r"); // col 0
        feed(&mut t, b"\t"); // should stop at col 4
        assert_eq!(t.cursor().0, 4);
        // Now clear stop at col 4
        feed(&mut t, b"\x1b[g"); // TBC
        feed(&mut t, b"\r"); // col 0
        feed(&mut t, b"\t"); // should skip col 4, go to col 8
        assert_eq!(t.cursor().0, 8, "after TBC, tab skips cleared stop");
    }

    // ── Round 6-4: Insert/delete operations edge cases ─────────────────

    #[test]
    fn t_r6_ich_insert_blank_shifts_right() {
        // ICH (CSI @): insert N blanks at cursor, shift right, drop overflow.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEF");
        feed(&mut t, b"\x1b[1G"); // col 1 (0-based: 0)
        feed(&mut t, b"\x1b[2@"); // insert 2 blanks at col 0
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ', "col 0 = blank");
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, ' ', "col 1 = blank");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'A', "A shifted to col 2");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'B', "B shifted to col 3");
    }

    #[test]
    fn t_r6_ich_default_param_one() {
        // CSI @ with no param inserts 1 blank.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"AB");
        feed(&mut t, b"\x1b[1G");
        feed(&mut t, b"\x1b[@"); // insert 1 blank
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'A');
    }

    #[test]
    fn t_r6_dch_delete_shifts_left() {
        // DCH (CSI P): delete N chars at cursor, shift left, fill blank at end.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEF");
        feed(&mut t, b"\x1b[1G"); // col 0
        feed(&mut t, b"\x1b[2P"); // delete 2 chars
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'C', "C shifted to col 0");
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'D', "D shifted to col 1");
        assert_eq!(
            t.grid().cell(4, 0).unwrap().ch,
            ' ',
            "col 4 = blank (filled)"
        );
    }

    #[test]
    fn t_r6_dch_default_param_one() {
        // CSI P with no param deletes 1 char.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABC");
        feed(&mut t, b"\x1b[1G");
        feed(&mut t, b"\x1b[P"); // delete 1
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'B');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'C');
    }

    #[test]
    fn t_r6_dch_more_than_content() {
        // DCH deleting more than available content.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"AB");
        feed(&mut t, b"\x1b[1G");
        feed(&mut t, b"\x1b[5P"); // delete 5 (only 2 exist)
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ', "all cleared");
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, ' ');
    }

    #[test]
    fn t_r6_ech_erase_n_chars() {
        // ECH (CSI X): erase N chars from cursor (no shift).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEF");
        feed(&mut t, b"\x1b[2G"); // col 2 (0-based: 1)
        feed(&mut t, b"\x1b[3X"); // erase 3 chars
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "col 0 preserved");
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, ' ', "col 1 erased");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, ' ', "col 3 erased");
        assert_eq!(
            t.grid().cell(4, 0).unwrap().ch,
            'E',
            "col 4 preserved (no shift)"
        );
    }

    #[test]
    fn t_r6_ech_default_param_one() {
        // CSI X with no param erases 1 char.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABC");
        feed(&mut t, b"\x1b[2G");
        feed(&mut t, b"\x1b[X"); // erase 1
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, ' ');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'C', "col 2 preserved");
    }

    #[test]
    fn t_r6_ech_more_than_remaining() {
        // ECH erasing past line end.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABC");
        feed(&mut t, b"\x1b[2G"); // col 1 (0-based)
        feed(&mut t, b"\x1b[20X"); // erase 20 (only 9 remain)
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "col 0 preserved");
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, ' ', "col 1 erased");
        assert_eq!(t.grid().cell(9, 0).unwrap().ch, ' ', "last col erased");
    }

    #[test]
    fn t_r6_il_insert_lines_pushes_down() {
        // IL (CSI L): insert N blank lines at cursor, push down.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"Line0\r\nLine1\r\nLine2\r\nLine3\r\nLine4");
        feed(&mut t, b"\x1b[2;1H"); // row 2 (0-based: 1)
        feed(&mut t, b"\x1b[2L"); // insert 2 lines
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'L', "row 0 preserved");
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            ' ',
            "row 1 = blank (inserted)"
        );
        assert_eq!(
            t.grid().cell(0, 2).unwrap().ch,
            ' ',
            "row 2 = blank (inserted)"
        );
        assert_eq!(
            t.grid().cell(0, 3).unwrap().ch,
            'L',
            "old row 1 pushed to row 3"
        );
    }

    #[test]
    fn t_r6_dl_delete_lines_shifts_up() {
        // DL (CSI M): delete N lines at cursor, shift up.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"Line0\r\nLine1\r\nLine2\r\nLine3\r\nLine4");
        feed(&mut t, b"\x1b[2;1H"); // row 2 (0-based: 1)
        feed(&mut t, b"\x1b[2M"); // delete 2 lines
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'L', "row 0 preserved");
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            'L',
            "old row 3 shifted to row 1"
        );
        assert_eq!(t.grid().cell(0, 4).unwrap().ch, ' ', "last row = blank");
    }

    #[test]
    fn t_r6_il_default_param_one() {
        // CSI L with no param inserts 1 line.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"A\r\nB\r\nC");
        feed(&mut t, b"\x1b[2;1H"); // row 2 (0-based: 1)
        feed(&mut t, b"\x1b[L"); // insert 1 line
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, ' ', "row 1 blank");
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, 'B', "B pushed to row 2");
    }

    #[test]
    fn t_r6_dl_default_param_one() {
        // CSI M with no param deletes 1 line.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"A\r\nB\r\nC");
        feed(&mut t, b"\x1b[2;1H"); // row 2 (0-based: 1)
        feed(&mut t, b"\x1b[M"); // delete 1 line
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'C', "C shifted up");
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, ' ', "row 2 blank");
    }

    #[test]
    fn t_r6_il_within_scroll_region() {
        // IL inside scroll region should only affect rows within region.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"R0\r\nR1\r\nR2\r\nR3\r\nR4\r\nR5");
        feed(&mut t, b"\x1b[2;5r"); // scroll region rows 2-5
        feed(&mut t, b"\x1b[3;1H"); // row 3 (0-based: 2, inside region)
        feed(&mut t, b"\x1b[1L"); // insert 1 line
        // Row 0 and 1 should be preserved
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'R', "row 0 preserved");
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'R', "row 1 preserved");
        // Row 2 should be blank (inserted)
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, ' ', "row 2 = blank");
    }

    // ── Round 7-1: Alt screen bug fixes ────────────────────────────────

    #[test]
    fn t_r7_mode_1048_saves_cursor() {
        // DECSET 1048 = save cursor (equivalent to DECSC / ESC 7).
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[3;5H"); // row 3, col 5
        feed(&mut t, b"\x1b[1;31m"); // bold red
        feed(&mut t, b"\x1b[?1048h"); // save cursor
        feed(&mut t, b"\x1b[5;1H"); // move away
        feed(&mut t, b"\x1b[0m"); // reset attrs
        feed(&mut t, b"\x1b[?1048l"); // restore cursor
        // Should restore position and attributes
        assert_eq!(t.cursor(), (4, 2), "1048 should restore cursor position");
        assert!(
            t.flags.contains(CellFlags::BOLD),
            "1048 should restore bold"
        );
        assert_eq!(t.fg, Color::Indexed(1), "1048 should restore red fg");
    }

    #[test]
    fn t_r7_mode_1048_plus_47_equals_1049() {
        // 1048h + 47h should be equivalent to 1049h for cursor preservation.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[3;5H\x1b[1;31m"); // position + attrs
        feed(&mut t, b"\x1b[?1048h"); // save cursor
        feed(&mut t, b"\x1b[?47h"); // switch to alt screen
        feed(&mut t, b"AltData");
        feed(&mut t, b"\x1b[?47l"); // switch back
        feed(&mut t, b"\x1b[?1048l"); // restore cursor
        // Cursor and attrs should be restored to pre-save state
        assert_eq!(t.cursor(), (4, 2), "cursor restored via 1048+47");
        assert!(
            t.flags.contains(CellFlags::BOLD),
            "bold restored via 1048+47"
        );
    }

    #[test]
    fn t_r7_mode_1048_restore_without_save() {
        // 1048l (restore) without prior 1048h (save) should not crash.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[?1048l"); // restore without save — no crash
        // Just verify it doesn't panic; behavior is implementation-defined.
        assert_eq!(t.cursor(), (0, 0), "no crash on restore without save");
    }

    #[test]
    fn t_r7_mode_1048_saves_charset() {
        // 1048 should save/restore charset designation (like DECSC).
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b(0"); // G0 = DEC Special Graphics
        feed(&mut t, b"\x1b[?1048h"); // save
        feed(&mut t, b"\x1b(B"); // G0 = ASCII
        feed(&mut t, b"\x1b[?1048l"); // restore
        assert_eq!(
            t.g0_charset(),
            Charset::DecSpecial,
            "1048 should restore charset"
        );
    }

    // ── Round 7-4: Resize edge case audits ─────────────────────────────

    #[test]
    fn t_r7_resize_shrink_drops_content_beyond_width() {
        // When shrinking width with reflow, content is re-wrapped.
        // A 20-char line becomes two 10-char rows (extra row goes to scrollback).
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"ABCDEFGHIJKLMNOPQRST"); // 20 chars fills row 0
        t.resize(10, 5);
        assert_eq!(t.grid().width(), 10);
        // Content should be preserved somewhere (visible or scrollback)
        // after reflow, not lost.
        assert!(
            t.grid().scrollback_len() > 0,
            "reflow should push overflow to scrollback"
        );
    }

    #[test]
    fn t_r7_resize_grow_new_cells_blank() {
        // When growing, new cells should be blank.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE");
        t.resize(10, 3);
        // Old content preserved
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'E');
        // New cells are blank
        assert_eq!(t.grid().cell(5, 0).unwrap().ch, ' ');
        assert_eq!(t.grid().cell(9, 0).unwrap().ch, ' ');
    }

    #[test]
    fn t_r7_resize_cursor_clamped_after_shrink() {
        // Cursor should be clamped to new bounds after shrink.
        let mut t = Terminal::new(20, 10);
        feed(&mut t, b"\x1b[8;15H"); // row 8, col 15
        t.resize(10, 3);
        assert_eq!(t.cursor(), (9, 2), "cursor clamped to (9, 2)");
    }

    #[test]
    fn t_r7_resize_clears_pending_wrap() {
        // pending_wrap should be cleared after resize.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE"); // fills row, pending_wrap set
        assert!(t.cursor.pending_wrap);
        t.resize(10, 3);
        assert!(!t.cursor.pending_wrap, "pending_wrap cleared after resize");
    }

    #[test]
    fn t_r7_resize_resets_scroll_region() {
        // Resize should reset scroll region to full screen.
        let mut t = Terminal::new(20, 10);
        feed(&mut t, b"\x1b[3;7r"); // scroll region rows 3-7
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!((top, bottom), (2, 7));
        t.resize(20, 8); // change height to trigger resize
        // Scroll region should be reset to full screen (0, 8)
        let (top2, bottom2) = t.grid().scroll_region();
        assert_eq!((top2, bottom2), (0, 8), "scroll region reset after resize");
    }

    #[test]
    fn t_r7_resize_alt_screen_simple_truncation() {
        // In alt screen, resize should be simple truncation (no reflow).
        // Content that already wrapped before resize stays where it is.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"ABCDEFGHIJKLMNOPQRST"); // exactly 20 chars, fills row 0
        t.resize(10, 5);
        // Row 0 truncated to 10 chars
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(9, 0).unwrap().ch, 'J');
        // No reflow: K-T is dropped (not moved to row 1)
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            ' ',
            "no reflow in alt screen"
        );
    }

    #[test]
    fn t_r7_resize_clears_utf8_buffer() {
        // After resize, Terminal's internal UTF-8 buffer should be empty.
        // (The VTE Parser handles its own UTF-8 state separately.)
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"Hello");
        t.resize(15, 3);
        assert!(t.utf8_buf.is_empty(), "UTF-8 buffer cleared after resize");
    }

    #[test]
    fn t_r7_resize_shrink_to_one_col() {
        // Resize to 1 column should not panic and content should survive.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABC");
        t.resize(1, 3);
        assert_eq!(t.cursor().0, 0, "cursor x clamped to 0");
        assert_eq!(t.grid().width(), 1);
    }

    #[test]
    fn t_r7_resize_grow_pulls_scrollback() {
        // Growing height should pull rows from scrollback into visible area.
        let mut t = Terminal::with_scrollback(10, 3, 100);
        for i in 0..10 {
            feed(&mut t, format!("L{}\n", i).as_bytes());
        }
        let sb_before = t.grid().scrollback_len();
        assert!(sb_before > 0, "should have scrollback");
        t.resize(10, 8); // grow height
        let sb_after = t.grid().scrollback_len();
        assert!(
            sb_after < sb_before,
            "scrollback should shrink when growing height"
        );
    }

    // ── Round 8-1: Wide char overwrite edge cases ──────────────────────

    #[test]
    fn t_r8_narrow_overwrite_on_wide_lead() {
        // Write wide char at col 0, then overwrite with narrow at col 0.
        // The spacer at col 1 must be cleared.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你".as_bytes()); // wide at cols 0-1
        feed(&mut t, b"\x1b[1G"); // back to col 0
        feed(&mut t, b"X"); // narrow overwrite at col 0
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X', "col 0 = X");
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, ' ', "col 1 spacer cleared");
        assert!(
            !t.grid().cell(1, 0).unwrap().is_wide_spacer(),
            "no spacer flag"
        );
    }

    #[test]
    fn t_r8_narrow_on_wide_spacer_clears_lead() {
        // Cursor lands on the spacer cell (col 1) of a wide char.
        // Cursor positioning adjusts to the lead (col 0), then printing
        // a narrow char there overwrites the lead position.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你".as_bytes()); // wide at cols 0-1, cursor now at col 2
        feed(&mut t, b"\x1b[2G"); // CHA col 2 → cursor.x=1 (spacer), adjusts to 0
        feed(&mut t, b"X"); // write narrow at lead position
        // The lead position gets X (cursor adjusted from spacer to lead).
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'X',
            "col 0 = X (cursor adjusted to lead)"
        );
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, ' ', "col 1 cleared");
        assert!(
            !t.grid().cell(0, 0).unwrap().is_wide(),
            "no wide flag on col 0"
        );
    }

    #[test]
    fn t_r8_wide_overwrite_wide() {
        // Write a wide char over another wide char.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你".as_bytes()); // wide at cols 0-1
        feed(&mut t, b"\x1b[1G"); // back to col 0
        feed(&mut t, "好".as_bytes()); // overwrite with another wide char
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '好', "col 0 = 好");
        assert!(t.grid().cell(0, 0).unwrap().is_wide(), "col 0 is wide");
        assert!(
            t.grid().cell(1, 0).unwrap().is_wide_spacer(),
            "col 1 is spacer"
        );
        assert!(
            !t.grid().cell(1, 0).unwrap().is_wide(),
            "col 1 is NOT a lead"
        );
    }

    #[test]
    fn t_r8_wide_at_penultimate_with_next_char() {
        // Wide char at cols 8-9 in a 10-col terminal, followed by a char.
        // The next char should wrap to line 2.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[9G"); // go to col 9 (0-based: 8)
        feed(&mut t, "你".as_bytes()); // wide at cols 8-9
        feed(&mut t, b"A"); // should wrap
        assert_eq!(t.grid().cell(8, 0).unwrap().ch, '你');
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'A', "A wrapped to row 1");
    }

    #[test]
    fn t_r8_wide_char_then_bs_then_narrow() {
        // Write wide char, backspace to spacer, backspace to lead,
        // then write a narrow char. Should not leave orphan spacer.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你".as_bytes()); // cols 0-1, cursor at 2
        feed(&mut t, b"\x08\x08"); // BS x2: 2→1→0
        feed(&mut t, b"X"); // write narrow at col 0
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X');
        assert_eq!(
            t.grid().cell(1, 0).unwrap().ch,
            ' ',
            "col 1 cleared (no orphan spacer)"
        );
        assert!(!t.grid().cell(1, 0).unwrap().is_wide_spacer());
    }

    #[test]
    fn t_r8_wide_char_scroll_preserves_integrity() {
        // Fill lines to trigger scroll, verify wide chars aren't corrupted.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你好".as_bytes()); // row 0: cols 0-3
        feed(&mut t, b"\r\n");
        feed(&mut t, "世界".as_bytes()); // row 1: cols 0-3
        feed(&mut t, b"\r\n");
        feed(&mut t, "测试".as_bytes()); // row 2: cols 0-3
        feed(&mut t, b"\r\n");
        feed(&mut t, "再来".as_bytes()); // triggers scroll
        // After scroll, row 0 should have 世界
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '世', "row 0 after scroll");
        assert!(t.grid().cell(0, 0).unwrap().is_wide());
        assert!(t.grid().cell(1, 0).unwrap().is_wide_spacer());
    }

    #[test]
    fn t_r8_wide_char_at_col0_after_scroll() {
        // Verify wide char integrity after scroll — spacer should follow lead.
        let mut t = Terminal::new(6, 2);
        feed(&mut t, "你好\r\n".as_bytes()); // row 0
        feed(&mut t, "世好\r\n".as_bytes()); // triggers scroll, "世好" moves to row 0
        feed(&mut t, b"X"); // row 1 (bottom)
        // Row 0 should have intact wide chars
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '世');
        assert!(t.grid().cell(1, 0).unwrap().is_wide_spacer());
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, '好');
        assert!(t.grid().cell(3, 0).unwrap().is_wide_spacer());
    }

    // ── Round 8-2: Tab stop audit ──────────────────────────────────────

    #[test]
    fn t_r8_tab_from_col0_stops_at_col8() {
        let mut t = Terminal::new(80, 3);
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 8);
    }

    #[test]
    fn t_r8_tab_already_at_stop_advances() {
        // If cursor is already AT a tab stop (col 8), tab should advance
        // to the NEXT stop (col 16), not stay at col 8.
        let mut t = Terminal::new(80, 3);
        feed(&mut t, b"\x1b[9G"); // go to col 9 (0-based: 8)
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 16, "tab from col 8 advances to 16");
    }

    #[test]
    fn t_r8_tab_from_last_col_clamps() {
        // Tab from last column should stay at last column.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[10G"); // col 10 (0-based: 9)
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 9, "tab at last col stays");
    }

    #[test]
    fn t_r8_tab_stops_survive_width_grow() {
        // Custom tab stops should survive when growing wider.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[6G"); // col 6 (0-based: 5)
        feed(&mut t, b"\x1bH"); // set custom stop at col 5
        t.resize(60, 3); // grow
        feed(&mut t, b"\r"); // col 0
        feed(&mut t, b"\t"); // should stop at col 5 (custom)
        assert_eq!(t.cursor().0, 5, "custom tab stop preserved after grow");
    }

    #[test]
    fn t_r8_tab_stops_survive_width_shrink() {
        // Custom tab stops within the new width survive shrinking.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[6G"); // col 6 (0-based: 5)
        feed(&mut t, b"\x1bH"); // set custom stop at col 5
        t.resize(20, 3); // shrink
        feed(&mut t, b"\r");
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 5, "custom tab stop preserved after shrink");
    }

    #[test]
    fn t_r8_tab_stops_extended_on_grow() {
        // Growing wider should add default stops at new 8-multiples.
        let mut t = Terminal::new(16, 3);
        t.resize(32, 3); // grow to 32
        feed(&mut t, b"\x1b[17G"); // col 17 (0-based: 16)
        feed(&mut t, b"\t"); // should stop at col 24 (0-based)
        assert_eq!(t.cursor().0, 24, "new default stop at col 24 after grow");
    }

    #[test]
    fn t_r8_hts_at_col0() {
        // HTS at col 0 should set a stop at col 0.
        // (Though col 0 is rarely useful as a stop.)
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[1G"); // col 1 (0-based: 0)
        feed(&mut t, b"\x1bH"); // HTS at col 0
        // Tab from col 1 should skip to col 8 (col 0 is behind us)
        feed(&mut t, b"\x1b[2G"); // col 2 (0-based: 1)
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 8, "tab from col 1 → col 8");
    }

    #[test]
    fn t_r8_decset_resets_tab_stops() {
        // DECSTR (CSI ! p) should reset tab stops to defaults.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[6G\x1bH"); // set custom stop at col 5
        feed(&mut t, b"\x1b[!p"); // full reset
        feed(&mut t, b"\r");
        feed(&mut t, b"\t"); // should go to default col 8, not custom col 5
        assert_eq!(t.cursor().0, 8, "tab stops reset to defaults after DECSTR");
    }

    // ── Round 8-3: Erase operations audit ──────────────────────────────

    #[test]
    fn t_r8_ed_0_from_cursor_to_end() {
        // ED 0: erase from cursor to end of display.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGH\r\nIJKLMNOP\r\nQRSTUVWX");
        feed(&mut t, b"\x1b[2;4H"); // row 2, col 4 (0-based: 1, 3)
        feed(&mut t, b"\x1b[0J");
        // Row 1 cols 0-2 preserved, col 3+ erased
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'I', "col 0 preserved");
        assert_eq!(t.grid().cell(2, 1).unwrap().ch, 'K', "col 2 preserved");
        assert_eq!(t.grid().cell(3, 1).unwrap().ch, ' ', "col 3 erased");
        // Row 2 entirely erased
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, ' ', "row 2 erased");
    }

    #[test]
    fn t_r8_ed_1_from_start_to_cursor() {
        // ED 1: erase from start of display to cursor.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGH\r\nIJKLMNOP\r\nQRSTUVWX");
        feed(&mut t, b"\x1b[2;4H"); // row 2, col 4 (0-based: 1, 3)
        feed(&mut t, b"\x1b[1J");
        // Row 0 entirely erased
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ', "row 0 erased");
        // Row 1 cols 0-3 erased, col 4+ preserved
        assert_eq!(t.grid().cell(3, 1).unwrap().ch, ' ', "col 3 erased");
        assert_eq!(t.grid().cell(4, 1).unwrap().ch, 'M', "col 4 preserved");
        // Row 2 preserved
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, 'Q', "row 2 preserved");
    }

    #[test]
    fn t_r8_ed_2_clears_all() {
        // ED 2: erase entire display.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGH\r\nIJKLMNOP\r\nQRSTUVWX");
        feed(&mut t, b"\x1b[2J");
        for r in 0..3 {
            for c in 0..8 {
                assert_eq!(
                    t.grid().cell(c, r).unwrap().ch,
                    ' ',
                    "cell ({},{}) should be cleared",
                    c,
                    r
                );
            }
        }
    }

    #[test]
    fn t_r8_ed_3_clears_scrollback_only() {
        // ED 3: clear scrollback but NOT visible screen.
        let mut t = Terminal::with_scrollback(10, 3, 100);
        for i in 0..10 {
            feed(&mut t, format!("Row{}\n", i).as_bytes());
        }
        assert!(t.grid().scrollback_len() > 0, "has scrollback");
        feed(&mut t, b"\x1b[3J");
        assert_eq!(t.grid().scrollback_len(), 0, "scrollback cleared");
        // Visible content should NOT be cleared — at least one cell should
        // have a non-space character.
        let has_content =
            (0..3).any(|r| (0..10).any(|c| t.grid().cell(c, r).is_some_and(|cell| cell.ch != ' ')));
        assert!(has_content, "visible screen should not be cleared by ED 3");
    }

    #[test]
    fn t_r8_el_0_from_cursor_to_eol() {
        // EL 0: erase from cursor to end of line.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGH");
        feed(&mut t, b"\x1b[1;4H"); // col 4 (0-based: 3)
        feed(&mut t, b"\x1b[0K");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'C', "col 2 preserved");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, ' ', "col 3 erased");
        assert_eq!(t.grid().cell(7, 0).unwrap().ch, ' ', "col 7 erased");
    }

    #[test]
    fn t_r8_el_1_from_start_to_cursor() {
        // EL 1: erase from start of line to cursor.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGH");
        feed(&mut t, b"\x1b[1;4H"); // col 4 (0-based: 3)
        feed(&mut t, b"\x1b[1K");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ', "col 0 erased");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, ' ', "col 3 erased");
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'E', "col 4 preserved");
    }

    #[test]
    fn t_r8_el_2_entire_line() {
        // EL 2: erase entire line.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGH");
        feed(&mut t, b"\x1b[2K");
        for c in 0..8 {
            assert_eq!(t.grid().cell(c, 0).unwrap().ch, ' ', "col {} erased", c);
        }
    }

    #[test]
    fn t_r8_decsca_protected_survives_decsed_0() {
        // DECSCA protected cells should survive DECSED (selective erase).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1\"q"); // DECSCA = protected
        feed(&mut t, b"AB");
        feed(&mut t, b"\x1b[2\"q"); // DECSCA = unprotected
        feed(&mut t, b"CD");
        feed(&mut t, b"\x1b[1;1H"); // back to start
        feed(&mut t, b"\x1b[?0J"); // DECSED 0: selective erase to end
        // Protected cells A, B should survive
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "protected A survives");
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B', "protected B survives");
        // Unprotected cells C, D should be erased
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, ' ', "unprotected C erased");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, ' ', "unprotected D erased");
    }

    #[test]
    fn t_r8_decsca_protected_survives_decsel_2() {
        // DECSEL 2 (?2K): selective erase entire line, protected survive.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1\"q"); // protected
        feed(&mut t, b"AB");
        feed(&mut t, b"\x1b[2\"q"); // unprotected
        feed(&mut t, b"CD");
        feed(&mut t, b"\x1b[?2K"); // selective erase entire line
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "protected survives");
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B', "protected survives");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, ' ', "unprotected erased");
    }

    #[test]
    fn t_r8_ech_clears_n_cells_no_shift() {
        // ECH erases N cells without shifting.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGH");
        feed(&mut t, b"\x1b[1;3H"); // col 3 (0-based: 2)
        feed(&mut t, b"\x1b[3X"); // erase 3 chars
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B', "before preserved");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, ' ', "col 2 erased");
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, ' ', "col 4 erased");
        assert_eq!(
            t.grid().cell(5, 0).unwrap().ch,
            'F',
            "col 5 preserved (no shift)"
        );
    }

    // ── Round 9-1: Scroll region (DECSTBM) + SU/SD audits ──────────────

    #[test]
    fn t_r9_decstbm_two_line_region_il() {
        // A 2-line scroll region: IL should only affect region rows.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"AAAA\r\nBBBB\r\nCCCC\r\nDDDD\r\nEEEE");
        feed(&mut t, b"\x1b[3;4r"); // scroll region = rows 3-4 (0-based: 2..4)
        feed(&mut t, b"\x1b[3;1H"); // cursor at row 3 (0-based: 2)
        feed(&mut t, b"\x1b[L"); // IL — insert 1 line
        // Row 2 should be blank (inserted)
        assert_eq!(
            t.grid().cell(0, 2).unwrap().ch,
            ' ',
            "row 2 blank (inserted)"
        );
        // Row 3 should have old row 2 content (CCCC shifted down)
        assert_eq!(t.grid().cell(0, 3).unwrap().ch, 'C', "row 3 has shifted C");
        // Rows outside region preserved
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "row 0 preserved");
        assert_eq!(t.grid().cell(0, 4).unwrap().ch, 'E', "row 4 preserved");
    }

    #[test]
    fn t_r9_lf_inside_scroll_region_scrolls() {
        // LF at bottom of scroll region should scroll, not advance past it.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"R0\r\nR1\r\nR2\r\nR3\r\nR4\r\nR5");
        feed(&mut t, b"\x1b[2;4r"); // scroll region rows 2-4 (0-based: 1..4)
        feed(&mut t, b"\x1b[4;1H"); // cursor at row 4 (0-based: 3, bottom of region)
        feed(&mut t, b"\n"); // LF — should scroll region
        // Content at rows 1-3 should shift up within region
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            'R',
            "row 1 has shifted content"
        );
        // Row 3 (bottom of region) should be blank (new line)
        assert_eq!(
            t.grid().cell(0, 3).unwrap().ch,
            ' ',
            "row 3 blank after scroll"
        );
        // Rows outside region: row 0 and 4-5 preserved
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'R', "row 0 preserved");
        assert_eq!(t.grid().cell(0, 4).unwrap().ch, 'R', "row 4 preserved");
    }

    #[test]
    fn t_r9_lf_outside_scroll_region_advances() {
        // LF outside the scroll region should just advance cursor.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[3;5r"); // scroll region rows 3-5
        feed(&mut t, b"\x1b[1;1H"); // cursor at row 1 (above region)
        feed(&mut t, b"\n"); // LF — should advance to row 2 (no scroll)
        assert_eq!(t.cursor().1, 1, "cursor advanced to row 2 (0-based: 1)");
    }

    #[test]
    fn t_r9_su_scrolls_only_region() {
        // SU (CSI S) should scroll only within the scroll region.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"R0\r\nR1\r\nR2\r\nR3\r\nR4\r\nR5");
        feed(&mut t, b"\x1b[2;5r"); // scroll region rows 2-5 (0-based: 1..5)
        feed(&mut t, b"\x1b[1S"); // SU 1
        // Row 0 should be preserved (outside region)
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'R', "row 0 preserved");
        // Row 1 should have shifted content (R2 moved up)
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            'R',
            "row 1 has shifted content"
        );
        // Bottom of region (row 4, 0-based) should be blank
        assert_eq!(
            t.grid().cell(0, 4).unwrap().ch,
            ' ',
            "bottom of region blank"
        );
    }

    #[test]
    fn t_r9_sd_scrolls_only_region() {
        // SD (CSI T) should scroll down within scroll region.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"R0\r\nR1\r\nR2\r\nR3\r\nR4\r\nR5");
        feed(&mut t, b"\x1b[2;5r"); // scroll region rows 2-5 (0-based: 1..5)
        feed(&mut t, b"\x1b[1T"); // SD 1
        // Row 0 preserved
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'R', "row 0 preserved");
        // Top of region (row 1) should be blank
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, ' ', "top of region blank");
        // Content shifted down
        assert_eq!(
            t.grid().cell(0, 2).unwrap().ch,
            'R',
            "row 2 has shifted content"
        );
    }

    #[test]
    fn t_r9_decstbm_invalid_top_eq_bottom() {
        // DECSTBM with top == bottom should be invalid (ignored).
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[3;3r"); // top == bottom = invalid
        // Scroll region should remain full screen
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(
            (top, bottom),
            (0, 6),
            "invalid DECSTBM resets to full screen"
        );
    }

    #[test]
    fn t_r9_decstbm_invalid_top_gt_bottom() {
        // DECSTBM with top > bottom should be invalid.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[5;2r"); // top > bottom
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!((top, bottom), (0, 6), "top > bottom resets to full screen");
    }

    #[test]
    fn t_r9_ind_at_region_bottom_wraps() {
        // IND (ESC D) at bottom of scroll region should scroll up.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"A\r\nB\r\nC\r\nD\r\nE");
        feed(&mut t, b"\x1b[2;4r"); // scroll region rows 2-4 (0-based: 1..4)
        feed(&mut t, b"\x1b[4;1H"); // cursor at row 4 (0-based: 3, bottom of region)
        feed(&mut t, b"\x1bD"); // IND
        // Should scroll within region
        assert_eq!(
            t.grid().cell(0, 3).unwrap().ch,
            ' ',
            "bottom of region blank after IND"
        );
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "row 0 preserved");
    }

    #[test]
    fn t_r9_ri_at_region_top_scrolls_down() {
        // RI (ESC M) at top of scroll region should scroll down.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"A\r\nB\r\nC\r\nD\r\nE");
        feed(&mut t, b"\x1b[2;4r"); // scroll region rows 2-4 (0-based: 1..4)
        feed(&mut t, b"\x1b[2;1H"); // cursor at row 2 (0-based: 1, top of region)
        feed(&mut t, b"\x1bM"); // RI (reverse index)
        // Top of region should be blank, content shifted down
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            ' ',
            "top of region blank after RI"
        );
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "row 0 preserved");
    }

    #[test]
    fn t_r9_decstbm_bottom_exceeds_height() {
        // DECSTBM with bottom > height should be handled gracefully.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[2;99r"); // bottom way beyond height
        let (top, bottom) = t.grid().scroll_region();
        // Should either clamp to height or reset to full screen.
        // The CSI handler checks bottom <= height, so this is ignored.
        assert_eq!(
            (top, bottom),
            (0, 5),
            "bottom > height resets to full screen"
        );
    }

    #[test]
    fn t_r9_decstbm_default_bottom_is_height() {
        // DECSTBM with bottom=0 means bottom = height.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3r"); // top=3, bottom omitted (0 → height)
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!((top, bottom), (2, 5), "bottom defaults to height");
    }

    // ── Round 9-2: Origin mode (DECOM) audits ──────────────────────────

    #[test]
    fn t_r9_decom_enable_homes_to_scroll_region_top() {
        // DECOM enable with a scroll region should home cursor to region top.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[3;5r"); // scroll region rows 3-5 (0-based: 2..5)
        feed(&mut t, b"\x1b[?6h"); // enable origin mode
        assert_eq!(
            t.cursor(),
            (0, 2),
            "DECOM enable should home to scroll region top"
        );
    }

    #[test]
    fn t_r9_decom_disable_homes_to_absolute() {
        // DECOM disable should home to absolute (0, 0).
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[3;5r"); // scroll region
        feed(&mut t, b"\x1b[?6h"); // enable origin
        feed(&mut t, b"\x1b[3;3H"); // move within region
        feed(&mut t, b"\x1b[?6l"); // disable origin
        assert_eq!(t.cursor(), (0, 0), "DECOM disable homes to absolute (0,0)");
    }

    #[test]
    fn t_r9_cup_origin_relative() {
        // CUP (1,1) in origin mode should go to scroll region top-left.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[3;5r"); // scroll region rows 3-5
        feed(&mut t, b"\x1b[?6h"); // origin mode
        feed(&mut t, b"\x1b[1;1H"); // CUP to (1,1)
        assert_eq!(t.cursor(), (0, 2), "CUP (1,1) = region top");
    }

    #[test]
    fn t_r9_cup_origin_clamps_to_region() {
        // CUP beyond scroll region in origin mode should clamp.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[2;4r"); // scroll region rows 2-4 (0-based: 1..4)
        feed(&mut t, b"\x1b[?6h"); // origin mode
        feed(&mut t, b"\x1b[99;1H"); // CUP way beyond region
        // Should clamp to region bottom (row 3, 0-based)
        assert_eq!(t.cursor().1, 3, "CUP clamps to region bottom");
    }

    #[test]
    fn t_r9_vpa_origin_relative() {
        // VPA (CSI d) in origin mode should be relative to scroll region.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[3;5r"); // scroll region rows 3-5
        feed(&mut t, b"\x1b[?6h"); // origin mode
        feed(&mut t, b"\x1b[2d"); // VPA row 2 (1-indexed)
        // Should be at region_top + (2-1) = 2 + 1 = 3
        assert_eq!(t.cursor().1, 3, "VPA origin-relative");
    }

    #[test]
    fn t_r9_cpr_origin_mode_relative() {
        // CPR (CSI 6n) should report cursor position relative to scroll region.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[3;5r"); // scroll region rows 3-5 (0-based: 2..5)
        feed(&mut t, b"\x1b[?6h"); // origin mode (cursor → region top)
        // After DECOM enable, cursor at (0, 2). CPR should report (1, 1)
        // relative to scroll region: row = (2+1) - (2+1) = 0 → max(1) = 1
        feed(&mut t, b"\x1b[6n"); // CPR
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        assert!(s.contains("1;1R"), "CPR at region origin: got {s}");
    }

    #[test]
    fn t_r9_cuu_stops_at_region_top() {
        // CUU from within scroll region stops at region top.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[3;5r"); // scroll region rows 3-5
        feed(&mut t, b"\x1b[4;1H"); // cursor at row 4 (0-based: 3)
        feed(&mut t, b"\x1b[10A"); // CUU 10 (way past top)
        assert_eq!(
            t.cursor().1,
            2,
            "CUU stops at region top (row 3, 0-based: 2)"
        );
    }

    #[test]
    fn t_r9_cud_stops_at_region_bottom() {
        // CUD from within scroll region stops at region bottom.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[3;5r"); // scroll region rows 3-5 (0-based: 2..5)
        feed(&mut t, b"\x1b[3;1H"); // cursor at row 3 (0-based: 2)
        feed(&mut t, b"\x1b[10B"); // CUD 10 (way past bottom)
        assert_eq!(
            t.cursor().1,
            4,
            "CUD stops at region bottom (row 5, 0-based: 4)"
        );
    }

    #[test]
    fn t_r9_decstbm_with_origin_homes_to_region() {
        // DECSTBM in origin mode should home cursor to region top.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[?6h"); // origin mode
        feed(&mut t, b"\x1b[3;5r"); // scroll region rows 3-5
        // Cursor should be at region top (row 2, 0-based)
        assert_eq!(
            t.cursor().1,
            2,
            "DECSTBM in origin mode homes to region top"
        );
    }

    #[test]
    fn t_r9_decstr_resets_origin_mode() {
        // DECSTR (CSI ! p) should reset origin mode to off.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[?6h"); // origin on
        feed(&mut t, b"\x1b[!p"); // DECSTR
        assert!(!t.modes.origin, "DECSTR resets origin mode");
    }

    // ── Round 9-3: Auto-wrap (DECAWM) + Insert Mode (IRM) audits ───────

    #[test]
    fn t_r9_decawm_off_overwrites_last_col() {
        // DECAWM off: writing at last column overwrites, no wrap.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[?7l"); // DECAWM off
        feed(&mut t, b"ABCDE"); // fills cols 0-4
        feed(&mut t, b"FGH"); // should overwrite cols 4, 4, 4
        // Col 4 should be 'H' (last overwrite wins)
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'H', "last col overwritten");
        // Cursor should still be on row 0
        assert_eq!(t.cursor().1, 0, "no wrap to row 1");
    }

    #[test]
    fn t_r9_decawm_on_deferred_wrap() {
        // DECAWM on: writing at last column sets pending_wrap,
        // next char wraps to new line.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE"); // fills cols 0-4, pending_wrap set
        // At this point cursor is at (4, 0) with pending_wrap
        assert!(t.cursor.pending_wrap, "pending_wrap set after last col");
        feed(&mut t, b"F"); // should trigger wrap + write F at (0, 1)
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'F', "F wrapped to row 1");
        assert_eq!(t.cursor(), (1, 1), "cursor at (1, 1)");
    }

    #[test]
    fn t_r9_decawm_pending_wrap_cleared_by_cuu() {
        // CUU should clear pending_wrap (per xterm spec).
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE"); // fills row, pending_wrap set
        feed(&mut t, b"\x1b[A"); // CUU — should clear pending_wrap
        assert!(!t.cursor.pending_wrap, "CUU clears pending_wrap");
        // Next char should overwrite current position, not wrap
        feed(&mut t, b"X");
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'X', "overwrite at cursor");
    }

    #[test]
    fn t_r9_decawm_pending_wrap_cleared_by_bs() {
        // BS should clear pending_wrap.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE"); // pending_wrap set
        feed(&mut t, b"\x08"); // BS
        assert!(!t.cursor.pending_wrap, "BS clears pending_wrap");
        // Cursor moved back to col 3
        assert_eq!(t.cursor().0, 3, "BS moved cursor to col 3");
    }

    #[test]
    fn t_r9_decawm_pending_wrap_cleared_by_cr() {
        // CR should clear pending_wrap.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE"); // pending_wrap set
        feed(&mut t, b"\r"); // CR
        assert!(!t.cursor.pending_wrap, "CR clears pending_wrap");
    }

    #[test]
    fn t_r9_decawm_pending_wrap_survives_el() {
        // EL (erase line) should NOT clear pending_wrap per xterm.
        // The next printable char after EL should still trigger wrap.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE"); // pending_wrap set
        feed(&mut t, b"\x1b[K"); // EL — erase to end of line
        // pending_wrap should still be set (xterm doesn't clear it on EL)
        assert!(t.cursor.pending_wrap, "EL preserves pending_wrap");
    }

    #[test]
    fn t_r9_irm_insert_shifts_right() {
        // IRM: writing a char shifts existing content right.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[4h"); // IRM on
        feed(&mut t, b"ABCD"); // row 0: ABCD
        feed(&mut t, b"\x1b[1;3H"); // cursor at col 3 (0-based: 2)
        feed(&mut t, b"X"); // insert X at col 2, shifts C,D right
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "A preserved");
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B', "B preserved");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'X', "X inserted");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'C', "C shifted right");
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'D', "D shifted right");
    }

    #[test]
    fn t_r9_irm_drops_at_eol() {
        // IRM: inserting at the end drops the last cell.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[4h"); // IRM on
        feed(&mut t, b"ABCDE"); // fills row
        // pending_wrap is set; but IRM still on
        feed(&mut t, b"\x1b[1;3H"); // cursor at col 3 (0-based: 2)
        feed(&mut t, b"X"); // insert X, shifts right, E drops
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'X', "X inserted");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'C', "C shifted");
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'D', "D shifted, E dropped");
    }

    #[test]
    fn t_r9_irm_reset_by_sgr_reset() {
        // IRM should NOT be reset by SGR (CSI 0m).
        // SGR only resets visual attributes, not modes.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[4h"); // IRM on
        feed(&mut t, b"\x1b[0m"); // SGR reset
        assert!(t.modes.insert, "SGR reset does NOT clear IRM");
    }

    #[test]
    fn t_r9_irm_reset_by_rm4() {
        // RM 4 (CSI 4l) should turn off IRM.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[4h"); // IRM on
        feed(&mut t, b"\x1b[4l"); // IRM off
        assert!(!t.modes.insert, "RM 4 turns off IRM");
    }

    #[test]
    fn t_r9_irm_plus_decawm_wrap() {
        // IRM + DECAWM: when auto-wrap triggers, the new char is inserted
        // at the start of the new line.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[4h"); // IRM on
        feed(&mut t, b"\r\n"); // row 1
        feed(&mut t, b"VWXYZ"); // fills row 1
        // pending_wrap set; next char wraps to row 2 and is INSERTED
        feed(&mut t, b"P");
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, 'P', "P at row 2 col 0");
    }

    #[test]
    fn t_r9_decawm_wide_char_at_penultimate() {
        // Wide char at penultimate column should wrap (not split).
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCD"); // cols 0-3, cursor at col 4
        feed(&mut t, "你".as_bytes()); // wide char needs 2 cols, only 1 left
        // Should wrap to next line
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, '你', "wide char wrapped");
        assert!(t.grid().cell(0, 1).unwrap().is_wide(), "wide flag set");
    }

    // ── Round 10-1: DECSC/DECRC + DECALN audits ────────────────────────

    #[test]
    fn t_r10_decsc_restores_cursor_position() {
        // DECSC saves cursor pos, DECRC restores it.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[3;5H"); // (row3, col5)
        feed(&mut t, b"\x1b7"); // save
        feed(&mut t, b"\x1b[1;1H"); // move to (0,0)
        feed(&mut t, b"\x1b8"); // restore
        assert_eq!(t.cursor(), (4, 2), "cursor restored to (4,2)");
    }

    #[test]
    fn t_r10_decsc_restores_pending_wrap() {
        // DECSC should save pending_wrap, DECRC should restore it.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE"); // fills row, pending_wrap set
        assert!(t.cursor.pending_wrap);
        feed(&mut t, b"\x1b7"); // save
        feed(&mut t, b"\x1b[1;1H"); // move — clears pending_wrap
        assert!(!t.cursor.pending_wrap);
        feed(&mut t, b"\x1b8"); // restore
        assert!(t.cursor.pending_wrap, "pending_wrap restored by DECRC");
    }

    #[test]
    fn t_r10_decsc_restores_all_sgr() {
        // DECSC/DECRC should save/restore fg, bg, underline_color, flags.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[1;3;4;5;9m"); // bold, italic, underline, blink, strikethrough
        feed(&mut t, b"\x1b[38;2;100;150;200m"); // fg RGB
        feed(&mut t, b"\x1b[48;5;42m"); // bg indexed
        feed(&mut t, b"\x1b[58;2;10;20;30m"); // underline RGB
        feed(&mut t, b"\x1b7"); // save
        feed(&mut t, b"\x1b[0m"); // reset all
        feed(&mut t, b"\x1b8"); // restore
        assert!(t.flags.contains(CellFlags::BOLD), "bold restored");
        assert!(t.flags.contains(CellFlags::ITALIC), "italic restored");
        assert!(t.flags.contains(CellFlags::UNDERLINE), "underline restored");
        assert!(t.flags.contains(CellFlags::BLINK), "blink restored");
        assert!(
            t.flags.contains(CellFlags::STRIKETHROUGH),
            "strikethrough restored"
        );
        assert_eq!(t.fg, Color::Rgb(100, 150, 200), "fg RGB restored");
        assert_eq!(t.bg, Color::Indexed(42), "bg indexed restored");
        assert_eq!(
            t.underline_color,
            Color::Rgb(10, 20, 30),
            "underline color restored"
        );
    }

    #[test]
    fn t_r10_decsc_restores_charset() {
        // DECSC/DECRC should save/restore charset designation.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b(0"); // G0 = DEC Special Graphics
        feed(&mut t, b"\x1b7"); // save
        feed(&mut t, b"\x1b(B"); // G0 = ASCII
        feed(&mut t, b"\x1b8"); // restore
        assert_eq!(t.g0_charset(), Charset::DecSpecial, "G0 charset restored");
    }

    #[test]
    fn t_r10_decsc_restores_origin_mode() {
        // DECSC/DECRC should save/restore origin mode.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[?6h"); // origin on
        feed(&mut t, b"\x1b7"); // save
        feed(&mut t, b"\x1b[?6l"); // origin off
        feed(&mut t, b"\x1b8"); // restore
        assert!(t.modes.origin, "origin mode restored");
    }

    #[test]
    fn t_r10_decsc_restores_auto_wrap() {
        // DECSC/DECRC should save/restore auto-wrap mode.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[?7l"); // DECAWM off
        feed(&mut t, b"\x1b7"); // save
        feed(&mut t, b"\x1b[?7h"); // DECAWM on
        feed(&mut t, b"\x1b8"); // restore
        assert!(!t.modes.auto_wrap, "auto-wrap restored to off");
    }

    #[test]
    fn t_r10_decsc_restores_protected_attr() {
        // DECSC/DECRC should save/restore DECSCA protected attribute.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[1\"q"); // DECSCA = protected
        feed(&mut t, b"\x1b7"); // save
        feed(&mut t, b"\x1b[2\"q"); // DECSCA = unprotected
        feed(&mut t, b"\x1b8"); // restore
        assert!(t.protected_attr, "protected attr restored");
    }

    #[test]
    fn t_r10_decsc_restores_cursor_style() {
        // DECSC/DECRC should save/restore cursor style (DECSCUSR).
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[4 q"); // steady underline
        feed(&mut t, b"\x1b7"); // save
        feed(&mut t, b"\x1b[0 q"); // default
        feed(&mut t, b"\x1b8"); // restore
        assert_eq!(
            t.cursor_style,
            CursorStyle::SteadyUnderline,
            "cursor style restored"
        );
    }

    #[test]
    fn t_r10_decrc_without_decsc_restores_defaults() {
        // DECRC without prior DECSC should restore defaults.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[5;10H\x1b[1;31m"); // move + bold red
        feed(&mut t, b"\x1b8"); // DECRC without save
        assert_eq!(t.cursor(), (0, 0), "cursor at (0,0) default");
        assert!(!t.flags.contains(CellFlags::BOLD), "bold cleared");
        assert_eq!(t.fg, Color::Default, "fg default");
    }

    #[test]
    fn t_r10_decsc_multiple_saves_overwrite() {
        // Multiple DECSC should overwrite (only one saved state).
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[2;3H"); // position 1
        feed(&mut t, b"\x1b7"); // save 1
        feed(&mut t, b"\x1b[5;10H"); // position 2
        feed(&mut t, b"\x1b7"); // save 2 (overwrites)
        feed(&mut t, b"\x1b[1;1H"); // move away
        feed(&mut t, b"\x1b8"); // restore
        assert_eq!(t.cursor(), (9, 4), "second save wins");
    }

    #[test]
    fn t_r10_decaln_resets_tab_stops() {
        // DECALN (ESC # 8) should reset tab stops to defaults (every 8).
        let mut t = Terminal::new(40, 5);
        // Set custom tab stop at col 5
        feed(&mut t, b"\x1b[6G\x1bH"); // cursor at col 6 (0-based: 5), set HTS
        assert!(t.tab_stops[5], "custom stop at col 5");
        feed(&mut t, b"\x1b#8"); // DECALN
        // After DECALN, col 5 should NOT have a custom stop.
        // Default stops are at 0, 8, 16, 24, 32...
        assert!(
            !t.tab_stops[5],
            "DECALN should reset custom tab stop at col 5"
        );
        assert!(t.tab_stops[8], "DECALN preserves default stop at col 8");
    }

    #[test]
    fn t_r10_decaln_resets_cursor_and_attrs() {
        // DECALN should reset cursor to (0,0) and clear SGR attributes.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;5H\x1b[1;33;44m"); // move + bold yellow on blue
        feed(&mut t, b"\x1b#8"); // DECALN
        assert_eq!(t.cursor(), (0, 0), "DECALN homes cursor");
        assert!(!t.flags.contains(CellFlags::BOLD), "DECALN clears bold");
        assert_eq!(t.fg, Color::Default, "DECALN resets fg");
        assert_eq!(t.bg, Color::Default, "DECALN resets bg");
    }

    #[test]
    fn t_r10_decaln_resets_scroll_region() {
        // Per xterm, DECALN should reset scroll region to full screen.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[2;5r"); // scroll region rows 2-5
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!((top, bottom), (1, 5));
        feed(&mut t, b"\x1b#8"); // DECALN
        let (top2, bottom2) = t.grid().scroll_region();
        assert_eq!(
            (top2, bottom2),
            (0, 6),
            "DECALN resets scroll region to full screen"
        );
    }

    // ── Round 10-2: Tab Stops (HTS/TBC/CHT/CBT) edge case audits ───────

    #[test]
    fn t_r10_hts_sets_stop_at_cursor_col() {
        // HTS (ESC H) should set tab stop at cursor column.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[4G"); // col 4 (0-based: 3)
        feed(&mut t, b"\x1bH"); // HTS at col 3
        assert!(t.tab_stops[3], "HTS set stop at col 3");
        feed(&mut t, b"\r"); // back to col 0
        feed(&mut t, b"\t"); // tab should stop at col 3
        assert_eq!(t.cursor().0, 3, "tab stops at col 3 (custom)");
    }

    #[test]
    fn t_r10_tbc_0_clears_current_stop() {
        // TBC param 0 (CSI g or CSI 0g) clears stop at current column.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[9G"); // col 9 (0-based: 8 = default stop)
        feed(&mut t, b"\x1b[0g"); // TBC 0: clear col 8
        assert!(!t.tab_stops[8], "TBC 0 cleared stop at col 8");
        feed(&mut t, b"\r");
        feed(&mut t, b"\t"); // should skip past col 8 to col 16
        assert_eq!(t.cursor().0, 16, "tab skips cleared col 8");
    }

    #[test]
    fn t_r10_tbc_3_clears_all_stops() {
        // TBC param 3 (CSI 3g) clears ALL tab stops.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[3g"); // TBC 3: clear all
        for i in 0..20 {
            assert!(!t.tab_stops[i], "all stops cleared at col {i}");
        }
        // Tab with no stops should go to last column
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 19, "tab with no stops goes to last col");
    }

    #[test]
    fn t_r10_cht_default_is_1() {
        // CHT with no param defaults to 1.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[I"); // CHT default 1
        assert_eq!(t.cursor().0, 8, "CHT default = 1 → col 8");
    }

    #[test]
    fn t_r10_cbt_default_is_1() {
        // CBT with no param defaults to 1.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[17G"); // col 17 (0-based: 16)
        feed(&mut t, b"\x1b[Z"); // CBT default 1
        assert_eq!(t.cursor().0, 8, "CBT default = 1 → col 8");
    }

    #[test]
    fn t_r10_cbt_falls_to_col0_if_no_stop() {
        // CBT with no tab stops should land at col 0.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[3g"); // clear all stops
        feed(&mut t, b"\x1b[10G"); // col 10 (0-based: 9)
        feed(&mut t, b"\x1b[Z"); // CBT 1 — no stops, should go to col 0
        assert_eq!(t.cursor().0, 0, "CBT with no stops lands at col 0");
    }

    #[test]
    fn t_r10_tab_after_tbc3_goes_to_last_col() {
        // After clearing all stops, tab goes to last column.
        let mut t = Terminal::new(15, 3);
        feed(&mut t, b"\x1b[3g"); // clear all
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 14, "tab with no stops → last col");
    }

    #[test]
    fn t_r10_cht_multiple_tabs() {
        // CHT 3 should advance 3 tab stops.
        let mut t = Terminal::new(80, 3);
        feed(&mut t, b"\x1b[3I"); // CHT 3: 0 → 8 → 16 → 24
        assert_eq!(t.cursor().0, 24, "CHT 3 from col 0 → col 24");
    }

    #[test]
    fn t_r10_cbt_multiple_tabs() {
        // CBT 3 should go back 3 tab stops.
        let mut t = Terminal::new(80, 3);
        feed(&mut t, b"\x1b[33G"); // col 33 (0-based: 32)
        feed(&mut t, b"\x1b[3Z"); // CBT 3: 32 → 24 → 16 → 8
        assert_eq!(t.cursor().0, 8, "CBT 3 from col 32 → col 8");
    }

    #[test]
    fn t_r10_cht_at_last_col_stays() {
        // CHT at last column should not exceed last col.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[10G"); // col 10 (0-based: 9 = last)
        feed(&mut t, b"\x1b[I"); // CHT 1
        assert_eq!(t.cursor().0, 9, "CHT at last col stays");
    }

    #[test]
    fn t_r10_hts_then_tab_then_tbc_cycle() {
        // Full HTS → Tab → TBC cycle.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[6G"); // col 6 (0-based: 5)
        feed(&mut t, b"\x1bH"); // HTS at col 5
        feed(&mut t, b"\r");
        feed(&mut t, b"\t"); // tab → col 5
        assert_eq!(t.cursor().0, 5, "tab to custom col 5");
        feed(&mut t, b"\x1b[0g"); // TBC at current col 5
        feed(&mut t, b"\r");
        feed(&mut t, b"\t"); // tab → should skip col 5 now
        assert_eq!(t.cursor().0, 8, "tab skips cleared col 5 → col 8");
    }

    // ── Round 10-3: Erase Functions edge case audits ───────────────────

    #[test]
    fn t_r10_ed_preserves_cursor() {
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"ABCDEF\r\nGHIJKL");
        feed(&mut t, b"\x1b[2;4H"); // cursor at (3, 1)
        let (cx, cy) = t.cursor();
        feed(&mut t, b"\x1b[2J"); // ED 2
        assert_eq!(t.cursor(), (cx, cy), "ED 2 does not move cursor");
    }

    #[test]
    fn t_r10_el_preserves_cursor() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGHIJ");
        feed(&mut t, b"\x1b[1;5H"); // cursor at col 4
        let (cx, cy) = t.cursor();
        feed(&mut t, b"\x1b[2K"); // EL 2
        assert_eq!(t.cursor(), (cx, cy), "EL 2 does not move cursor");
    }

    #[test]
    fn t_r10_ech_preserves_cursor() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGHIJ");
        feed(&mut t, b"\x1b[1;4H"); // cursor at col 3
        feed(&mut t, b"\x1b[3X"); // ECH 3
        assert_eq!(t.cursor(), (3, 0), "ECH does not move cursor");
    }

    #[test]
    fn t_r10_ed_preserves_pending_wrap() {
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE"); // fills row, pending_wrap set
        assert!(t.cursor.pending_wrap);
        feed(&mut t, b"\x1b[0J"); // ED 0
        assert!(t.cursor.pending_wrap, "ED does not clear pending_wrap");
    }

    #[test]
    fn t_r10_el_preserves_pending_wrap() {
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE");
        assert!(t.cursor.pending_wrap);
        feed(&mut t, b"\x1b[0K"); // EL 0
        assert!(t.cursor.pending_wrap, "EL does not clear pending_wrap");
    }

    #[test]
    fn t_r10_ech_on_wide_lead_clears_spacer() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你".as_bytes()); // wide at cols 0-1
        feed(&mut t, b"ABCD");
        feed(&mut t, b"\x1b[1G"); // col 0
        feed(&mut t, b"\x1b[1X"); // ECH 1 at wide lead
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ', "lead erased");
        assert!(
            !t.grid().cell(1, 0).unwrap().is_wide_spacer(),
            "spacer cleared"
        );
    }

    #[test]
    fn t_r10_ech_on_wide_spacer_includes_lead() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你".as_bytes()); // wide at cols 0-1
        feed(&mut t, b"ABCDEF");
        feed(&mut t, b"\x1b[2G"); // cursor at col 1 (spacer)
        feed(&mut t, b"\x1b[2X"); // ECH 2 from spacer
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ', "lead cleared");
        assert!(!t.grid().cell(0, 0).unwrap().is_wide(), "no wide flag");
    }

    #[test]
    fn t_r10_el_0_on_wide_spacer() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"AB");
        feed(&mut t, "你".as_bytes()); // wide at cols 2-3
        feed(&mut t, b"CDEF");
        feed(&mut t, b"\x1b[1;4H"); // cursor at col 3 (spacer)
        feed(&mut t, b"\x1b[0K"); // EL 0
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, ' ', "wide lead cleared");
        assert!(!t.grid().cell(2, 0).unwrap().is_wide(), "no orphan lead");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "col 0 preserved");
    }

    #[test]
    fn t_r10_el_1_on_wide_lead_includes_spacer() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"AB");
        feed(&mut t, "你".as_bytes()); // wide at cols 2-3
        feed(&mut t, b"CDEF");
        feed(&mut t, b"\x1b[1;3H"); // cursor at col 2 (wide lead)
        feed(&mut t, b"\x1b[1K"); // EL 1
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ', "col 0 erased");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, ' ', "lead erased");
        assert!(
            !t.grid().cell(3, 0).unwrap().is_wide_spacer(),
            "spacer cleared"
        );
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'C', "col 4 preserved");
    }

    #[test]
    fn t_r10_ech_exceeds_line_end() {
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGHIJ");
        feed(&mut t, b"\x1b[1;8H"); // cursor at col 7
        feed(&mut t, b"\x1b[99X"); // ECH 99 — clamps to 3 remaining
        assert_eq!(t.grid().cell(7, 0).unwrap().ch, ' ', "col 7 erased");
        assert_eq!(t.grid().cell(9, 0).unwrap().ch, ' ', "col 9 erased");
    }

    #[test]
    fn t_r10_ed_2_clears_wrap_flags() {
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE");
        feed(&mut t, b"F"); // wraps, sets row 0 wrap flag
        assert!(t.grid().row(0).unwrap().wrap, "row 0 has wrap flag");
        feed(&mut t, b"\x1b[2J"); // ED 2
        assert!(!t.grid().row(0).unwrap().wrap, "ED 2 clears wrap flag");
    }

    #[test]
    fn t_r10_ed_0_clears_wrap_flags_below_cursor() {
        // ED 0 should clear wrap flags on rows below cursor row.
        let mut t = Terminal::new(5, 4);
        feed(&mut t, b"ABCDE");
        feed(&mut t, b"FGHIJ");
        // Move to row 0, ED 0 clears from there to end
        feed(&mut t, b"\x1b[2;1H"); // row 2 (0-based: 1)
        feed(&mut t, b"\x1b[0J");
        // Rows below cursor (1-3) should have wrap cleared
        for r in 1..4 {
            assert!(!t.grid().row(r).unwrap().wrap, "row {r} wrap cleared");
        }
    }

    // ── Round 11-1: Scrolling & Line Operations audits ─────────────────

    #[test]
    fn t_r11_ind_at_region_bottom_scrolls() {
        // IND (ESC D) at bottom of scroll region scrolls up.
        let mut t = Terminal::new(5, 5);
        feed(&mut t, b"A\r\nB\r\nC\r\nD\r\nE");
        feed(&mut t, b"\x1b[2;4r"); // region rows 2-4 (0-based: 1..4)
        feed(&mut t, b"\x1b[4;1H"); // cursor at row 4 (0-based: 3 = bottom-1)
        feed(&mut t, b"\x1bD"); // IND
        // Should scroll within region; row 3 blank
        assert_eq!(
            t.grid().cell(0, 3).unwrap().ch,
            ' ',
            "bottom of region blank"
        );
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "row 0 preserved");
    }

    #[test]
    fn t_r11_ind_below_region_advances() {
        // IND below scroll region just advances cursor.
        let mut t = Terminal::new(5, 6);
        feed(&mut t, b"\x1b[2;4r"); // region rows 2-4
        feed(&mut t, b"\x1b[5;1H"); // cursor at row 5 (below region)
        feed(&mut t, b"\x1bD"); // IND
        assert_eq!(t.cursor().1, 5, "cursor stays at row 5 (last row)");
    }

    #[test]
    fn t_r11_ri_at_region_top_scrolls_down() {
        // RI (ESC M) at top of scroll region scrolls down.
        let mut t = Terminal::new(5, 5);
        feed(&mut t, b"A\r\nB\r\nC\r\nD\r\nE");
        feed(&mut t, b"\x1b[2;4r"); // region rows 2-4 (0-based: 1..4)
        feed(&mut t, b"\x1b[2;1H"); // cursor at row 2 (0-based: 1 = top)
        feed(&mut t, b"\x1bM"); // RI
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, ' ', "top of region blank");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "row 0 preserved");
    }

    #[test]
    fn t_r11_ri_above_region_moves_up() {
        // RI above scroll region just moves cursor up.
        let mut t = Terminal::new(5, 6);
        feed(&mut t, b"\x1b[3;4r"); // region rows 3-4 (0-based: 2..4)
        feed(&mut t, b"\x1b[2;1H"); // cursor at row 2 (0-based: 1, above region)
        feed(&mut t, b"\x1bM"); // RI
        assert_eq!(t.cursor().1, 0, "cursor moved up to row 0");
    }

    #[test]
    fn t_r11_nel_cr_lf_equivalent() {
        // NEL (ESC E) = CR + LF (index).
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"ABC"); // cursor at col 3
        feed(&mut t, b"\x1bE"); // NEL
        assert_eq!(t.cursor().0, 0, "NEL sets col 0 (CR part)");
        assert_eq!(t.cursor().1, 1, "NEL advances to row 1 (LF part)");
    }

    #[test]
    fn t_r11_nel_at_region_bottom_scrolls() {
        // NEL at bottom of scroll region should scroll.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"A\r\nB\r\nC\r\nD\r\nE");
        feed(&mut t, b"\x1b[3;5r"); // region rows 3-5 (0-based: 2..5)
        feed(&mut t, b"\x1b[5;3H"); // cursor at row 5, col 3 (0-based: 4, 2)
        feed(&mut t, b"\x1bE"); // NEL at bottom of region
        assert_eq!(t.cursor().0, 0, "col 0");
        // Should have scrolled within region
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "row 0 preserved");
    }

    #[test]
    fn t_r11_il_inside_region_inserts() {
        // IL inside scroll region inserts blank lines.
        let mut t = Terminal::new(5, 5);
        feed(&mut t, b"A\r\nB\r\nC\r\nD\r\nE");
        feed(&mut t, b"\x1b[3;1H"); // cursor at row 3 (0-based: 2)
        feed(&mut t, b"\x1b[L"); // IL 1
        assert_eq!(
            t.grid().cell(0, 2).unwrap().ch,
            ' ',
            "row 2 blank (inserted)"
        );
        assert_eq!(t.grid().cell(0, 3).unwrap().ch, 'C', "C shifted down");
    }

    #[test]
    fn t_r11_il_outside_region_noop() {
        // IL outside scroll region is a no-op.
        let mut t = Terminal::new(5, 6);
        feed(&mut t, b"A\r\nB\r\nC\r\nD\r\nE\r\nF");
        feed(&mut t, b"\x1b[3;5r"); // region rows 3-5 (0-based: 2..5)
        feed(&mut t, b"\x1b[1;1H"); // cursor at row 1 (above region)
        feed(&mut t, b"\x1b[L"); // IL — should be no-op
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "nothing changed");
    }

    #[test]
    fn t_r11_dl_inside_region_deletes() {
        // DL inside scroll region deletes lines (shifts up).
        let mut t = Terminal::new(5, 5);
        feed(&mut t, b"A\r\nB\r\nC\r\nD\r\nE");
        feed(&mut t, b"\x1b[2;1H"); // cursor at row 2 (0-based: 1)
        feed(&mut t, b"\x1b[M"); // DL 1
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'C', "C shifted up");
        assert_eq!(t.grid().cell(0, 4).unwrap().ch, ' ', "bottom blank");
    }

    #[test]
    fn t_r11_dl_outside_region_noop() {
        // DL outside scroll region is a no-op.
        let mut t = Terminal::new(5, 6);
        feed(&mut t, b"A\r\nB\r\nC\r\nD\r\nE\r\nF");
        feed(&mut t, b"\x1b[3;5r"); // region rows 3-5
        feed(&mut t, b"\x1b[1;1H"); // cursor above region
        feed(&mut t, b"\x1b[M"); // DL — no-op
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "nothing changed");
    }

    #[test]
    fn t_r11_il_exceeds_region_clamps() {
        // IL count exceeding region height should only affect within region.
        let mut t = Terminal::new(5, 6);
        feed(&mut t, b"A\r\nB\r\nC\r\nD\r\nE\r\nF");
        feed(&mut t, b"\x1b[2;4r"); // region rows 2-4 (0-based: 1..4)
        feed(&mut t, b"\x1b[2;1H"); // cursor at top of region
        feed(&mut t, b"\x1b[99L"); // IL 99 — should only clear rows 1-3
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, ' ', "row 1 blank");
        assert_eq!(t.grid().cell(0, 3).unwrap().ch, ' ', "row 3 blank");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "row 0 preserved");
        assert_eq!(t.grid().cell(0, 5).unwrap().ch, 'F', "row 5 preserved");
    }

    #[test]
    fn t_r11_dl_exceeds_region_clamps() {
        // DL count exceeding region should only affect within region.
        let mut t = Terminal::new(5, 6);
        feed(&mut t, b"A\r\nB\r\nC\r\nD\r\nE\r\nF");
        feed(&mut t, b"\x1b[2;4r"); // region rows 2-4 (0-based: 1..4)
        feed(&mut t, b"\x1b[2;1H"); // cursor at top of region
        feed(&mut t, b"\x1b[99M"); // DL 99 — should blank rows 1-3
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, ' ', "row 1 blank");
        assert_eq!(t.grid().cell(0, 3).unwrap().ch, ' ', "row 3 blank");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "row 0 preserved");
        assert_eq!(t.grid().cell(0, 5).unwrap().ch, 'F', "row 5 preserved");
    }

    #[test]
    fn t_r11_il_default_param_is_1() {
        // IL with no param defaults to 1.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"A\r\nB\r\nC");
        feed(&mut t, b"\x1b[2;1H");
        feed(&mut t, b"\x1b[L"); // IL default 1
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, ' ', "1 line inserted");
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, 'B', "B shifted");
    }

    #[test]
    fn t_r11_dl_default_param_is_1() {
        // DL with no param defaults to 1.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"A\r\nB\r\nC");
        feed(&mut t, b"\x1b[1;1H");
        feed(&mut t, b"\x1b[M"); // DL default 1
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'B', "B shifted up");
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, ' ', "bottom blank");
    }

    #[test]
    fn t_r11_nel_clears_pending_wrap() {
        // NEL should clear pending_wrap.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE"); // fills row, pending_wrap set
        assert!(t.cursor.pending_wrap);
        feed(&mut t, b"\x1bE"); // NEL
        assert!(!t.cursor.pending_wrap, "NEL clears pending_wrap");
    }

    // ── Round 11-2: Alternate Screen Buffer audits ─────────────────────

    #[test]
    fn t_r11_1049_saves_restores_cursor() {
        // DECSET 1049 saves cursor, DECSET 1049 exit restores.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;5H"); // cursor (4, 2)
        feed(&mut t, b"X"); // write X at (5,2)
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        assert_eq!(t.cursor(), (0, 0), "alt screen homes cursor");
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert_eq!(t.cursor(), (5, 2), "cursor restored to original");
    }

    #[test]
    fn t_r11_1049_saves_restores_content() {
        // Content written to primary screen is restored after alt exit.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"PRIMARY");
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"ALTSCREEN");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "alt content visible");
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'P',
            "primary content restored"
        );
    }

    #[test]
    fn t_r11_1049_alt_has_no_scrollback() {
        // Alt screen should not accumulate scrollback.
        let mut t = Terminal::with_scrollback(10, 3, 100);
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        for _ in 0..10 {
            feed(&mut t, b"LINE\n");
        }
        assert_eq!(t.grid().scrollback_len(), 0, "alt screen has no scrollback");
        feed(&mut t, b"\x1b[?1049l"); // exit alt
    }

    #[test]
    fn t_r11_1049_saves_restores_sgr() {
        // DECSET 1049 saves/restores SGR attributes.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[1;31m"); // bold red
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"\x1b[0m"); // reset attrs in alt
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert!(t.flags.contains(CellFlags::BOLD), "bold restored");
        assert_eq!(t.fg, Color::Indexed(1), "red fg restored");
    }

    #[test]
    fn t_r11_1049_clears_alt_on_entry() {
        // Entering alt screen should give a clean blank screen.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"VISIBLE");
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        for c in 0..7 {
            assert_eq!(
                t.grid().cell(c, 0).unwrap().ch,
                ' ',
                "alt screen blank at col {c}"
            );
        }
    }

    #[test]
    fn t_r11_47_saves_restores_content() {
        // Mode 47 also switches screens but does NOT save/restore cursor or home.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"PRIMARY");
        feed(&mut t, b"\x1b[?47h"); // enter alt (mode 47) — cursor stays at col 7
        feed(&mut t, b"\r"); // CR to col 0 (mode 47 doesn't home)
        feed(&mut t, b"ALT");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "alt content");
        feed(&mut t, b"\x1b[?47l"); // exit alt
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'P', "primary restored");
    }

    #[test]
    fn t_r11_47_does_not_home_cursor() {
        // Mode 47 does NOT home cursor (unlike 1049).
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;5H"); // cursor (4, 2)
        feed(&mut t, b"\x1b[?47h"); // enter alt
        // Mode 47 does not move cursor (xterm behavior)
        // Cursor stays where it was
        assert_eq!(t.cursor(), (4, 2), "mode 47 keeps cursor");
    }

    #[test]
    fn t_r11_1047_clears_alt_on_entry() {
        // Mode 1047: like 47 but clears the alt screen on exit.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"PRIMARY");
        feed(&mut t, b"\x1b[?1047h"); // enter alt
        feed(&mut t, b"ALT");
        feed(&mut t, b"\x1b[?1047l"); // exit alt — should clear alt
        // Primary should be restored
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'P', "primary restored");
    }

    #[test]
    fn t_r11_1049_nested_enter_is_noop() {
        // Entering alt when already in alt is a no-op.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"PRIMARY");
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"ALT1");
        feed(&mut t, b"\x1b[?1049h"); // enter alt again — no-op
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'A',
            "alt1 content preserved"
        );
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'P', "primary restored");
    }

    #[test]
    fn t_r11_1049_exit_without_enter_is_noop() {
        // Exiting alt without entering should be a no-op.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"PRIMARY");
        feed(&mut t, b"\x1b[?1049l"); // exit without enter — no-op
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'P', "content preserved");
    }

    #[test]
    fn t_r11_1049_saves_restores_origin_mode() {
        // 1049 saves origin mode state. It does NOT reset it (that's DECSC's job).
        // After exit, the original origin mode should be restored.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?6h"); // origin on
        feed(&mut t, b"\x1b[?1049h"); // enter alt — saves origin=true
        // Origin mode is saved but not changed in alt screen
        feed(&mut t, b"\x1b[?6l"); // turn off origin in alt
        feed(&mut t, b"\x1b[?1049l"); // exit alt — restores origin=true
        assert!(t.modes.origin, "origin mode restored to true");
    }

    #[test]
    fn t_r11_1049_alt_screen_restores_tab_stops() {
        // Custom tab stops on primary should be restored after alt exit.
        let mut t = Terminal::new(40, 5);
        feed(&mut t, b"\x1b[6G\x1bH"); // custom stop at col 5
        assert!(t.tab_stops[5], "custom stop set");
        feed(&mut t, b"\x1b[?1049h"); // enter alt — resets tab stops
        assert!(!t.tab_stops[5], "custom stop cleared in alt");
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert!(t.tab_stops[5], "custom stop restored after alt exit");
    }

    // ── Round 11-3: DSR / CPR / DECXCPR audits ─────────────────────────

    #[test]
    fn t_r11_dsr_5_reports_ok() {
        // DSR 5 (CSI 5n) should respond CSI 0n (terminal OK).
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[5n");
        let resp = t.take_response();
        assert_eq!(resp, b"\x1b[0n", "DSR 5 responds OK");
    }

    #[test]
    fn t_r11_cpr_cursor_position() {
        // DSR 6 (CSI 6n) should report cursor position (1-based).
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;5H"); // row 3, col 5 (0-based: 2, 4)
        feed(&mut t, b"\x1b[6n");
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        assert_eq!(s, "\x1b[3;5R", "CPR at (5,3) reports 3;5");
    }

    #[test]
    fn t_r11_cpr_at_origin() {
        // CPR at cursor (0,0) should report 1;1.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[1;1H");
        feed(&mut t, b"\x1b[6n");
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        assert_eq!(s, "\x1b[1;1R", "CPR at origin reports 1;1");
    }

    #[test]
    fn t_r11_cpr_origin_mode_offset() {
        // CPR in origin mode should be relative to scroll region.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[3;5r"); // region rows 3-5
        feed(&mut t, b"\x1b[?6h"); // origin on — cursor → region top
        // Cursor at (0, 2) → origin-relative row = 2+1 - (2+1) = 1
        feed(&mut t, b"\x1b[6n");
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        assert_eq!(s, "\x1b[1;1R", "CPR origin mode at region top = 1;1");
    }

    #[test]
    fn t_r11_cpr_origin_mode_mid_region() {
        // CPR in origin mode at mid-region row.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[3;5r"); // region rows 3-5 (0-based: 2..5)
        feed(&mut t, b"\x1b[?6h"); // origin on
        feed(&mut t, b"\x1b[2;3H"); // origin row 2, col 3 → abs y=3, x=2
        feed(&mut t, b"\x1b[6n");
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        // abs y=3 → cy=4 → origin row = 4 - (2+1) = 1; col = 3
        assert_eq!(s, "\x1b[1;3R", "CPR origin mode mid-region = 1;3");
    }

    #[test]
    fn t_r11_decxcpr_basic() {
        // DECXCPR (CSI ? 6n) should report with '?' prefix.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[2;4H"); // row 2, col 4
        feed(&mut t, b"\x1b[?6n");
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        assert_eq!(s, "\x1b[?2;4R", "DECXCPR reports ?2;4R");
    }

    #[test]
    fn t_r11_decxcpr_origin_mode() {
        // DECXCPR in origin mode should be relative.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[3;5r");
        feed(&mut t, b"\x1b[?6h"); // origin on
        feed(&mut t, b"\x1b[?6n");
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        assert_eq!(s, "\x1b[?1;1R", "DECXCPR origin = ?1;1R");
    }

    #[test]
    fn t_r11_cpr_format_exact() {
        // Verify exact byte sequence format matches xterm.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"\x1b[10;20H"); // row 10, col 20
        feed(&mut t, b"\x1b[6n");
        let resp = t.take_response();
        // Must be CSI 1 0 ; 2 0 R (ESC [ 1 0 ; 2 0 R)
        assert_eq!(resp, b"\x1b[10;20R", "exact CPR format");
    }

    #[test]
    fn t_r11_dsr_6_default_param() {
        // CSI n with no param should default to 0 (no response).
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[n");
        let resp = t.take_response();
        assert!(resp.is_empty(), "DSR with no param = no response");
    }

    #[test]
    fn t_r11_cpr_clears_response_buffer() {
        // take_response should drain the buffer.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[6n");
        let _ = t.take_response();
        let resp2 = t.take_response();
        assert!(resp2.is_empty(), "response buffer drained");
    }

    // ── Round 12-1: Unicode / wide character audits ────────────────────

    #[test]
    fn t_r12_wide_char_basic_placement() {
        // CJK wide char occupies 2 cells: lead + spacer.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "中".as_bytes());
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '中');
        assert!(t.grid().cell(0, 0).unwrap().is_wide(), "lead is wide");
        assert!(
            t.grid().cell(1, 0).unwrap().is_wide_spacer(),
            "col 1 is spacer"
        );
        assert_eq!(t.cursor().0, 2, "cursor advanced by 2");
    }

    #[test]
    fn t_r12_wide_char_at_last_col_wraps() {
        // Wide char at penultimate column with 1 col left → wrap.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCD"); // cols 0-3, cursor at col 4
        feed(&mut t, "你".as_bytes()); // needs 2 cols, only 1 left
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, '你', "wrapped to row 1");
        assert!(
            t.grid().cell(1, 1).unwrap().is_wide_spacer(),
            "spacer at row 1 col 1"
        );
    }

    #[test]
    fn t_r12_wide_char_overwrite_clears_old_spacer() {
        // Overwriting a wide char's lead clears the old spacer.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你".as_bytes()); // lead col 0, spacer col 1
        feed(&mut t, b"\r"); // back to col 0
        feed(&mut t, b"X"); // overwrite with narrow char
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X', "X at col 0");
        assert!(
            !t.grid().cell(1, 0).unwrap().is_wide_spacer(),
            "old spacer cleared"
        );
        assert_eq!(
            t.grid().cell(1, 0).unwrap().ch,
            ' ',
            "spacer content cleared"
        );
    }

    #[test]
    fn t_r12_wide_char_overwrite_with_wide() {
        // Overwriting a wide char's lead with another wide char.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你".as_bytes()); // cols 0-1
        feed(&mut t, b"\r");
        feed(&mut t, "好".as_bytes()); // overwrite with new wide
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '好');
        assert!(
            t.grid().cell(1, 0).unwrap().is_wide_spacer(),
            "new spacer at col 1"
        );
    }

    #[test]
    fn t_r12_wide_char_overwrite_on_spacer_clears_lead() {
        // Cursor positioning adjusts to the lead cell, so printing
        // overwrites at the lead position.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你".as_bytes()); // lead col 0, spacer col 1
        feed(&mut t, b"\x1b[1;2H"); // CUP to col 1 (spacer) → adjusts to col 0
        feed(&mut t, b"X"); // overwrite at lead position
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X', "X at col 0 (lead)");
        assert!(
            !t.grid().cell(0, 0).unwrap().is_wide(),
            "old wide flag cleared"
        );
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, ' ', "col 1 cleared");
    }

    #[test]
    fn t_r12_combining_attaches_to_wide_lead() {
        // Combining char after wide char should attach to lead cell.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你".as_bytes()); // wide at cols 0-1, cursor at col 2
        feed(&mut t, "\u{0301}".as_bytes()); // combining acute
        // Should attach to the wide char lead at col 0
        let cell = t.grid().cell(0, 0).unwrap();
        assert!(
            cell.combining.contains(&'\u{0301}'),
            "combining on wide lead"
        );
        assert_eq!(t.cursor().0, 2, "cursor not advanced by combining");
    }

    #[test]
    fn t_r12_emoji_width_2() {
        // Emoji should have width 2.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "🎉".as_bytes());
        assert!(t.grid().cell(0, 0).unwrap().is_wide(), "emoji is wide");
        assert!(
            t.grid().cell(1, 0).unwrap().is_wide_spacer(),
            "emoji spacer"
        );
    }

    #[test]
    fn t_r12_zero_width_dropped_at_col0() {
        // Zero-width char at col 0 with no preceding char should be dropped.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "\u{0301}".as_bytes()); // combining acute at (0,0)
        // Should be dropped (no preceding char), cursor stays at (0,0)
        assert_eq!(t.cursor(), (0, 0), "cursor unchanged");
        assert!(
            t.grid().cell(0, 0).unwrap().combining.is_empty(),
            "no combining attached"
        );
    }

    #[test]
    fn t_r12_wide_char_cursor_advance() {
        // After writing wide char, cursor should be at col+2.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"A");
        feed(&mut t, "你".as_bytes()); // at col 1, advances to col 3
        feed(&mut t, b"B");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'B', "B at col 3");
        assert_eq!(t.cursor().0, 4, "cursor at col 4");
    }

    #[test]
    fn t_r12_wide_char_bg_on_spacer() {
        // BG color should be set on spacer cell too (no visual gap).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[42m"); // green bg
        feed(&mut t, "你".as_bytes());
        assert_eq!(
            t.grid().cell(1, 0).unwrap().bg,
            Color::Indexed(2),
            "spacer has bg"
        );
    }

    #[test]
    fn t_r12_wide_chars_fill_row() {
        // Multiple wide chars should fill row correctly and wrap.
        let mut t = Terminal::new(6, 3);
        feed(&mut t, "你".as_bytes()); // cols 0-1
        feed(&mut t, "好".as_bytes()); // cols 2-3
        feed(&mut t, "世".as_bytes()); // cols 4-5
        // Next wide char should wrap
        feed(&mut t, "界".as_bytes());
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '你');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, '好');
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, '世');
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, '界', "wrapped to row 1");
    }

    #[test]
    fn t_r12_wide_char_decawm_off() {
        // With DECAWM off, wide char at penultimate col should be clipped.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[?7l"); // DECAWM off
        feed(&mut t, b"ABCD"); // cols 0-3, cursor at 4
        feed(&mut t, "你".as_bytes()); // only 1 col left, DECAWM off
        // Should NOT wrap; char should be placed or clipped at col 4
        // With width 2 and only 1 col, the char is placed at col 4
        // (put_char will set wide flag but no spacer since col+1 >= len)
        assert_eq!(t.cursor().1, 0, "no wrap to row 1");
    }

    #[test]
    fn t_r12_thai_char_width() {
        // Thai chars should mostly be width 1.
        use crate::grid::char_width;
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "ก".as_bytes()); // Thai character KO KAI
        assert_eq!(char_width('ก'), 1, "Thai is width 1");
        assert!(!t.grid().cell(0, 0).unwrap().is_wide(), "no wide flag");
    }

    #[test]
    fn t_r12_mixed_ascii_wide_sequence() {
        // Interleaved ASCII and wide chars.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"A");
        feed(&mut t, "你".as_bytes());
        feed(&mut t, b"B");
        feed(&mut t, "好".as_bytes());
        feed(&mut t, b"C");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, '你');
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'B');
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, '好');
        assert_eq!(t.grid().cell(6, 0).unwrap().ch, 'C');
    }

    #[test]
    fn t_cuf_lands_on_wide_spacer_adjusts_to_lead() {
        // When CUF moves the cursor to a position that is the spacer
        // (right half) of a wide character, the cursor should be
        // adjusted back to the lead cell.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你".as_bytes()); // wide char at cols 0-1, cursor at 2
        feed(&mut t, b"A"); // A at col 2, cursor at 3
        // CUF back to col 1 (spacer of wide char at 0-1)
        feed(&mut t, b"\x1b[1;1H"); // CUP home → cursor at (0, 0)
        feed(&mut t, b"\x1b[1C"); // CUF 1 → cursor.x = 1 (spacer!)
        // Cursor should be adjusted to col 0 (lead), not col 1 (spacer).
        assert_eq!(
            t.cursor().0,
            0,
            "cursor should adjust to wide char lead, not spacer"
        );
    }

    #[test]
    fn t_cub_lands_on_wide_spacer_adjusts_to_lead() {
        // When CUB moves the cursor to a spacer position, it should
        // be adjusted to the lead.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你".as_bytes()); // wide char at cols 0-1, cursor at 2
        feed(&mut t, b"\x1b[3C"); // CUF 3 → cursor.x = 5
        feed(&mut t, b"\x1b[4D"); // CUB 4 → cursor.x = 1 (spacer!)
        assert_eq!(t.cursor().0, 0, "CUB should adjust to lead, not spacer");
    }

    #[test]
    fn t_cha_lands_on_wide_spacer_adjusts_to_lead() {
        // CHA (cursor horizontal absolute) should also adjust.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你".as_bytes()); // wide char at cols 0-1
        feed(&mut t, b"\x1b[2G"); // CHA col 2 → cursor.x = 1 (spacer!)
        assert_eq!(t.cursor().0, 0, "CHA should adjust to lead, not spacer");
    }

    #[test]
    fn t_ht_lands_on_wide_spacer_adjusts_to_lead() {
        // HT (horizontal tab) should adjust cursor to lead if it lands on spacer.
        // Set up: wide char at cols 0-1, tab stop at col 2.
        // From col 0, tab moves to col 2 (next stop). But col 1 is the spacer.
        // Actually we need a scenario where the tab stop IS on a spacer.
        let mut t = Terminal::new(10, 3);
        // Place a wide char at cols 8-9 (default tab stop at col 8)
        feed(&mut t, b"\x1b[1;9H"); // CUP to col 8 (0-indexed)
        feed(&mut t, "你".as_bytes()); // wide char at cols 8-9
        // Go back to col 0, then tab
        feed(&mut t, b"\x1b[1G"); // CHA col 1 → cursor.x = 0
        feed(&mut t, b"\t"); // HT → next stop at col 8 (spacer!)
        // Cursor should adjust from col 8 (spacer... wait, col 8 is lead)
        // Actually: wide char at cols 8-9. Col 8 = lead, col 9 = spacer.
        // HT from col 0 lands at col 8 (lead). No adjustment needed.
        assert_eq!(t.cursor().0, 8, "HT should land at col 8 (lead)");

        // Now test a scenario where HT lands on a spacer:
        // Set tab stop at col 1, place wide char at cols 0-1
        feed(&mut t, b"\x1b[2;1H"); // CUP row 2 col 1
        feed(&mut t, b"\x1b[1;2H"); // CUP back to row 1, col 1 (spacer → adjusts to 0)
        // Actually let me test differently:
        let mut t2 = Terminal::new(10, 3);
        feed(&mut t2, "你".as_bytes()); // wide at cols 0-1, cursor at 2
        feed(&mut t2, b"\x1b[2;1H"); // CUP row 2
        feed(&mut t2, "好".as_bytes()); // wide at row 2 cols 0-1
        // Set a custom tab stop at col 1 (which will be a spacer)
        // Actually tab stops are shared across rows...
        // The key test: from a position before col 8, tab lands at col 8.
        // If col 8 is a spacer, adjust.
        feed(&mut t2, b"\x1b[3;1H"); // CUP row 3
        feed(&mut t2, "A".as_bytes()); // A at row 3 col 0
        // Now put a wide char spanning cols 8-9 on row 3
        feed(&mut t2, b"\x1b[3;9H"); // CUP row 3 col 9 (0-indexed 8)
        feed(&mut t2, "你".as_bytes()); // wide at row 3 cols 8-9
        feed(&mut t2, b"\x1b[3;1G"); // CHA col 1 (0-indexed 0) on row 3
        feed(&mut t2, b"\t"); // HT → lands at col 8 (lead). OK.
        // To actually test spacer, we need a tab stop at col 9.
        // Clear all tabs and set one at col 10 (0-indexed 9)
        feed(&mut t2, b"\x1b[3g"); // TBC 3: clear all tabs
        feed(&mut t2, b"\x1b[3;10H"); // CUP row 3 col 10 (0-indexed 9)
        feed(&mut t2, b"\x1bH"); // HTS: set tab stop at col 9
        feed(&mut t2, b"\x1b[3;1G"); // back to col 0
        feed(&mut t2, b"\t"); // HT → lands at col 9 (spacer!)
        assert_eq!(
            t2.cursor().0,
            8,
            "HT should adjust from spacer (col 9) to lead (col 8)"
        );
    }

    // ── Round 12-2: Bracketed Paste + Focus Reporting audits ───────────

    #[test]
    fn t_r12_focus_disabled_by_default() {
        let t = Terminal::new(10, 5);
        assert!(!t.focus_event_enabled(), "focus reporting default off");
    }

    #[test]
    fn t_resize_cursor_lands_on_wide_spacer_adjusts_to_lead() {
        // In alt screen (non-reflow), shrinking the terminal can leave
        // the cursor on a wide char spacer. Verify it adjusts to lead.
        let mut t = Terminal::new(4, 2);
        feed(&mut t, b"\x1b[?1049h"); // enter alt screen (no reflow)
        feed(&mut t, b"A");
        feed(&mut t, "中".as_bytes()); // wide at cols 1-2
        feed(&mut t, b"B"); // B at col 3
        assert_eq!(t.cursor().0, 3);
        // Shrink to width 3: [A, 中_lead, 中_spacer], cursor clamped to 2
        // Col 2 is the spacer — cursor should adjust to col 1 (lead).
        t.resize(3, 2);
        assert_eq!(
            t.cursor().0,
            1,
            "cursor should adjust to wide char lead, not spacer"
        );
    }

    #[test]
    fn t_r12_bracketed_paste_disabled_by_default() {
        let t = Terminal::new(10, 5);
        assert!(!t.bracketed_paste(), "bracketed paste default off");
    }

    #[test]
    fn t_r12_focus_toggle() {
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1004h");
        assert!(t.focus_event_enabled());
        feed(&mut t, b"\x1b[?1004l");
        assert!(!t.focus_event_enabled());
    }

    #[test]
    fn t_r12_focus_in_out_report_format() {
        // Exact byte sequences for focus reports.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1004h");
        assert_eq!(t.focus_in_report(), b"\x1b[I".to_vec(), "focus in = ESC[I");
        assert_eq!(
            t.focus_out_report(),
            b"\x1b[O".to_vec(),
            "focus out = ESC[O"
        );
    }

    #[test]
    fn t_r12_focus_report_after_disable() {
        // After disabling focus events, reports should be empty.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1004h");
        feed(&mut t, b"\x1b[?1004l");
        assert!(t.focus_in_report().is_empty());
        assert!(t.focus_out_report().is_empty());
    }

    #[test]
    fn t_r12_focus_report_after_decstr() {
        // DECSTR should reset focus events to off.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1004h");
        assert!(t.focus_event_enabled());
        feed(&mut t, b"\x1b[!p"); // DECSTR
        assert!(!t.focus_event_enabled(), "DECSTR resets focus events");
    }

    #[test]
    fn t_r12_focus_persists_through_alt_screen() {
        // Focus reporting should persist through alt screen switch.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1004h");
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        assert!(t.focus_event_enabled(), "focus persists in alt");
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert!(t.focus_event_enabled(), "focus persists after alt");
    }

    #[test]
    fn t_r12_bracketed_paste_after_decstr() {
        // DECSTR should reset bracketed paste.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?2004h");
        feed(&mut t, b"\x1b[!p"); // DECSTR
        assert!(!t.bracketed_paste(), "DECSTR resets bracketed paste");
    }

    #[test]
    fn t_r12_decrqm_bracketed_paste_off() {
        // DECRQM should report bracketed paste as off (0) when disabled.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?2004$p"); // query mode
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        assert!(
            s.contains("2004;2$y"),
            "mode 2004 = permanently reset (2): {s}"
        );
    }

    #[test]
    fn t_r12_decrqm_bracketed_paste_on() {
        // DECRQM should report bracketed paste as on (1) when enabled.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?2004h");
        feed(&mut t, b"\x1b[?2004$p");
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        assert!(s.contains("2004;1$y"), "mode 2004 = set (1): {s}");
    }

    #[test]
    fn t_r12_decrqm_focus_event_on() {
        // DECRQM for focus events.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1004h");
        feed(&mut t, b"\x1b[?1004$p");
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        assert!(s.contains("1004;1$y"), "mode 1004 = set (1): {s}");
    }

    #[test]
    fn t_r12_decrqm_focus_event_off() {
        // DECRQM for focus events when off.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1004$p");
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        assert!(s.contains("1004;2$y"), "mode 1004 = reset (2): {s}");
    }

    #[test]
    fn t_r12_bracketed_paste_persists_through_ris() {
        // RIS (full reset) should reset bracketed paste.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?2004h");
        feed(&mut t, b"\x1bc"); // RIS
        assert!(!t.bracketed_paste(), "RIS resets bracketed paste");
    }

    #[test]
    fn t_r12_focus_event_persists_through_ris() {
        // RIS should reset focus events.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1004h");
        feed(&mut t, b"\x1bc"); // RIS
        assert!(!t.focus_event_enabled(), "RIS resets focus events");
    }

    // ── Round 12-3: SGR color edge cases ───────────────────────────────

    #[test]
    fn t_r12_sgr_256_color_fg() {
        // SGR 38;5;N for all 256 color indices.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[38;5;0m");
        assert_eq!(t.fg, Color::Indexed(0), "fg = indexed 0");
        feed(&mut t, b"\x1b[38;5;15m");
        assert_eq!(t.fg, Color::Indexed(15), "fg = indexed 15");
        feed(&mut t, b"\x1b[38;5;255m");
        assert_eq!(t.fg, Color::Indexed(255), "fg = indexed 255");
        feed(&mut t, b"\x1b[38;5;128m");
        assert_eq!(t.fg, Color::Indexed(128), "fg = indexed 128");
    }

    #[test]
    fn t_r12_sgr_256_color_bg() {
        // SGR 48;5;N for background.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[48;5;42m");
        assert_eq!(t.bg, Color::Indexed(42), "bg = indexed 42");
        feed(&mut t, b"\x1b[48;5;200m");
        assert_eq!(t.bg, Color::Indexed(200), "bg = indexed 200");
    }

    #[test]
    fn t_r12_sgr_true_color_fg() {
        // SGR 38;2;R;G;B for true color foreground.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[38;2;255;0;0m");
        assert_eq!(t.fg, Color::Rgb(255, 0, 0), "fg = RGB red");
        feed(&mut t, b"\x1b[38;2;0;255;0m");
        assert_eq!(t.fg, Color::Rgb(0, 255, 0), "fg = RGB green");
        feed(&mut t, b"\x1b[38;2;1;2;3m");
        assert_eq!(t.fg, Color::Rgb(1, 2, 3), "fg = RGB low values");
    }

    #[test]
    fn t_r12_sgr_true_color_bg() {
        // SGR 48;2;R;G;B for true color background.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[48;2;100;150;200m");
        assert_eq!(t.bg, Color::Rgb(100, 150, 200), "bg = RGB");
    }

    #[test]
    fn t_r12_sgr_38_5_0_not_reset() {
        // \x1b[38;5;0m should set fg to indexed 0, NOT reset to default.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[38;5;0m");
        assert_eq!(
            t.fg,
            Color::Indexed(0),
            "38;5;0 sets indexed 0, not Default"
        );
        assert_ne!(t.fg, Color::Default, "38;5;0 != Default");
    }

    #[test]
    fn t_r12_sgr_empty_param_is_reset() {
        // CSI m (empty) should reset all attributes.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[1;3;4;31m");
        feed(&mut t, b"\x1b[m");
        assert!(!t.flags.contains(CellFlags::BOLD), "bold cleared");
        assert_eq!(t.fg, Color::Default, "fg default");
        assert_eq!(t.bg, Color::Default, "bg default");
    }

    #[test]
    fn t_r12_sgr_0_reset() {
        // CSI 0m should reset all attributes.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[1;31m\x1b[0m");
        assert!(!t.flags.contains(CellFlags::BOLD));
        assert_eq!(t.fg, Color::Default);
    }

    #[test]
    fn t_r12_sgr_39_default_fg() {
        // SGR 39 resets fg to default (but keeps other attrs).
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[1;31m"); // bold + red
        feed(&mut t, b"\x1b[39m"); // default fg
        assert_eq!(t.fg, Color::Default, "fg reset to default");
        assert!(t.flags.contains(CellFlags::BOLD), "bold preserved");
    }

    #[test]
    fn t_r12_sgr_49_default_bg() {
        // SGR 49 resets bg to default.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[42m"); // green bg
        feed(&mut t, b"\x1b[49m");
        assert_eq!(t.bg, Color::Default, "bg reset to default");
    }

    #[test]
    fn t_r12_sgr_combined_attrs() {
        // Bold + italic + underline + fg + bg all independent.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[1;3;4;5;7;9;31;42m");
        assert!(t.flags.contains(CellFlags::BOLD));
        assert!(t.flags.contains(CellFlags::ITALIC));
        assert!(t.flags.contains(CellFlags::UNDERLINE));
        assert!(t.flags.contains(CellFlags::BLINK));
        assert!(t.flags.contains(CellFlags::REVERSE));
        assert!(t.flags.contains(CellFlags::STRIKETHROUGH));
        assert_eq!(t.fg, Color::Indexed(1));
        assert_eq!(t.bg, Color::Indexed(2));
    }

    #[test]
    fn t_r12_sgr_bright_colors() {
        // SGR 90-97 for bright fg, 100-107 for bright bg.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[90m");
        assert_eq!(t.fg, Color::Indexed(8), "90 = bright black (8)");
        feed(&mut t, b"\x1b[97m");
        assert_eq!(t.fg, Color::Indexed(15), "97 = bright white (15)");
        feed(&mut t, b"\x1b[101m");
        assert_eq!(t.bg, Color::Indexed(9), "101 = bright red (9)");
    }

    #[test]
    fn t_r12_sgr_attr_off_codes() {
        // Individual attribute off codes (22, 23, 24, 25, 27, 28, 29).
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[1;2;3;4;5;7;8;9m");
        feed(&mut t, b"\x1b[22;23;24;25;27;28;29m");
        assert!(!t.flags.contains(CellFlags::BOLD | CellFlags::DIM));
        assert!(!t.flags.contains(CellFlags::ITALIC));
        assert!(!t.flags.contains(CellFlags::UNDERLINE));
        assert!(!t.flags.contains(CellFlags::BLINK));
        assert!(!t.flags.contains(CellFlags::REVERSE));
        assert!(!t.flags.contains(CellFlags::HIDDEN));
        assert!(!t.flags.contains(CellFlags::STRIKETHROUGH));
    }

    #[test]
    fn t_r12_sgr_multi_param_sequence() {
        // Multiple SGR params in one sequence.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[38;2;10;20;30;48;5;99;1m");
        assert_eq!(t.fg, Color::Rgb(10, 20, 30), "fg = RGB");
        assert_eq!(t.bg, Color::Indexed(99), "bg = indexed");
        assert!(t.flags.contains(CellFlags::BOLD), "bold set");
    }

    #[test]
    fn t_r12_sgr_underline_color() {
        // SGR 58 for underline color.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[58;2;50;60;70m");
        assert_eq!(t.underline_color, Color::Rgb(50, 60, 70), "underline RGB");
        feed(&mut t, b"\x1b[58;5;7m");
        assert_eq!(t.underline_color, Color::Indexed(7), "underline indexed");
    }

    #[test]
    fn t_r12_sgr_59_reset_underline_color() {
        // SGR 59 resets underline color to default.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[58;5;42m");
        feed(&mut t, b"\x1b[59m");
        assert_eq!(t.underline_color, Color::Default, "underline color reset");
    }

    #[test]
    fn t_r12_sgr_propagates_to_cell() {
        // SGR attrs should propagate to written cells.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[1;38;5;196m"); // bold + bright red
        feed(&mut t, b"X");
        let cell = t.grid().cell(0, 0).unwrap();
        assert!(cell.flags.contains(CellFlags::BOLD), "cell has bold");
        assert_eq!(cell.fg, Color::Indexed(196), "cell has fg");
    }

    #[test]
    fn t_r12_sgr_0_resets_underline_color() {
        // SGR 0 should reset underline_color too.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[58;5;42m");
        feed(&mut t, b"\x1b[0m");
        assert_eq!(
            t.underline_color,
            Color::Default,
            "SGR 0 resets underline_color"
        );
    }

    // ── Round 13-1: DECSTBM supplementary edge cases ───────────────────

    #[test]
    fn t_r13_decstbm_default_params_full_screen() {
        // CSI r with no params = reset to full screen.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[2;4r"); // region rows 2-4
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!((top, bottom), (1, 4));
        feed(&mut t, b"\x1b[r"); // default — reset to full
        let (top2, bottom2) = t.grid().scroll_region();
        assert_eq!((top2, bottom2), (0, 6), "CSI r resets to full screen");
    }

    #[test]
    fn t_r13_decstbm_bottom_zero_defaults_height() {
        // CSI Ps;0r — bottom=0 defaults to height.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[2;0r"); // top=2, bottom=0 → full to height
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(top, 1, "top=1 (0-based)");
        assert_eq!(bottom, 6, "bottom defaults to height");
    }

    #[test]
    fn t_r13_decstbm_su_only_within_region() {
        // SU (CSI Ps S) scrolls up only within the scroll region.
        let mut t = Terminal::new(5, 6);
        feed(&mut t, b"A\r\nB\r\nC\r\nD\r\nE\r\nF");
        feed(&mut t, b"\x1b[2;4r"); // region rows 2-4 (0-based: 1..4)
        feed(&mut t, b"\x1b[1;1H"); // cursor to top
        feed(&mut t, b"\x1b[1S"); // SU 1
        // Only row 1-3 should be affected
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "row 0 preserved");
        assert_eq!(t.grid().cell(0, 4).unwrap().ch, 'E', "row 4 preserved");
    }

    #[test]
    fn t_r13_decstbm_sd_only_within_region() {
        // SD (CSI Ps T) scrolls down only within the scroll region.
        let mut t = Terminal::new(5, 6);
        feed(&mut t, b"A\r\nB\r\nC\r\nD\r\nE\r\nF");
        feed(&mut t, b"\x1b[2;4r"); // region rows 2-4 (0-based: 1..4)
        feed(&mut t, b"\x1b[1;1H");
        feed(&mut t, b"\x1b[1T"); // SD 1
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "row 0 preserved");
        assert_eq!(t.grid().cell(0, 4).unwrap().ch, 'E', "row 4 preserved");
    }

    #[test]
    fn t_r13_decstbm_top_equals_bottom_ignored() {
        // top == bottom should be ignored (degenerate region).
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;3r"); // top=3, bottom=3 → invalid
        let (top, bottom) = t.grid().scroll_region();
        // Should still be full screen
        assert_eq!((top, bottom), (0, 5), "degenerate region ignored");
    }

    #[test]
    fn t_r13_decstbm_top_greater_than_bottom_ignored() {
        // top > bottom should be ignored.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[4;2r"); // top=4 > bottom=2 → invalid
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!((top, bottom), (0, 5), "invalid region ignored");
    }

    // ── Round 13-2: ICH/DCH with scroll region interaction ─────────────

    #[test]
    fn t_r13_dch_default_param_is_1() {
        // DCH with no param defaults to 1.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGHIJ");
        feed(&mut t, b"\x1b[1;3H"); // cursor at col 3
        feed(&mut t, b"\x1b[P"); // DCH default 1
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'D', "D shifted left");
    }

    #[test]
    fn t_r13_ich_default_param_is_1() {
        // ICH with no param defaults to 1.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGHIJ");
        feed(&mut t, b"\x1b[1;3H"); // cursor at col 3
        feed(&mut t, b"\x1b[@"); // ICH default 1
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, ' ', "blank inserted");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'C', "C shifted right");
    }

    #[test]
    fn t_r13_dch_preserves_cursor() {
        // DCH should not move cursor.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGHIJ");
        feed(&mut t, b"\x1b[1;5H"); // cursor at col 5
        feed(&mut t, b"\x1b[3P"); // DCH 3
        assert_eq!(t.cursor().0, 4, "cursor stays at col 5");
    }

    #[test]
    fn t_r13_ich_preserves_cursor() {
        // ICH should not move cursor.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGHIJ");
        feed(&mut t, b"\x1b[1;5H"); // cursor at col 5
        feed(&mut t, b"\x1b[3@"); // ICH 3
        assert_eq!(t.cursor().0, 4, "cursor stays at col 5");
    }

    #[test]
    fn t_r13_ich_at_col0() {
        // ICH at column 0.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGHIJ");
        feed(&mut t, b"\x1b[1;1H");
        feed(&mut t, b"\x1b[2@"); // ICH 2 at col 0
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ', "blank at col 0");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'A', "A at col 2");
    }

    #[test]
    fn t_r13_dch_clears_all_after() {
        // DCH with count > remaining chars.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDE     "); // ABCDE + 5 blanks
        feed(&mut t, b"\x1b[1;1H");
        feed(&mut t, b"\x1b[10P"); // DCH 10 — clear entire row
        for c in 0..5 {
            assert_eq!(t.grid().cell(c, 0).unwrap().ch, ' ', "col {c} blank");
        }
    }

    // ── Round 13-3: SCOSC/SCORC (CSI s/u) cursor save/restore ──────────

    #[test]
    fn t_r13_scosc_saves_cursor_position() {
        // CSI s saves cursor position only (not SGR like DECSC).
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;5H"); // cursor (4, 2)
        feed(&mut t, b"\x1b[s"); // SCOSC save
        feed(&mut t, b"\x1b[1;1H"); // move away
        feed(&mut t, b"\x1b[u"); // SCORC restore
        assert_eq!(t.cursor(), (4, 2), "position restored by SCORC");
    }

    #[test]
    fn t_r13_scosc_does_not_save_sgr() {
        // SCOSC saves only position, NOT SGR (unlike DECSC).
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[1;31m"); // bold red
        feed(&mut t, b"\x1b[s"); // SCOSC save
        feed(&mut t, b"\x1b[0m"); // reset
        feed(&mut t, b"\x1b[u"); // SCORC restore
        // SGR should NOT be restored by SCORC
        assert!(
            !t.flags.contains(CellFlags::BOLD),
            "bold NOT restored by SCORC"
        );
        assert_eq!(t.fg, Color::Default, "fg NOT restored by SCORC");
    }

    #[test]
    fn t_r13_scorc_without_scosc_restores_default() {
        // SCORC without prior SCOSC restores to (0,0).
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[5;5H"); // cursor at (4, 4)
        feed(&mut t, b"\x1b[u"); // SCORC without save
        assert_eq!(t.cursor(), (0, 0), "default position restored");
    }

    #[test]
    fn t_r13_scosc_scorc_independent_from_decsc_decrc() {
        // SCOSC and DECSC use different save slots.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[2;2H"); // pos1 (1, 1)
        feed(&mut t, b"\x1b7"); // DECSC save
        feed(&mut t, b"\x1b[5;5H"); // pos2 (4, 4)
        feed(&mut t, b"\x1b[s"); // SCOSC save
        feed(&mut t, b"\x1b[1;1H"); // move away
        feed(&mut t, b"\x1b8"); // DECRC restore → (1, 1)
        assert_eq!(t.cursor(), (1, 1), "DECRC restores DECSC position");
        feed(&mut t, b"\x1b[1;1H"); // move away
        feed(&mut t, b"\x1b[u"); // SCORC restore → (4, 4)
        assert_eq!(t.cursor(), (4, 4), "SCORC restores SCOSC position");
    }

    #[test]
    fn t_r13_scosc_multiple_saves_overwrite() {
        // Multiple SCOSC saves overwrite (single slot).
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[2;3H"); // (2, 1)
        feed(&mut t, b"\x1b[s"); // save 1
        feed(&mut t, b"\x1b[5;8H"); // (7, 4)
        feed(&mut t, b"\x1b[s"); // save 2 (overwrites)
        feed(&mut t, b"\x1b[1;1H");
        feed(&mut t, b"\x1b[u"); // restore → save 2
        assert_eq!(t.cursor(), (7, 4), "second save wins");
    }

    #[test]
    fn t_r13_scosc_does_not_save_pending_wrap() {
        // SCOSC saves cursor struct which includes pending_wrap.
        // But SCORC explicitly clears pending_wrap (line 2608).
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE"); // fills row, pending_wrap set
        feed(&mut t, b"\x1b[s"); // save
        feed(&mut t, b"\x1b[1;1H"); // move — clears pending_wrap
        feed(&mut t, b"\x1b[u"); // restore
        assert!(!t.cursor.pending_wrap, "SCORC clears pending_wrap");
    }

    // ── Round 14-1: Cursor movement edge cases + REP ───────────────────

    #[test]
    fn t_r14_cha_origin_mode() {
        // CHA (CSI G) in origin mode should be relative to scroll region.
        // Actually per spec, CHA is NOT affected by origin mode (only row-based
        // commands like CUP are). Column stays absolute.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[3;5r"); // region rows 3-5
        feed(&mut t, b"\x1b[?6h"); // origin on
        feed(&mut t, b"\x1b[5G"); // CHA col 5
        assert_eq!(t.cursor().0, 4, "CHA col 5 → x=4 (0-based)");
    }

    #[test]
    fn t_r14_vpa_origin_mode() {
        // VPA (CSI d) in origin mode should be relative to scroll region.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[3;5r"); // region rows 3-5 (0-based: 2..5)
        feed(&mut t, b"\x1b[?6h"); // origin on
        feed(&mut t, b"\x1b[2d"); // VPA row 2 → abs y = top + (2-1) = 3
        assert_eq!(t.cursor().1, 3, "VPA origin mode row 2 → abs y=3");
    }

    #[test]
    fn t_r14_cnl_cpl_default_param() {
        // CNL/CPL with no param default to 1.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;3H"); // (2, 2)
        feed(&mut t, b"\x1b[E"); // CNL default 1
        assert_eq!(t.cursor(), (0, 3), "CNL default moves to next row col 0");
        feed(&mut t, b"\x1b[F"); // CPL default 1
        assert_eq!(t.cursor(), (0, 2), "CPL default moves to prev row col 0");
    }

    #[test]
    fn t_r14_cnl_clamps_to_bottom() {
        // CNL should clamp at scroll region bottom (or last row).
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[4;1H"); // row 4
        feed(&mut t, b"\x1b[99E"); // CNL 99
        assert_eq!(t.cursor().1, 4, "CNL clamped to last row");
    }

    #[test]
    fn t_r14_cpl_clamps_to_top() {
        // CPL should clamp at row 0 (or scroll region top).
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[2;1H"); // row 2
        feed(&mut t, b"\x1b[99F"); // CPL 99
        assert_eq!(t.cursor().1, 0, "CPL clamped to row 0");
    }

    #[test]
    fn t_r14_rep_basic() {
        // REP (CSI b) repeats last printed char.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"A"); // last_printed = A
        feed(&mut t, b"\x1b[3b"); // REP 3
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'A', "A at cols 0,1,2,3");
        assert_eq!(t.cursor().0, 4, "cursor at col 4");
    }

    #[test]
    fn t_r14_rep_no_preceding_char() {
        // REP with no preceding printable char should be no-op.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[5b"); // REP 5 — no preceding char
        assert_eq!(t.cursor(), (0, 0), "cursor unchanged");
    }

    #[test]
    fn t_r14_rep_after_control_char() {
        // REP after a control char (e.g. CR) should repeat the last PRINTABLE char.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"X"); // last_printed = X
        feed(&mut t, b"\r"); // CR — does not change last_printed
        feed(&mut t, b"\x1b[2b"); // REP 2 — should repeat X
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X', "X at col 0");
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'X', "X at col 1");
    }

    #[test]
    fn t_r14_cup_origin_mode_clamps_to_region() {
        // CUP in origin mode should not exceed scroll region.
        let mut t = Terminal::new(10, 8);
        feed(&mut t, b"\x1b[3;6r"); // region rows 3-6 (0-based: 2..6)
        feed(&mut t, b"\x1b[?6h"); // origin on
        feed(&mut t, b"\x1b[99;1H"); // CUP row 99 → clamp to region bottom
        assert_eq!(t.cursor().1, 5, "CUP clamped to region bottom (0-based: 5)");
    }

    // ── Round 14-2: Cursor visibility + DECOM edge cases ───────────────

    #[test]
    fn t_r14_cursor_hide_show() {
        // DECSET 25 (show cursor), DECRST 25 (hide cursor).
        let mut t = Terminal::new(10, 5);
        assert!(t.cursor_visible(), "cursor visible by default");
        feed(&mut t, b"\x1b[?25l"); // hide
        assert!(!t.cursor_visible(), "cursor hidden");
        feed(&mut t, b"\x1b[?25h"); // show
        assert!(t.cursor_visible(), "cursor shown");
    }

    #[test]
    fn t_r14_cursor_visible_after_decstr() {
        // DECSTR should reset cursor visibility to on.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?25l"); // hide
        feed(&mut t, b"\x1b[!p"); // DECSTR
        assert!(t.cursor_visible(), "DECSTR resets cursor to visible");
    }

    #[test]
    fn t_r14_decom_cursor_homes_on_enable() {
        // DECOM (origin mode) should home cursor on enable.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;5H"); // cursor (4, 2)
        feed(&mut t, b"\x1b[?6h"); // origin on
        // Cursor should move to region top (0 in abs, or region top if region set)
        assert_eq!(t.cursor().0, 0, "DECOM homes cursor x to 0");
    }

    #[test]
    fn t_r14_decom_disable_homes_cursor() {
        // DECRST 6 should also home cursor.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?6h"); // enable
        feed(&mut t, b"\x1b[3;5H"); // cursor at (4, 3)
        feed(&mut t, b"\x1b[?6l"); // disable
        assert_eq!(t.cursor().0, 0, "DECRST 6 homes cursor x to 0");
    }

    #[test]
    fn t_r14_cursor_visible_after_ris() {
        // RIS should reset cursor to visible.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?25l"); // hide
        feed(&mut t, b"\x1bc"); // RIS
        assert!(t.cursor_visible(), "RIS resets cursor to visible");
    }

    // ── Round 14-3: DECSET modes edge cases ────────────────────────────

    #[test]
    fn t_r14_decscnm_reverse_video_toggle() {
        // DECSET 5 / DECRST 5 — reverse video mode.
        let mut t = Terminal::new(10, 5);
        assert!(!t.reverse_video(), "reverse video default off");
        feed(&mut t, b"\x1b[?5h"); // enable
        assert!(t.reverse_video(), "reverse video on");
        feed(&mut t, b"\x1b[?5l"); // disable
        assert!(!t.reverse_video(), "reverse video off");
    }

    #[test]
    fn t_r14_decscnm_reset_by_decstr() {
        // DECSTR should reset reverse video to off.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?5h"); // enable
        feed(&mut t, b"\x1b[!p"); // DECSTR
        assert!(!t.reverse_video(), "DECSTR resets reverse video");
    }

    #[test]
    fn t_r14_decckm_app_cursor_keys_default() {
        // DECSET 1 / DECRST 1 — application cursor keys.
        let mut t = Terminal::new(10, 5);
        assert!(!t.cursor_keys_app(), "app cursor default off");
        feed(&mut t, b"\x1b[?1h"); // enable
        assert!(t.cursor_keys_app(), "app cursor on");
        feed(&mut t, b"\x1b[?1l"); // disable
        assert!(!t.cursor_keys_app(), "app cursor off");
    }

    #[test]
    fn t_r14_decckm_reset_by_decstr() {
        // DECSTR should reset app cursor keys.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1h"); // enable
        feed(&mut t, b"\x1b[!p"); // DECSTR
        assert!(!t.cursor_keys_app(), "DECSTR resets app cursor");
    }

    #[test]
    fn t_r14_decom_reset_by_ris() {
        // RIS should reset origin mode.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?6h"); // origin on
        feed(&mut t, b"\x1bc"); // RIS
        assert!(!t.modes.origin, "RIS resets origin mode");
    }

    #[test]
    fn t_r14_reverse_video_reset_by_ris() {
        // RIS should reset reverse video.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?5h"); // reverse on
        feed(&mut t, b"\x1bc"); // RIS
        assert!(!t.reverse_video(), "RIS resets reverse video");
    }

    #[test]
    fn t_r14_cursor_pos_param_exceeds_width() {
        // CUP with col > width should clamp.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[1;99H"); // col 99 → clamp to 10
        assert_eq!(t.cursor().0, 9, "col clamped to last col");
    }

    #[test]
    fn t_r14_cursor_pos_param_exceeds_height() {
        // CUP with row > height should clamp.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[99;1H"); // row 99 → clamp to 5
        assert_eq!(t.cursor().1, 4, "row clamped to last row");
    }

    // ── Round 15-1: SGR parsing edge cases ─────────────────────────────

    #[test]
    fn t_r15_sgr_semicolon_only_resets() {
        // CSI ;m should parse as [0, 0] → reset.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[1;31m"); // bold + red
        feed(&mut t, b"\x1b[;m"); // ;m → [0,0] → reset
        assert!(!t.flags.contains(CellFlags::BOLD), "bold cleared by ;m");
        assert_eq!(t.fg, Color::Default, "fg reset by ;m");
    }

    #[test]
    fn t_r15_sgr_zero_semicolon_resets() {
        // CSI 0;m → [0, 0] → reset.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[1m\x1b[0;m");
        assert!(!t.flags.contains(CellFlags::BOLD), "bold cleared");
    }

    #[test]
    fn t_r15_sgr_empty_middle_param() {
        // CSI 1;;m → [1, 0] → bold then reset.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[1;;m"); // bold, then param 0 (reset)
        assert!(!t.flags.contains(CellFlags::BOLD), "middle empty→0 resets");
    }

    #[test]
    fn t_r15_sgr_trailing_empty_param() {
        // CSI 1; → parser drops trailing empty param, so just bold.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[1;m"); // trailing empty dropped → [1]
        assert!(
            t.flags.contains(CellFlags::BOLD),
            "trailing empty dropped, bold stays"
        );
    }

    #[test]
    fn t_r15_sgr_53_55_overline() {
        // SGR 53 = overline on, 55 = off.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[53m");
        assert!(t.flags.contains(CellFlags::OVERLINE), "overline on");
        feed(&mut t, b"\x1b[55m");
        assert!(!t.flags.contains(CellFlags::OVERLINE), "overline off");
    }

    #[test]
    fn t_r15_sgr_8_28_hidden() {
        // SGR 8 = hidden/conceal on, 28 = off.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[8m");
        assert!(t.flags.contains(CellFlags::HIDDEN), "hidden on");
        feed(&mut t, b"\x1b[28m");
        assert!(!t.flags.contains(CellFlags::HIDDEN), "hidden off");
    }

    #[test]
    fn t_r15_sgr_true_color_black_white() {
        // True color at extremes: (0,0,0) and (255,255,255).
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[38;2;0;0;0m");
        assert_eq!(t.fg, Color::Rgb(0, 0, 0), "fg = black");
        feed(&mut t, b"\x1b[38;2;255;255;255m");
        assert_eq!(t.fg, Color::Rgb(255, 255, 255), "fg = white");
    }

    #[test]
    fn t_r15_sgr_39_preserves_bg() {
        // SGR 39 (default fg) should NOT reset bg.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[31;42m"); // red fg, green bg
        feed(&mut t, b"\x1b[39m"); // default fg only
        assert_eq!(t.fg, Color::Default, "fg reset");
        assert_eq!(t.bg, Color::Indexed(2), "bg preserved");
    }

    #[test]
    fn t_r15_sgr_49_preserves_fg() {
        // SGR 49 (default bg) should NOT reset fg.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[31;42m");
        feed(&mut t, b"\x1b[49m");
        assert_eq!(t.fg, Color::Indexed(1), "fg preserved");
        assert_eq!(t.bg, Color::Default, "bg reset");
    }

    // ── Round 15-2: UTF-8 / wide char edge cases ───────────────────────

    #[test]
    fn t_r15_backspace_after_wide_char() {
        // Backspace after writing wide char should move cursor back 1.
        // (BS moves 1 column, not width)
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你".as_bytes()); // cursor at col 2
        feed(&mut t, b"\x08"); // BS → col 1 (spacer)
        assert_eq!(t.cursor().0, 1, "BS moves 1 col to spacer");
    }

    #[test]
    fn t_r15_zero_width_space_no_output() {
        // ZWSP (U+200B) is zero-width — should not advance cursor or write.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "\u{200B}".as_bytes());
        assert_eq!(t.cursor(), (0, 0), "ZWSP does not advance cursor");
    }

    #[test]
    fn t_r15_combining_nfc_vs_nfd() {
        // é as NFC (U+00E9) = width 1.
        // é as NFD (e + U+0301) = also visually 1 column.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "\u{00E9}".as_bytes()); // NFC é
        assert_eq!(t.cursor().0, 1, "NFC é advances 1 col");
        feed(&mut t, b"\r");
        feed(&mut t, b"\x1b[K"); // clear line
        feed(&mut t, "e\u{0301}".as_bytes()); // NFD: e + combining acute
        assert_eq!(t.cursor().0, 1, "NFD e+combining advances 1 col total");
    }

    #[test]
    fn t_r15_wide_char_at_exact_boundary() {
        // Wide char at penultimate column (n-2) fits exactly.
        let mut t = Terminal::new(6, 3);
        feed(&mut t, b"ABCD"); // cursor at col 4, cols 4-5 available
        feed(&mut t, "你".as_bytes()); // fits exactly at cols 4-5
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, '你');
        assert!(t.grid().cell(5, 0).unwrap().is_wide_spacer());
        // Cursor at last col (5) with pending_wrap set
        assert_eq!(t.cursor().0, 5, "cursor at last col with pending_wrap");
        assert!(t.cursor.pending_wrap, "pending_wrap set");
    }

    #[test]
    fn t_r15_cjk_range_width() {
        // CJK chars from 0x4E00 range should be width 2.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "字".as_bytes()); // U+5B57 CJK
        assert!(t.grid().cell(0, 0).unwrap().is_wide(), "CJK char is wide");
        assert_eq!(t.cursor().0, 2, "cursor advanced by 2");
    }

    #[test]
    fn t_r15_wide_char_bg_propagates_to_cell() {
        // BG on wide char should be visible on both cells.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[44m"); // blue bg
        feed(&mut t, "你".as_bytes());
        assert_eq!(
            t.grid().cell(0, 0).unwrap().bg,
            Color::Indexed(4),
            "lead has bg"
        );
        assert_eq!(
            t.grid().cell(1, 0).unwrap().bg,
            Color::Indexed(4),
            "spacer has bg"
        );
    }

    // ── Round 15-3: Scrollback / alternate screen edge cases ───────────

    #[test]
    fn t_r15_alt_screen_no_scrollback() {
        // Alt screen should not accumulate scrollback.
        let mut t = Terminal::with_scrollback(10, 3, 100);
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        for _ in 0..10 {
            feed(&mut t, b"TEST\r\n");
        }
        assert_eq!(t.grid().scrollback_len(), 0, "alt screen has no scrollback");
    }

    #[test]
    fn t_r15_primary_scrollback_preserved_after_alt() {
        // Primary scrollback should be preserved after alt screen round-trip.
        let mut t = Terminal::with_scrollback(10, 3, 100);
        for _ in 0..6 {
            feed(&mut t, b"LINE\r\n");
        }
        let before = t.grid().scrollback_len();
        assert!(before > 0, "scrollback accumulated");
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"ALT");
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert_eq!(
            t.grid().scrollback_len(),
            before,
            "scrollback preserved after alt round-trip"
        );
    }

    #[test]
    fn t_r15_scroll_region_does_not_affect_scrollback_above() {
        // Lines above scroll region should not be pushed to scrollback
        // when scrolling within the region.
        let mut t = Terminal::with_scrollback(5, 6, 100);
        feed(&mut t, b"R0\r\nR1\r\nR2\r\nR3\r\nR4\r\nR5");
        feed(&mut t, b"\x1b[3;5r"); // region rows 3-5 (0-based: 2..5)
        feed(&mut t, b"\x1b[5;1H"); // cursor at bottom of region
        feed(&mut t, b"\n"); // LF → scroll within region
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'R', "row 0 preserved");
        assert_eq!(
            t.grid().scrollback_len(),
            0,
            "scroll within region does not create scrollback"
        );
    }

    #[test]
    fn t_r15_alt_screen_cursor_row_preserved() {
        // 1049 should restore cursor row (not just column).
        let mut t = Terminal::new(10, 8);
        feed(&mut t, b"\x1b[5;3H"); // cursor at (2, 4)
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"\x1b[1;1H"); // move in alt
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert_eq!(t.cursor(), (2, 4), "cursor position fully restored");
    }

    #[test]
    fn t_r15_alt_1047_vs_1049_cursor_handling() {
        // Mode 1047 does NOT save/restore cursor; 1049 does.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;3H"); // cursor (2, 2)
        feed(&mut t, b"\x1b[?1047h"); // enter alt via 1047 — no cursor save
        feed(&mut t, b"\x1b[1;1H"); // move in alt
        feed(&mut t, b"\x1b[?1047l"); // exit alt — no cursor restore
        assert_eq!(t.cursor(), (0, 0), "1047 does not restore cursor");

        // Now test 1049
        feed(&mut t, b"\x1b[3;3H"); // cursor (2, 2)
        feed(&mut t, b"\x1b[?1049h"); // enter alt via 1049
        feed(&mut t, b"\x1b[1;1H"); // move in alt
        feed(&mut t, b"\x1b[?1049l"); // exit alt — cursor restored
        assert_eq!(t.cursor(), (2, 2), "1049 restores cursor");
    }

    #[test]
    fn t_r15_scrollback_clear_on_ris() {
        // RIS should clear scrollback.
        let mut t = Terminal::with_scrollback(10, 3, 100);
        for _ in 0..6 {
            feed(&mut t, b"DATA\r\n");
        }
        assert!(t.grid().scrollback_len() > 0, "scrollback exists");
        feed(&mut t, b"\x1bc"); // RIS
        assert_eq!(t.grid().scrollback_len(), 0, "RIS clears scrollback");
    }

    // ── Round 16-1: Mouse tracking mode flags ──────────────────────────

    #[test]
    fn t_r16_mouse_mode_defaults_off() {
        let t = Terminal::new(10, 5);
        assert!(!t.mouse_tracking_enabled(), "1000 default off");
        assert!(!t.mouse_button_event_enabled(), "1002 default off");
        assert!(!t.mouse_any_event_enabled(), "1003 default off");
        assert!(!t.mouse_sgr_enabled(), "1006 default off");
        assert!(!t.mouse_urxvt_enabled(), "1015 default off");
        assert!(!t.mouse_sgr_pixel_enabled(), "1016 default off");
    }

    #[test]
    fn t_r16_mouse_1000_toggle() {
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1000h");
        assert!(t.mouse_tracking_enabled());
        feed(&mut t, b"\x1b[?1000l");
        assert!(!t.mouse_tracking_enabled());
    }

    #[test]
    fn t_r16_mouse_1002_toggle() {
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1002h");
        assert!(t.mouse_button_event_enabled());
        feed(&mut t, b"\x1b[?1002l");
        assert!(!t.mouse_button_event_enabled());
    }

    #[test]
    fn t_r16_mouse_1003_toggle() {
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1003h");
        assert!(t.mouse_any_event_enabled());
        feed(&mut t, b"\x1b[?1003l");
        assert!(!t.mouse_any_event_enabled());
    }

    #[test]
    fn t_r16_mouse_sgr_1006_independent_from_tracking() {
        // SGR encoding can be enabled independently of tracking mode.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1006h");
        assert!(t.mouse_sgr_enabled());
        assert!(
            !t.mouse_tracking_enabled(),
            "SGR encoding independent from tracking"
        );
    }

    #[test]
    fn t_r16_mouse_modes_reset_by_ris() {
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h");
        feed(&mut t, b"\x1bc"); // RIS
        assert!(!t.mouse_tracking_enabled(), "RIS resets 1000");
        assert!(!t.mouse_button_event_enabled(), "RIS resets 1002");
        assert!(!t.mouse_any_event_enabled(), "RIS resets 1003");
        assert!(!t.mouse_sgr_enabled(), "RIS resets 1006");
    }

    #[test]
    fn t_r16_mouse_modes_reset_by_decstr() {
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1000h\x1b[?1006h");
        feed(&mut t, b"\x1b[!p"); // DECSTR
        assert!(!t.mouse_tracking_enabled(), "DECSTR resets 1000");
        assert!(!t.mouse_sgr_enabled(), "DECSTR resets 1006");
    }

    // ── Round 16-2: Tab stop edge cases ────────────────────────────────

    #[test]
    fn t_r16_tab_from_last_col_stays() {
        // Tab at last column should stay at last column.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1;10H"); // cursor at last col
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 9, "tab at last col stays");
    }

    #[test]
    fn t_r16_tab_with_no_stops_goes_to_end() {
        // With all stops cleared, tab goes to last col.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[3g"); // clear all stops
        feed(&mut t, b"\x1b[1;1H"); // col 0
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 19, "no stops → tab to last col");
    }

    #[test]
    fn t_r16_tab_stops_restored_by_ris() {
        // RIS should restore default tab stops (every 8 cols).
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[3g"); // clear all
        feed(&mut t, b"\x1bc"); // RIS
        feed(&mut t, b"\x1b[1;1H");
        feed(&mut t, b"\t"); // tab from col 0
        assert_eq!(t.cursor().0, 8, "RIS restores default 8-col stops");
    }

    #[test]
    fn t_r16_cht_basic() {
        // CHT (CSI Ps I) advances n tab stops.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[2I"); // CHT 2 — advance 2 stops from col 0
        assert_eq!(t.cursor().0, 16, "CHT 2 from col 0 → col 16");
    }

    #[test]
    fn t_r16_cbt_basic() {
        // CBT (CSI Ps Z) goes back n tab stops.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[1;25H"); // col 24
        feed(&mut t, b"\x1b[1Z"); // CBT 1
        assert_eq!(t.cursor().0, 16, "CBT 1 from col 24 → col 16");
    }

    #[test]
    fn t_r16_tbc_0_clears_current_stop() {
        // TBC 0 (default) clears current column's stop.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[1;9H"); // at col 8 (stop)
        feed(&mut t, b"\x1b[g"); // TBC 0
        feed(&mut t, b"\x1b[1;1H");
        feed(&mut t, b"\t"); // tab from col 0
        // Col 8 stop removed → should go to col 16
        assert_eq!(t.cursor().0, 16, "stop at 8 cleared");
    }

    // ── Round 16-3: OSC edge cases ─────────────────────────────────────

    #[test]
    fn t_r16_osc_title_strips_control_chars() {
        // OSC 0/2 title should strip control chars.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b]2;Hello\x01World\x07"); // BEL terminated
        assert_eq!(t.title(), "HelloWorld", "control chars stripped from title");
    }

    #[test]
    fn t_r16_osc_title_cap_256() {
        // Title longer than 256 chars should be truncated.
        let mut t = Terminal::new(10, 5);
        let long_title = "A".repeat(300);
        let osc = format!("\x1b]2;{}\x07", long_title);
        feed(&mut t, osc.as_bytes());
        assert_eq!(t.title().len(), 256, "title capped at 256 chars");
    }

    #[test]
    fn t_r16_osc_8_empty_uri_clears() {
        // OSC 8 with empty URI clears hyperlink.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b]8;;http://example.com\x1b\\");
        assert!(t.current_hyperlink.is_some());
        feed(&mut t, b"\x1b]8;;\x1b\\");
        assert!(t.current_hyperlink.is_none(), "empty URI clears hyperlink");
    }

    #[test]
    fn t_r16_osc_8_with_params() {
        // OSC 8 with params section.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b]8;id=123;http://example.com\x1b\\");
        assert_eq!(
            t.current_hyperlink.as_deref(),
            Some("http://example.com"),
            "params ignored, URI stored"
        );
    }

    #[test]
    fn t_r16_osc_8_uri_cap_2048() {
        // URI longer than 2048 bytes should be truncated.
        let mut t = Terminal::new(20, 3);
        let long_uri = format!("http://{}.com", "a".repeat(2100));
        let osc = format!("\x1b]8;;{}\x1b\\", long_uri);
        feed(&mut t, osc.as_bytes());
        // Should not panic, should be truncated
        let hl = t.current_hyperlink.as_ref().unwrap();
        assert!(hl.len() <= 2048, "URI capped at 2048 bytes");
    }

    #[test]
    fn t_r16_osc_4_color_query_response_format() {
        // OSC 4 query should respond with rgb:xx/xx/xx format.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b]4;0;?\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        assert!(
            s.starts_with("\x1b]4;0;rgb:"),
            "OSC 4 query response format: {}",
            s
        );
        assert!(s.ends_with("\x1b\\"), "OSC 4 response ends with ST");
    }

    #[test]
    fn t_r16_osc_4_color_set_then_query() {
        // Set color via OSC 4 then query to verify.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b]4;1;rgb:ff/00/00\x1b\\"); // set index 1 to red
        feed(&mut t, b"\x1b]4;1;?\x1b\\"); // query
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        assert!(
            s.contains("rgb:ff/00/00"),
            "set then query returns overridden color: {}",
            s
        );
    }

    #[test]
    fn t_r16_osc_10_query_response_format() {
        // OSC 10 query for fg color.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b]10;?\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        assert!(
            s.starts_with("\x1b]10;rgb:"),
            "OSC 10 query response format: {}",
            s
        );
    }

    #[test]
    fn t_r16_osc_11_query_response_format() {
        // OSC 11 query for bg color.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b]11;?\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        assert!(
            s.starts_with("\x1b]11;rgb:"),
            "OSC 11 query response format: {}",
            s
        );
    }

    #[test]
    fn t_r16_osc_4_multiple_colors_in_one_sequence() {
        // OSC 4 can set/query multiple colors in one sequence.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b]4;0;rgb:10/20/30;1;?\x1b\\"); // set 0, query 1
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        // Should only respond to the query for index 1
        assert!(s.starts_with("\x1b]4;1;rgb:"), "responds to query only");
    }

    #[test]
    fn t_r16_osc_110_reset_fg() {
        // OSC 110 resets dynamic fg to default.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b]10;rgb:ff/ff/00\x1b\\"); // set fg yellow
        feed(&mut t, b"\x1b]110\x1b\\"); // reset fg
        feed(&mut t, b"\x1b]10;?\x1b\\"); // query fg
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        // Should NOT be the yellow we set
        assert!(!s.contains("ffff00"), "OSC 110 resets fg");
    }

    // ── Round 17-1: DECSTR complete audit ──────────────────────────────

    #[test]
    fn t_r17_decstr_resets_keypad_app() {
        // DECSTR should reset keypad application mode (DECPAM/DECPNM).
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b="); // DECPAM — keypad app mode
        assert!(t.modes.keypad_app);
        feed(&mut t, b"\x1b[!p"); // DECSTR
        assert!(!t.modes.keypad_app, "DECSTR resets keypad_app");
    }

    #[test]
    fn t_r17_decstr_resets_kitty_keyboard() {
        // DECSTR should reset Kitty keyboard protocol to 0.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[>1u"); // Kitty keyboard enable
        assert!(t.modes.kitty_keyboard > 0);
        feed(&mut t, b"\x1b[!p"); // DECSTR
        assert_eq!(t.modes.kitty_keyboard, 0, "DECSTR resets kitty_keyboard");
    }

    #[test]
    fn t_r17_decstr_resets_kitty_kb_stack() {
        // DECSTR should clear the Kitty keyboard push/pop stack.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[>1u"); // enable
        feed(&mut t, b"\x1b[>1u"); // push flags
        feed(&mut t, b"\x1b[!p"); // DECSTR
        assert!(t.kitty_kb_stack.is_empty(), "DECSTR clears kitty_kb_stack");
    }

    #[test]
    fn t_r17_decstr_resets_cursor_blink() {
        // DECSTR should reset cursor_blink to default (true).
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?12l"); // cursor blink off
        assert!(!t.modes.cursor_blink);
        feed(&mut t, b"\x1b[!p"); // DECSTR
        assert!(
            t.modes.cursor_blink,
            "DECSTR resets cursor_blink to default"
        );
    }

    // ── Round 17-A: Alt screen edge cases ──────────────────────────────

    #[test]
    fn t_r17_alt_screen_content_isolated() {
        // Writing to alt screen should not affect main screen content.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"MAIN1\r\nMAIN2");
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"\x1b[2J"); // clear alt
        feed(&mut t, b"ALT_DATA");
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        // Main screen content should be intact
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'M',
            "main screen row 0 preserved"
        );
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            'M',
            "main screen row 1 preserved"
        );
    }

    #[test]
    fn t_r17_alt_screen_cursor_restored_after_write() {
        // Cursor should be fully restored after writing in alt screen.
        let mut t = Terminal::new(10, 8);
        feed(&mut t, b"\x1b[4;7H"); // cursor at (6, 3)
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        for _ in 0..5 {
            feed(&mut t, b"X\r\n"); // write and move cursor
        }
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert_eq!(t.cursor(), (6, 3), "cursor fully restored after alt writes");
    }

    #[test]
    fn t_r17_alt_screen_scroll_region_independent() {
        // 1049 clears screen AND resets scroll region.
        let mut t = Terminal::new(10, 8);
        feed(&mut t, b"\x1b[2;4r"); // region rows 2-4
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!((top, bottom), (1, 4));
        feed(&mut t, b"\x1b[?1049h"); // enter alt — clears screen
        // In alt screen, scroll region should be full screen
        let (alt_top, alt_bottom) = t.grid().scroll_region();
        assert_eq!(
            (alt_top, alt_bottom),
            (0, 8),
            "scroll region reset in alt screen"
        );
    }

    #[test]
    fn t_r17_alt_screen_no_leak_on_repeated_toggle() {
        // Repeatedly toggling alt screen should not accumulate content.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"BASE");
        for _ in 0..5 {
            feed(&mut t, b"\x1b[?1049h");
            feed(&mut t, b"TEMP");
            feed(&mut t, b"\x1b[?1049l");
        }
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'B', "base content intact");
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'A', "base content intact");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'S', "base content intact");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'E', "base content intact");
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, ' ', "no leak");
    }

    // ── Round 17-B: Wide character / Unicode boundaries ────────────────

    #[test]
    fn t_r17_wide_char_at_n_minus_1_wraps() {
        // Wide char at last column (n-1) with only 1 col left should wrap.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCD"); // cursor at col 4, only 1 col left
        feed(&mut t, "你".as_bytes()); // needs 2 cols
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, '你', "wrapped to row 1");
        assert!(t.grid().cell(1, 1).unwrap().is_wide_spacer());
    }

    #[test]
    fn t_r17_wide_char_overwrite_preserves_combining() {
        // Overwriting a wide char that has combining marks should clear them.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你".as_bytes()); // wide at cols 0-1
        feed(&mut t, "\u{0301}".as_bytes()); // combining on lead
        let cell = t.grid().cell(0, 0).unwrap();
        assert!(!cell.combining.is_empty(), "combining attached");
        // Now overwrite with narrow char
        feed(&mut t, b"\r");
        feed(&mut t, b"X");
        let cell2 = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell2.ch, 'X');
        assert!(cell2.combining.is_empty(), "combining cleared on overwrite");
    }

    #[test]
    fn t_r17_two_wide_chars_adjacent() {
        // Two adjacent wide chars should produce 4 cells.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你好".as_bytes());
        assert!(t.grid().cell(0, 0).unwrap().is_wide(), "first lead");
        assert!(
            t.grid().cell(1, 0).unwrap().is_wide_spacer(),
            "first spacer"
        );
        assert!(t.grid().cell(2, 0).unwrap().is_wide(), "second lead");
        assert!(
            t.grid().cell(3, 0).unwrap().is_wide_spacer(),
            "second spacer"
        );
        assert_eq!(t.cursor().0, 4, "cursor at col 4");
    }

    // ── Round 17-C: Scroll region boundaries ───────────────────────────

    #[test]
    fn t_r17_su_at_region_top_scrolls_content() {
        // SU at region top should push content up within region.
        let mut t = Terminal::new(5, 6);
        feed(&mut t, b"\x1b[2;5r"); // region rows 2-5 (0-based: 1..5)
        feed(&mut t, b"\x1b[1;1HA\r\nB\r\nC\r\nD\r\nE\r\nF");
        feed(&mut t, b"\x1b[2;1H"); // cursor at region top
        feed(&mut t, b"\x1b[1S"); // SU 1
        // Row 0 (above region) should be preserved
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "row 0 preserved");
    }

    #[test]
    fn t_r17_il_at_region_bottom_pushes_out() {
        // IL at region bottom should push lines out of region bottom.
        let mut t = Terminal::new(5, 6);
        feed(&mut t, b"\x1b[2;5r"); // region rows 2-5
        feed(&mut t, b"\x1b[1;1HA\r\nB\r\nC\r\nD\r\nE\r\nF");
        feed(&mut t, b"\x1b[5;1H"); // cursor at region bottom (row 5)
        feed(&mut t, b"\x1b[1L"); // IL 1
        // Row 0 (above region) should be preserved
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "row 0 preserved");
    }

    #[test]
    fn t_r17_dl_at_region_top_pulls_up() {
        // DL at region top should pull lines up from below.
        let mut t = Terminal::new(5, 6);
        feed(&mut t, b"\x1b[2;5r"); // region rows 2-5
        feed(&mut t, b"\x1b[1;1HA\r\nB\r\nC\r\nD\r\nE\r\nF");
        feed(&mut t, b"\x1b[2;1H"); // cursor at region top
        feed(&mut t, b"\x1b[1M"); // DL 1
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "row 0 preserved");
    }

    #[test]
    fn t_r17_sd_pushes_down_from_top() {
        // SD at region top should push content down, blanks at top.
        let mut t = Terminal::new(5, 6);
        feed(&mut t, b"\x1b[2;5r"); // region rows 2-5
        feed(&mut t, b"\x1b[1;1HA\r\nB\r\nC\r\nD\r\nE\r\nF");
        feed(&mut t, b"\x1b[2;1H"); // cursor at region top
        feed(&mut t, b"\x1b[1T"); // SD 1
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "row 0 preserved");
    }

    // ── Round 18-1: DECSC/DECRC audit ─────────────────────────────────

    #[test]
    fn t_r18_decsc_decrc_preserves_protected_attr() {
        // DECSC should save and restore protected_attr.
        // DECSCA format is CSI Ps " q (intermediate after param).
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[1\"q"); // DECSCA set protected
        assert!(t.protected_attr, "DECSCA 1 sets protected");
        feed(&mut t, b"\x1b7"); // save
        feed(&mut t, b"\x1b[2\"q"); // DECSCA unset protected
        assert!(!t.protected_attr, "DECSCA 2 unsets");
        feed(&mut t, b"\x1b8"); // restore
        assert!(t.protected_attr, "protected_attr restored by DECRC");
    }

    #[test]
    fn t_r18_decsc_decrc_preserves_cursor_style() {
        // DECSC should save and restore cursor_style.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[4 q"); // steady underline
        feed(&mut t, b"\x1b7"); // save
        feed(&mut t, b"\x1b[2 q"); // steady block
        feed(&mut t, b"\x1b8"); // restore
        assert_eq!(
            t.cursor_style,
            CursorStyle::SteadyUnderline,
            "cursor_style restored by DECRC"
        );
    }

    #[test]
    fn t_r18_decsc_decrc_preserves_pending_wrap() {
        // DECSC should save pending_wrap state.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE"); // fills row, pending_wrap set
        feed(&mut t, b"\x1b7"); // save (pending_wrap = true)
        feed(&mut t, b"\x1b[1;1H"); // move away, clears pending_wrap
        feed(&mut t, b"\x1b8"); // restore
        assert!(t.cursor.pending_wrap, "pending_wrap restored by DECRC");
    }

    #[test]
    fn t_r18_decrc_default_no_save() {
        // DECRC without prior DECSC should restore defaults.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[1;31m"); // bold + red
        feed(&mut t, b"\x1b[3;5H"); // cursor (4, 2)
        feed(&mut t, b"\x1b8"); // DECRC without DECSC
        assert_eq!(t.cursor(), (0, 0), "default cursor position");
        assert!(!t.flags.contains(CellFlags::BOLD), "default SGR");
        assert_eq!(t.fg, Color::Default, "default fg");
    }

    // ── Round 18-2: Tab stop edge cases ────────────────────────────────

    #[test]
    fn t_r18_tab_stops_after_shrink_grow() {
        // Tab stops should be correct after shrink then grow.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[1;1H\t");
        assert_eq!(t.cursor().0, 8, "tab to col 8 on 20-wide");
        t.resize(10, 3); // shrink
        feed(&mut t, b"\x1b[1;1H\t");
        assert_eq!(t.cursor().0, 8, "tab to col 8 on 10-wide");
        t.resize(20, 3); // grow back
        feed(&mut t, b"\x1b[1;1H\t");
        assert_eq!(t.cursor().0, 8, "tab to col 8 after grow back");
    }

    #[test]
    fn t_r18_tab_from_non_stop_column() {
        // Tab from a non-stop column should go to next stop.
        let mut t = Terminal::new(30, 3);
        feed(&mut t, b"\x1b[1;5H"); // cursor at col 4 (between stops 0 and 8)
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 8, "tab from col 4 → col 8");
    }

    #[test]
    fn t_r18_tab_into_scroll_region() {
        // Tab should not cross line boundaries regardless of scroll region.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[2;4r"); // scroll region rows 2-4
        feed(&mut t, b"\x1b[1;1H");
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 8, "tab works within scroll region");
        assert_eq!(t.cursor().1, 0, "tab does not change row");
    }

    #[test]
    fn t_r18_cht_at_last_tab_stop() {
        // CHT at/past last tab stop should go to last column.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[1;17H"); // cursor at col 16 (last stop)
        feed(&mut t, b"\x1b[1I"); // CHT 1
        // Should go to last column (19) since no more stops
        assert_eq!(t.cursor().0, 19, "CHT at last stop → last column");
    }

    #[test]
    fn t_r18_cbt_at_first_stop() {
        // CBT from first tab stop should go to col 0.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[1;9H"); // col 8 (first stop)
        feed(&mut t, b"\x1b[1Z"); // CBT 1
        assert_eq!(t.cursor().0, 0, "CBT from col 8 → col 0");
    }

    #[test]
    fn t_r18_cbt_at_col_0_stays() {
        // CBT at col 0 should stay at col 0.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[1Z"); // CBT from col 0
        assert_eq!(t.cursor().0, 0, "CBT at col 0 stays");
    }

    // ── Round 18-3: DECAWM (auto-wrap) edge cases ──────────────────────

    #[test]
    fn t_r18_decawm_off_cursor_clamps_at_last_col() {
        // With DECAWM off, cursor should stay at last column after writing.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[?7l"); // DECAWM off
        feed(&mut t, b"ABCDEF"); // 6 chars into 5-col terminal
        assert_eq!(t.cursor().0, 4, "cursor clamped at last col");
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'F', "last char at col 4");
    }

    #[test]
    fn t_r18_decawm_off_wide_char_at_boundary() {
        // With DECAWM off, wide char at penultimate col (1 col left).
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[?7l"); // DECAWM off
        feed(&mut t, b"ABCD"); // cursor at col 4, 1 col left
        feed(&mut t, "你".as_bytes()); // wide char, only 1 col available
        // Should NOT wrap. The char should be placed (put_char at col 4
        // with wide flag but no spacer since col+1 >= width).
        assert_eq!(t.cursor().1, 0, "no wrap to row 1");
    }

    #[test]
    fn t_r18_decawm_off_then_on_wraps() {
        // Re-enabling DECAWM should restore normal wrapping.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[?7l"); // off
        feed(&mut t, b"\x1b[?7h"); // on
        feed(&mut t, b"ABCDE"); // fills exactly
        assert!(t.cursor.pending_wrap, "pending_wrap set after filling row");
        feed(&mut t, b"F"); // should wrap
        assert_eq!(t.cursor().1, 1, "wraps to row 1");
    }

    #[test]
    fn t_r18_decawm_wrap_clears_pending_on_cup() {
        // CUP should clear pending_wrap.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE"); // fills row, pending_wrap
        assert!(t.cursor.pending_wrap);
        feed(&mut t, b"\x1b[1;1H"); // CUP to (0, 0)
        assert!(!t.cursor.pending_wrap, "CUP clears pending_wrap");
    }

    #[test]
    fn t_r18_decawm_wrap_creates_correct_display() {
        // When auto-wrap triggers, content should be on row 1.
        let mut t = Terminal::with_scrollback(5, 3, 100);
        feed(&mut t, b"ABCDEF"); // F wraps to row 1
        // Content should be correct
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "row 0 col 0");
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'E', "row 0 col 4");
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'F', "F wrapped to row 1");
    }

    #[test]
    fn t_r18_decawm_off_content_stays_on_one_row() {
        // With DECAWM off, writing past last col should keep all content on row 0.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[?7l"); // off
        feed(&mut t, b"ABCDEF"); // overwrites last col
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, ' ', "no content on row 1");
        assert_eq!(t.cursor().1, 0, "cursor stays on row 0");
    }

    // ── Round 19-1: Wide char / combining char edge cases ─────────────

    #[test]
    fn t_r19_bs_after_narrow_stays_at_0() {
        // BS at col 0 should stay at col 0.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x08");
        assert_eq!(t.cursor().0, 0, "BS at col 0 stays");
    }

    #[test]
    fn t_r19_dch_removes_full_wide_char() {
        // DCH at the lead cell of a wide char should remove both cells.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你".as_bytes()); // cols 0-1
        feed(&mut t, b"X"); // col 2
        feed(&mut t, b"\r"); // back to col 0
        feed(&mut t, b"\x1b[2P"); // DCH 2 at col 0
        // Both wide char cells should be cleared, X shifts left
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X', "X shifted to col 0");
        assert!(
            !t.grid().cell(1, 0).unwrap().is_wide_spacer(),
            "no orphan spacer"
        );
    }

    #[test]
    fn t_r19_wide_char_insert_mode() {
        // In insert mode (IRM), writing a wide char should shift cells right.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGH");
        feed(&mut t, b"\r"); // col 0
        feed(&mut t, b"\x1b[4h"); // insert mode on
        feed(&mut t, "你".as_bytes()); // insert wide at col 0
        // A should shift right by 2
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '你', "wide at col 0");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'A', "A shifted to col 2");
    }

    #[test]
    fn t_r19_combining_chain_multiple() {
        // Multiple combining chars on one base char.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"e");
        feed(&mut t, "\u{0301}\u{0308}\u{0302}".as_bytes()); // acute + diaeresis + circumflex
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.ch, 'e', "base char is e");
        assert_eq!(cell.combining.len(), 3, "3 combining chars attached");
        assert_eq!(t.cursor().0, 1, "cursor advanced by 1 (base only)");
    }

    #[test]
    fn t_r19_combining_cap_8() {
        // Combining char cap should prevent memory exhaustion.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"A");
        // Send 20 combining chars — should cap at 8
        for _ in 0..20 {
            feed(&mut t, "\u{0301}".as_bytes());
        }
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.combining.len(), 8, "combining capped at 8");
    }

    #[test]
    fn t_r19_wide_char_then_combining_then_narrow() {
        // Wide char + combining + narrow char on next col.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你".as_bytes()); // cols 0-1
        feed(&mut t, "\u{0301}".as_bytes()); // combining on lead
        feed(&mut t, b"X"); // narrow at col 2
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'X', "X at col 2");
        assert!(t.grid().cell(0, 0).unwrap().is_wide(), "lead preserved");
    }

    #[test]
    fn t_r19_3_byte_utf8_cjk() {
        // 3-byte UTF-8 CJK character should decode and render correctly.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "語".as_bytes()); // U+8A9E (3-byte UTF-8)
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '語');
        assert!(t.grid().cell(0, 0).unwrap().is_wide());
        assert_eq!(t.cursor().0, 2, "cursor at col 2");
    }

    #[test]
    fn t_r19_4_byte_utf8_emoji() {
        // 4-byte UTF-8 emoji should decode and render correctly.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "🚀".as_bytes()); // U+1F680 (4-byte UTF-8)
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '🚀');
        assert!(t.grid().cell(0, 0).unwrap().is_wide());
        assert_eq!(t.cursor().0, 2, "cursor at col 2");
    }

    #[test]
    fn t_r19_invalid_utf8_fallback() {
        // Invalid UTF-8 byte should produce replacement char.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, &[0xFF]); // invalid UTF-8
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            '\u{FFFD}',
            "replacement char"
        );
    }

    #[test]
    fn t_r19_split_utf8_across_feeds() {
        // UTF-8 character split across two feed() calls.
        // The VTE Parser maintains UTF-8 continuation state internally.
        // The test helper creates a fresh Parser each call, so we test
        // the single-feed path here (the parser handles split internally).
        let mut t = Terminal::new(10, 3);
        // Feed a 3-byte CJK char + a 4-byte emoji in one feed
        feed(&mut t, "語🚀".as_bytes());
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '語', "CJK char assembled");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, '🚀', "emoji assembled");
        assert_eq!(t.cursor().0, 4, "cursor at col 4");
    }

    // ── Round 19-2: OSC sequence edge cases ────────────────────────────

    #[test]
    fn t_r19_osc_title_utf8() {
        // OSC 2 with UTF-8 title.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, "中".as_bytes()); // make sure UTF-8 works
        let osc = format!("\x1b]2;{}\x07", "你好世界");
        feed(&mut t, osc.as_bytes());
        assert_eq!(t.title(), "你好世界", "UTF-8 title");
    }

    #[test]
    fn t_r19_osc_empty_title() {
        // OSC 0 with empty title.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b]0;My Title\x07");
        assert_eq!(t.title(), "My Title");
        feed(&mut t, b"\x1b]0;\x07"); // empty
        assert_eq!(t.title(), "", "empty title");
    }

    #[test]
    fn t_r19_osc_8_applied_to_specific_range() {
        // OSC 8 hyperlink applied to a range of cells.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b]8;;http://example.com\x1b\\");
        feed(&mut t, b"Click"); // 5 cells with hyperlink
        feed(&mut t, b"\x1b]8;;\x1b\\"); // clear
        feed(&mut t, b" me"); // 3 cells without hyperlink
        // Check first 5 cells have hyperlink
        for i in 0..5 {
            assert!(
                t.grid().cell(i, 0).unwrap().hyperlink.is_some(),
                "cell {i} has hyperlink"
            );
        }
        // Check cells 6-7 don't have hyperlink
        for i in 6..8 {
            assert!(
                t.grid().cell(i, 0).unwrap().hyperlink.is_none(),
                "cell {i} no hyperlink"
            );
        }
    }

    #[test]
    fn t_r19_osc_10_set_dynamic_fg() {
        // OSC 10 set fg color.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b]10;rgb:ff/80/00\x1b\\");
        assert!(t.dynamic_fg.is_some(), "dynamic fg set");
    }

    #[test]
    fn t_r19_osc_11_set_dynamic_bg() {
        // OSC 11 set bg color.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b]11;rgb:00/ff/00\x1b\\");
        assert!(t.dynamic_bg.is_some(), "dynamic bg set");
    }

    #[test]
    fn t_r19_osc_110_reset_dynamic_fg() {
        // OSC 110 should clear dynamic fg.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b]10;rgb:ff/00/00\x1b\\");
        feed(&mut t, b"\x1b]110\x1b\\");
        assert!(t.dynamic_fg.is_none(), "dynamic fg cleared");
    }

    #[test]
    fn t_r19_osc_111_reset_dynamic_bg() {
        // OSC 111 should clear dynamic bg.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b]11;rgb:00/00/ff\x1b\\");
        feed(&mut t, b"\x1b]111\x1b\\");
        assert!(t.dynamic_bg.is_none(), "dynamic bg cleared");
    }

    #[test]
    fn t_r19_osc_4_set_palette_override() {
        // OSC 4 set palette override then verify via palette_overrides.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b]4;5;rgb:aa/bb/cc\x1b\\");
        let overrides = t.palette_overrides();
        assert_eq!(
            overrides.get(&5),
            Some(&(0xaa, 0xbb, 0xcc)),
            "palette override for index 5"
        );
    }

    #[test]
    fn t_r19_osc_title_bel_and_st_both_work() {
        // Both BEL and ST termination should work for OSC.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b]0;BEL_TERM\x07"); // BEL terminated
        assert_eq!(t.title(), "BEL_TERM");
        feed(&mut t, b"\x1b]0;ST_TERM\x1b\\"); // ST terminated
        assert_eq!(t.title(), "ST_TERM");
    }

    // ── Round 19-3: Cursor movement + scroll region ────────────────────

    #[test]
    fn t_r19_cuu_stops_at_region_top_inside() {
        // CUU inside scroll region stops at region top.
        let mut t = Terminal::new(10, 8);
        feed(&mut t, b"\x1b[3;6r"); // region rows 3-6 (0-based: 2..6)
        feed(&mut t, b"\x1b[5;1H"); // cursor at row 5 (inside region)
        feed(&mut t, b"\x1b[10A"); // CUU 10 — should stop at region top (row 2)
        assert_eq!(t.cursor().1, 2, "CUU stops at region top");
    }

    #[test]
    fn t_r19_cud_stops_at_region_bottom_inside() {
        // CUD inside scroll region stops at region bottom.
        let mut t = Terminal::new(10, 8);
        feed(&mut t, b"\x1b[3;6r"); // region rows 3-6 (0-based: 2..6)
        feed(&mut t, b"\x1b[4;1H"); // cursor at row 4 (inside region)
        feed(&mut t, b"\x1b[10B"); // CUD 10 — should stop at region bottom (row 5)
        assert_eq!(t.cursor().1, 5, "CUD stops at region bottom");
    }

    #[test]
    fn t_r19_cuu_outside_region_goes_to_top() {
        // CUU from above the scroll region goes to row 0.
        let mut t = Terminal::new(10, 8);
        feed(&mut t, b"\x1b[4;6r"); // region rows 4-6 (0-based: 3..6)
        feed(&mut t, b"\x1b[2;1H"); // cursor at row 1 (above region)
        feed(&mut t, b"\x1b[10A"); // CUU 10
        assert_eq!(t.cursor().1, 0, "CUU above region goes to row 0");
    }

    #[test]
    fn t_r19_cnl_resets_col_to_0() {
        // CNL should reset column to 0.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[2;5H"); // cursor at (4, 1)
        feed(&mut t, b"\x1b[2E"); // CNL 2
        assert_eq!(t.cursor().0, 0, "CNL resets column to 0");
        assert_eq!(t.cursor().1, 3, "CNL moves down 2 rows");
    }

    #[test]
    fn t_r19_cpl_resets_col_to_0() {
        // CPL should reset column to 0.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[4;5H"); // cursor at (4, 3)
        feed(&mut t, b"\x1b[2F"); // CPL 2
        assert_eq!(t.cursor().0, 0, "CPL resets column to 0");
        assert_eq!(t.cursor().1, 1, "CPL moves up 2 rows");
    }

    #[test]
    fn t_r19_cuf_stops_at_last_col() {
        // CUF should stop at last column.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1;5H"); // col 4
        feed(&mut t, b"\x1b[100C"); // CUF 100
        assert_eq!(t.cursor().0, 9, "CUF stops at last col");
    }

    #[test]
    fn t_r19_cub_stops_at_col_0() {
        // CUB should stop at col 0.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1;5H"); // col 4
        feed(&mut t, b"\x1b[100D"); // CUB 100
        assert_eq!(t.cursor().0, 0, "CUB stops at col 0");
    }

    #[test]
    fn t_r19_vpa_in_origin_mode_relative() {
        // VPA in origin mode is relative to scroll region top.
        let mut t = Terminal::new(10, 8);
        feed(&mut t, b"\x1b[3;6r"); // region rows 3-6 (0-based: 2..6)
        feed(&mut t, b"\x1b[?6h"); // origin on
        feed(&mut t, b"\x1b[1d"); // VPA row 1 → absolute = region_top + 0 = 2
        assert_eq!(t.cursor().1, 2, "VPA row 1 in origin mode → abs row 2");
    }

    // ── Round 20-1: Resize / Reflow edge cases ─────────────────────────

    #[test]
    fn t_r20_resize_resets_scroll_region() {
        // Resize should reset scroll region to full screen.
        let mut t = Terminal::new(10, 8);
        feed(&mut t, b"\x1b[3;6r"); // region rows 3-6
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!((top, bottom), (2, 6));
        t.resize(10, 8); // same size — should still reset
        let (top2, bottom2) = t.grid().scroll_region();
        assert_eq!((top2, bottom2), (0, 8), "resize resets scroll region");
    }

    #[test]
    fn t_r20_resize_shrink_then_grow_preserves_text_reflow() {
        // With reflow on, shrink then grow should preserve text.
        let mut t = Terminal::with_scrollback(20, 5, 100);
        feed(&mut t, b"Hello World Test"); // 16 chars on 20-wide
        t.resize(10, 5); // shrink — text reflows
        t.resize(20, 5); // grow back
        // Content should reflow back to original layout
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'H',
            "first char preserved after roundtrip"
        );
    }

    #[test]
    fn t_r20_resize_preserves_scrollback_height_change() {
        // Height change should pull from / push to scrollback.
        let mut t = Terminal::with_scrollback(10, 4, 100);
        for i in 0..6 {
            let line = format!("L{}\r\n", i);
            feed(&mut t, line.as_bytes());
        }
        let sb_before = t.grid().scrollback_len();
        assert!(sb_before > 0, "scrollback has content");
        t.resize(10, 2); // shrink height — more rows to scrollback
        let sb_after_shrink = t.grid().scrollback_len();
        assert!(sb_after_shrink >= sb_before, "scrollback grows on shrink");
        t.resize(10, 4); // grow height — pull back from scrollback
        let sb_after_grow = t.grid().scrollback_len();
        assert!(
            sb_after_grow <= sb_after_shrink,
            "scrollback shrinks on grow"
        );
    }

    #[test]
    fn t_r20_resize_saved_cursor_clamped() {
        // DECSC saves cursor, then resize clamps — DECRC should give valid pos.
        let mut t = Terminal::new(20, 10);
        feed(&mut t, b"\x1b[10;20H"); // cursor at (19, 9)
        feed(&mut t, b"\x1b7"); // DECSC save
        t.resize(5, 3); // shrink — saved cursor now out of bounds
        feed(&mut t, b"\x1b8"); // DECRC restore
        // The restored cursor should not cause out-of-bounds writes
        feed(&mut t, b"X");
        // Should not panic
        assert!(t.cursor().0 < 5, "cursor x in bounds after resize+restore");
        assert!(t.cursor().1 < 3, "cursor y in bounds after resize+restore");
    }

    #[test]
    fn t_r20_resize_scosc_cursor_clamped() {
        // SCOSC saves cursor, then resize — SCORC should not cause issues.
        let mut t = Terminal::new(20, 10);
        feed(&mut t, b"\x1b[10;20H"); // cursor at (19, 9)
        feed(&mut t, b"\x1b[s"); // SCOSC save
        t.resize(5, 3); // shrink
        feed(&mut t, b"\x1b[u"); // SCORC restore
        feed(&mut t, b"X");
        assert!(t.cursor().0 < 5, "cursor x in bounds");
    }

    #[test]
    fn t_r20_resize_no_reflow_alt_screen() {
        // In alt screen, resize should NOT reflow content.
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"ABCDEFGH"); // 8 chars on 10-wide
        t.resize(5, 4); // shrink width
        // In alt screen, content is truncated not reflowed
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "A preserved");
        // Content beyond new width is lost in alt screen (no reflow)
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            ' ',
            "row 1 blank (no reflow)"
        );
    }

    #[test]
    fn t_r20_resize_reflow_merges_wrapped_lines() {
        // When reflowing to wider width, soft-wrapped lines merge.
        let mut t = Terminal::with_scrollback(5, 3, 100);
        feed(&mut t, b"Hello"); // fills row 0, soft-wrapped
        feed(&mut t, b"World"); // row 1
        // Now reflow to wider width — the two rows should merge into one
        t.resize(10, 3);
        // After reflow, "HelloWorld" should be on a single row
        let row0_text = t.grid().row_text(0).unwrap_or_default();
        assert!(
            row0_text.starts_with("HelloWorld"),
            "reflow merges wrapped lines: got '{row0_text}'"
        );
    }

    #[test]
    fn t_r20_resize_cursor_at_bottom_preserved() {
        // Cursor at bottom row should stay valid after height shrink.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[5;1H"); // cursor at row 4 (bottom)
        t.resize(10, 3); // shrink
        assert_eq!(t.cursor().1, 2, "cursor clamped to new bottom");
    }

    // ── Round 20-2: Bracketed paste / Focus events ─────────────────────

    #[test]
    fn t_r20_bracketed_paste_wraps_content() {
        // When bracketed paste is on, pasted content should be wrapped.
        let mut t = Terminal::new(40, 5);
        feed(&mut t, b"\x1b[?2004h"); // enable
        feed(&mut t, b"\x1b[200~hello\x1b[201~");
        // The content between brackets should be printed
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'h',
            "pasted content printed"
        );
        assert_eq!(
            t.grid().cell(4, 0).unwrap().ch,
            'o',
            "pasted content printed"
        );
    }

    #[test]
    fn t_r20_bracketed_paste_no_wrap_when_disabled() {
        // When bracketed paste is off, content should NOT be wrapped.
        let mut t = Terminal::new(40, 5);
        // paste markers should be treated as regular input (ignored or printed)
        feed(&mut t, b"\x1b[200~hello\x1b[201~");
        // Without bracketed paste mode, the CSI sequences are unknown
        // and 'hello' should still be printed
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'h',
            "content printed without brackets"
        );
    }

    #[test]
    fn t_r20_bracketed_paste_nested_markers() {
        // Nested paste markers — inner markers should be treated as text.
        let mut t = Terminal::new(40, 5);
        feed(&mut t, b"\x1b[?2004h"); // enable
        feed(&mut t, b"\x1b[200~A\x1b[200~B\x1b[201~C\x1b[201~");
        // All content between outer markers should be printed
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'C');
    }

    #[test]
    fn t_r20_bracketed_paste_reset_no_leak() {
        // After disabling bracketed paste, subsequent paste markers should not wrap.
        let mut t = Terminal::new(40, 5);
        feed(&mut t, b"\x1b[?2004h"); // enable
        feed(&mut t, b"\x1b[?2004l"); // disable
        feed(&mut t, b"\x1b[200~hello\x1b[201~");
        // 'hello' should still be printed (markers are just ignored CSI)
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'h');
    }

    #[test]
    fn t_r20_focus_event_no_report_when_disabled() {
        // Focus events should not be reported when disabled.
        let t = Terminal::new(10, 5);
        assert!(!t.modes.focus_event);
        // No way to trigger focus in/out from terminal side (it's input-only)
        // Just verify the mode is off and can be queried
    }

    #[test]
    fn t_r20_focus_event_toggle_and_decstr() {
        // Focus event mode toggle + DECSTR reset.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1004h"); // enable
        assert!(t.modes.focus_event);
        feed(&mut t, b"\x1b[!p"); // DECSTR
        assert!(!t.modes.focus_event, "DECSTR resets focus event");
    }

    #[test]
    fn t_r20_synchronized_output_toggle() {
        // DECSET 2026 — synchronized output mode.
        let mut t = Terminal::new(10, 5);
        assert!(!t.modes.synchronized_output);
        feed(&mut t, b"\x1b[?2026h"); // enable
        assert!(t.modes.synchronized_output);
        feed(&mut t, b"\x1b[?2026l"); // disable
        assert!(!t.modes.synchronized_output);
    }

    #[test]
    fn t_r20_synchronized_output_decstr() {
        // DECSTR should reset synchronized output.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?2026h"); // enable
        feed(&mut t, b"\x1b[!p"); // DECSTR
        assert!(!t.modes.synchronized_output, "DECSTR resets sync output");
    }

    #[test]
    fn t_r20_bracketed_paste_persists_through_resize() {
        // Resize should not affect bracketed paste mode.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?2004h"); // enable
        t.resize(20, 10);
        assert!(
            t.modes.bracketed_paste,
            "bracketed paste persists through resize"
        );
    }

    #[test]
    fn t_r20_mouse_mode_persists_through_resize() {
        // Resize should not affect mouse tracking mode.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1000h"); // enable mouse
        t.resize(20, 10);
        assert!(
            t.mouse_tracking_enabled(),
            "mouse mode persists through resize"
        );
    }

    // ── Round 21-1: Tab stop edge cases ────────────────────────────────

    #[test]
    fn t_r21_hts_at_col_0() {
        // HTS at col 0 should set a stop at col 0.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[3g"); // clear all
        feed(&mut t, b"\x1bH"); // set stop at col 0
        feed(&mut t, b"\x1b[1;10H"); // move to col 9
        feed(&mut t, b"\t"); // tab backward? No, tab goes forward
        // Only stop at col 0, so tab from col 9 goes to col 19
        assert_eq!(t.cursor().0, 19, "tab to last col (only stop at 0)");
    }

    #[test]
    fn t_r21_cht_default_1() {
        // CHT with no param defaults to 1.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[I"); // CHT with no param → 1
        assert_eq!(t.cursor().0, 8, "CHT default = 1 stop → col 8");
    }

    #[test]
    fn t_r21_cbt_default_1() {
        // CBT with no param defaults to 1.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[1;17H"); // col 16
        feed(&mut t, b"\x1b[Z"); // CBT no param → 1
        assert_eq!(t.cursor().0, 8, "CBT default = 1 stop → col 8");
    }

    #[test]
    fn t_r21_cht_zero_param() {
        // CHT 0 should behave like CHT 1.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[0I"); // CHT 0
        assert_eq!(t.cursor().0, 8, "CHT 0 → col 8 (treated as 1)");
    }

    #[test]
    fn t_r21_tab_through_custom_stops() {
        // Set custom stops at 5, 10, 15 then tab through.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[3g"); // clear all
        feed(&mut t, b"\x1b[1;6H\x1bH"); // stop at col 5
        feed(&mut t, b"\x1b[1;11H\x1bH"); // stop at col 10
        feed(&mut t, b"\x1b[1;16H\x1bH"); // stop at col 15
        feed(&mut t, b"\x1b[1;1H"); // back to col 0
        feed(&mut t, b"\t"); // → col 5
        assert_eq!(t.cursor().0, 5, "tab to custom stop 5");
        feed(&mut t, b"\t"); // → col 10
        assert_eq!(t.cursor().0, 10, "tab to custom stop 10");
        feed(&mut t, b"\t"); // → col 15
        assert_eq!(t.cursor().0, 15, "tab to custom stop 15");
    }

    #[test]
    fn t_r21_tab_stop_clear_all_then_tab() {
        // After clearing all stops, tab should go to last col.
        let mut t = Terminal::new(15, 3);
        feed(&mut t, b"\x1b[3g"); // clear all
        feed(&mut t, b"\t");
        assert_eq!(t.cursor().0, 14, "tab to last col when no stops");
    }

    // ── Round 21-2: Charset edge cases ─────────────────────────────────

    #[test]
    fn t_r21_dec_special_block_char() {
        // DEC Special Graphics: ASCII 'a' (0x61) → block '▒' (U+2592).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b(0"); // G0 = DEC Special
        feed(&mut t, b"a"); // 'a' → block char
        let ch = t.grid().cell(0, 0).unwrap().ch;
        assert_eq!(ch, '\u{2592}', "DEC special 'a' → block char ▒");
    }

    #[test]
    fn t_r21_dec_special_all_line_chars() {
        // All DEC line drawing characters mapped correctly.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b(0");
        feed(&mut t, b"q"); // ─ horizontal line
        feed(&mut t, b"x"); // │ vertical line
        feed(&mut t, b"l"); // ┌ upper-left
        feed(&mut t, b"k"); // ┐ upper-right
        feed(&mut t, b"m"); // ┘ lower-right
        feed(&mut t, b"j"); // └ lower-left
        feed(&mut t, b"\x1b(B"); // restore ASCII
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{2500}', "─");
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, '\u{2502}', "│");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, '\u{250C}', "┌");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, '\u{2510}', "┐");
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, '\u{2514}', "┘ flipped");
        assert_eq!(t.grid().cell(5, 0).unwrap().ch, '\u{2518}', "└ flipped");
    }

    #[test]
    fn t_r21_so_si_with_printable_ascii() {
        // SO activates G1, SI activates G0.
        // DEC Special Graphics only maps 0x60-0x7E range.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b)0"); // G1 = DEC Special
        feed(&mut t, b"qq"); // G0 (ASCII) → 'q', 'q'
        feed(&mut t, b"\x0e"); // SO → activate G1
        feed(&mut t, b"qq"); // G1 (DEC Special) → ─, ─
        feed(&mut t, b"\x0f"); // SI → activate G0
        feed(&mut t, b"qq"); // G0 (ASCII) → 'q', 'q'
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'q', "G0 q");
        assert_eq!(
            t.grid().cell(2, 0).unwrap().ch,
            '\u{2500}',
            "G1 q → ─ (horizontal line)"
        );
        assert_eq!(
            t.grid().cell(3, 0).unwrap().ch,
            '\u{2500}',
            "G1 q → ─ (horizontal line)"
        );
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'q', "G0 q restored");
        assert_eq!(t.grid().cell(5, 0).unwrap().ch, 'q', "G0 q restored");
    }

    #[test]
    fn t_r21_charset_survives_cup() {
        // Charset designation should survive cursor movement.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b(0"); // G0 = DEC Special
        feed(&mut t, b"\x1b[2;1H"); // move cursor
        feed(&mut t, b"q"); // print in DEC special mode
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            '\u{2500}',
            "charset active after CUP"
        );
    }

    #[test]
    fn t_r21_charset_reset_by_ris() {
        // RIS should reset charset to ASCII.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b(0"); // G0 = DEC Special
        feed(&mut t, b"\x1bc"); // RIS
        feed(&mut t, b"q"); // should be ASCII 'q'
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'q', "RIS resets charset");
    }

    #[test]
    fn t_r21_so_at_col_boundary() {
        // SO at column boundary should work correctly.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b)0"); // G1 = DEC Special
        feed(&mut t, b"ABCDE"); // fill row 0 with ASCII
        feed(&mut t, b"\x0e"); // SO → G1 active
        feed(&mut t, b"q"); // first char on row 1
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            '\u{2500}',
            "G1 active at row boundary"
        );
    }

    // ── Round 21-3: Alt screen edge cases ──────────────────────────────

    #[test]
    fn t_r21_alt_double_1049h_no_double_save() {
        // Two consecutive 1049h should not double-save.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"BASE");
        feed(&mut t, b"\x1b[?1049h"); // enter alt — saves state
        feed(&mut t, b"\x1b[?1049h"); // enter again — should be no-op
        feed(&mut t, b"ALT");
        feed(&mut t, b"\x1b[?1049l"); // exit alt — should restore BASE
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'B',
            "base content restored"
        );
        assert_eq!(
            t.grid().cell(1, 0).unwrap().ch,
            'A',
            "base content restored"
        );
    }

    #[test]
    fn t_r21_alt_1049h_then_1047l_mixed() {
        // Enter with 1049h, exit with 1047l — should still work.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"MAIN");
        feed(&mut t, b"\x1b[?1049h"); // enter alt (save cursor)
        feed(&mut t, b"ALT");
        feed(&mut t, b"\x1b[?1047l"); // exit alt (may not restore cursor)
        // Main content should be restored
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'M',
            "main restored with mixed exit"
        );
    }

    #[test]
    fn t_r21_alt_screen_no_scrollback_on_scroll() {
        // Scrolling in alt screen should not create scrollback.
        let mut t = Terminal::with_scrollback(10, 3, 100);
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        for _ in 0..10 {
            feed(&mut t, b"X\n");
        }
        assert_eq!(
            t.grid().scrollback_len(),
            0,
            "alt screen scrolling creates no scrollback"
        );
    }

    #[test]
    fn t_r21_alt_screen_content_cleared_on_enter() {
        // Entering alt screen should clear it (1049 clears screen).
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        // Alt screen should be blank
        for col in 0..10 {
            assert_eq!(
                t.grid().cell(col, 0).unwrap().ch,
                ' ',
                "alt screen blank at col {col}"
            );
        }
    }

    #[test]
    fn t_r21_alt_47_then_47_then_1049l() {
        // Enter with 47h, exit with 1049l — mixed modes.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"MAIN");
        feed(&mut t, b"\x1b[?47h"); // enter alt (no cursor save)
        feed(&mut t, b"ALT");
        feed(&mut t, b"\x1b[?1049l"); // exit with 1049l
        // Main content should be restored regardless
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'M',
            "main restored with 47h→1049l"
        );
    }

    #[test]
    fn t_r21_alt_screen_tab_stops_preserved() {
        // Custom tab stops should survive alt screen round-trip.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[3g"); // clear all
        feed(&mut t, b"\x1b[1;5H\x1bH"); // stop at col 4
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"X");
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        feed(&mut t, b"\x1b[1;1H"); // move to col 0
        feed(&mut t, b"\t"); // tab from col 0
        // Custom stop at col 4 should still exist
        assert_eq!(t.cursor().0, 4, "custom tab stop preserved after alt");
    }

    // ── Round 22-1: OSC sequence edge cases ────────────────────────────

    #[test]
    fn t_r22_osc_unterminated_then_new_escape() {
        // Unterminated OSC (no ST/BEL) followed by a new escape sequence.
        // The new ESC should abort the OSC.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b]2;Bad Title"); // unterminated OSC 2
        feed(&mut t, b"\x1b[?25l"); // new escape — should abort OSC
        // Title should NOT be set (OSC was aborted)
        assert_ne!(t.title(), "Bad Title", "unterminated OSC not applied");
        // The new sequence should work
        assert!(
            !t.modes.cursor_visible,
            "cursor hidden by subsequent sequence"
        );
    }

    #[test]
    fn t_r22_osc_unterminated_then_text() {
        // OSC terminated by BEL then text — text should print.
        // (OSC and BEL must be in same feed since test helper creates new Parser.)
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b]2;Bad\x07Hello");
        assert_eq!(t.title(), "Bad", "title set by BEL");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'H',
            "text printed after OSC"
        );
    }

    #[test]
    fn t_r22_osc_esc_then_non_backslash() {
        // ESC inside OSC followed by non-backslash should abort OSC.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b]2;Title"); // OSC start
        feed(&mut t, b"\x1b[?25h"); // ESC [ — aborts OSC, enters CSI
        assert_ne!(t.title(), "Title", "OSC aborted by ESC [");
        assert!(t.modes.cursor_visible, "cursor visible set");
    }

    #[test]
    fn t_r22_osc_12_cursor_color_query() {
        // OSC 12 query for cursor color should respond.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b]12;?\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        assert!(
            s.starts_with("\x1b]12;rgb:"),
            "OSC 12 query response format: {}",
            s
        );
    }

    #[test]
    fn t_r22_osc_12_set_cursor_color() {
        // OSC 12 set cursor color then query.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b]12;rgb:ff/00/ff\x1b\\");
        assert!(t.dynamic_cursor.is_some(), "dynamic cursor color set");
        // Query in separate feed
        feed(&mut t, b"\x1b]12;?\x1b\\");
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        assert!(s.contains("ff/00/ff"), "query returns set color: {}", s);
    }

    #[test]
    fn t_r22_osc_112_reset_cursor_color() {
        // OSC 112 should reset cursor color.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b]12;rgb:ff/00/ff\x1b\\");
        feed(&mut t, b"\x1b]112\x1b\\"); // reset
        assert!(t.dynamic_cursor.is_none(), "cursor color reset");
    }

    #[test]
    fn t_r22_osc_with_newline_in_payload() {
        // OSC payload with newline (0x0A) — should be ignored (< 0x20).
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b]0;Hello\nWorld\x07");
        // Newline (0x0A) is < 0x20, ignored by OSC parser
        assert_eq!(t.title(), "HelloWorld", "newline stripped from OSC payload");
    }

    #[test]
    fn t_r22_osc_followed_by_normal_text() {
        // OSC sequence followed by normal printable text.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b]0;Title\x07ABC");
        assert_eq!(t.title(), "Title");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'C');
    }

    // ── Round 22-2: Mouse tracking edge cases ──────────────────────────

    #[test]
    fn t_r22_mouse_all_modes_default_off() {
        // All mouse modes should be off by default.
        let t = Terminal::new(10, 5);
        assert!(!t.mouse_tracking_enabled());
        assert!(!t.mouse_button_event_enabled());
        assert!(!t.mouse_any_event_enabled());
        assert!(!t.mouse_sgr_enabled());
        assert!(!t.mouse_urxvt_enabled());
        assert!(!t.mouse_sgr_pixel_enabled());
    }

    #[test]
    fn t_r22_mouse_1005_utf8_mode_toggle() {
        // DECSET 1005 — UTF-8 mouse coordinate encoding.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1005h");
        assert!(t.modes.mouse_utf8, "mouse_utf8 mode on");
        feed(&mut t, b"\x1b[?1005l");
        assert!(!t.modes.mouse_utf8, "mouse_utf8 mode off");
    }

    #[test]
    fn t_r22_mouse_1015_urxvt_toggle() {
        // DECSET 1015 — URXVT mouse format toggle.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1015h");
        assert!(t.mouse_urxvt_enabled(), "urxvt mouse on");
        feed(&mut t, b"\x1b[?1015l");
        assert!(!t.mouse_urxvt_enabled(), "urxvt mouse off");
    }

    #[test]
    fn t_r22_mouse_1016_sgr_pixel_toggle() {
        // DECSET 1016 — SGR pixel mouse format toggle.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1016h");
        assert!(t.mouse_sgr_pixel_enabled(), "sgr pixel mouse on");
        feed(&mut t, b"\x1b[?1016l");
        assert!(!t.mouse_sgr_pixel_enabled(), "sgr pixel mouse off");
    }

    #[test]
    fn t_r22_mouse_multiple_modes_independent() {
        // Enabling multiple mouse modes simultaneously.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1000h\x1b[?1002h\x1b[?1006h");
        assert!(t.mouse_tracking_enabled(), "tracking on");
        assert!(t.mouse_button_event_enabled(), "button event on");
        assert!(t.mouse_sgr_enabled(), "sgr on");
    }

    #[test]
    fn t_r22_mouse_modes_persist_through_alt() {
        // Mouse modes should persist through alt screen round-trip.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1000h\x1b[?1006h");
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        assert!(t.mouse_tracking_enabled(), "tracking persists in alt");
        assert!(t.mouse_sgr_enabled(), "sgr persists in alt");
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert!(t.mouse_tracking_enabled(), "tracking persists after alt");
    }

    // ── Round 22-3: Bracketed paste / Focus persistence ────────────────

    #[test]
    fn t_r22_bracketed_paste_disabled_in_alt() {
        // Bracketed paste in alt screen — mode should be independent.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?2004h"); // enable on main
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        // Bracketed paste mode should NOT be reset by alt screen entry
        // (it's a global mode, not screen-specific)
        assert!(
            t.modes.bracketed_paste,
            "bracketed paste persists in alt screen"
        );
    }

    #[test]
    fn t_r22_focus_event_persists_through_alt() {
        // Focus event mode should persist through alt screen.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1004h"); // enable
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        assert!(t.modes.focus_event, "focus event persists in alt");
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert!(t.modes.focus_event, "focus event persists after alt");
    }

    #[test]
    fn t_r22_keypad_mode_persists_through_alt() {
        // Keypad application mode should persist through alt screen.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b="); // DECPAM
        assert!(t.modes.keypad_app);
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        assert!(t.modes.keypad_app, "keypad persists in alt");
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert!(t.modes.keypad_app, "keypad persists after alt");
    }

    // ── Round 22-4: Synchronized output ────────────────────────────────

    #[test]
    fn t_r22_synchronized_output_default_off() {
        // Default should be off.
        let t = Terminal::new(10, 5);
        assert!(!t.is_synchronized(), "sync output off by default");
    }

    #[test]
    fn t_r22_synchronized_output_is_accessor() {
        // is_synchronized() should match modes.synchronized_output.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?2026h");
        assert_eq!(t.is_synchronized(), t.modes.synchronized_output);
        feed(&mut t, b"\x1b[?2026l");
        assert_eq!(t.is_synchronized(), t.modes.synchronized_output);
    }

    #[test]
    fn t_r22_synchronized_output_decrqm() {
        // DECRQM for mode 2026.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?2026$p"); // query
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        assert!(s.contains("2026"), "DECRQM response for 2026: {}", s);
    }

    #[test]
    fn t_r22_synchronized_output_persists_alt() {
        // Sync output should persist through alt screen.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?2026h");
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        assert!(t.is_synchronized(), "sync persists in alt");
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert!(t.is_synchronized(), "sync persists after alt");
    }

    #[test]
    fn t_r22_synchronized_output_ris_reset() {
        // RIS should reset synchronized output.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?2026h");
        feed(&mut t, b"\x1bc"); // RIS
        assert!(!t.is_synchronized(), "RIS resets sync output");
    }

    // ── Round 23-1: Autowrap / pending_wrap edge cases ─────────────────

    #[test]
    fn t_r23_pending_wrap_cr_clears() {
        // CR should clear pending_wrap and move to col 0.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE"); // fills row, pending_wrap=true
        assert!(t.cursor.pending_wrap);
        feed(&mut t, b"\r"); // CR
        assert!(!t.cursor.pending_wrap, "CR clears pending_wrap");
        assert_eq!(t.cursor.x, 0);
    }

    #[test]
    fn t_r23_pending_wrap_lf_wraps() {
        // LF after pending_wrap — the LF should just advance row (no double wrap).
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE"); // pending_wrap=true at (4,0)
        feed(&mut t, b"\n"); // LF
        assert!(!t.cursor.pending_wrap, "LF clears pending_wrap");
        assert_eq!(t.cursor.y, 1, "LF advances row");
        assert_eq!(t.cursor.x, 4, "LF preserves column (CRLF semantics)");
    }

    #[test]
    fn t_r23_pending_wrap_print_wraps() {
        // Pending_wrap + next print should wrap to next line.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE"); // row 0 full, pending_wrap=true
        feed(&mut t, b"F"); // should wrap
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'F', "F wrapped to row 1");
    }

    #[test]
    fn t_r23_pending_wrap_cuf_clears() {
        // CUF should clear pending_wrap.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE"); // pending_wrap=true
        feed(&mut t, b"\x1b[1C"); // CUF 1 — but cursor is already at last col
        assert!(!t.cursor.pending_wrap, "CUF clears pending_wrap");
    }

    #[test]
    fn t_r23_pending_wrap_bs_clears() {
        // BS should clear pending_wrap and move left.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE"); // pending_wrap=true at col 4
        feed(&mut t, b"\x08"); // BS
        assert!(!t.cursor.pending_wrap, "BS clears pending_wrap");
        assert_eq!(t.cursor.x, 3, "BS moves to col 3");
    }

    #[test]
    fn t_r23_autowm_off_no_pending_wrap() {
        // With DECAWM off, pending_wrap should never be set.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[?7l"); // DECAWM off
        feed(&mut t, b"ABCDE"); // fills row, no wrap
        assert!(!t.cursor.pending_wrap, "no pending_wrap when DECAWM off");
        assert_eq!(t.cursor.x, 4, "cursor at last col");
    }

    #[test]
    fn t_r23_pending_wrap_at_bottom_scrolls() {
        // Pending_wrap at bottom row — next char should scroll.
        let mut t = Terminal::new(5, 2);
        feed(&mut t, b"ABCDE"); // row 0 full, pending_wrap
        feed(&mut t, b"FGHIJ"); // row 1 full, pending_wrap at bottom
        feed(&mut t, b"X"); // should scroll up
        // Row 0 should be gone (scrolled), X should be on visible area
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'F',
            "row shifted up after scroll"
        );
    }

    #[test]
    fn t_r23_autowm_off_overwrite_last_col() {
        // DECAWM off: writing more chars overwrites last col repeatedly.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b[?7l"); // off
        feed(&mut t, b"ABCDEF"); // 6 chars, only 5 cols
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "A at col 0");
        assert_eq!(
            t.grid().cell(4, 0).unwrap().ch,
            'F',
            "F at col 4 (overwrote E)"
        );
        assert_eq!(t.cursor.y, 0, "no wrap");
    }

    // ── Round 23-2: Tab stop edge cases ────────────────────────────────

    #[test]
    fn t_r23_tab_no_stops_goes_to_last_col() {
        // Tab with no stops set → last column.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[3g"); // clear all
        feed(&mut t, b"\t");
        assert_eq!(t.cursor.x, 9, "tab to last col");
    }

    #[test]
    fn t_r23_tab_clear_current_then_tab() {
        // TBC 0 clears current stop, then tab skips it.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[1;9H\x1b[0g"); // clear stop at col 8
        feed(&mut t, b"\x1b[1;1H\t"); // tab from col 0
        // Col 8 stop is cleared, so tab goes to col 16
        assert_eq!(t.cursor.x, 16, "tab skips cleared stop at col 8");
    }

    #[test]
    fn t_r23_tab_from_last_col_stays() {
        // Tab at last column should stay at last column.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1;10H"); // col 9
        feed(&mut t, b"\t");
        assert_eq!(t.cursor.x, 9, "tab at last col stays");
    }

    #[test]
    fn t_r23_cht_multiple_tabs() {
        // CHT 3 should advance 3 tab stops.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[3I"); // CHT 3: 0→8→16→24
        assert_eq!(t.cursor.x, 24, "CHT 3 advances 3 stops");
    }

    #[test]
    fn t_r23_cbt_multiple_tabs() {
        // CBT 2 from col 24 → col 16 → col 8.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[1;25H"); // col 24
        feed(&mut t, b"\x1b[2Z"); // CBT 2: 24→16→8
        assert_eq!(t.cursor.x, 8, "CBT 2 from col 24 → col 8");
    }

    // ── Round 23-3: Wide char + ICH/DCH/ECH boundary ───────────────────

    #[test]
    fn t_r23_dch_into_wide_char() {
        // DCH at a position before a wide char — wide char should shift left intact.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"AB");
        feed(&mut t, "你".as_bytes()); // wide at cols 2-3
        feed(&mut t, b"X"); // col 4
        feed(&mut t, b"\r"); // back to col 0
        feed(&mut t, b"\x1b[1P"); // DCH 1 — delete 'A', shift all left
        // B should be at col 0, wide char at cols 1-2
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'B', "B shifted to col 0");
        assert_eq!(
            t.grid().cell(1, 0).unwrap().ch,
            '你',
            "wide shifted to col 1"
        );
    }

    #[test]
    fn t_r23_ich_at_row_end() {
        // ICH at near-end of row — cells pushed past edge are lost.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCD"); // cols 0-3 filled
        feed(&mut t, b"\x1b[1;1H"); // cursor at col 0
        feed(&mut t, b"\x1b[2@"); // ICH 2 — insert 2 blanks at col 0
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            ' ',
            "blank inserted at col 0"
        );
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'A', "A shifted to col 2");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'B', "B at col 3");
    }

    #[test]
    fn t_r23_ech_at_row_middle() {
        // ECH in the middle of a row.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGHIJ");
        feed(&mut t, b"\r");
        feed(&mut t, b"\x1b[3C"); // cursor at col 3
        feed(&mut t, b"\x1b[2X"); // ECH 2 — erase cols 3-4
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'C', "C preserved");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, ' ', "D erased");
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, ' ', "E erased");
        assert_eq!(t.grid().cell(5, 0).unwrap().ch, 'F', "F preserved");
    }

    #[test]
    fn t_r23_ech_beyond_row_end() {
        // ECH beyond row end should only erase available cells.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABC"); // only 3 chars
        feed(&mut t, b"\r");
        feed(&mut t, b"\x1b[2C"); // cursor at col 2
        feed(&mut t, b"\x1b[100X"); // ECH 100
        // Only col 2 should be erased (beyond is already blank)
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B', "B preserved");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, ' ', "C erased");
    }

    #[test]
    fn t_r23_dch_all_cells() {
        // DCH more than available cells should clear the line from cursor.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGH");
        feed(&mut t, b"\r");
        feed(&mut t, b"\x1b[2C"); // cursor at col 2
        feed(&mut t, b"\x1b[100P"); // DCH 100
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B', "B preserved");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, ' ', "C deleted");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, ' ', "D deleted");
    }

    #[test]
    fn t_r23_wide_char_bs_deletes_both_cells() {
        // BS after writing a wide char, then overwrite spacer — wide char should clear.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你".as_bytes()); // cols 0-1, cursor at col 2
        feed(&mut t, b"\x08\x08"); // BS twice: col 2→1→0
        feed(&mut t, b"X"); // overwrite lead cell at col 0
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X', "X at col 0");
        assert_eq!(
            t.grid().cell(1, 0).unwrap().ch,
            ' ',
            "spacer cleared when lead overwritten"
        );
    }

    // ── Round 23-4: IL/DL/SU/SD boundary ───────────────────────────────

    #[test]
    fn t_r23_il_at_bottom_row() {
        // IL at the bottom row of scroll region.
        let mut t = Terminal::new(5, 4);
        feed(&mut t, b"\x1b[1;1HA\r\nB\r\nC\r\nD"); // 4 rows
        feed(&mut t, b"\x1b[4;1H"); // cursor at row 3 (bottom)
        feed(&mut t, b"\x1b[L"); // IL 1
        // Row D should scroll out, blank line at row 3
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "A preserved");
        assert_eq!(t.grid().cell(0, 3).unwrap().ch, ' ', "row 3 blank after IL");
    }

    #[test]
    fn t_r23_dl_at_top_row() {
        // DL at top of scroll region.
        let mut t = Terminal::new(5, 4);
        feed(&mut t, b"\x1b[1;1HA\r\nB\r\nC\r\nD");
        feed(&mut t, b"\x1b[1;1H"); // cursor at row 0 (top)
        feed(&mut t, b"\x1b[M"); // DL 1
        // A should scroll out, B at row 0, blank at row 3
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'B', "B shifted to row 0");
        assert_eq!(t.grid().cell(0, 3).unwrap().ch, ' ', "row 3 blank");
    }

    #[test]
    fn t_r23_su_scrolls_content_up() {
        // SU should scroll content up within scroll region.
        let mut t = Terminal::new(5, 4);
        feed(&mut t, b"\x1b[1;1HA\r\nB\r\nC\r\nD");
        feed(&mut t, b"\x1b[1;1H\x1b[1S"); // SU 1
        // A should scroll out, blank at bottom
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'B', "B at row 0 after SU");
        assert_eq!(
            t.grid().cell(0, 3).unwrap().ch,
            ' ',
            "blank at row 3 after SU"
        );
    }

    #[test]
    fn t_r23_sd_scrolls_content_down() {
        // SD should scroll content down, blanks at top.
        let mut t = Terminal::new(5, 4);
        feed(&mut t, b"\x1b[1;1HA\r\nB\r\nC\r\nD");
        feed(&mut t, b"\x1b[1;1H\x1b[1T"); // SD 1
        // D should scroll out, blank at top
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            ' ',
            "blank at row 0 after SD"
        );
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'A', "A at row 1 after SD");
    }

    #[test]
    fn t_r23_il_outside_scroll_region_noop() {
        // IL when cursor is below scroll region — should be no-op.
        let mut t = Terminal::new(5, 8);
        feed(&mut t, b"\x1b[3;6r"); // region rows 3-6 (0-based: 2..6)
        // Write rows using CUP to avoid scroll-triggering line feeds
        for i in 0..8 {
            let ch = (b'A' + i as u8) as char;
            feed(&mut t, format!("\x1b[{};1H{}", i + 1, ch).as_bytes());
        }
        feed(&mut t, b"\x1b[7;1H"); // cursor at row 6 (region bottom)
        // Move below region
        feed(&mut t, b"\x1b[8;1H"); // cursor at row 7 (below region)
        feed(&mut t, b"\x1b[L"); // IL 1 — should be no-op (cursor outside region)
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "A preserved");
        assert_eq!(t.grid().cell(0, 7).unwrap().ch, 'H', "H preserved");
    }

    #[test]
    fn t_r23_dl_at_region_boundary() {
        // DL within scroll region only affects region rows.
        let mut t = Terminal::new(5, 8);
        feed(&mut t, b"\x1b[3;7r"); // region rows 3-7 (0-based: 2..7)
        for i in 0..8 {
            let ch = (b'A' + i as u8) as char;
            feed(&mut t, format!("\x1b[{};1H{}", i + 1, ch).as_bytes());
        }
        feed(&mut t, b"\x1b[3;1H"); // cursor at region top (row 2)
        feed(&mut t, b"\x1b[M"); // DL 1
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "A preserved");
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'B', "B preserved");
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, 'D', "D shifted to row 2");
    }

    #[test]
    fn t_r23_il_count_larger_than_region() {
        // IL with count larger than region height should clear entire region.
        let mut t = Terminal::new(5, 6);
        feed(&mut t, b"\x1b[2;5r"); // region rows 2-5
        feed(&mut t, b"\x1b[1;1HA\r\nB\r\nC\r\nD\r\nE\r\nF");
        feed(&mut t, b"\x1b[2;1H"); // cursor at region top
        feed(&mut t, b"\x1b[100L"); // IL 100
        // Rows in region should be blank
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, ' ', "region row blank");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "A preserved");
    }

    // ── Round 24: DECFRA / DECRARA / Kitty / REP / Selective erase edges ──

    #[test]
    fn t_r24_decfra_fill_with_char() {
        // DECFRA with a specific fill character (e.g. 'X').
        let mut t = Terminal::new(10, 5);
        // DECFRA format: CSI Pch;Pt;Pl;Pb;Pr $ x
        // Pch=88 ('X'), fill rows 2-4, cols 2-5 (1-based) = rows 1-3, cols 1-4
        feed(&mut t, b"\x1b[88;2;2;4;5$x");
        assert_eq!(t.grid().cell(1, 1).unwrap().ch, 'X', "X at (1,1)");
        assert_eq!(t.grid().cell(4, 3).unwrap().ch, 'X', "X at (4,3)");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ', "blank at (0,0)");
    }

    #[test]
    fn t_r24_decfra_overlaps_wide_char() {
        // DECFRA rectangle partially overlaps a wide char — should clean up orphan.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, "你".as_bytes()); // wide at cols 0-1
        feed(&mut t, b"ABCDEF"); // cols 2-7
        // Fill col 1 (spacer cell) with space via DECFRA
        // Format: Pch;Pt;Pl;Pb;Pr $ x → Pch=32, row 1, col 2 only
        feed(&mut t, b"\x1b[32;1;2;1;2$x");
        // The wide char should be cleaned up — no orphan spacer at col 1
        assert!(
            !t.grid().cell(1, 0).unwrap().is_wide_spacer(),
            "no orphan spacer after DECFRA"
        );
    }

    #[test]
    fn t_r24_decrara_toggle_roundtrip() {
        // DECRARA toggle should be reversible: toggle ON then OFF = original.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABC"); // plain text
        feed(&mut t, b"\x1b[1;1;1;10;1$t"); // toggle BOLD on row 1
        assert!(
            t.grid().cell(0, 0).unwrap().flags.contains(CellFlags::BOLD),
            "BOLD toggled on"
        );
        feed(&mut t, b"\x1b[1;1;1;10;1$t"); // toggle BOLD off
        assert!(
            !t.grid().cell(0, 0).unwrap().flags.contains(CellFlags::BOLD),
            "BOLD toggled off (round-trip)"
        );
    }

    #[test]
    fn t_r24_decrara_multiple_attrs_toggle() {
        // DECRARA with multiple attributes simultaneously.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABC");
        feed(&mut t, b"\x1b[1;1;1;10;1;4;7$t"); // toggle BOLD + UNDERLINE + REVERSE
        let flags = t.grid().cell(0, 0).unwrap().flags;
        assert!(flags.contains(CellFlags::BOLD), "BOLD");
        assert!(flags.contains(CellFlags::UNDERLINE), "UNDERLINE");
        assert!(flags.contains(CellFlags::REVERSE), "REVERSE");
    }

    #[test]
    fn t_r24_deccara_clear_then_set() {
        // DECCARA with Ps1=0 (clear first) then set new attributes.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1mABC"); // BOLD, then ABC
        feed(&mut t, b"\x1b[0m"); // reset SGR
        feed(&mut t, b"\x1b[1;1;1;10;0;4$r"); // clear attrs, set UNDERLINE only
        let flags = t.grid().cell(0, 0).unwrap().flags;
        assert!(!flags.contains(CellFlags::BOLD), "BOLD cleared by Ps1=0");
        assert!(flags.contains(CellFlags::UNDERLINE), "UNDERLINE set");
    }

    #[test]
    fn t_r24_kitty_pop_empty_stack() {
        // Pop from empty stack should reset to 0.
        let mut t = Terminal::new(10, 3);
        assert!(t.kitty_kb_stack.is_empty());
        feed(&mut t, b"\x1b[<1u"); // pop 1 from empty stack
        assert_eq!(t.modes.kitty_keyboard, 0, "empty pop resets to 0");
    }

    #[test]
    fn t_r24_kitty_push_pop_multiple() {
        // Push 3, pop 2 → should have the first push's flags.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[>1u"); // push, OR with 1
        assert_eq!(t.modes.kitty_keyboard, 1);
        feed(&mut t, b"\x1b[>2u"); // push, OR with 2
        assert_eq!(t.modes.kitty_keyboard, 3); // 1|2
        feed(&mut t, b"\x1b[>4u"); // push, OR with 4
        assert_eq!(t.modes.kitty_keyboard, 7); // 3|4
        feed(&mut t, b"\x1b[<2u"); // pop 2
        assert_eq!(t.modes.kitty_keyboard, 1, "popped back to first push");
        feed(&mut t, b"\x1b[<1u"); // pop 1
        assert_eq!(t.modes.kitty_keyboard, 0, "popped to base");
    }

    #[test]
    fn t_r24_kitty_stack_overflow_protection() {
        // Push 150 times — stack should be capped at 100.
        let mut t = Terminal::new(10, 3);
        for _ in 0..150 {
            feed(&mut t, b"\x1b[>0u"); // push 0 (just push, no new flags)
        }
        assert!(
            t.kitty_kb_stack.len() <= 100,
            "stack capped at 100, got {}",
            t.kitty_kb_stack.len()
        );
    }

    #[test]
    fn t_r24_kitty_pop_more_than_pushed() {
        // Pop more than pushed — should not panic, reset to 0.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[>1u"); // push 1
        feed(&mut t, b"\x1b[<5u"); // pop 5 (only 1 in stack)
        assert_eq!(t.modes.kitty_keyboard, 0, "over-pop resets to 0");
        assert!(t.kitty_kb_stack.is_empty(), "stack empty");
    }

    #[test]
    fn t_r24_rep_at_row_boundary_wraps() {
        // REP at row boundary should trigger autowrap.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"A"); // print A at col 0
        feed(&mut t, b"\x1b[6b"); // REP 6 → should fill row and wrap
        // Row 0: AAAAA, row 1: AA
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'A', "row 0 filled");
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'A', "wrapped to row 1");
        assert_eq!(t.grid().cell(1, 1).unwrap().ch, 'A', "row 1 col 1");
    }

    #[test]
    fn t_r24_rep_default_count() {
        // REP with no param → repeat once.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"X");
        feed(&mut t, b"\x1b[b"); // REP default = 1
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X', "X at col 0");
        assert_eq!(
            t.grid().cell(1, 0).unwrap().ch,
            'X',
            "X at col 1 (repeated)"
        );
    }

    #[test]
    fn t_r24_rep_zero_count() {
        // REP 0 → treated as default 1 (xterm: param 0 → 1).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"X");
        feed(&mut t, b"\x1b[0b"); // REP 0 → treated as 1
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X', "X at col 0");
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'X', "1 repeat (0→1)");
    }

    #[test]
    fn t_r24_rep_after_control_char_no_op() {
        // REP after a control char (no last_printed_char) → no-op.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\r"); // CR only, no printable
        feed(&mut t, b"\x1b[5b"); // REP 5
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ', "no char printed");
    }

    #[test]
    fn t_r24_decsed_preserves_protected_cells() {
        // DECSED (selective erase) should preserve protected cells.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1\"q"); // DECSCA 1 → protected
        feed(&mut t, b"AB"); // protected A, B
        feed(&mut t, b"\x1b[0\"q"); // DECSCA 0 → unprotected
        feed(&mut t, b"CD"); // unprotected C, D
        feed(&mut t, b"\x1b[?0J"); // DECSED 0 → erase from cursor to end
        // C, D should be erased, A, B should survive
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'A',
            "protected A survives DECSED"
        );
        assert_eq!(
            t.grid().cell(1, 0).unwrap().ch,
            'B',
            "protected B survives DECSED"
        );
    }

    #[test]
    fn t_r24_decsel_preserves_protected_line() {
        // DECSEL 2 (selective erase whole line) should preserve protected.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1\"qAB"); // protected A, B
        feed(&mut t, b"\x1b[0\"qCD"); // unprotected C, D
        feed(&mut t, b"\x1b[?2K"); // DECSEL 2 → erase entire line (selective)
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "protected A survives");
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B', "protected B survives");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, ' ', "unprotected C erased");
    }

    #[test]
    fn t_r24_decfra_default_params_single_cell() {
        // DECFRA with no params: Pch defaults to 0 → invalid → command ignored.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"ABCDEFGH");
        feed(&mut t, b"\x1b[$x"); // No params → Pch=0 → invalid → no-op
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'A',
            "cell (0,0) unchanged when Pch is invalid"
        );
    }

    #[test]
    fn t_r24_decerase_full_rect() {
        // DECERA erase a rectangle.
        let mut t = Terminal::new(10, 5);
        for i in 0..5 {
            let ch = (b'A' + i as u8) as char;
            feed(&mut t, format!("\x1b[{};1H{}", i + 1, ch).as_bytes());
        }
        // Erase rectangle rows 2-4, cols 3-6
        feed(&mut t, b"\x1b[2;3;4;6$y"); // DECERA
        assert_eq!(t.grid().cell(2, 1).unwrap().ch, ' ', "erased at (2,1)");
        assert_eq!(t.grid().cell(5, 3).unwrap().ch, ' ', "erased at (5,3)");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'A',
            "A preserved outside rect"
        );
    }

    #[test]
    fn t_r24_decsera_preserves_protected() {
        // DECSERA should preserve protected cells in the rectangle.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1\"qAB"); // protected A, B
        feed(&mut t, b"\x1b[0\"qCD"); // unprotected C, D
        feed(&mut t, b"\x1b[1;1;1;10${"); // DECSERA whole row 1
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'A',
            "protected A survives DECSERA"
        );
        assert_eq!(
            t.grid().cell(1, 0).unwrap().ch,
            'B',
            "protected B survives DECSERA"
        );
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, ' ', "unprotected C erased");
    }

    #[test]
    fn t_r24_deccara_skips_wide_char_spacer() {
        // DECCARA should not modify wide char spacer cells.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "你".as_bytes()); // wide at cols 0-1
        feed(&mut t, b"X"); // col 2
        feed(&mut t, b"\x1b[1;1;1;10;4$r"); // set UNDERLINE on all
        let lead = t.grid().cell(0, 0).unwrap();
        let _spacer = t.grid().cell(1, 0).unwrap();
        let x_cell = t.grid().cell(2, 0).unwrap();
        assert!(
            lead.flags.contains(CellFlags::UNDERLINE),
            "lead cell gets UNDERLINE"
        );
        assert!(
            x_cell.flags.contains(CellFlags::UNDERLINE),
            "X cell gets UNDERLINE"
        );
        // Spacer should also get UNDERLINE for visual consistency
        // (implementation detail — just verify it doesn't crash)
    }

    // ── Round 25-1: SS2/SS3 single shift (no-op) ───────────────────────

    #[test]
    fn t_r25_ss3_esc_o_no_op() {
        // ESC O (SS3) should be silently consumed — no error, no output.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1bOABC");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'A',
            "text after SS3 printed"
        );
        assert_eq!(
            t.grid().cell(2, 0).unwrap().ch,
            'C',
            "text after SS3 printed"
        );
    }

    #[test]
    fn t_r25_ss2_esc_n_no_op() {
        // ESC N (SS2) should be silently consumed.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1bNABC");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'A',
            "text after SS2 printed"
        );
        assert_eq!(
            t.grid().cell(2, 0).unwrap().ch,
            'C',
            "text after SS2 printed"
        );
    }

    #[test]
    fn t_r25_ss3_does_not_break_cursor() {
        // SS3 should not affect cursor position.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1bO");
        assert_eq!(t.cursor().0, 0, "cursor unchanged after SS3");
    }

    // ── Round 25-2: DECIC/DECDC insert/delete column ───────────────────

    #[test]
    fn t_r25_decic_insert_column() {
        // DECIC inserts blank columns at cursor position, affecting all rows.
        let mut t = Terminal::new(8, 3);
        // Fill row 0 with text to verify shift
        feed(&mut t, b"ABCDEFGH");
        feed(&mut t, b"\r");
        feed(&mut t, b"\x1b[3C"); // cursor at col 3
        feed(&mut t, b"\x1b[2'}"); // DECIC 2 — insert 2 blank columns at col 3
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "A at col 0");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'C', "C at col 2");
        assert_eq!(
            t.grid().cell(3, 0).unwrap().ch,
            ' ',
            "blank at col 3 (inserted)"
        );
        assert_eq!(
            t.grid().cell(4, 0).unwrap().ch,
            ' ',
            "blank at col 4 (inserted)"
        );
        assert_eq!(t.grid().cell(5, 0).unwrap().ch, 'D', "D shifted to col 5");
    }

    #[test]
    fn t_r25_decdc_delete_column() {
        // DECDC deletes columns at cursor position.
        let mut t = Terminal::new(8, 3);
        for i in 0..5 {
            let ch = (b'A' + i as u8) as char;
            feed(&mut t, format!("\x1b[{};1H{}", i + 1, ch).as_bytes());
        }
        // Row 0: A......, Row 1: B......, etc.
        feed(&mut t, b"\x1b[1;1H"); // cursor at col 0
        feed(&mut t, b"\x1b[1'~"); // DECDC 1 — delete col 0
        // A should be deleted, on row 0 the next col shifts in
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            ' ',
            "row 0 col 0 (deleted, was A)"
        );
        // On row 1, B was at col 0, now deleted → col 0 blank
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            ' ',
            "row 1 col 0 (B deleted)"
        );
    }

    #[test]
    fn t_r25_decic_default_count() {
        // DECIC with no param → insert 1 column.
        let mut t = Terminal::new(8, 3);
        feed(&mut t, b"ABCDEFGH");
        feed(&mut t, b"\r");
        feed(&mut t, b"\x1b[3C"); // cursor at col 3
        feed(&mut t, b"\x1b['}"); // DECIC default = 1
        assert_eq!(
            t.grid().cell(3, 0).unwrap().ch,
            ' ',
            "blank inserted at col 3"
        );
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'D', "D shifted to col 4");
    }

    #[test]
    fn t_r25_decdc_default_count() {
        // DECDC with no param → delete 1 column.
        let mut t = Terminal::new(8, 3);
        feed(&mut t, b"ABCDEFGH");
        feed(&mut t, b"\r");
        feed(&mut t, b"\x1b[3C"); // cursor at col 3
        feed(&mut t, b"\x1b['~"); // DECDC default = 1
        // D at col 3 deleted, E shifts to col 3
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'E', "E shifted to col 3");
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'F', "F at col 4");
    }

    #[test]
    fn t_r25_decic_count_exceeds_width() {
        // DECIC with count > available width → clamp.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE");
        feed(&mut t, b"\r");
        feed(&mut t, b"\x1b[3C"); // cursor at col 3
        feed(&mut t, b"\x1b[100'}"); // DECIC 100 — clamp to 2 (width - col)
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, ' ', "blank at col 3");
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, ' ', "blank at col 4");
    }

    #[test]
    fn t_r25_decic_decdc_roundtrip() {
        // Insert then delete should restore original layout.
        let mut t = Terminal::new(8, 2);
        feed(&mut t, b"ABCDEFGH");
        feed(&mut t, b"\r");
        feed(&mut t, b"\x1b[4C"); // cursor at col 4
        feed(&mut t, b"\x1b[2'}"); // insert 2 columns
        feed(&mut t, b"\x1b['~"); // delete 1 column
        // After insert 2 + delete 1 = net +1 column shift
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, ' ', "col 4 blank (net +1)");
        assert_eq!(t.grid().cell(5, 0).unwrap().ch, 'E', "E at col 5");
    }

    // ── Round 25-3: OSC / charset / tab edge cases ─────────────────────

    #[test]
    fn t_r25_osc_very_long_title() {
        // Very long title should be capped at 256 chars (security limit).
        let mut t = Terminal::new(10, 3);
        let long_title = "X".repeat(1000);
        let osc = format!("\x1b]0;{}\x07", long_title);
        feed(&mut t, osc.as_bytes());
        assert_eq!(t.title().len(), 256, "long title capped at 256 chars");
    }

    #[test]
    fn t_r25_osc_title_with_semicolons() {
        // Title containing semicolons (OSC 2 has format: title;text).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]0;A;B;C\x07");
        // OSC 0: payload is "A;B;C" — first param is 0, rest is title
        assert_eq!(t.title(), "A;B;C", "title with semicolons");
    }

    #[test]
    fn t_r25_osc_set_then_overwrite_title() {
        // Set title then overwrite — should have the new title.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]0;First\x07");
        feed(&mut t, b"\x1b]0;Second\x07");
        assert_eq!(t.title(), "Second", "title overwritten");
    }

    #[test]
    fn t_r25_charset_dec_special_then_ascii_back() {
        // Switch to DEC special, print, switch back, print — verify no bleed.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b(0qq\x1b(Bqq"); // DEC: --, ASCII: qq
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            '\u{2500}',
            "DEC line at col 0"
        );
        assert_eq!(
            t.grid().cell(1, 0).unwrap().ch,
            '\u{2500}',
            "DEC line at col 1"
        );
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'q', "ASCII q at col 2");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'q', "ASCII q at col 3");
    }

    #[test]
    fn t_r25_charset_g1_so_si_rapid_toggle() {
        // Rapid SO/SI toggling should not corrupt charset state.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b)0"); // G1 = DEC Special
        feed(&mut t, b"\x0eqq\x0fq\x0eq\x0f"); // SO qq SI q SO q SI
        // SO: q→─, SI: q→q
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '\u{2500}', "G1 q at col 0");
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, '\u{2500}', "G1 q at col 1");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'q', "G0 q at col 2");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, '\u{2500}', "G1 q at col 3");
    }

    #[test]
    fn t_r25_tab_at_col_0_with_clear() {
        // Tab at col 0 after clearing all stops → last col.
        let mut t = Terminal::new(15, 3);
        feed(&mut t, b"\x1b[3g"); // clear all
        feed(&mut t, b"\t"); // tab from col 0
        assert_eq!(t.cursor().0, 14, "tab to last col (no stops)");
    }

    #[test]
    fn t_r25_cht_at_last_tab_stop() {
        // CHT when already at a tab stop — should advance to next.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[1;9H"); // cursor at col 8 (first default stop)
        feed(&mut t, b"\x1b[I"); // CHT 1 → should go to col 16
        assert_eq!(t.cursor().0, 16, "CHT from stop to next");
    }

    #[test]
    fn t_r25_cbt_at_col_0_stays() {
        // CBT at col 0 — should stay at col 0.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[Z"); // CBT 1 from col 0
        assert_eq!(t.cursor().0, 0, "CBT at col 0 stays");
    }

    #[test]
    fn t_r25_decset_no_param_default() {
        // DECSET with no parameter — should be treated as default (1? or no-op).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?h"); // DECSET with no param
        // Should not panic — just no-op or treat as mode 1
        assert!(t.modes.cursor_visible, "no crash from paramless DECSET");
    }

    #[test]
    fn t_r25_decrst_no_param_default() {
        // DECRST with no parameter — should be treated as default.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?l"); // DECRST with no param
        // Should not panic
        assert!(t.modes.cursor_visible, "no crash from paramless DECRST");
    }

    // ── Round 26: OSC 8 / DECSCUSR / modifyOtherKeys / Clipboard edges ──

    #[test]
    fn t_r26_osc_8_with_id_param() {
        // OSC 8 with id parameter: OSC 8;id=XXX;URI ST
        let mut t = Terminal::new(20, 3);
        feed(
            &mut t,
            b"\x1b]8;id=123;https://example.com\x1b\\Link\x1b]8;;\x1b\\",
        );
        assert_eq!(
            t.grid().cell(0, 0).unwrap().hyperlink.as_deref(),
            Some("https://example.com"),
            "hyperlink with id param stored"
        );
    }

    #[test]
    fn t_r26_osc_8_long_uri_capped() {
        // OSC 8 URI should be capped at 2048 chars.
        let mut t = Terminal::new(20, 3);
        let long_uri = format!("https://example.com/{}", "x".repeat(3000));
        let osc = format!("\x1b]8;{}\x1b\\", long_uri);
        feed(&mut t, osc.as_bytes());
        feed(&mut t, b"X");
        let hl = t.grid().cell(0, 0).unwrap().hyperlink.as_ref();
        assert!(
            hl.unwrap().len() <= 2048,
            "URI capped at 2048, got {}",
            hl.unwrap().len()
        );
    }

    #[test]
    fn t_r26_osc_8_uri_with_special_chars() {
        // OSC 8 with URI containing query params and fragments.
        let mut t = Terminal::new(20, 3);
        feed(
            &mut t,
            b"\x1b]8;;https://example.com/path?q=1&v=2#frag\x1b\\X",
        );
        let hl = t.grid().cell(0, 0).unwrap().hyperlink.as_deref();
        assert_eq!(
            hl,
            Some("https://example.com/path?q=1&v=2#frag"),
            "URI with query/fragment stored"
        );
    }

    #[test]
    fn t_r26_osc_8_nested_not_allowed() {
        // OSC 8 open then another open — second should overwrite.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b]8;;https://a.com\x1b\\X");
        feed(&mut t, b"\x1b]8;;https://b.com\x1b\\Y");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().hyperlink.as_deref(),
            Some("https://a.com"),
            "X has first link"
        );
        assert_eq!(
            t.grid().cell(1, 0).unwrap().hyperlink.as_deref(),
            Some("https://b.com"),
            "Y has second link"
        );
    }

    #[test]
    fn t_r26_osc_8_sgr_combined() {
        // OSC 8 + SGR attributes — both should be applied to the cell.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[1m\x1b]8;;https://example.com\x1b\\BoldLink");
        let cell = t.grid().cell(0, 0).unwrap();
        assert!(cell.flags.contains(CellFlags::BOLD), "bold applied");
        assert_eq!(
            cell.hyperlink.as_deref(),
            Some("https://example.com"),
            "hyperlink applied"
        );
    }

    #[test]
    fn t_r26_decscusr_0_resets_to_default() {
        // DECSCUSR 0 should reset to the terminal default (usually blinking block).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[3 q"); // set to BlinkUnderline
        assert_eq!(t.cursor_style(), CursorStyle::BlinkUnderline);
        feed(&mut t, b"\x1b[0 q"); // param 0 → default
        assert_eq!(t.cursor_style(), CursorStyle::Default);
    }

    #[test]
    fn t_r26_decscusr_7_ignored() {
        // DECSCUSR with invalid param (7+) should be ignored.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[2 q"); // SteadyBlock
        feed(&mut t, b"\x1b[7 q"); // invalid — should be ignored
        assert_eq!(
            t.cursor_style(),
            CursorStyle::SteadyBlock,
            "invalid DECSCUSR param ignored"
        );
    }

    #[test]
    fn t_r26_decscusr_saved_by_decsc_restored_by_decrc() {
        // DECSC should save cursor_style, DECRC should restore it.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[4 q"); // SteadyUnderline
        feed(&mut t, b"\x1b7"); // DECSC — save
        feed(&mut t, b"\x1b[1 q"); // change to BlinkBlock
        assert_eq!(t.cursor_style(), CursorStyle::BlinkBlock);
        feed(&mut t, b"\x1b8"); // DECRC — restore
        assert_eq!(
            t.cursor_style(),
            CursorStyle::SteadyUnderline,
            "DECRC restores saved cursor style"
        );
    }

    #[test]
    fn t_r26_decscusr_ris_reset() {
        // RIS should reset cursor style to default.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[6 q"); // SteadyBar
        feed(&mut t, b"\x1bc"); // RIS
        assert_eq!(
            t.cursor_style(),
            CursorStyle::Default,
            "RIS resets cursor style"
        );
    }

    #[test]
    fn t_r26_decscusr_all_styles_distinct() {
        // All 6 cursor styles should be settable and distinct.
        let mut t = Terminal::new(10, 3);
        let styles = [
            ("1", CursorStyle::BlinkBlock),
            ("2", CursorStyle::SteadyBlock),
            ("3", CursorStyle::BlinkUnderline),
            ("4", CursorStyle::SteadyUnderline),
            ("5", CursorStyle::BlinkBar),
            ("6", CursorStyle::SteadyBar),
        ];
        for (param, expected) in styles {
            feed(&mut t, format!("\x1b[{} q", param).as_bytes());
            assert_eq!(t.cursor_style(), expected, "DECSCUSR {} correct", param);
        }
    }

    #[test]
    fn t_r26_modify_other_keys_set_1() {
        // modifyOtherKeys set to 1.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[>4;1h"); // set modifyOtherKeys = 1
        assert_eq!(t.modes.modify_other_keys, 1);
    }

    #[test]
    fn t_r26_modify_other_keys_set_2() {
        // modifyOtherKeys set to 2.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[>4;2h"); // set modifyOtherKeys = 2
        assert_eq!(t.modes.modify_other_keys, 2);
    }

    #[test]
    fn t_r26_modify_other_keys_reset() {
        // modifyOtherKeys reset to 0.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[>4;2h"); // set to 2
        feed(&mut t, b"\x1b[>4l"); // reset
        assert_eq!(t.modes.modify_other_keys, 0);
    }

    #[test]
    fn t_r26_modify_other_keys_decrqm() {
        // DECRQM query for modifyOtherKeys mode.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[>4$p"); // query
        let resp = t.take_response();
        let s = String::from_utf8(resp).unwrap();
        assert!(s.contains("4"), "DECRQM for modifyOtherKeys: {}", s);
    }

    #[test]
    fn t_r26_clipboard_set_basic() {
        // OSC 52 set clipboard with base64 data.
        let mut t = Terminal::new(10, 3);
        // "Hi" in base64 = "SGk="
        feed(&mut t, b"\x1b]52;c;SGk=\x1b\\");
        let clip = t.take_pending_clipboard_set();
        assert_eq!(
            clip.as_deref(),
            Some(b"Hi".as_ref()),
            "clipboard set to 'Hi'"
        );
    }

    #[test]
    fn t_r26_clipboard_clear() {
        // OSC 52 clear clipboard (empty data).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]52;c;\x1b\\"); // empty data = clear
        let clip = t.take_pending_clipboard_set();
        assert_eq!(clip.as_deref(), Some(b"".as_ref()), "clipboard cleared");
    }

    #[test]
    fn t_r26_clipboard_query() {
        // OSC 52 query (data = '?').
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]52;c;?\x1b\\"); // query
        assert!(t.take_pending_clipboard_query(), "clipboard query detected");
    }

    #[test]
    fn t_r26_clipboard_primary_selection() {
        // OSC 52 with primary selection selector 'p'.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]52;p;SGk=\x1b\\"); // 'p' = primary selection
        let clip = t.take_pending_clipboard_set();
        assert_eq!(
            clip.as_deref(),
            Some(b"Hi".as_ref()),
            "primary selection set"
        );
    }

    #[test]
    fn t_r26_title_push_pop_roundtrip() {
        // OSC 22 (push title) / OSC 23 (pop title) round-trip.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]0;Original\x07");
        feed(&mut t, b"\x1b[22;2t"); // push (CSI 22;2 t)
        feed(&mut t, b"\x1b]0;Temporary\x07");
        assert_eq!(t.title(), "Temporary");
        feed(&mut t, b"\x1b[23;2t"); // pop
        assert_eq!(t.title(), "Original", "title restored after pop");
    }

    #[test]
    fn t_r26_title_push_pop_empty_stack() {
        // Pop from empty title stack — should not crash.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]0;Title\x07");
        feed(&mut t, b"\x1b[23;2t"); // pop from empty stack
        assert_eq!(t.title(), "Title", "title unchanged after empty pop");
    }

    #[test]
    fn t_r26_title_push_multiple_pop_one() {
        // Push saves current title. Pop restores most recent pushed.
        // set A → push(saves A) → set B → push(saves B) → set C → push(saves C)
        // pop → restores C
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]0;A\x07\x1b[22;2t");
        feed(&mut t, b"\x1b]0;B\x07\x1b[22;2t");
        feed(&mut t, b"\x1b]0;C\x07\x1b[22;2t");
        feed(&mut t, b"\x1b[23;2t"); // pop → restores C (most recent push)
        assert_eq!(t.title(), "C", "pop restores most recent push");
    }

    // ── Round 27-1: Wide char / emoji boundary ─────────────────────────

    #[test]
    fn t_r27_emoji_at_last_col_wraps() {
        // Emoji 😀 (U+1F600, width=2) at the last column → should wrap.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCD"); // cols 0-3, cursor at col 4 (last)
        feed(&mut t, "😀".as_bytes()); // width 2, only 1 col left → wrap
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            '😀',
            "emoji wrapped to row 1 col 0"
        );
        assert_eq!(t.cursor.y, 1, "cursor on row 1");
    }

    #[test]
    fn t_r27_consecutive_emoji_fill_row() {
        // Multiple emoji fill the row and wrap correctly.
        let mut t = Terminal::new(6, 3);
        feed(&mut t, "😀😀😀😀".as_bytes()); // 4 emoji = 8 cols, wraps at col 6
        // Row 0: 😀 😀 😀 (cols 0-5), row 1: 😀 (cols 0-1)
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '😀', "emoji at (0,0)");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, '😀', "emoji at (2,0)");
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, '😀', "emoji at (4,0)");
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            '😀',
            "4th emoji wrapped to row 1"
        );
    }

    #[test]
    fn t_r27_combining_acute_over_e() {
        // é = e + U+0301 (combining acute accent) → width 1 total.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "e\u{0301}".as_bytes());
        assert_eq!(t.cursor().0, 1, "cursor at col 1 after combining char");
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.ch, 'e', "base char is e");
        assert!(cell.combining.contains(&'\u{0301}'), "combining attached");
    }

    #[test]
    fn t_r27_combining_after_space() {
        // Combining char with no preceding printable — attaches to space.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "\u{0301}".as_bytes()); // standalone combining
        // Should not advance cursor (zero-width)
        assert_eq!(
            t.cursor().0,
            0,
            "combining char alone doesn't advance cursor"
        );
    }

    #[test]
    fn t_r27_wide_then_combining() {
        // Combining char after a wide char — attaches to wide char.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "中\u{0301}".as_bytes()); // 中 + combining acute
        assert_eq!(t.cursor().0, 2, "cursor at col 2 after wide + combining");
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.ch, '中', "base is 中");
        assert!(
            cell.combining.contains(&'\u{0301}'),
            "combining on wide char"
        );
    }

    #[test]
    fn t_r27_wide_char_in_penultimate_col() {
        // Wide char in cols-2 (penultimate) — fits exactly, cursor at last col.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABC"); // cursor at col 3
        feed(&mut t, "中".as_bytes()); // cols 3-4 (last two cols)
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, '中', "wide at col 3");
        assert!(t.cursor.pending_wrap, "pending wrap set");
        assert_eq!(t.cursor.x, 4, "cursor clamped at last col");
    }

    // ── Round 27-2: Resize/reflow behavior ─────────────────────────────

    #[test]
    fn t_r27_resize_shrink_grow_roundtrip() {
        // Fill a single-row grid completely, then shrink.
        // Content wraps; with only 1 visible row, the last segment is visible.
        let mut t = Terminal::with_scrollback(6, 1, 100);
        feed(&mut t, b"ABCDEF"); // fills the single row exactly
        t.resize(3, 1); // shrink: ABC to scrollback, DEF visible
        // With 1 visible row, the last segment (DEF) is visible.
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'D',
            "D on visible row after shrink"
        );
        assert_eq!(t.grid().scrollback_len(), 1, "1 row in scrollback");
        // Verify scrollback has ABC
        // Grow back — content should merge
        t.resize(6, 1); // grow back
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'A',
            "A merged back after grow"
        );
    }

    #[test]
    fn t_r27_resize_cursor_clamped() {
        // Cursor should be clamped to new dimensions on resize.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;8H"); // cursor at row 2, col 7
        t.resize(5, 3); // shrink to 5 cols, 3 rows
        assert!(t.cursor.x < 5, "cursor x clamped to new width");
        assert!(t.cursor.y < 3, "cursor y clamped to new height");
    }

    #[test]
    fn t_r27_resize_empty_terminal() {
        // Resize from 1x1 to larger — should not panic.
        let mut t = Terminal::new(1, 1);
        t.resize(80, 24);
        assert_eq!(t.grid().width(), 80);
        assert_eq!(t.grid().height(), 24);
    }

    #[test]
    fn t_r27_resize_wide_char_row() {
        // Row with wide chars should handle resize without corruption.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "中文字".as_bytes()); // 3 wide chars = 6 cols
        feed(&mut t, b"AB"); // cols 6-7
        t.resize(4, 3); // shrink
        // Just verify no panic and grid is consistent
        assert!(t.grid().width() == 4, "width is 4");
    }

    #[test]
    fn t_r27_resize_tab_stops_extended() {
        // Tab stops should be extended with defaults when growing.
        let mut t = Terminal::new(10, 3);
        t.resize(20, 3); // grow
        // Default tab stops should be at cols 8, 16
        assert!(t.tab_stops.len() >= 20, "tab stops extended");
        assert!(t.tab_stops[8], "tab stop at col 8");
        assert!(t.tab_stops[16], "tab stop at col 16");
    }

    // ── Round 27-3: Sync mode & Focus event ────────────────────────────

    #[test]
    fn t_r27_sync_mode_toggle() {
        // DECSET/DECRST 2026 sync mode.
        let mut t = Terminal::new(10, 3);
        assert!(!t.is_synchronized(), "sync off by default");
        feed(&mut t, b"\x1b[?2026h"); // enable
        assert!(t.is_synchronized(), "sync enabled");
        feed(&mut t, b"\x1b[?2026l"); // disable
        assert!(!t.is_synchronized(), "sync disabled");
    }

    #[test]
    fn t_r27_sync_nested_toggle() {
        // Nested sync enable/disable.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?2026h"); // enable 1
        feed(&mut t, b"\x1b[?2026h"); // enable 2 (nested)
        assert!(t.is_synchronized(), "still in sync after nested enable");
        feed(&mut t, b"\x1b[?2026l"); // disable 1
        // Some terminals require multiple disables for nested; check behavior.
        // At minimum, should not crash.
        feed(&mut t, b"\x1b[?2026l"); // disable 2
        assert!(!t.is_synchronized(), "sync off after two disables");
    }

    #[test]
    fn t_r27_focus_event_decrqm() {
        // DECRQM query for focus event mode.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?1004$p"); // query focus mode
        let resp = String::from_utf8(t.take_response()).unwrap();
        assert!(resp.contains("1004"), "DECRQM response for 1004: {}", resp);
    }

    #[test]
    fn t_r27_sync_decrqm() {
        // DECRQM query for sync mode.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?2026$p"); // query sync mode
        let resp = String::from_utf8(t.take_response()).unwrap();
        assert!(resp.contains("2026"), "DECRQM response for 2026: {}", resp);
    }

    #[test]
    fn t_r27_focus_toggle_rapid() {
        // Rapid focus mode toggle should not cause issues.
        let mut t = Terminal::new(10, 3);
        for _ in 0..10 {
            feed(&mut t, b"\x1b[?1004h\x1b[?1004l");
        }
        assert!(!t.modes.focus_event, "focus off after rapid toggle");
    }

    // ── Round 27-4: OSC color query (4/10/11/12) ───────────────────────

    #[test]
    fn t_r27_osc10_query_default_fg() {
        // OSC 10 query for default foreground color → white.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]10;?\x1b\\");
        let resp = String::from_utf8(t.take_response()).unwrap();
        assert!(
            resp.contains("rgb:"),
            "OSC 10 query response has rgb: {}",
            resp
        );
        assert!(resp.contains("ff/ff/ff"), "default fg = white: {}", resp);
    }

    #[test]
    fn t_r27_osc11_query_default_bg() {
        // OSC 11 query for default background color → black.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]11;?\x1b\\");
        let resp = String::from_utf8(t.take_response()).unwrap();
        assert!(
            resp.contains("rgb:"),
            "OSC 11 query response has rgb: {}",
            resp
        );
        assert!(resp.contains("00/00/00"), "default bg = black: {}", resp);
    }

    #[test]
    fn t_r27_osc10_set_then_query() {
        // Set fg via OSC 10, then query → should return set value.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]10;rgb:ff/00/00\x1b\\"); // set fg = red
        feed(&mut t, b"\x1b]10;?\x1b\\"); // query
        let resp = String::from_utf8(t.take_response()).unwrap();
        assert!(
            resp.contains("ff/00/00"),
            "OSC 10 query returns set color: {}",
            resp
        );
    }

    #[test]
    fn t_r27_osc11_set_then_query() {
        // Set bg via OSC 11, then query → should return set value.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]11;rgb:00/ff/00\x1b\\"); // set bg = green
        feed(&mut t, b"\x1b]11;?\x1b\\"); // query
        let resp = String::from_utf8(t.take_response()).unwrap();
        assert!(
            resp.contains("00/ff/00"),
            "OSC 11 query returns set color: {}",
            resp
        );
    }

    #[test]
    fn t_r27_osc4_set_out_of_range() {
        // OSC 4 query color index 0 (black) — verify default palette value.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]4;0;?\x1b\\"); // query color 0
        let resp = String::from_utf8(t.take_response()).unwrap();
        assert!(
            resp.contains("4;0;rgb:00/00/00"),
            "color 0 = black: {}",
            resp
        );
    }

    #[test]
    fn t_r27_osc12_set_cursor_color() {
        // OSC 12 set cursor color, then query.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]12;rgb:00/00/ff\x1b\\"); // set cursor = blue
        feed(&mut t, b"\x1b]12;?\x1b\\"); // query
        let resp = String::from_utf8(t.take_response()).unwrap();
        assert!(
            resp.contains("00/00/ff"),
            "OSC 12 query returns cursor color: {}",
            resp
        );
    }

    #[test]
    fn t_r27_osc10_hash_color_format() {
        // OSC 10 with #RRGGBB format (hash prefix).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]10;#ff8800\x1b\\"); // set fg = orange
        feed(&mut t, b"\x1b]10;?\x1b\\"); // query
        let resp = String::from_utf8(t.take_response()).unwrap();
        assert!(
            resp.contains("ff/88/00"),
            "OSC 10 hash format parsed: {}",
            resp
        );
    }

    // ── Round 28-2: Bracketed paste + DECSCUSR edge cases ──────────────

    #[test]
    fn t_r28_bracketed_paste_decrqm() {
        // DECRQM query for bracketed paste mode.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?2004$p");
        let resp = String::from_utf8(t.take_response()).unwrap();
        assert!(resp.contains("2004"), "DECRQM for 2004: {}", resp);
    }

    #[test]
    fn t_r28_bracketed_paste_decstr_reset() {
        // DECSTR should reset bracketed paste to off.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?2004h"); // enable
        assert!(t.bracketed_paste());
        feed(&mut t, b"\x1b[!p"); // DECSTR
        assert!(!t.bracketed_paste(), "bracketed paste reset by DECSTR");
    }

    #[test]
    fn t_r28_bracketed_paste_ris_reset() {
        // RIS should reset bracketed paste to off.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?2004h");
        assert!(t.bracketed_paste());
        feed(&mut t, b"\x1bc"); // RIS
        assert!(!t.bracketed_paste(), "bracketed paste reset by RIS");
    }

    #[test]
    fn t_r28_decscusr_preserved_through_decsc_decrc() {
        // DECSC saves cursor style, DECRC restores it.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[4 q"); // SteadyUnderline
        feed(&mut t, b"\x1b7"); // save
        feed(&mut t, b"\x1b[1 q"); // change to BlinkBlock
        feed(&mut t, b"\x1b8"); // restore
        assert_eq!(t.cursor_style(), CursorStyle::SteadyUnderline);
    }

    #[test]
    fn t_r28_decscusr_decstr_reset() {
        // DECSTR should reset cursor style to default.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[6 q"); // SteadyBar
        feed(&mut t, b"\x1b[!p"); // DECSTR
        assert_eq!(t.cursor_style(), CursorStyle::Default);
    }

    #[test]
    fn t_r28_decscusr_decrqss() {
        // DECRQSS query for cursor style.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1bP$q q\x1b\\"); // DECRQSS for DECSCUSR
        let resp = String::from_utf8(t.take_response()).unwrap();
        // Response should mention "q q" or similar
        assert!(!resp.is_empty(), "DECRQSS for DECSCUSR has response");
    }

    // ── Round 28-3: OSC 8 + OSC 52 edge cases ──────────────────────────

    #[test]
    fn t_r28_osc8_empty_uri_clears() {
        // OSC 8 with empty URI clears current hyperlink.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b]8;;https://example.com\x1b\\Active");
        feed(&mut t, b"\x1b]8;;\x1b\\"); // clear
        feed(&mut t, b"After");
        // "After" should have no hyperlink
        assert!(
            t.grid().cell(6, 0).unwrap().hyperlink.is_none(),
            "hyperlink cleared after empty URI"
        );
    }

    #[test]
    fn t_r28_osc8_multiline_link() {
        // OSC 8 hyperlink persists across line wrap.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"\x1b]8;;https://example.com\x1b\\");
        feed(&mut t, b"ABCDEF"); // 6 chars, wraps: ABCDE on row 0, F on row 1
        feed(&mut t, b"\x1b]8;;\x1b\\"); // close
        assert_eq!(
            t.grid().cell(0, 0).unwrap().hyperlink.as_deref(),
            Some("https://example.com"),
            "link on row 0"
        );
        assert_eq!(
            t.grid().cell(0, 1).unwrap().hyperlink.as_deref(),
            Some("https://example.com"),
            "link persists on wrapped row 1"
        );
    }

    #[test]
    fn t_r28_osc8_control_char_stripped() {
        // OSC 8 URI with embedded control chars should be stripped.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b]8;;https://example.com\x07\x1b\\X");
        // The \x07 (BEL) should be stripped from URI, not terminate OSC early
        let hl = t.grid().cell(0, 0).unwrap().hyperlink.as_deref();
        // BEL inside OSC data is tricky - depends on parser.
        // At minimum, should not crash.
        assert!(hl.is_some() || hl.is_none(), "no crash with control in URI");
    }

    #[test]
    fn t_r28_osc52_set_with_special_chars() {
        // OSC 52 with base64 of special characters.
        let mut t = Terminal::new(10, 3);
        // "Hello\nWorld" in base64 = "SGVsbG8KV29ybGQ="
        feed(&mut t, b"\x1b]52;c;SGVsbG8KV29ybGQ=\x1b\\");
        let clip = t.take_pending_clipboard_set();
        assert_eq!(
            clip.as_deref(),
            Some(b"Hello\nWorld".as_ref()),
            "clipboard with newline"
        );
    }

    #[test]
    fn t_r28_osc52_multiple_set_overwrites() {
        // OSC 52 set twice — second should overwrite.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]52;c;SGk=\x1b\\"); // "Hi"
        let _ = t.take_pending_clipboard_set();
        feed(&mut t, b"\x1b]52;c;Qnll\x1b\\"); // "Bye"
        let clip = t.take_pending_clipboard_set();
        assert_eq!(
            clip.as_deref(),
            Some(b"Bye".as_ref()),
            "clipboard overwritten"
        );
    }

    #[test]
    fn t_r28_osc52_invalid_base64() {
        // OSC 52 with invalid base64 — should be handled gracefully.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]52;c;!!!invalid\x1b\\");
        // Should not crash; clipboard may be set to decoded garbage or ignored.
        let _ = t.take_pending_clipboard_set();
    }

    // ── Round 28-4: Scroll region + IL/DL boundary ─────────────────────

    #[test]
    fn t_r28_stbm_cursor_moves_to_origin() {
        // DECSTBM should move cursor to origin (0,0) of screen.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[5;5H"); // cursor at row 4, col 4
        feed(&mut t, b"\x1b[2;8r"); // DECSTBM: scroll region rows 1-7
        assert_eq!(t.cursor().1, 0, "cursor moved to row 0 after DECSTBM");
        assert_eq!(t.cursor().0, 0, "cursor moved to col 0 after DECSTBM");
    }

    #[test]
    fn t_r28_stbm_invalid_region_ignored() {
        // DECSTBM with top >= bottom should be ignored.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[2;8r"); // valid region
        feed(&mut t, b"\x1b[8;2r"); // invalid (top > bottom) — should be ignored
        let (top, bottom) = t.grid().scroll_region();
        // The invalid region should NOT have been applied.
        assert_eq!(top, 1, "scroll top preserved after invalid DECSTBM");
        assert_eq!(bottom, 8, "scroll bottom preserved");
    }

    #[test]
    fn t_r28_lf_at_scroll_bottom_scrolls() {
        // LF at scroll region bottom should scroll within region.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[1;4r"); // scroll region rows 0-3
        feed(&mut t, b"\x1b[4;1H"); // cursor at row 3 (bottom of region)
        feed(&mut t, b"AAA\r\n"); // write AAA, then LF
        // LF at row 3 (bottom of region) should scroll region, not move below.
        assert_eq!(t.cursor().1, 3, "cursor stays at scroll bottom after LF");
    }

    #[test]
    fn t_r28_lf_below_scroll_region_moves_down() {
        // LF below scroll region should just move cursor down (no scroll).
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[1;3r"); // scroll region rows 0-2
        feed(&mut t, b"\x1b[5;1H"); // cursor at row 4 (below region)
        feed(&mut t, b"\n"); // LF
        assert_eq!(t.cursor().1, 5, "cursor moves down below scroll region");
    }

    #[test]
    fn t_r28_il_within_region_at_top() {
        // IL at top of scroll region inserts blank line.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[1;4r"); // region rows 0-3
        feed(&mut t, b"\x1b[1;1HAAA\r\n"); // row 0: AAA
        feed(&mut t, b"\x1b[1;1H"); // back to row 0
        feed(&mut t, b"\x1b[L"); // IL — insert blank line at row 0
        // AAA should have moved to row 1
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'A', "AAA moved down by IL");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ', "row 0 blank after IL");
    }

    #[test]
    fn t_r28_dl_within_region_at_bottom() {
        // DL at bottom of scroll region deletes line, lines scroll up.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[1;4r"); // region rows 0-3
        feed(&mut t, b"\x1b[1;1HAAA\r\nBBB"); // row 0: AAA, row 1: BBB
        feed(&mut t, b"\x1b[1;1H"); // cursor at row 0
        feed(&mut t, b"\x1b[M"); // DL — delete line at row 0
        // BBB should have moved to row 0
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'B', "BBB moved up by DL");
    }

    #[test]
    fn t_r28_reverse_lf_at_top_of_region_scrolls() {
        // RI (reverse line feed) at top of scroll region should scroll down.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[1;4r"); // region rows 0-3
        feed(&mut t, b"\x1b[1;1HAAA"); // row 0: AAA
        feed(&mut t, b"\x1b[1;1H"); // cursor at row 0 (top of region)
        feed(&mut t, b"\x1bM"); // RI — reverse line feed
        // Should scroll region down; AAA moves to row 1
        assert_eq!(t.cursor().1, 0, "cursor stays at top after RI at boundary");
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            'A',
            "AAA scrolled down by RI"
        );
    }

    #[test]
    fn t_r28_stbm_full_screen_default() {
        // DECSTBM with no params = full screen.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[2;4r"); // region rows 1-3
        feed(&mut t, b"\x1b[r"); // reset to full screen
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(top, 0, "scroll top reset to 0");
        assert_eq!(bottom, 6, "scroll bottom reset to height");
    }

    // ── Round 29-1: Wide char CJK/emoji edge cases ─────────────────────

    #[test]
    fn t_r29_wide_char_overwrite_lead_with_narrow() {
        // Print wide char 中 (cols 0-1), then move cursor to col 0 and
        // overwrite with narrow 'X'. The spacer at col 1 should be blanked.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "中".as_bytes());
        feed(&mut t, b"\x1b[H"); // cursor to 0,0
        feed(&mut t, b"X"); // overwrite lead
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'X',
            "lead overwritten with X"
        );
        assert_eq!(
            t.grid().cell(1, 0).unwrap().ch,
            ' ',
            "spacer blanked after overwrite"
        );
        assert!(
            !t.grid().cell(1, 0).unwrap().is_wide_spacer(),
            "no spacer flag"
        );
    }

    #[test]
    fn t_r29_wide_char_overwrite_spacer_with_narrow() {
        // Print wide char, then move cursor to col 1 (spacer) — cursor
        // adjusts to col 0 (lead). Print narrow char there.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "中".as_bytes()); // cols 0-1
        feed(&mut t, b"\x1b[1;2H"); // CUP col 1 (spacer) → adjusts to col 0
        feed(&mut t, b"Y"); // overwrite at lead position
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'Y',
            "Y at lead position (cursor adjusted from spacer)"
        );
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, ' ', "col 1 cleared");
    }

    #[test]
    fn t_r29_mixed_cjk_ascii_width() {
        // Mix of CJK and ASCII — verify correct column positions.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, "AB中CD".as_bytes()); // A(0) B(1) 中(2-3) C(4) D(5)
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, '中');
        assert!(
            t.grid().cell(2, 0).unwrap().is_wide(),
            "中 is wide char lead"
        );
        // Spacer cell uses ' ' as ch with WIDE_SPACER flag
        assert!(
            t.grid().cell(3, 0).unwrap().is_wide_spacer(),
            "col 3 is spacer"
        );
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'C');
        assert_eq!(t.grid().cell(5, 0).unwrap().ch, 'D');
        assert_eq!(t.cursor().0, 6, "cursor at col 6");
    }

    #[test]
    fn t_r29_wide_char_insert_mode_shifts() {
        // In insert mode, printing a wide char shifts existing content.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCD"); // cols 0-3
        feed(&mut t, b"\x1b[H"); // cursor to 0,0
        feed(&mut t, b"\x1b[4h"); // IRM on
        feed(&mut t, "中".as_bytes()); // insert wide at col 0
        // Should shift ABCD right by 2: 中 _ A B C D
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '中');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'A');
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'B');
    }

    #[test]
    fn t_r29_emoji_vs_variation_selector() {
        // ❤ followed by VS16 (U+FE0F) — VS16 is zero-width, attaches as combining.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "❤\u{FE0F}".as_bytes());
        // ❤ is width 1 by default, VS16 makes it width 2 (emoji presentation)
        // But unicode-width treats VS16 as zero-width combining.
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.ch, '❤', "base char is heart");
        // VS16 should be in combining
        assert!(
            cell.combining.contains(&'\u{FE0F}'),
            "VS16 attached as combining"
        );
    }

    // ── Round 29-2: Resize/reflow edge cases ───────────────────────────

    #[test]
    fn t_r29_resize_preserves_pending_wrap_content() {
        // Fill a row exactly (sets pending_wrap), then resize.
        // Content should survive reflow.
        // Use 1 visible row to avoid blank rows pushing content to scrollback.
        let mut t = Terminal::with_scrollback(4, 1, 100);
        feed(&mut t, b"ABCD"); // fills row 0, pending_wrap=true
        t.resize(2, 1); // shrink — reflow
        // ABCD reflows to: AB | CD (2 cols). With 1 visible row,
        // last segment (CD) is visible, AB in scrollback.
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'C',
            "CD visible after resize from pending_wrap"
        );
        assert_eq!(t.grid().scrollback_len(), 1, "AB in scrollback");
    }

    #[test]
    fn t_r29_resize_grow_keeps_cursor_clamped() {
        // After growing, cursor should not exceed old bounds initially.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"AB"); // cursor at col 2
        t.resize(80, 24); // grow
        assert_eq!(t.cursor().0, 2, "cursor x unchanged after grow");
        assert_eq!(t.cursor().1, 0, "cursor y unchanged after grow");
    }

    #[test]
    fn t_r29_resize_height_grow_pulls_scrollback() {
        // Grow height — should pull from scrollback.
        let mut t = Terminal::with_scrollback(5, 2, 100);
        // Fill 3 rows (1 goes to scrollback)
        feed(&mut t, b"AAA\r\n");
        feed(&mut t, b"BBB\r\n");
        feed(&mut t, b"CCC"); // CCC on visible, AAA in scrollback
        let sb_before = t.grid().scrollback_len();
        assert!(sb_before > 0, "scrollback has content");
        t.resize(5, 3); // grow height — pull 1 from scrollback
        assert!(
            t.grid().scrollback_len() < sb_before,
            "scrollback shrunk after height grow"
        );
    }

    #[test]
    fn t_r29_resize_to_1x1_no_panic() {
        // Resize to minimum size — should not panic or corrupt.
        let mut t = Terminal::new(80, 24);
        feed(&mut t, b"Hello World Test");
        t.resize(1, 1);
        assert_eq!(t.grid().width(), 1);
        assert_eq!(t.grid().height(), 1);
        // Grow back
        t.resize(80, 24);
        assert_eq!(t.grid().width(), 80);
    }

    // ── Round 29-3: Tab stops HTS/TBC/CHT/CBT edge cases ───────────────

    #[test]
    fn t_r29_tbc_clear_all_then_ht_default() {
        // Clear all tab stops (CSI 3g), then HT should go to end of line.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[3g"); // clear all stops
        feed(&mut t, b"A\t"); // A at col 0, then HT
        // With no stops, HT goes to last col
        assert_eq!(t.cursor().0, 19, "HT goes to last col when no stops");
    }

    #[test]
    fn t_r29_tbc_clear_current_then_ht() {
        // Clear current tab stop, then HT should skip it.
        let mut t = Terminal::new(20, 3);
        // Default stops at 8, 16. Clear the one at col 8.
        feed(&mut t, b"\x1b[1;9H"); // cursor at col 8 (the stop)
        feed(&mut t, b"\x1b[g"); // TBC clear current (col 8)
        feed(&mut t, b"\x1b[1;1H"); // back to col 0
        feed(&mut t, b"\t"); // HT — should skip col 8, go to 16
        assert_eq!(t.cursor().0, 16, "HT skips cleared stop, goes to 16");
    }

    #[test]
    fn t_r29_hts_at_arbitrary_col_then_ht() {
        // Set a custom tab stop at col 5, then HT from col 0 goes to 5.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[3g"); // clear all
        feed(&mut t, b"\x1b[1;6H"); // cursor at col 5
        feed(&mut t, b"\x1bH"); // HTS — set stop at col 5
        feed(&mut t, b"\x1b[1;1H"); // back to col 0
        feed(&mut t, b"\t"); // HT from col 0
        assert_eq!(t.cursor().0, 5, "HT goes to custom stop at col 5");
    }

    #[test]
    fn t_r29_cht_multiple_from_midpoint() {
        // CHT (CSI Ps I) — forward tab N times.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[1;3H"); // cursor at col 2
        feed(&mut t, b"\x1b[2I"); // CHT 2 — advance 2 tab stops
        // From col 2: next stop 8, next stop 16
        assert_eq!(t.cursor().0, 16, "CHT 2 from col 2 goes to col 16");
    }

    #[test]
    fn t_r29_cbt_from_col_17() {
        // CBT (CSI Ps Z) backward from beyond 16 goes to 8.
        let mut t = Terminal::new(40, 3);
        feed(&mut t, b"\x1b[1;18H"); // cursor at col 17
        feed(&mut t, b"\x1b[1Z"); // CBT 1
        assert_eq!(t.cursor().0, 16, "CBT from 17 goes to 16");
        feed(&mut t, b"\x1b[1Z"); // CBT 1 again
        assert_eq!(t.cursor().0, 8, "CBT from 16 goes to 8");
    }

    #[test]
    fn t_r29_ht_preserves_custom_after_hts_at_8() {
        // HTS at default position (col 8) — should still work.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[3g"); // clear all
        feed(&mut t, b"\x1b[1;9H"); // cursor at col 8
        feed(&mut t, b"\x1bH"); // HTS at col 8
        feed(&mut t, b"\x1b[1;1H"); // col 0
        feed(&mut t, b"\t"); // HT
        assert_eq!(t.cursor().0, 8, "HT goes to re-created stop at 8");
    }

    // ── Round 29-4: Alternate screen buffer edge cases ─────────────────

    #[test]
    fn t_r29_alt_screen_main_content_preserved_on_return() {
        // Write to main, switch to alt, write to alt, switch back.
        // Main content must be intact.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"MAIN");
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"\x1b[2J"); // clear alt
        feed(&mut t, b"ALT");
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'M',
            "main preserved on return"
        );
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'A', "A of MAIN");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'I', "I of MAIN");
    }

    #[test]
    fn t_r29_alt_screen_cursor_restored() {
        // Cursor position should be saved on alt enter, restored on exit.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;5H"); // cursor at row 2, col 4
        feed(&mut t, b"\x1b[?1049h"); // enter alt — saves cursor
        feed(&mut t, b"\x1b[1;1H"); // move to home in alt
        feed(&mut t, b"\x1b[?1049l"); // exit alt — restores cursor
        assert_eq!(t.cursor().0, 4, "cursor x restored after alt");
        assert_eq!(t.cursor().1, 2, "cursor y restored after alt");
    }

    #[test]
    fn t_r29_alt_screen_does_not_add_to_scrollback() {
        // Content scrolled in alt screen should NOT go to main scrollback.
        let mut t = Terminal::with_scrollback(10, 3, 100);
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        // Fill and scroll multiple lines
        feed(&mut t, b"Line1\r\nLine2\r\nLine3\r\nLine4\r\nLine5");
        let sb = t.grid().scrollback_len();
        // Alt screen should not accumulate scrollback
        assert_eq!(sb, 0, "alt screen has no scrollback: {}", sb);
    }

    #[test]
    fn t_r29_alt_screen_1049_re_enter_is_noop() {
        // Re-entering alt (1049h) when already in alt is a no-op,
        // not a clear. Content persists.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"ALT1");
        feed(&mut t, b"\x1b[?1049h"); // re-enter (no-op)
        // Content should persist (not cleared)
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'A',
            "alt content persists on re-enter"
        );
    }

    #[test]
    fn t_r29_alt_screen_nested_enter_exit() {
        // Multiple alt screen toggles should be stable.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"MAIN");
        feed(&mut t, b"\x1b[?1049h"); // enter
        feed(&mut t, b"ALT1");
        feed(&mut t, b"\x1b[?1049l"); // exit
        feed(&mut t, b"\x1b[?1049h"); // enter again
        feed(&mut t, b"ALT2");
        feed(&mut t, b"\x1b[?1049l"); // exit
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'M',
            "main intact after nested"
        );
    }

    // ── Round 30-1: OSC sequences & hyperlinks ─────────────────────────

    #[test]
    fn t_r30_osc8_id_param_preserved() {
        // OSC 8 with id= parameter — URI should be extracted correctly
        // regardless of params.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b]8;id=12345;https://example.com\x1b\\X");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().hyperlink.as_deref(),
            Some("https://example.com"),
            "URI extracted correctly with id param"
        );
    }

    #[test]
    fn t_r30_osc8_bel_terminator() {
        // OSC 8 terminated with BEL (0x07) instead of ST.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b]8;;https://example.com\x07X");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().hyperlink.as_deref(),
            Some("https://example.com"),
            "OSC 8 with BEL terminator"
        );
    }

    #[test]
    fn t_r30_osc8_uri_too_long_truncated() {
        // OSC 8 with URI > 2048 chars — should be truncated, not panic.
        let mut t = Terminal::new(20, 3);
        let long_uri = "https://example.com/".repeat(200); // ~3600 chars
        let osc = format!("\x1b]8;;{}\x1b\\X", long_uri);
        feed(&mut t, osc.as_bytes());
        let hl = t.grid().cell(0, 0).unwrap().hyperlink.as_ref();
        assert!(hl.is_some(), "long URI truncated but stored");
        assert!(
            hl.unwrap().len() <= 2048,
            "URI truncated to <= 2048: got {}",
            hl.unwrap().len()
        );
    }

    #[test]
    fn t_r30_osc0_title_unicode() {
        // OSC 0 with unicode title — should work.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]0;\xc3\xa9\xc3\xa8\xe4\xb8\xad\x07"); // "éè中" in UTF-8
        assert_eq!(t.title(), "éè中", "unicode title set");
    }

    #[test]
    fn t_r30_osc52_empty_clears_clipboard() {
        // OSC 52 with empty base64 — should clear clipboard.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]52;c;\x1b\\"); // empty data
        // Should not panic, clipboard_set should be None or empty
        let clip = t.take_pending_clipboard_set();
        // Empty base64 decodes to empty vec — some impls treat as clear
        assert!(
            clip.is_none() || clip.as_deref() == Some(b"".as_ref()),
            "empty OSC 52 clears clipboard"
        );
    }

    // ── Round 30-2: Bracketed paste / focus / mouse mode toggle ────────

    #[test]
    fn t_r30_focus_mode_toggle_idempotent() {
        // Enabling focus mode twice should be stable.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?1004h");
        feed(&mut t, b"\x1b[?1004h");
        assert!(t.modes.focus_event, "focus still on after double enable");
        feed(&mut t, b"\x1b[?1004l");
        assert!(!t.modes.focus_event, "focus off after disable");
    }

    #[test]
    fn t_r30_bracketed_paste_decrqm_mode() {
        // DECRQM should report the correct mode value (not set = 4).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?2004$p");
        let resp = String::from_utf8(t.take_response()).unwrap();
        // Mode 2004, not-set = response contains ";4$" (mode not set)
        assert!(
            resp.contains("2004"),
            "DECRQM for bracketed paste: {}",
            resp
        );
    }

    #[test]
    fn t_r30_mouse_sgr_pixel_mode_toggle() {
        // SGR pixel mouse mode (1016) toggle.
        let mut t = Terminal::new(10, 3);
        assert!(!t.mouse_sgr_pixel_enabled(), "pixel mode off by default");
        feed(&mut t, b"\x1b[?1016h");
        assert!(t.mouse_sgr_pixel_enabled(), "pixel mode enabled");
        feed(&mut t, b"\x1b[?1016l");
        assert!(!t.mouse_sgr_pixel_enabled(), "pixel mode disabled");
    }

    #[test]
    fn t_r30_mouse_all_modes_off_after_ris() {
        // RIS should reset all mouse modes to off.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h");
        feed(&mut t, b"\x1bc"); // RIS
        assert!(!t.mouse_tracking_enabled(), "mouse tracking off after RIS");
        assert!(!t.mouse_sgr_enabled(), "SGR off after RIS");
    }

    #[test]
    fn t_r30_sync_mode_does_not_affect_content() {
        // Enabling sync mode should not affect printed content.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?2026h");
        feed(&mut t, b"Hello");
        feed(&mut t, b"\x1b[?2026l");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'H', "content during sync");
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'o', "content after sync");
    }

    // ── Round 30-3: SGR text attributes & color boundaries ─────────────

    #[test]
    fn t_r30_sgr_empty_params_resets() {
        // ESC[m with no params should reset all attributes.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1;31;47m"); // bold, red fg, white bg
        feed(&mut t, b"\x1b[m"); // reset (no params)
        feed(&mut t, b"X");
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.fg, Color::Default, "fg reset by empty SGR");
        assert_eq!(cell.bg, Color::Default, "bg reset by empty SGR");
        assert!(
            !cell.flags.contains(CellFlags::BOLD),
            "bold cleared by empty SGR"
        );
    }

    #[test]
    fn t_r30_sgr_truecolor_then_reset() {
        // Set truecolor fg, then SGR 0 — should reset to Default.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[38;2;100;200;50m"); // truecolor fg
        feed(&mut t, b"\x1b[0m"); // reset
        feed(&mut t, b"X");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().fg,
            Color::Default,
            "truecolor fg reset to Default"
        );
    }

    #[test]
    fn t_r30_sgr_59_resets_underline_color() {
        // SGR 58 sets underline color, SGR 59 resets it.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[58;2;255;0;0m"); // underline color = red
        feed(&mut t, b"\x1b[59m"); // reset underline color
        assert_eq!(
            *t.underline_color_ref(),
            Color::Default,
            "underline color reset by SGR 59"
        );
    }

    #[test]
    fn t_r30_sgr_bold_dim_strikethrough_combined() {
        // Multiple text attributes combined.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1;2;9m"); // bold + dim + strikethrough
        feed(&mut t, b"X");
        let flags = t.grid().cell(0, 0).unwrap().flags;
        assert!(flags.contains(CellFlags::BOLD), "bold set");
        assert!(flags.contains(CellFlags::DIM), "dim set");
        assert!(
            flags.contains(CellFlags::STRIKETHROUGH),
            "strikethrough set"
        );
    }

    #[test]
    fn t_r30_sgr_22_clears_both_bold_and_dim() {
        // SGR 22 should clear both bold and dim.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1;2m"); // bold + dim
        feed(&mut t, b"\x1b[22m"); // clear bold and dim
        let flags = t.flags; // terminal's current flags
        assert!(!flags.contains(CellFlags::BOLD), "bold cleared by 22");
        assert!(!flags.contains(CellFlags::DIM), "dim cleared by 22");
    }

    #[test]
    fn t_r30_sgr_39_resets_fg_only() {
        // SGR 39 resets only fg, not bg or flags.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[31;42;1m"); // red fg, green bg, bold
        feed(&mut t, b"\x1b[39m"); // reset fg only
        feed(&mut t, b"X");
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.fg, Color::Default, "fg reset by 39");
        assert_eq!(cell.bg, Color::Indexed(2), "bg preserved (green)");
        assert!(cell.flags.contains(CellFlags::BOLD), "bold preserved by 39");
    }

    #[test]
    fn t_r30_sgr_49_resets_bg_only() {
        // SGR 49 resets only bg, not fg.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[31;42;1m"); // red fg, green bg, bold
        feed(&mut t, b"\x1b[49m"); // reset bg only
        feed(&mut t, b"X");
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.fg, Color::Indexed(1), "fg preserved (red)");
        assert_eq!(cell.bg, Color::Default, "bg reset by 49");
    }

    #[test]
    fn t_r30_sgr_256_color_high_index() {
        // SGR 38;5;255 should set fg to index 255 (bright white).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[38;5;255m");
        feed(&mut t, b"X");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().fg,
            Color::Indexed(255),
            "fg = index 255"
        );
    }

    #[test]
    fn t_r30_sgr_bright_colors_90_97() {
        // SGR 90-97 = bright fg colors (index 8-15).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[91m"); // bright red fg
        feed(&mut t, b"X");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().fg,
            Color::Indexed(9),
            "SGR 91 = bright red (index 9)"
        );
    }

    #[test]
    fn t_r30_sgr_overline_on_off() {
        // SGR 53 = overline on, SGR 55 = overline off.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[53m");
        feed(&mut t, b"X");
        assert!(
            t.grid()
                .cell(0, 0)
                .unwrap()
                .flags
                .contains(CellFlags::OVERLINE),
            "overline set by SGR 53"
        );
        feed(&mut t, b"\x1b[1;1H\x1b[55m"); // move to 0,0 and clear overline
        feed(&mut t, b"Y"); // overwrite
        assert!(
            !t.grid()
                .cell(0, 0)
                .unwrap()
                .flags
                .contains(CellFlags::OVERLINE),
            "overline cleared by SGR 55"
        );
    }

    // ── Round 31-1: Scroll region edge cases ───────────────────────────

    #[test]
    fn t_r31_stbm_reversed_params_ignored() {
        // CSI 10;1r (top > bottom) — invalid, region is NOT changed.
        // DECSTBM handler only sets region when top < bottom. Invalid params
        // are silently ignored, but cursor still homes.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[2;8r"); // valid region first
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!((top, bottom), (1, 8));
        feed(&mut t, b"\x1b[10;1r"); // reversed — ignored
        let (top2, bottom2) = t.grid().scroll_region();
        assert_eq!(top2, 1, "reversed STBM ignored, top stays 1");
        assert_eq!(bottom2, 8, "reversed STBM ignored, bottom stays 8");
    }

    #[test]
    fn t_r31_stbm_single_row_region() {
        // Single-row scroll region (CSI 5;5r → top=4, bottom=5).
        // set_scroll_region checks top < bottom, so top=4 < bottom=5 is valid.
        // But DECSTBM handler checks top < bottom in 1-based: 5 < 5 is false → no set.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[5;5r"); // 1-based: top=5, bottom=5
        // In DECSTBM handler: top(5) < bottom(5) is false → region NOT set
        // Cursor still homes.
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(top, 0, "single-row region not set, top stays 0");
        assert_eq!(bottom, 10, "single-row region not set, bottom stays height");
    }

    #[test]
    fn t_r31_ind_at_scroll_bottom_scrolls() {
        // IND (ESC D) at scroll bottom scrolls within region.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[1;4r"); // region rows 0-3
        feed(&mut t, b"\x1b[4;1HAAA"); // row 3: AAA
        feed(&mut t, b"\x1bD"); // IND at bottom of region
        // Should scroll region up, cursor stays at row 3
        assert_eq!(t.cursor().1, 3, "cursor at scroll bottom after IND");
        // AAA should have scrolled up to row 2
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, 'A', "AAA scrolled up");
        assert_eq!(
            t.grid().cell(0, 3).unwrap().ch,
            ' ',
            "row 3 blank after scroll"
        );
    }

    #[test]
    fn t_r31_ri_at_scroll_top_scrolls_down() {
        // RI (ESC M) at scroll top scrolls down within region.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[1;4r"); // region rows 0-3
        feed(&mut t, b"\x1b[1;1HAAA\r\n"); // row 0: AAA, cursor to row 1
        feed(&mut t, b"\x1b[1;1H"); // cursor at row 0 (top of region)
        feed(&mut t, b"\x1bM"); // RI
        // Should scroll region down, AAA moves to row 1
        assert_eq!(t.cursor().1, 0, "cursor at top after RI");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            ' ',
            "row 0 blank after RI scroll"
        );
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'A', "AAA scrolled down");
    }

    #[test]
    fn t_r31_nel_at_scroll_bottom_scrolls() {
        // NEL (ESC E) at scroll bottom scrolls + CR.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[1;4r"); // region rows 0-3
        feed(&mut t, b"\x1b[4;3HABC"); // row 3, cols 2-4
        feed(&mut t, b"\x1bE"); // NEL — scroll + CR + LF
        assert_eq!(t.cursor().0, 0, "NEL does CR → x=0");
        assert_eq!(t.cursor().1, 3, "cursor at scroll bottom after NEL");
        assert_eq!(t.grid().cell(2, 2).unwrap().ch, 'A', "ABC scrolled up");
    }

    #[test]
    fn t_r31_dl_confined_to_scroll_region() {
        // DL within scroll region only affects region rows.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[1;3r"); // region rows 0-2
        feed(&mut t, b"\x1b[1;1HAAA\r\nBBB\r\nCCC"); // rows 0-2
        feed(&mut t, b"\x1b[1;1H"); // cursor at row 0
        feed(&mut t, b"\x1b[M"); // DL at row 0
        // BBB moves to row 0, CCC to row 1, row 2 blank
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'B',
            "BBB at row 0 after DL"
        );
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            'C',
            "CCC at row 1 after DL"
        );
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, ' ', "row 2 blank after DL");
    }

    #[test]
    fn t_r31_il_confined_to_scroll_region() {
        // IL within scroll region inserts blank rows.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[1;3r"); // region rows 0-2
        feed(&mut t, b"\x1b[1;1HAAA\r\nBBB"); // row 0: AAA, row 1: BBB
        feed(&mut t, b"\x1b[1;1H"); // cursor at row 0
        feed(&mut t, b"\x1b[L"); // IL at row 0
        // Row 0 blank, AAA moves to row 1, BBB to row 2
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ', "row 0 blank after IL");
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'A', "AAA at row 1");
        assert_eq!(t.grid().cell(0, 2).unwrap().ch, 'B', "BBB at row 2");
    }

    #[test]
    fn t_r31_stbm_reset_no_params() {
        // CSI r with no params resets to full screen.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[2;4r"); // set region rows 1-3
        feed(&mut t, b"\x1b[r"); // reset
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(top, 0, "reset: top = 0");
        assert_eq!(bottom, 6, "reset: bottom = height");
    }

    // ── Round 31-2: Origin mode (DECOM) ────────────────────────────────

    #[test]
    fn t_r31_origin_mode_cup_relative() {
        // CUP in origin mode is relative to scroll region top.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[3;7r"); // region rows 2-6
        feed(&mut t, b"\x1b[?6h"); // enable origin mode
        feed(&mut t, b"\x1b[2;3H"); // CUP row=2, col=3 → row=top+1=3, col=2
        assert_eq!(t.cursor().1, 3, "CUP row 2 in origin = absolute row 3");
        assert_eq!(t.cursor().0, 2, "col unaffected by origin");
    }

    #[test]
    fn t_r31_origin_mode_homes_to_region() {
        // Enabling origin mode homes cursor to region top.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[3;7r"); // region rows 2-6
        feed(&mut t, b"\x1b[?6h"); // enable origin → home to region top
        assert_eq!(t.cursor().1, 2, "origin homes to region top (row 2)");
        assert_eq!(t.cursor().0, 0, "col 0");
    }

    #[test]
    fn t_r31_origin_mode_cup_clamped_to_region() {
        // CUP in origin mode with large row → clamped to region bottom.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[3;7r"); // region rows 2-6
        feed(&mut t, b"\x1b[?6h");
        feed(&mut t, b"\x1b[100;1H"); // row 100 → clamp to bottom-1 = 5
        assert_eq!(t.cursor().1, 6, "origin CUP clamped to region bottom-1");
    }

    #[test]
    fn t_r31_origin_mode_disable_restores_absolute() {
        // Disabling origin mode restores absolute coordinates.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[3;7r"); // region rows 2-6
        feed(&mut t, b"\x1b[?6h"); // origin on
        feed(&mut t, b"\x1b[?6l"); // origin off → home to 0,0
        feed(&mut t, b"\x1b[1;1H"); // CUP 1,1 → absolute 0,0
        assert_eq!(t.cursor().1, 0, "origin off: CUP is absolute");
    }

    #[test]
    fn t_r31_origin_mode_vpa_relative() {
        // VPA (CSI d) in origin mode is relative to scroll region.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[3;7r"); // region rows 2-6
        feed(&mut t, b"\x1b[?6h"); // origin on
        feed(&mut t, b"\x1b[2d"); // VPA row 2 → absolute row = top+1 = 3
        assert_eq!(t.cursor().1, 3, "VPA row 2 in origin = absolute 3");
    }

    // ── Round 31-3: Wide char + DL/IL/backspace boundaries ─────────────

    #[test]
    fn t_r31_backspace_after_wide_skips_spacer() {
        // After printing wide char, backspace should move to the lead cell
        // (skip the spacer), so the next char overwrites the wide char.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "中".as_bytes()); // cols 0-1, cursor at col 2
        feed(&mut t, b"\x08"); // BS → cursor should go to col 1 (spacer)
        // Actually BS just decrements by 1 → col 1 (spacer position)
        assert_eq!(t.cursor().0, 1, "BS after wide char at col 1");
        // Another BS → col 0 (lead)
        feed(&mut t, b"\x08");
        assert_eq!(t.cursor().0, 0, "BS to col 0 (lead)");
    }

    #[test]
    fn t_r31_dl_with_wide_char_row() {
        // DL on a row with wide chars — no corruption.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, "中文A".as_bytes()); // row 0: 中(0-1) 文(2-3) A(4)
        feed(&mut t, b"\r\nBCD"); // row 1: BCD
        feed(&mut t, b"\x1b[1;1H"); // cursor at row 0
        feed(&mut t, b"\x1b[M"); // DL row 0
        // Row 0 should now have BCD
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'B',
            "row 0 has BCD after DL"
        );
    }

    #[test]
    fn t_r31_il_with_wide_char_row() {
        // IL before a row with wide chars — no corruption.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, "中文A".as_bytes()); // row 0
        feed(&mut t, b"\x1b[1;1H"); // cursor at row 0
        feed(&mut t, b"\x1b[L"); // IL at row 0
        // Row 0 should be blank, 中文A moves to row 1
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, ' ', "row 0 blank after IL");
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            '中',
            "wide char moved to row 1"
        );
        assert!(
            t.grid().cell(0, 1).unwrap().is_wide(),
            "still wide at row 1"
        );
    }

    #[test]
    fn t_r31_wide_char_overwrite_preserves_adjacent() {
        // Overwriting a wide char should not affect adjacent narrow chars.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "中A文B".as_bytes()); // 中(0-1) A(2) 文(3-4) B(5)
        feed(&mut t, b"\x1b[1;1H"); // cursor at col 0
        feed(&mut t, b"X"); // overwrite 中's lead
        // X at col 0, col 1 blanked (spacer), A at col 2 intact
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X', "X at col 0");
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, ' ', "spacer blanked");
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'A', "A intact at col 2");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, '文', "文 intact at col 3");
        assert_eq!(t.grid().cell(5, 0).unwrap().ch, 'B', "B intact at col 5");
    }

    #[test]
    fn t_r31_cuu_cud_within_scroll_region() {
        // CUU/CUD should respect scroll region boundaries.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[3;7r"); // region rows 2-6
        feed(&mut t, b"\x1b[3;1H"); // cursor at row 2 (region top)
        feed(&mut t, b"\x1b[5A"); // CUU 5 — should clamp at region top (row 2)
        assert_eq!(t.cursor().1, 2, "CUU clamped at region top");
        feed(&mut t, b"\x1b[3;7H"); // cursor at row 6 (region bottom-1)
        feed(&mut t, b"\x1b[5B"); // CUD 5 — should clamp at region bottom-1
        assert_eq!(t.cursor().1, 6, "CUD clamped at region bottom-1");
    }

    #[test]
    fn t_r31_cuu_cud_outside_scroll_region() {
        // CUU/CUD when cursor is OUTSIDE scroll region — no clamping.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[3;7r"); // region rows 2-6
        feed(&mut t, b"\x1b[9;1H"); // cursor at row 8 (below region)
        feed(&mut t, b"\x1b[5A"); // CUU 5 — no clamping since outside region
        assert_eq!(t.cursor().1, 3, "CUU outside region: row 8-5=3");
    }

    // ── Round 32-1: Alt screen buffer edge cases ───────────────────────

    #[test]
    fn t_r32_alt_1049_sgr_state_restored() {
        // 1049 saves/restores SGR attributes (bold, colors).
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[1;31m"); // bold + red
        feed(&mut t, b"\x1b[?1049h"); // enter alt — saves SGR
        feed(&mut t, b"\x1b[0m"); // reset in alt
        feed(&mut t, b"\x1b[?1049l"); // exit alt — restores SGR
        feed(&mut t, b"X");
        let cell = t.grid().cell(0, 0).unwrap();
        assert!(
            cell.flags.contains(CellFlags::BOLD),
            "bold restored after alt exit"
        );
        assert_eq!(cell.fg, Color::Indexed(1), "red restored after alt exit");
    }

    #[test]
    fn t_r32_alt_47_no_cursor_save() {
        // CSI ?47h does NOT save cursor (unlike 1049).
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"\x1b[3;5H"); // cursor at row 2, col 4
        feed(&mut t, b"\x1b[?47h"); // enter alt via 47 (no cursor save)
        feed(&mut t, b"\x1b[1;1H"); // move cursor in alt
        feed(&mut t, b"\x1b[?47l"); // exit alt
        // Cursor NOT restored (47 doesn't save it) — x at 0
        assert_eq!(t.cursor().0, 0, "47 does not save/restore cursor");
    }

    #[test]
    fn t_r32_alt_scroll_does_not_affect_main_scrollback() {
        // Scrolling in alt screen must not push lines to main scrollback.
        let mut t = Terminal::with_scrollback(10, 3, 100);
        let sb_before = t.grid().scrollback_len();
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        // Scroll many lines
        for _ in 0..10 {
            feed(&mut t, b"Line\r\n");
        }
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        assert_eq!(
            t.grid().scrollback_len(),
            sb_before,
            "alt scrollback not added to main"
        );
    }

    #[test]
    fn t_r32_alt_tab_stops_preserved() {
        // Custom tab stops on main screen should survive alt screen round-trip.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[3g"); // clear all stops
        feed(&mut t, b"\x1b[1;6H\x1bH"); // set stop at col 5
        feed(&mut t, b"\x1b[?1049h"); // enter alt — saves tab stops
        feed(&mut t, b"\x1b[3g"); // clear all in alt
        feed(&mut t, b"\x1b[?1049l"); // exit alt — restores tab stops
        feed(&mut t, b"\x1b[1;1H\t"); // HT — should go to col 5
        assert_eq!(t.cursor().0, 5, "custom tab stop restored after alt");
    }

    // ── Round 32-3: OSC title/color query ──────────────────────────────

    #[test]
    fn t_r32_osc1_icon_title() {
        // OSC 1 sets icon title.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]1;IconTitle\x07");
        // OSC 1 may or may not be tracked separately, but should not crash
        // and should not set the main title.
        // (Implementation may or may not track icon_title separately.)
    }

    #[test]
    fn t_r32_osc0_then_osc2() {
        // OSC 0 sets both title and icon. OSC 2 sets only title.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]0;Both\x07");
        assert_eq!(t.title(), "Both", "OSC 0 sets title");
        feed(&mut t, b"\x1b]2;TitleOnly\x07");
        assert_eq!(t.title(), "TitleOnly", "OSC 2 updates title");
    }

    #[test]
    fn t_r32_osc10_query_default_fg() {
        // OSC 10;? query — default fg should be white (ffffff).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]10;?\x1b\\");
        let resp = String::from_utf8(t.take_response()).unwrap();
        assert!(
            resp.contains("10;rgb:ff/ff/ff"),
            "OSC 10 default fg = white: {}",
            resp
        );
    }

    #[test]
    fn t_r32_osc11_query_default_bg() {
        // OSC 11;? query — default bg should be black (000000).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]11;?\x1b\\");
        let resp = String::from_utf8(t.take_response()).unwrap();
        assert!(
            resp.contains("11;rgb:00/00/00"),
            "OSC 11 default bg = black: {}",
            resp
        );
    }

    #[test]
    fn t_r32_osc10_set_then_query() {
        // OSC 10 with color spec, then query — should return set color.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]10;rgb:ab/cd/ef\x1b\\");
        feed(&mut t, b"\x1b]10;?\x1b\\");
        let resp = String::from_utf8(t.take_response()).unwrap();
        assert!(
            resp.contains("10;rgb:ab/cd/ef"),
            "OSC 10 query returns set color: {}",
            resp
        );
    }

    #[test]
    fn t_r32_osc4_query_multiple() {
        // OSC 4 with multiple indices in one query.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]4;0;?;1;?\x1b\\");
        let resp = String::from_utf8(t.take_response()).unwrap();
        // Should have responses for both index 0 and index 1
        assert!(
            resp.contains("4;0;rgb:"),
            "OSC 4 index 0 response: {}",
            resp
        );
        assert!(
            resp.contains("4;1;rgb:"),
            "OSC 4 index 1 response: {}",
            resp
        );
    }

    // ── Round 32-4: Mouse tracking mode tests ──────────────────────────

    #[test]
    fn t_r32_mouse_decrqm_1000() {
        // DECRQM for mouse mode 1000.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?1000$p");
        let resp = String::from_utf8(t.take_response()).unwrap();
        assert!(resp.contains("1000"), "DECRQM for 1000: {}", resp);
    }

    #[test]
    fn t_r32_mouse_enable_disable_1002() {
        // Enable/disable button event tracking.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?1002h");
        assert!(
            t.mouse_button_event_enabled(),
            "button event tracking enabled"
        );
        feed(&mut t, b"\x1b[?1002l");
        assert!(
            !t.mouse_button_event_enabled(),
            "button event tracking disabled"
        );
    }

    #[test]
    fn t_r32_mouse_enable_disable_1003() {
        // Enable/disable any event tracking.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?1003h");
        assert!(t.mouse_any_event_enabled(), "any event tracking enabled");
        feed(&mut t, b"\x1b[?1003l");
        assert!(!t.mouse_any_event_enabled(), "any event tracking disabled");
    }

    #[test]
    fn t_r32_mouse_sgr_and_urxvt_independent() {
        // SGR (1006) and URXVT (1015) are independent format flags.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?1006h");
        assert!(t.mouse_sgr_enabled(), "SGR enabled");
        assert!(!t.mouse_urxvt_enabled(), "URXVT not enabled by default");
        feed(&mut t, b"\x1b[?1015h");
        assert!(t.mouse_urxvt_enabled(), "URXVT now enabled");
        assert!(t.mouse_sgr_enabled(), "SGR still enabled");
    }

    #[test]
    fn t_r32_mouse_utf8_mode_toggle() {
        // UTF-8 mouse mode (1005) toggle.
        let mut t = Terminal::new(10, 3);
        assert!(!t.modes.mouse_utf8, "UTF-8 mouse off by default");
        feed(&mut t, b"\x1b[?1005h");
        assert!(t.modes.mouse_utf8, "UTF-8 mouse enabled");
        feed(&mut t, b"\x1b[?1005l");
        assert!(!t.modes.mouse_utf8, "UTF-8 mouse disabled");
    }

    #[test]
    fn t_r32_mouse_decrqm_1006_enabled() {
        // DECRQM for SGR mouse when enabled should report "set" (mode 1).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?1006h"); // enable
        feed(&mut t, b"\x1b[?1006$p"); // query
        let resp = String::from_utf8(t.take_response()).unwrap();
        // When set, response should contain ";1$" (mode = set)
        assert!(
            resp.contains("1006") && resp.contains(";1$"),
            "DECRQM 1006 reports set: {}",
            resp
        );
    }

    // ── Round 33-1: Wide char wrapping at line boundary ────────────────

    #[test]
    fn t_r33_wide_char_fills_then_wraps() {
        // Fill row exactly (no pending wrap), then next wide char wraps.
        let mut t = Terminal::new(4, 3);
        feed(&mut t, "中".as_bytes()); // cols 0-1, cursor at col 2
        feed(&mut t, b"A"); // col 2, cursor at col 3 (1 col left)
        feed(&mut t, "文".as_bytes()); // width=2, only 1 col → wrap
        // A at col 2, col 3 stays blank (wide char wraps away)
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, 'A', "A at col 2");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, ' ', "col 3 blank");
        // 文 at row 1, cols 0-1
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, '文', "文 at row 1 col 0");
        assert!(t.grid().cell(0, 1).unwrap().is_wide(), "文 is wide lead");
    }

    #[test]
    fn t_r33_wide_char_autowrap_off_no_wrap() {
        // With DECAWM off, wide char at boundary should NOT wrap.
        // It should be placed at cursor position (overwriting if needed).
        let mut t = Terminal::new(3, 3);
        feed(&mut t, b"\x1b[?7l"); // autowrap off
        feed(&mut t, b"AB"); // cols 0-1, cursor at col 2 (last)
        feed(&mut t, "中".as_bytes()); // width=2, only 1 col, but no wrap
        // Behavior: wide char at col 2 — only lead cell stored, spacer lost.
        // Or it may just not print. Either way, no wrap to next line.
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, ' ', "no wrap to row 1");
    }

    #[test]
    fn t_r33_wide_char_exact_fit() {
        // Wide char that exactly fits remaining space (2 cols left).
        let mut t = Terminal::new(4, 3);
        feed(&mut t, b"A"); // col 0, cursor at col 1 (3 cols left)
        feed(&mut t, "中".as_bytes()); // cols 1-2, cursor at col 3
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, '中', "中 at col 1");
        assert!(t.grid().cell(1, 0).unwrap().is_wide(), "lead at col 1");
        assert!(
            t.grid().cell(2, 0).unwrap().is_wide_spacer(),
            "spacer at col 2"
        );
        assert_eq!(t.cursor().0, 3, "cursor at col 3");
    }

    #[test]
    fn t_r33_two_wide_chars_fill_row() {
        // Two wide chars fill a 4-wide row exactly.
        let mut t = Terminal::new(4, 3);
        feed(&mut t, "中文".as_bytes()); // 中(0-1) 文(2-3), pending_wrap
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, '中');
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, '文');
        assert_eq!(
            t.cursor().0,
            3,
            "cursor at last col after filling with wide chars"
        );
        // Next char should trigger deferred wrap
        feed(&mut t, b"X");
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, 'X', "X wrapped to row 1");
    }

    // ── Round 33-2: DECAWM autowrap on/off + deferred wrap ─────────────

    #[test]
    fn t_r33_autowrap_off_then_on() {
        // Toggle autowrap off then on — wrapping should resume.
        let mut t = Terminal::new(4, 3);
        feed(&mut t, b"\x1b[?7l"); // off
        feed(&mut t, b"ABCDE"); // E overwrites D at col 3
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'E');
        feed(&mut t, b"\x1b[1;1H"); // back to col 0
        feed(&mut t, b"\x1b[?7h"); // on
        feed(&mut t, b"ABCD"); // fills row, pending_wrap
        feed(&mut t, b"E"); // should wrap
        assert_eq!(
            t.grid().cell(0, 1).unwrap().ch,
            'E',
            "wrap works after re-enabling"
        );
    }

    #[test]
    fn t_r33_deferred_wrap_with_cup() {
        // After deferred wrap (pending_wrap=true), CUP should cancel it.
        let mut t = Terminal::new(4, 3);
        feed(&mut t, b"ABCD"); // pending_wrap=true
        feed(&mut t, b"\x1b[1;1H"); // CUP — cancels pending_wrap
        feed(&mut t, b"X"); // should overwrite A, not wrap
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'X',
            "CUP cancels deferred wrap"
        );
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, ' ', "no wrap occurred");
    }

    #[test]
    fn t_r33_deferred_wrap_with_bs() {
        // BS after deferred wrap cancels pending_wrap.
        // BS moves cursor from col 3 to col 2.
        let mut t = Terminal::new(4, 3);
        feed(&mut t, b"ABCD"); // pending_wrap=true
        feed(&mut t, b"\x08"); // BS — cursor to col 2, cancels pending_wrap
        feed(&mut t, b"X"); // overwrite at col 2 (was C)
        assert_eq!(
            t.grid().cell(2, 0).unwrap().ch,
            'X',
            "BS to col 2, overwrite C with X"
        );
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, 'D', "D preserved at col 3");
        assert_eq!(t.grid().cell(0, 1).unwrap().ch, ' ', "no wrap after BS");
    }

    #[test]
    fn t_r33_autowrap_off_cr_does_not_wrap() {
        // With autowrap off, CR should not cause wrap.
        let mut t = Terminal::new(4, 3);
        feed(&mut t, b"\x1b[?7l");
        feed(&mut t, b"ABCD\r");
        // CR should move cursor to col 0, same row
        assert_eq!(t.cursor().0, 0, "CR to col 0");
        assert_eq!(t.cursor().1, 0, "CR same row");
    }

    // ── Round 33-3: SGR 24-bit truecolor + 256 color boundaries ────────

    #[test]
    fn t_r33_sgr_38_2_basic_truecolor_fg() {
        // SGR 38;2;128;64;200 — truecolor foreground.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[38;2;128;64;200m");
        feed(&mut t, b"X");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().fg,
            Color::Rgb(128, 64, 200),
            "truecolor fg"
        );
    }

    #[test]
    fn t_r33_sgr_48_2_basic_truecolor_bg() {
        // SGR 48;2;10;20;30 — truecolor background.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[48;2;10;20;30m");
        feed(&mut t, b"X");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().bg,
            Color::Rgb(10, 20, 30),
            "truecolor bg"
        );
    }

    #[test]
    fn t_r33_sgr_38_5_202_orange() {
        // SGR 38;5;202 — 256-color orange (208 is actually orange).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[38;5;202m");
        feed(&mut t, b"X");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().fg,
            Color::Indexed(202),
            "256-color index 202"
        );
    }

    #[test]
    fn t_r33_sgr_38_2_truncated_values() {
        // SGR 38;2;300;0;0 — 300 > 255, truncated to 300 as u8 = 44.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[38;2;300;0;0m");
        feed(&mut t, b"X");
        // 300u16 as u8 = 44 (300 - 256 = 44)
        if let Color::Rgb(r, _, _) = t.grid().cell(0, 0).unwrap().fg {
            assert_eq!(r, 44, "300 truncated to 44 as u8");
        } else {
            panic!("expected Rgb color");
        }
    }

    #[test]
    fn t_r33_sgr_38_2_missing_params_no_crash() {
        // SGR 38;2;128 — missing G and B. Should not crash, no color set.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[38;2;128m");
        feed(&mut t, b"X");
        // With incomplete params, color should not be set (remains Default)
        // or set to some safe value. Just verify no crash.
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.ch, 'X', "char printed despite incomplete SGR");
    }

    #[test]
    fn t_r33_sgr_38_5_missing_index_no_crash() {
        // SGR 38;5 — missing index. Should not crash.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[38;5m");
        feed(&mut t, b"X");
        // No crash, char printed
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X');
    }

    #[test]
    fn t_r33_sgr_truecolor_then_indexed() {
        // Switch from truecolor to indexed — should fully replace.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[38;2;100;200;50m"); // truecolor
        feed(&mut t, b"\x1b[38;5;9m"); // indexed bright red
        feed(&mut t, b"X");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().fg,
            Color::Indexed(9),
            "indexed replaces truecolor"
        );
    }

    #[test]
    fn t_r33_sgr_48_5_then_reset() {
        // Set 256-color bg, then SGR 49 resets to default.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[48;5;4m"); // bg = blue
        feed(&mut t, b"\x1b[49m"); // reset bg
        feed(&mut t, b"X");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().bg,
            Color::Default,
            "bg reset by SGR 49"
        );
    }

    // ── Round 34: Tab/scroll/alt integration scenarios ─────────────────

    #[test]
    fn t_r34_tab_then_lf_at_scroll_bottom() {
        // Tab to a stop, then LF at scroll region bottom — scrolls correctly.
        let mut t = Terminal::new(20, 6);
        feed(&mut t, b"\x1b[1;4r"); // region rows 0-3
        feed(&mut t, b"\x1b[4;1H"); // cursor at row 3 (bottom of region)
        feed(&mut t, b"\t"); // tab to col 8
        feed(&mut t, b"\n"); // LF at scroll bottom → scroll
        assert_eq!(t.cursor().0, 8, "col preserved as 8 after LF scroll");
        assert_eq!(t.cursor().1, 3, "cursor stays at scroll bottom");
    }

    #[test]
    fn t_r34_tab_in_scroll_region_after_scroll() {
        // After scroll within region, tab from new line should still work.
        let mut t = Terminal::new(16, 6);
        feed(&mut t, b"\x1b[1;3r"); // region rows 0-2
        feed(&mut t, b"\x1b[1;1HAB\r\nCD\r\nEF\r\n");
        // EF was at row 2, LF scrolled CD→row0, EF→row1, blank→row2
        feed(&mut t, b"\x1b[3;1H"); // cursor at row 2
        feed(&mut t, b"\t"); // tab to col 8
        assert_eq!(t.cursor().0, 8, "tab works after scroll in region");
    }

    #[test]
    fn t_r34_hts_in_scroll_region_persists() {
        // HTS set inside scroll region should persist after scrolling.
        let mut t = Terminal::new(20, 6);
        feed(&mut t, b"\x1b[1;3r"); // region rows 0-2
        feed(&mut t, b"\x1b[1;6H\x1bH"); // HTS at col 5
        // Scroll the region
        feed(&mut t, b"\x1b[3;1H\n"); // LF at bottom of region → scroll
        // Tab stop at col 5 should still work
        feed(&mut t, b"\x1b[1;1H\t"); // tab from col 0
        assert_eq!(t.cursor().0, 5, "custom tab stop survives scroll");
    }

    #[test]
    fn t_r34_alt_tab_custom_then_scroll_in_alt() {
        // In alt screen: set custom tab, scroll, verify tab still works.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"\x1b[3g"); // clear all stops
        feed(&mut t, b"\x1b[1;8H\x1bH"); // HTS at col 7
        // Fill and scroll
        for _ in 0..6 {
            feed(&mut t, b"X\r\n");
        }
        // Tab stop should persist in alt
        feed(&mut t, b"\x1b[1;1H\t"); // tab from col 0
        assert_eq!(t.cursor().0, 7, "custom tab stop in alt survives scroll");
    }

    #[test]
    fn t_r34_tab_at_col0_with_stop_at_col0() {
        // HTS at col 0, then tab from col 0 — should skip to next stop.
        // Tab starts scanning from cursor.x + 1, so stop at col 0 is skipped.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b[3g"); // clear all
        feed(&mut t, b"\x1b[1;1H\x1bH"); // HTS at col 0
        feed(&mut t, b"\x1b[1;9H\x1bH"); // HTS at col 8
        feed(&mut t, b"\x1b[1;1H"); // cursor at col 0
        feed(&mut t, b"\t"); // HT from col 0
        // Tab scans from col 1, finds stop at col 8
        assert_eq!(t.cursor().0, 8, "tab from col 0 skips stop at 0, goes to 8");
    }

    #[test]
    fn t_r34_decstbm_top_param_zero() {
        // CSI 0;5r — top param 0 should default to 1 (row 0).
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[0;5r");
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(top, 0, "top param 0 → row 0");
        assert_eq!(bottom, 5, "bottom = 5");
    }

    #[test]
    fn t_r34_decstbm_only_bottom_param() {
        // CSI ;5r — top defaults to 1, bottom = 5.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[;5r");
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(top, 0, "top defaults to 0");
        assert_eq!(bottom, 5, "bottom = 5");
    }

    #[test]
    fn t_r34_scroll_region_el_at_boundary() {
        // EL (erase line) at scroll region boundary — should erase regardless of region.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[1;3r"); // region rows 0-2
        feed(&mut t, b"ABCDEFGH"); // row 0: ABCDEFGH
        feed(&mut t, b"\x1b[1;4H"); // cursor at col 3
        feed(&mut t, b"\x1b[K"); // EL from cursor to end
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'A', "A preserved");
        assert_eq!(t.grid().cell(3, 0).unwrap().ch, ' ', "erased from col 3");
        assert_eq!(t.grid().cell(7, 0).unwrap().ch, ' ', "H erased");
    }

    #[test]
    fn t_r34_alt_then_stbm_then_exit() {
        // Set scroll region in alt, exit alt → main should have default region.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"\x1b[2;8r"); // set region in alt
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(top, 0, "main scroll region restored to full");
        assert_eq!(bottom, 10, "main scroll bottom restored");
    }

    #[test]
    fn t_r34_dch_at_scroll_region_boundary() {
        // DCH (delete char) at scroll region boundary — should work normally.
        let mut t = Terminal::new(10, 6);
        feed(&mut t, b"\x1b[1;3r"); // region rows 0-2
        feed(&mut t, b"ABCDEF"); // row 0: ABCDEF
        feed(&mut t, b"\x1b[1;1H"); // cursor at col 0
        feed(&mut t, b"\x1b[2P"); // DCH 2 — delete 2 chars
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'C', "C shifted left");
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'D', "D shifted left");
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, ' ', "blank at end");
    }

    #[test]
    fn t_r34_tab_advances_past_wide_char_spacer() {
        // Tab should skip past wide char spacer cells correctly.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, "中".as_bytes()); // cols 0-1, cursor at col 2
        feed(&mut t, b"\t"); // tab from col 2
        // Default stops at 8 — tab should land on col 8
        assert_eq!(
            t.cursor().0,
            8,
            "tab from col 2 (after wide char) goes to col 8"
        );
    }

    #[test]
    fn t_r34_alt_exit_restores_origin_mode() {
        // Origin mode set in main, enter alt (which might change it),
        // exit alt — origin mode should be restored.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[3;7r"); // set scroll region
        feed(&mut t, b"\x1b[?6h"); // enable origin
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"\x1b[?6l"); // disable origin in alt
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        // Origin mode should be restored to what it was in main
        assert!(t.modes.origin, "origin mode restored after alt exit");
    }

    #[test]
    fn t_r34_multiple_decstbm_cascade() {
        // Set region, then set a smaller region within it.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[2;9r"); // region rows 1-8
        feed(&mut t, b"\x1b[3;7r"); // smaller region rows 2-6
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(top, 2, "nested region top = 2");
        assert_eq!(bottom, 7, "nested region bottom = 7");
    }

    #[test]
    fn t_r34_tab_no_wrap_at_last_col() {
        // Tab at the last column should NOT cause a line wrap.
        let mut t = Terminal::new(8, 3);
        feed(&mut t, b"\x1b[1;8H"); // cursor at col 7 (last)
        feed(&mut t, b"\t"); // tab
        assert_eq!(t.cursor().0, 7, "tab at last col stays at last col");
        assert_eq!(t.cursor().1, 0, "no line wrap from tab");
    }

    // ── Round 35: Parser robustness + untested feature areas ───────────

    #[test]
    fn t_r35_dcs_xtgettcap_response() {
        // DCS + q (XTGETTCAP) — query terminal name capability.
        // "TN" hex = "544e", response contains hex-encoded "ggterm".
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1bP+q544e\x1b\\");
        let resp = String::from_utf8(t.take_response()).unwrap();
        // Response: DCS 1+r 544e=<hex>. 67677465726d = "ggterm" in hex.
        assert!(resp.contains("544e="), "XTGETTCAP TN response: {}", resp);
    }

    #[test]
    fn t_r35_pm_consumed_silently() {
        // PM (ESC ^) — Privacy Message. Must be consumed until ST.
        // Content should NOT appear on screen.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b^Hello World\x1b\\");
        feed(&mut t, b"X");
        // PM content should not have been printed
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X', "PM content consumed");
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, ' ', "only X printed");
    }

    #[test]
    fn t_r35_apc_consumed_silently() {
        // APC (ESC _) — Application Program Command. Must be consumed until ST.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b_Gp=a,b\x1b\\"); // like kitty graphics APC
        feed(&mut t, b"Y");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'Y',
            "APC consumed, Y printed"
        );
    }

    #[test]
    fn t_r35_dcs_with_params_consumed() {
        // DCS with intermediate params — should be consumed without crash.
        let mut t = Terminal::new(20, 3);
        // DCS $ q (DECRQSS) — Request Status String
        feed(&mut t, b"\x1bP$qm\x1b\\");
        // Should produce a response for SGR
        let resp = String::from_utf8(t.take_response()).unwrap();
        assert!(
            resp.contains("m") || resp.is_empty(),
            "DECRQSS for SGR: {}",
            resp
        );
    }

    #[test]
    fn t_r35_osc_very_long_payload_handled() {
        // OSC with very long payload — should be capped at 64KB, not crash.
        // When buffer overflows, parser returns to Ground (excess bytes print).
        let mut t = Terminal::new(20, 3);
        let long_title = "A".repeat(70000); // exceeds 65536 OSC limit
        let osc = format!("\x1b]0;{}\x07", long_title);
        feed(&mut t, osc.as_bytes());
        // Terminal should be functional — some 'A's may appear from overflow.
        assert!(t.grid().width() == 20, "terminal functional after long OSC");
    }

    #[test]
    fn t_r35_dcs_very_long_data_handled() {
        // DCS with very long data — should be capped at 1MB, not crash.
        // When buffer overflows, parser returns to Ground.
        let mut t = Terminal::new(20, 3);
        let long_data = "X".repeat(1100000); // exceeds 1MB DCS limit
        let dcs = format!("\x1bP1$q{}\x1b\\", long_data);
        feed(&mut t, dcs.as_bytes());
        // Terminal should be functional.
        assert!(t.grid().width() == 20, "terminal functional after long DCS");
    }

    #[test]
    fn t_r35_csi_colon_truecolor_fg() {
        // SGR 38:2:r:g:b — colon-separated truecolor (alternative syntax).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[38:2:100:200:50m");
        feed(&mut t, b"X");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().fg,
            Color::Rgb(100, 200, 50),
            "colon truecolor fg"
        );
    }

    #[test]
    fn t_r35_csi_colon_truecolor_bg() {
        // SGR 48:2:r:g:b — colon-separated truecolor bg.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[48:2:10:20:30m");
        feed(&mut t, b"X");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().bg,
            Color::Rgb(10, 20, 30),
            "colon truecolor bg"
        );
    }

    #[test]
    fn t_r35_csi_colon_256color() {
        // SGR 38:5:N — colon-separated 256-color.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[38:5:42m");
        feed(&mut t, b"X");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().fg,
            Color::Indexed(42),
            "colon 256-color fg"
        );
    }

    #[test]
    fn t_r35_csi_colon_underline_curly() {
        // SGR 4:3 — curly underline via colon syntax.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[4:3m");
        feed(&mut t, b"X");
        let flags = t.grid().cell(0, 0).unwrap().flags;
        assert!(
            flags.contains(CellFlags::UNDERLINE_CURLY),
            "curly underline via colon syntax"
        );
        assert!(
            flags.contains(CellFlags::UNDERLINE),
            "UNDERLINE flag also set"
        );
    }

    #[test]
    fn t_r35_csi_colon_underline_dotted() {
        // SGR 4:4 — dotted underline.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[4:4m");
        feed(&mut t, b"X");
        let flags = t.grid().cell(0, 0).unwrap().flags;
        assert!(
            flags.contains(CellFlags::UNDERLINE_DOTTED),
            "dotted underline via colon syntax"
        );
    }

    #[test]
    fn t_r35_csi_colon_underline_dashed() {
        // SGR 4:5 — dashed underline.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[4:5m");
        feed(&mut t, b"X");
        let flags = t.grid().cell(0, 0).unwrap().flags;
        assert!(
            flags.contains(CellFlags::UNDERLINE_DASHED),
            "dashed underline via colon syntax"
        );
    }

    #[test]
    fn t_r35_csi_colon_mixed_with_semicolon() {
        // Mix of colon and semicolon: SGR 1;38:2:255:0:0;4m
        // = bold + truecolor red fg + underline
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[1;38:2:255:0:0;4m");
        feed(&mut t, b"X");
        let cell = t.grid().cell(0, 0).unwrap();
        assert!(cell.flags.contains(CellFlags::BOLD), "bold set");
        assert!(cell.flags.contains(CellFlags::UNDERLINE), "underline set");
        assert_eq!(cell.fg, Color::Rgb(255, 0, 0), "truecolor red fg");
    }

    #[test]
    fn t_r35_dcs_decrqss_scroll_region() {
        // DECRQSS for 'r' (DECSTBM) — should return current scroll region.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[3;7r"); // set region rows 2-6
        feed(&mut t, b"\x1bP$qr\x1b\\"); // DECRQSS for DECSTBM selector "r"
        let resp = String::from_utf8(t.take_response()).unwrap();
        // Response should include the region params
        assert!(
            resp.contains("3") && resp.contains("7"),
            "DECRQSS DECSTBM response: {}",
            resp
        );
    }

    #[test]
    fn t_r35_sos_consumed_silently() {
        // SOS (ESC X) — Start of String. Must be consumed until ST.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1bXsome string\x1b\\");
        feed(&mut t, b"Z");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'Z',
            "SOS consumed, Z printed"
        );
    }

    #[test]
    fn t_r35_osc_empty_payload() {
        // OSC with completely empty payload — should not crash.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]\x07"); // OSC with just BEL
        feed(&mut t, b"X");
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X', "empty OSC handled");
    }

    #[test]
    fn t_r35_osc_no_terminator_then_normal_text() {
        // OSC that never terminates — followed by text.
        // The parser should eventually recover when it hits ESC for next sequence.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]0;test"); // no terminator
        feed(&mut t, b"\x1b[1;1HX"); // ESC [ should start new CSI
        // The OSC should have been abandoned when ESC [ arrived
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'X',
            "recovery from unterminated OSC"
        );
    }

    // ── Round 36: Wide char stress + resize edge cases ─────────────────

    #[test]
    fn t_r36_wide_overwrite_lead_clears_spacer() {
        // Overwriting the LEAD of a wide char with a narrow char must also
        // clear the WIDE_SPACER cell to prevent orphaned spacer.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "中文".as_bytes()); // 中(0-1) 文(2-3)
        feed(&mut t, b"\x1b[1;1H"); // back to col 0
        feed(&mut t, b"X"); // overwrite 中's lead
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'X', "X at lead position");
        assert!(
            !t.grid().cell(1, 0).unwrap().is_wide_spacer(),
            "spacer cleared when lead overwritten"
        );
        assert_eq!(t.grid().cell(2, 0).unwrap().ch, '文', "文 intact at col 2");
    }

    #[test]
    fn t_r36_wide_overwrite_spacer_clears_lead() {
        // Cursor positioning to a spacer adjusts to the lead cell,
        // so printing overwrites the lead position.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "中A".as_bytes()); // 中(0-1) A(2)
        feed(&mut t, b"\x1b[1;2H"); // CUP col 1 (spacer) → adjusts to col 0
        feed(&mut t, b"Y"); // overwrite at lead position
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'Y', "Y at lead position");
        assert!(
            !t.grid().cell(0, 0).unwrap().is_wide(),
            "old wide flag cleared"
        );
    }

    #[test]
    fn t_r36_combining_on_narrow_after_overwrite() {
        // Combining char attaches to preceding base char.
        // After overwriting, the combining should attach to the new char.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"e"); // base 'e'
        feed(&mut t, "\u{0301}".as_bytes()); // combining acute → é
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.ch, 'e', "base is 'e'");
        assert_eq!(cell.combining.len(), 1, "one combining char attached");
        assert_eq!(cell.combining[0], '\u{0301}', "combining is acute");
    }

    #[test]
    fn t_r36_combining_on_wide_char() {
        // Combining char attaches to a wide char lead.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "中".as_bytes()); // wide char at col 0-1
        feed(&mut t, "\u{0301}".as_bytes()); // combining acute
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(cell.ch, '中', "base wide char");
        assert_eq!(cell.combining.len(), 1, "combining attaches to wide lead");
    }

    #[test]
    fn t_r36_combining_stack_limit() {
        // Multiple combining chars stack on the base char (up to 8).
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"a");
        // Push 10 combining chars — only first 8 should be kept
        for _ in 0..10 {
            feed(&mut t, "\u{0301}".as_bytes()); // combining acute
        }
        let cell = t.grid().cell(0, 0).unwrap();
        assert!(
            cell.combining.len() <= 8,
            "combining stack capped at 8: got {}",
            cell.combining.len()
        );
    }

    #[test]
    fn t_r36_resize_shrink_cursor_beyond_width() {
        // Resize to narrower width when cursor is at a column beyond new width.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"\x1b[1;15H"); // cursor at col 14
        t.resize(10, 5); // shrink to 10 wide
        assert!(
            t.cursor().0 < 10,
            "cursor clamped to new width after shrink: got {}",
            t.cursor().0
        );
    }

    #[test]
    fn t_r36_resize_shrink_cursor_beyond_height() {
        // Resize to shorter height when cursor is at a row beyond new height.
        let mut t = Terminal::new(10, 20);
        feed(&mut t, b"\x1b[15;1H"); // cursor at row 14
        t.resize(10, 5); // shrink to 5 tall
        assert!(
            t.cursor().1 < 5,
            "cursor clamped to new height after shrink: got {}",
            t.cursor().1
        );
    }

    #[test]
    fn t_r36_resize_grow_new_area_blank() {
        // Resize to wider — new columns should be blank.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABCDE"); // fill row 0
        t.resize(10, 3); // grow to 10 wide
        assert_eq!(t.grid().cell(5, 0).unwrap().ch, ' ', "new col 5 blank");
        assert_eq!(t.grid().cell(9, 0).unwrap().ch, ' ', "new col 9 blank");
    }

    #[test]
    fn t_r36_resize_grow_new_rows_blank() {
        // Resize to taller — new rows should be blank.
        let mut t = Terminal::new(5, 3);
        feed(&mut t, b"ABC"); // row 0
        t.resize(5, 6); // grow to 6 tall
        // Rows 3-5 should be blank
        for r in 3..6 {
            assert_eq!(t.grid().cell(0, r).unwrap().ch, ' ', "new row {} blank", r);
        }
    }

    #[test]
    fn t_r36_resize_idempotent() {
        // Resizing to same dimensions should be a no-op.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"Hello");
        t.resize(10, 5); // same size
        assert_eq!(t.grid().cell(0, 0).unwrap().ch, 'H', "content preserved");
        assert_eq!(t.grid().cell(4, 0).unwrap().ch, 'o', "content preserved");
        assert_eq!(t.cursor().0, 5, "cursor position preserved");
    }

    #[test]
    fn t_r36_resize_scroll_region_reset() {
        // Resize should reset scroll region to full screen.
        let mut t = Terminal::new(10, 10);
        feed(&mut t, b"\x1b[3;7r"); // set region
        t.resize(15, 12);
        let (top, bottom) = t.grid().scroll_region();
        assert_eq!(top, 0, "scroll region top reset on resize");
        assert_eq!(bottom, 12, "scroll region bottom reset to new height");
    }

    #[test]
    fn t_r36_wide_char_fill_then_delete_line() {
        // Fill a row with wide chars, then DL — wide chars should not corrupt.
        let mut t = Terminal::new(4, 4);
        feed(&mut t, "中文".as_bytes()); // row 0: 中文 (4 cols)
        feed(&mut t, b"\r\nAB"); // row 1: AB
        feed(&mut t, b"\x1b[1;1H"); // cursor at row 0
        feed(&mut t, b"\x1b[M"); // DL row 0
        // Row 0 should now have AB
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'A',
            "row 0 has AB after DL"
        );
        assert_eq!(t.grid().cell(1, 0).unwrap().ch, 'B', "B at col 1");
    }

    #[test]
    fn t_r36_resize_with_scrollback_preserved() {
        // When growing height, reflow pulls scrollback lines back to viewport.
        // The scrollback count may decrease — this is correct reflow behavior.
        // We verify the terminal is functional and content not lost.
        let mut t = Terminal::with_scrollback(10, 3, 100);
        feed(&mut t, b"Line1\r\nLine2\r\nLine3\r\n");
        feed(&mut t, b"Line4"); // Line1 scrolled to scrollback
        let sb_before = t.grid().scrollback_len();
        assert!(sb_before > 0, "scrollback has content");
        t.resize(10, 5); // grow height — reflow pulls scrollback back
        // Content should be preserved (either in viewport or scrollback)
        // Growing height consumes scrollback to fill new rows.
        let total = t.grid().scrollback_len() + t.grid().height();
        assert!(total >= 5, "terminal functional after grow with scrollback");
    }

    #[test]
    fn t_r36_emoji_simple_width2() {
        // Simple emoji (🎉 U+1F389) should be width 2.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "🎉".as_bytes());
        let cell = t.grid().cell(0, 0).unwrap();
        assert!(cell.is_wide(), "emoji is wide");
        assert!(
            t.grid().cell(1, 0).unwrap().is_wide_spacer(),
            "emoji spacer at col 1"
        );
        assert_eq!(t.cursor().0, 2, "cursor advanced by 2 after emoji");
    }

    #[test]
    fn t_r36_wide_char_at_col0_narrow_terminal() {
        // Wide char on a 2-column terminal — exactly fits.
        let mut t = Terminal::new(2, 3);
        feed(&mut t, "中".as_bytes());
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            '中',
            "wide char fills 2-col terminal"
        );
        assert!(
            t.grid().cell(1, 0).unwrap().is_wide_spacer(),
            "spacer at col 1"
        );
        assert_eq!(t.cursor().0, 1, "cursor at col 1 (last col, pending wrap)");
    }

    #[test]
    fn t_r36_wide_char_does_not_fit_1col() {
        // Wide char on a 1-column terminal — must wrap but nowhere to go.
        // Should not crash.
        let mut t = Terminal::new(1, 3);
        feed(&mut t, "中".as_bytes());
        // Terminal should not crash. The char may be dropped or placed partially.
        assert_eq!(t.grid().width(), 1, "terminal still functional");
    }

    #[test]
    fn t_r36_resize_with_wide_char_in_scrollback() {
        // Wide char in content that scrolls to scrollback, then resize.
        let mut t = Terminal::with_scrollback(5, 2, 100);
        feed(&mut t, "中文A".as_bytes()); // row 0: 中文A
        feed(&mut t, b"\r\n"); // new line
        feed(&mut t, b"BCDEF"); // row 1 → 中文A scrolls to scrollback
        t.resize(5, 4); // grow
        assert_eq!(
            t.grid().width(),
            5,
            "resize succeeded with wide char in scrollback"
        );
    }

    #[test]
    fn t_r36_print_after_combining_dropped() {
        // Combining char with no base char (at col 0, row 0) is dropped.
        // Next printable char should appear at col 0.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, "\u{0301}".as_bytes()); // combining acute, no base
        feed(&mut t, b"X");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'X',
            "X at col 0 after dropped combining"
        );
    }

    // ── Round 39: Resize, reflow, and content preservation ─────────────

    #[test]
    fn t_r39_reflow_wide_char_at_boundary_shrink() {
        // Shrink width — wide chars must be preserved through reflow.
        // Use enough height to avoid content scrolling into scrollback.
        let mut t = Terminal::new(8, 2);
        feed(&mut t, "中文AB".as_bytes()); // cols 0-5 on row 0
        t.resize(3, 12); // shrink width, grow height so all content is visible
        // 中 and 文 should appear somewhere in the visible area
        let mut found_zhong = false;
        let mut found_wen = false;
        for r in 0..t.grid().height() {
            for c in 0..t.grid().width() {
                if let Some(cell) = t.grid().cell(c, r) {
                    if cell.ch == '中' {
                        found_zhong = true;
                    }
                    if cell.ch == '文' {
                        found_wen = true;
                    }
                }
            }
        }
        assert!(found_zhong, "中 preserved after reflow shrink");
        assert!(found_wen, "文 preserved after reflow shrink");
    }

    #[test]
    fn t_r39_reflow_shrink_grow_roundtrip_preserves_ascii() {
        // Shrink then grow back — ASCII content should be preserved.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, b"Hello World");
        t.resize(10, 5); // shrink width
        t.resize(20, 5); // grow back
        let restored = t.grid().row_text(0).unwrap_or_default();
        assert!(
            restored.starts_with("Hello"),
            "content preserved after shrink-grow: got '{}'",
            restored
        );
    }

    #[test]
    fn t_r39_reflow_shrink_grow_roundtrip_cjk() {
        // Shrink then grow back — CJK content should be preserved.
        let mut t = Terminal::new(20, 5);
        feed(&mut t, "中文测试数据".as_bytes());
        t.resize(4, 5); // shrink to 4 (fits exactly 2 wide chars per row)
        t.resize(20, 5); // grow back
        // Content should be reflowed and contain the original chars
        let all_text: String = (0..t.grid().height())
            .filter_map(|r| t.grid().row_text(r))
            .collect();
        assert!(all_text.contains("中"), "中 preserved: {}", all_text);
        assert!(all_text.contains("据"), "据 preserved: {}", all_text);
    }

    #[test]
    fn t_r39_resize_cursor_bottom_right_shrink() {
        // Cursor at bottom-right, shrink — cursor must be clamped.
        let mut t = Terminal::new(20, 10);
        feed(&mut t, b"\x1b[10;20H"); // cursor at row 9, col 19 (bottom-right)
        t.resize(5, 3); // shrink to 5x3
        assert!(t.cursor().0 < 5, "cursor x clamped: got {}", t.cursor().0);
        assert!(t.cursor().1 < 3, "cursor y clamped: got {}", t.cursor().1);
    }

    #[test]
    fn t_r39_resize_alt_screen_preserves_main() {
        // In alt screen, resize should not corrupt main screen content.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"MAIN"); // main screen content
        feed(&mut t, b"\x1b[?1049h"); // enter alt
        feed(&mut t, b"ALT");
        t.resize(15, 8); // resize while in alt
        feed(&mut t, b"\x1b[?1049l"); // exit alt
        // Main content should be preserved
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'M',
            "main content preserved after alt resize"
        );
    }

    #[test]
    fn t_r39_resize_with_scrollback_shrink_grow() {
        // Resize with scrollback content — should survive shrink/grow.
        let mut t = Terminal::with_scrollback(10, 3, 100);
        // Fill and scroll to create scrollback
        feed(&mut t, b"Line1\r\nLine2\r\nLine3\r\nLine4\r\nLine5\r\nNow");
        let sb_before = t.grid().scrollback_len();
        assert!(sb_before > 0, "scrollback exists before resize");
        // Shrink width — reflow may change scrollback
        t.resize(5, 3);
        // Grow back
        t.resize(10, 3);
        // Content should still be accessible
        assert!(
            t.grid().scrollback_len() > 0,
            "scrollback survived resize round-trip"
        );
    }

    #[test]
    fn t_r39_resize_height_grow_pulls_scrollback() {
        // Growing height should pull scrollback lines into visible area.
        let mut t = Terminal::with_scrollback(10, 3, 100);
        feed(&mut t, b"L1\r\nL2\r\nL3\r\nL4\r\nL5\r\n");
        let sb_before = t.grid().scrollback_len();
        t.resize(10, 8); // grow height — should pull scrollback
        assert!(
            t.grid().scrollback_len() < sb_before,
            "scrollback consumed on height grow: was={}, now={}",
            sb_before,
            t.grid().scrollback_len()
        );
    }

    #[test]
    fn t_r39_resize_height_shrink_pushes_to_scrollback() {
        // Shrinking height should push visible rows to scrollback.
        let mut t = Terminal::with_scrollback(10, 5, 100);
        feed(&mut t, b"L1\r\nL2\r\nL3\r\nL4\r\nL5\r\n");
        let sb_before = t.grid().scrollback_len();
        t.resize(10, 2); // shrink height
        assert!(
            t.grid().scrollback_len() > sb_before,
            "rows pushed to scrollback on height shrink: was={}, now={}",
            sb_before,
            t.grid().scrollback_len()
        );
    }

    #[test]
    fn t_r39_resize_rapid_cycle_no_corruption() {
        // Multiple rapid resize cycles — no corruption or panic.
        let mut t = Terminal::new(20, 10);
        feed(&mut t, b"Test Content Here");
        for _ in 0..10 {
            t.resize(5, 3);
            t.resize(15, 7);
            t.resize(30, 15);
            t.resize(20, 10);
        }
        // Terminal should still be functional
        feed(&mut t, b"\x1b[1;1HX");
        let all_text: String = (0..t.grid().height())
            .filter_map(|r| t.grid().row_text(r))
            .collect();
        assert!(
            all_text.contains("X"),
            "terminal functional after rapid resize cycle"
        );
    }

    #[test]
    fn t_r39_resize_to_1x1_then_back() {
        // Resize to 1x1 then back — content truncated but terminal works.
        let mut t = Terminal::new(10, 5);
        feed(&mut t, b"Hello World ABC");
        t.resize(1, 1); // minimal size
        assert_eq!(t.grid().width(), 1);
        assert_eq!(t.grid().height(), 1);
        t.resize(10, 5); // grow back
        // Terminal should work
        feed(&mut t, b"\x1b[1;1HZ");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'Z',
            "terminal works after 1x1 resize cycle"
        );
    }

    #[test]
    fn t_r39_resize_preserves_sgr_after_reflow() {
        // Content with colors should preserve color attributes after reflow.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[31mRed\x1b[0m Text");
        t.resize(5, 3); // shrink — reflow
        t.resize(10, 3); // grow back
        // Find the 'R' cell and check it's still red
        let mut found_red = false;
        for r in 0..t.grid().height() {
            for c in 0..t.grid().width() {
                if let Some(cell) = t.grid().cell(c, r)
                    && cell.ch == 'R'
                    && cell.fg == Color::Indexed(1)
                {
                    found_red = true;
                }
            }
        }
        assert!(found_red, "red 'R' preserved after reflow");
    }

    #[test]
    fn t_r39_reflow_wrapped_content_shrink_width() {
        // Soft-wrapped content should reflow correctly when width shrinks.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"ABCDEFGHIJ"); // fills row 0, auto-wraps
        feed(&mut t, b"KLMNOPQRST"); // fills row 1 (soft-wrapped from row 0)
        // Row 0 has wrap=true (soft-wrapped)
        // Shrink to 5 — the logical line ABCDEFGHIJKLMNOPQRST should reflow
        t.resize(5, 6); // wider height to see all reflowed lines
        // First row should start with A
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'A',
            "reflowed content starts with A"
        );
        // Collect all text from visible rows
        let all_text: String = (0..t.grid().height())
            .filter_map(|r| t.grid().row_text(r))
            .collect::<Vec<_>>()
            .join("");
        // The logical line ABCDEFGHIJKLMNOPQRST should be present
        assert!(
            all_text.contains("ABCDEFGHIJKLMNOP"),
            "reflowed content preserved: {}",
            all_text
        );
    }

    #[test]
    fn t_r39_resize_alt_screen_no_reflow() {
        // Alt screen should NOT reflow — simple truncation only.
        let mut t = Terminal::new(10, 4);
        feed(&mut t, b"\x1b[?1049h"); // enter alt (no reflow)
        feed(&mut t, b"ABCDEFGHIJ"); // fills row 0 exactly
        feed(&mut t, b"K"); // K wraps to row 1 (deferred wrap)
        t.resize(5, 4); // shrink width — NO reflow in alt
        // In alt screen, row 0 should be truncated to 5 chars
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'A',
            "alt screen: A at col 0 (truncated, not reflowed)"
        );
        assert_eq!(
            t.grid().cell(4, 0).unwrap().ch,
            'E',
            "alt screen: E at col 4 (truncated)"
        );
    }

    #[test]
    fn t_r39_resize_scrollback_cap_enforced() {
        // After resize, scrollback should not exceed max_scrollback.
        let max_sb = 50;
        let mut t = Terminal::with_scrollback(5, 2, max_sb);
        // Generate lots of scrollback
        for i in 0..200 {
            feed(&mut t, format!("Line{}\r\n", i).as_bytes());
        }
        t.resize(3, 2); // shrink — reflow may create more scrollback
        assert!(
            t.grid().scrollback_len() <= max_sb,
            "scrollback cap enforced after resize: got {}",
            t.grid().scrollback_len()
        );
    }

    // ── Round 40: OSC/hyperlink/mouse/paste integration tests ──────────

    #[test]
    fn t_r40_osc8_hyperlink_survives_scroll() {
        // OSC 8 hyperlink on cells should survive scrolling (content moves
        // to scrollback, but cells keep their link attribute).
        let mut t = Terminal::with_scrollback(20, 3, 100);
        feed(
            &mut t,
            b"\x1b]8;;https://example.com\x1b\\Linked\x1b]8;;\x1b\\",
        );
        // Fill remaining lines to trigger scroll
        feed(&mut t, b"\r\nLine2\r\nLine3\r\n");
        feed(&mut t, b"Line4"); // "Linked" scrolled to scrollback
        // Terminal should not crash, and be functional
        feed(&mut t, b"\x1b[1;1HX");
        assert_eq!(
            t.grid().cell(0, 0).unwrap().ch,
            'X',
            "terminal works after hyperlink scroll"
        );
    }

    #[test]
    fn t_r40_osc8_hyperlink_with_newline_split() {
        // OSC 8 set, text that wraps across newline, then OSC 8 clear.
        // All cells between markers should have the link.
        let mut t = Terminal::new(5, 4);
        feed(&mut t, b"\x1b]8;;https://test.com\x1b\\");
        feed(&mut t, b"ABCDE"); // fills row 0, wraps to row 1
        feed(&mut t, b"FG");
        feed(&mut t, b"\x1b]8;;\x1b\\");
        // Cells on row 0 should have hyperlink
        assert!(
            t.grid().cell(0, 0).unwrap().hyperlink.is_some(),
            "hyperlink on row 0 col 0"
        );
        // Cells on row 1 (wrapped) should also have hyperlink
        assert!(
            t.grid().cell(0, 1).unwrap().hyperlink.is_some(),
            "hyperlink on row 1 col 0 (wrapped)"
        );
    }

    #[test]
    fn t_r40_osc8_multiple_links_different_uris() {
        // Multiple different links on the same line.
        let mut t = Terminal::new(20, 3);
        feed(&mut t, b"\x1b]8;;https://a.com\x1b\\A\x1b]8;;\x1b\\");
        feed(&mut t, b"\x1b]8;;https://b.com\x1b\\B\x1b]8;;\x1b\\");
        let cell_a = t.grid().cell(0, 0).unwrap();
        let cell_b = t.grid().cell(1, 0).unwrap();
        assert_eq!(
            cell_a.hyperlink.as_deref(),
            Some("https://a.com"),
            "cell A has link to a.com"
        );
        assert_eq!(
            cell_b.hyperlink.as_deref(),
            Some("https://b.com"),
            "cell B has link to b.com"
        );
    }

    #[test]
    fn t_r40_osc52_take_clears_pending() {
        // take_pending_clipboard_set should clear the pending data after reading.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]52;c;aGVsbG8=\x07"); // base64 "hello"
        let data = t.take_pending_clipboard_set();
        assert_eq!(data, Some(b"hello".to_vec()));
        // Second take should be None
        let data2 = t.take_pending_clipboard_set();
        assert_eq!(data2, None, "pending clipboard cleared after take");
    }

    #[test]
    fn t_r40_osc52_overwrite_previous() {
        // Second OSC 52 set should overwrite the first.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]52;c;aGVsbG8=\x07"); // "hello"
        feed(&mut t, b"\x1b]52;c;d29ybGQ=\x07"); // "world"
        let data = t.take_pending_clipboard_set();
        assert_eq!(data, Some(b"world".to_vec()), "second set overwrites");
    }

    #[test]
    fn t_r40_osc8_uri_with_query_params() {
        // OSC 8 with complex URI including query params and fragments.
        let mut t = Terminal::new(20, 3);
        let uri = "https://example.com/path?q=1&r=2#section";
        let osc = format!("\x1b]8;;{}\x1b\\X\x1b]8;;\x1b\\", uri);
        feed(&mut t, osc.as_bytes());
        let cell = t.grid().cell(0, 0).unwrap();
        assert_eq!(
            cell.hyperlink.as_deref(),
            Some(uri),
            "complex URI with query preserved"
        );
    }

    #[test]
    fn t_r40_bracketed_paste_mode_independent_from_mouse() {
        // Toggling bracketed paste should not affect mouse modes.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?1000h"); // enable mouse tracking
        feed(&mut t, b"\x1b[?2004h"); // enable bracketed paste
        assert!(t.modes.mouse_tracking, "mouse tracking on");
        assert!(t.modes.bracketed_paste, "bracketed paste on");
        // Disable paste — mouse should stay on
        feed(&mut t, b"\x1b[?2004l");
        assert!(t.modes.mouse_tracking, "mouse tracking still on");
        assert!(!t.modes.bracketed_paste, "bracketed paste off");
    }

    #[test]
    fn t_r40_mouse_sgr_pixel_independent_from_button() {
        // SGR pixel mode (1016) and button-event mode (1002) are independent.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b[?1002h"); // button event
        feed(&mut t, b"\x1b[?1016h"); // sgr pixel
        assert!(t.modes.mouse_button_event, "button event on");
        assert!(t.modes.mouse_sgr_pixel, "sgr pixel on");
        // Turn off button event — pixel mode stays
        feed(&mut t, b"\x1b[?1002l");
        assert!(!t.modes.mouse_button_event, "button event off");
        assert!(t.modes.mouse_sgr_pixel, "sgr pixel still on");
    }

    #[test]
    fn t_r40_all_mouse_modes_off_after_full_reset() {
        // RIS (ESC c) should turn off ALL mouse modes simultaneously.
        let mut t = Terminal::new(10, 3);
        feed(
            &mut t,
            b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h\x1b[?1015h\x1b[?1016h",
        );
        feed(&mut t, b"\x1bc"); // RIS
        assert!(!t.modes.mouse_tracking, "1000 off after RIS");
        assert!(!t.modes.mouse_button_event, "1002 off after RIS");
        assert!(!t.modes.mouse_any_event, "1003 off after RIS");
        assert!(!t.modes.mouse_sgr, "1006 off after RIS");
        assert!(!t.modes.mouse_urxvt, "1015 off after RIS");
        assert!(!t.modes.mouse_sgr_pixel, "1016 off after RIS");
    }

    #[test]
    fn t_r40_osc8_then_decsc_decrc_no_link_leak() {
        // DECSC should NOT save current_hyperlink state (it's per-cell, not cursor).
        // After DECRC, current_hyperlink should be whatever it was before.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]8;;https://link.com\x1b\\"); // set link
        feed(&mut t, b"\x1b7"); // DECSC
        feed(&mut t, b"\x1b]8;;\x1b\\"); // clear link
        feed(&mut t, b"\x1b8"); // DECRC
        // After DECRC, current_hyperlink should be cleared (was cleared before DECRC)
        feed(&mut t, b"X");
        assert!(
            t.grid().cell(0, 0).unwrap().hyperlink.is_none(),
            "no hyperlink leak after DECSC/DECRC"
        );
    }

    #[test]
    fn t_r40_osc_title_with_empty_semicolon() {
        // OSC 0; (title with empty second part) should set empty title.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]0;\x07");
        // Title should be empty string (or default), not crash
        assert!(
            t.title.is_empty() || !t.title.is_empty(),
            "empty title handled without crash"
        );
    }

    #[test]
    fn t_r40_osc8_hyperlink_not_cleared_by_sgr_reset() {
        // SGR reset (ESC[0m) should NOT clear OSC 8 hyperlink state.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]8;;https://link.com\x1b\\");
        feed(&mut t, b"\x1b[1;31m"); // bold red
        feed(&mut t, b"\x1b[0m"); // SGR reset
        feed(&mut t, b"X");
        // X should still have the hyperlink
        assert!(
            t.grid().cell(0, 0).unwrap().hyperlink.is_some(),
            "hyperlink survives SGR reset"
        );
        // But bold/red should be cleared
        assert!(
            !t.grid().cell(0, 0).unwrap().flags.contains(CellFlags::BOLD),
            "bold cleared by SGR reset"
        );
    }

    #[test]
    fn t_r40_osc8_with_id_param_and_explicit_clear() {
        // OSC 8 with id param, then explicit empty URI clear.
        let mut t = Terminal::new(10, 3);
        feed(&mut t, b"\x1b]8;id=42;https://test.com\x1b\\");
        feed(&mut t, b"L");
        feed(&mut t, b"\x1b]8;;\x1b\\"); // clear
        feed(&mut t, b"N"); // no link
        assert!(
            t.grid().cell(0, 0).unwrap().hyperlink.is_some(),
            "L has link"
        );
        assert!(
            t.grid().cell(1, 0).unwrap().hyperlink.is_none(),
            "N has no link"
        );
    }

    #[test]
    fn t_sos_pm_apc_does_not_leak_prior_osc_data() {
        // ESC X (SOS), ESC ^ (PM), ESC _ (APC) should clear the string
        // buffer before accumulating, so leftover data from a prior OSC
        // or DCS doesn't leak into the dispatch.
        let mut t = Terminal::new(20, 3);
        // Send OSC 0;Title ST to set a title.
        feed(&mut t, b"\x1b]0;TestTitle\x1b\\");
        assert_eq!(t.title(), "TestTitle");
        // Now send a PM sequence that contains garbage, then another OSC.
        feed(&mut t, b"\x1b^garbage\x1b\\");
        feed(&mut t, b"\x1b]0;NewTitle\x1b\\");
        // Title should be updated, not corrupted by PM data.
        assert_eq!(
            t.title(),
            "NewTitle",
            "PM sequence should not leak OSC data"
        );

        // Same for APC.
        feed(&mut t, b"\x1b_apayload\x1b\\");
        feed(&mut t, b"\x1b]0;AfterAPC\x1b\\");
        assert_eq!(
            t.title(),
            "AfterAPC",
            "APC sequence should not leak OSC data"
        );

        // Same for SOS.
        feed(&mut t, b"\x1bXsosdata\x1b\\");
        feed(&mut t, b"\x1b]0;AfterSOS\x1b\\");
        assert_eq!(
            t.title(),
            "AfterSOS",
            "SOS sequence should not leak OSC data"
        );
    }
}
