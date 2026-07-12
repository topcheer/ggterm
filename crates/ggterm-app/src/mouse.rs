//! Mouse support for the desktop terminal.
//!
//! Handles three concerns:
//!
//! 1. **SGR mouse reporting** — when the terminal enables mouse tracking
//!    (DECSET 1000/1002/1003), mouse events are encoded and sent to the
//!    child process as escape sequences.
//! 2. **Mouse wheel scrolling** — when mouse tracking is *off*, the wheel
//!    scrolls the scrollback buffer.
//! 3. **Text selection** — click-drag selects text; release copies to the
//!    system clipboard (via OSC 52 or the platform clipboard).

/// Mouse button for SGR encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
    Other(u8),
}

impl MouseButton {
    /// SGR button code (the lower 2 bits of the Cb parameter).
    fn sgr_code(&self) -> u8 {
        match self {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::WheelUp => 64,
            MouseButton::WheelDown => 65,
            MouseButton::WheelLeft => 66,
            MouseButton::WheelRight => 67,
            MouseButton::Other(n) => *n,
        }
    }
}

/// Modifier keys held during a mouse event.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MouseModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl MouseModifiers {
    fn sgr_code(&self) -> u8 {
        let mut bits = 0u8;
        if self.shift {
            bits |= 4;
        }
        if self.alt {
            bits |= 8;
        }
        if self.ctrl {
            bits |= 16;
        }
        bits
    }
}

/// Mouse press / release event.
#[derive(Debug, Clone, Copy)]
pub struct MouseEvent {
    pub button: MouseButton,
    pub x: u16, // 0-based column
    pub y: u16, // 0-based row
    pub mods: MouseModifiers,
}

/// Encode a mouse press event using SGR (mode 1006) encoding.
///
/// Format: `CSI < Cb ; Cx ; Cy M`  (press)
/// Format: `CSI < Cb ; Cx ; Cy m`  (release)
///
/// `Cb` = button code + modifier flags.
pub fn encode_sgr_press(ev: &MouseEvent) -> String {
    let cb = ev.button.sgr_code() | ev.mods.sgr_code();
    format!("\x1b[<{cb};{};{}M", ev.x + 1, ev.y + 1)
}

/// Encode a mouse release event using SGR encoding.
pub fn encode_sgr_release(ev: &MouseEvent) -> String {
    let cb = ev.button.sgr_code() | ev.mods.sgr_code();
    format!("\x1b[<{cb};{};{}m", ev.x + 1, ev.y + 1)
}

/// Encode a mouse event using URXVT (mode 1015) encoding.
///
/// Format: `CSI Cb ; Cx ; Cy M`
pub fn encode_urxvt(ev: &MouseEvent, pressed: bool) -> String {
    let cb = ev.button.sgr_code() | (ev.mods.sgr_code() + if pressed { 0 } else { 3 });
    format!("\x1b[{cb};{};{}M", ev.x + 1, ev.y + 1)
}

/// Encode a mouse event using legacy (X10 / mode 1000) encoding.
///
/// Format: `CSI Mb ; Mx ; My M` (all as raw bytes + 32)
/// Only works for coordinates 0..=222.
pub fn encode_legacy(ev: &MouseEvent) -> Option<Vec<u8>> {
    if ev.x + 32 > 255 || ev.y + 32 > 255 {
        return None;
    }
    let cb = ev.button.sgr_code() | ev.mods.sgr_code();
    let b = cb + 32;
    let x = ev.x as u8 + 32;
    let y = ev.y as u8 + 32;
    Some(vec![0x1b, b'[', b, x, y, b'M'])
}

/// Encode a mouse motion event (used in modes 1002/1003).
///
/// `pressed` is true when the button is held during motion.
pub fn encode_sgr_motion(ev: &MouseEvent, pressed: bool) -> String {
    // For motion events, bit 6 (32) is added to Cb.
    let cb = ev.button.sgr_code() | 32 | ev.mods.sgr_code();
    if pressed {
        format!("\x1b[<{cb};{};{}M", ev.x + 1, ev.y + 1)
    } else {
        format!("\x1b[<{cb};{};{}m", ev.x + 1, ev.y + 1)
    }
}

/// Determine whether a mouse motion event should be reported.
///
/// - Mode 1003 (any-event): report all motion
/// - Mode 1002 (button-event): report only when a button is held
pub fn should_report_motion(any_event: bool, button_event: bool, button_held: bool) -> bool {
    any_event || (button_event && button_held)
}

/// Encode a mouse event using the active encoding mode.
///
/// Returns the bytes to send to the PTY, or `None` if the event
/// should not be reported (e.g. legacy encoding with out-of-range coords).
pub fn encode_mouse_event(
    ev: &MouseEvent,
    sgr: bool,
    urxvt: bool,
    pressed: bool,
) -> Option<Vec<u8>> {
    let s = if sgr {
        if pressed {
            encode_sgr_press(ev)
        } else {
            encode_sgr_release(ev)
        }
    } else if urxvt {
        encode_urxvt(ev, pressed)
    } else {
        // Legacy encoding (X10 / 1000)
        String::from_utf8(encode_legacy(ev)?).ok()?
    };
    Some(s.into_bytes())
}

/// Encode a mouse event using SGR-pixel (mode 1016) encoding.
///
/// Same format as SGR (mode 1006) but coordinates are in pixels.
/// Programs like kitty/image viewers use pixel-level mouse tracking.
///
/// Format: `CSI < Cb ; Px ; Py M/m` (Px,Py are pixel coordinates)
pub fn encode_mouse_event_pixel(
    ev: &MouseEvent,
    pixel_x: u16,
    pixel_y: u16,
    pressed: bool,
) -> Option<Vec<u8>> {
    let cb = ev.button.sgr_code() | ev.mods.sgr_code();
    let suffix = if pressed { 'M' } else { 'm' };
    let s = format!("\x1b[<{cb};{pixel_x};{pixel_y}{suffix}");
    Some(s.into_bytes())
}

// ═════════════════════════════════════════════════════════════════════════
//  Text Selection
// ═════════════════════════════════════════════════════════════════════════

/// How text selection extends when dragging the mouse after a multi-click.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DragSelectMode {
    /// Normal character-by-character selection (single click + drag).
    #[default]
    Char,
    /// Word-by-word selection (after double-click + drag).
    Word,
    /// Line-by-line selection (after triple-click + drag).
    Line,
}

/// Selection state machine for click-drag text selection.
#[derive(Debug, Clone, Default)]
pub struct MouseSelection {
    /// Start cell (col, row), or None if no selection active.
    pub start: Option<(u16, u16)>,
    /// Current end cell (col, row).
    pub end: Option<(u16, u16)>,
    /// True while the user is actively dragging.
    pub dragging: bool,
    /// True when Alt-dragging to select a rectangular block.
    pub block_mode: bool,
}

impl MouseSelection {
    /// Begin a new selection at the given cell.
    pub fn start(&mut self, x: u16, y: u16) {
        self.start = Some((x, y));
        self.end = Some((x, y));
        self.dragging = true;
    }

    /// Begin a new block (rectangular) selection at the given cell.
    pub fn start_block(&mut self, x: u16, y: u16) {
        self.start = Some((x, y));
        self.end = Some((x, y));
        self.dragging = true;
        self.block_mode = true;
    }

    /// Extend the selection to a new end cell while dragging.
    pub fn extend(&mut self, x: u16, y: u16) {
        if self.dragging {
            self.end = Some((x, y));
        }
    }

    /// Finalize the selection (mouse released).
    pub fn finish(&mut self) {
        self.dragging = false;
    }

    /// Re-enable dragging state (for word-drag / line-drag after select_word_at / select_line_at).
    pub fn resume_dragging(&mut self) {
        self.dragging = true;
    }

    /// Clear the selection entirely.
    pub fn clear(&mut self) {
        self.start = None;
        self.end = None;
        self.dragging = false;
        self.block_mode = false;
    }

    /// Select all cells in the grid (0,0) to (last_col, last_row).
    pub fn select_all(&mut self, grid: &ggterm_core::grid::Grid) {
        let last_col = grid.width().saturating_sub(1) as u16;
        let last_row = grid.height().saturating_sub(1) as u16;
        self.start = Some((0, 0));
        self.end = Some((last_col, last_row));
        self.dragging = false;
        self.block_mode = false;
    }

    /// Return true if a non-empty selection exists.
    pub fn is_active(&self) -> bool {
        match (self.start, self.end) {
            (Some(s), Some(e)) => s != e || self.dragging,
            _ => false,
        }
    }

    /// Return the selection as an ordered (start, end) pair where
    /// start <= end in (row, col) order.
    pub fn normalized(&self) -> Option<((u16, u16), (u16, u16))> {
        let (sx, sy) = self.start?;
        let (ex, ey) = self.end?;
        if (sy, sx) <= (ey, ex) {
            Some(((sx, sy), (ex, ey)))
        } else {
            Some(((ex, ey), (sx, sy)))
        }
    }

    /// Return the block selection as (col_min, row_min, col_max, row_max).
    /// Only meaningful when `block_mode` is true.
    pub fn block_rect(&self) -> Option<(u16, u16, u16, u16)> {
        let (sx, sy) = self.start?;
        let (ex, ey) = self.end?;
        Some((sx.min(ex), sy.min(ey), sx.max(ex), sy.max(ey)))
    }
}

// ═════════════════════════════════════════════════════════════════════════
//  Coordinate conversion
// ═════════════════════════════════════════════════════════════════════════

/// Convert pixel coordinates to terminal cell coordinates.
pub fn pixel_to_cell(px: f64, py: f64, cell_width: f64, cell_height: f64) -> (u16, u16) {
    let col = (px / cell_width).floor() as u16;
    let row = (py / cell_height).floor() as u16;
    (col, row)
}

// ═════════════════════════════════════════════════════════════════════════
//  URL Detection (P17-C)
// ═════════════════════════════════════════════════════════════════════════

/// Find a URL in a line of text that overlaps the given column position.
///
/// Returns `(byte_start, byte_end, url)` if a URL is found covering `col`,
/// or `None` if no URL overlaps the position.
///
/// Detects `http://`, `https://`, `ftp://`, and `www.` prefixed URLs.
pub fn detect_url_at_position(line: &str, col: usize) -> Option<(usize, usize, String)> {
    for (start, url) in find_urls(line) {
        // URL occupies columns [start, start + url.chars().count())
        let end = start + url.chars().count();
        if col >= start && col < end {
            return Some((start, end, url));
        }
    }
    None
}

/// Find all URLs in a line of text.
///
/// Returns a list of `(char_column_start, url_string)` pairs.
/// Uses a simple state-machine parser rather than regex to avoid
/// adding a regex dependency.
pub fn find_urls(line: &str) -> Vec<(usize, String)> {
    let mut results = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let schemes = ["https://", "http://", "ftp://", "git://", "ssh://", "www."];

    // Common TLDs for bare hostname detection (e.g. github.com/user/repo)
    let tlds = [
        ".com", ".org", ".net", ".io", ".dev", ".app", ".xyz", ".ru", ".de", ".uk", ".jp", ".cn",
        ".fr", ".nl", ".edu", ".gov", ".mil", ".info", ".me", ".tv", ".cc", ".eu",
    ];

    let mut i = 0;
    while i < chars.len() {
        // Check if a known scheme starts at this position.
        let remaining: String = chars[i..].iter().collect();
        let matched = schemes.iter().find(|s| {
            remaining
                .to_ascii_lowercase()
                .starts_with(&s.to_ascii_lowercase())
        });

        if let Some(scheme) = matched {
            // Scan forward to find the end of the URL.
            let url_start = i;
            let mut j = i + scheme.len();
            while j < chars.len() && is_url_char(chars[j]) {
                j += 1;
            }
            // Trim trailing punctuation that's unlikely part of the URL.
            while j > url_start + scheme.len()
                && matches!(chars[j - 1], '.' | ',' | ';' | ')' | ']' | '}' | '\'')
            {
                j -= 1;
            }

            let url: String = chars[url_start..j].iter().collect();
            if url.len() > scheme.len() {
                results.push((url_start, url));
            }
            i = j;
        } else {
            // Try bare hostname detection: hostname + TLD + path/port
            // Must be at word boundary (preceded by space/punct/start)
            let at_word_boundary = i == 0 || !chars[i - 1].is_alphanumeric() && chars[i - 1] != '.';
            if at_word_boundary && chars[i].is_alphanumeric() {
                // Scan forward reading hostname chars (alnum, dot, hyphen)
                let mut host_end = i;
                while host_end < chars.len()
                    && (chars[host_end].is_alphanumeric()
                        || chars[host_end] == '.'
                        || chars[host_end] == '-')
                {
                    host_end += 1;
                }
                // Check if the hostname part ends with a known TLD
                let host: String = chars[i..host_end].iter().collect();
                let host_lower = host.to_ascii_lowercase();
                let has_tld = tlds
                    .iter()
                    .any(|t| host_lower.ends_with(t) && host_lower.len() > t.len());
                if has_tld {
                    // Continue scanning for path/query/port after hostname
                    let mut j = host_end;
                    while j < chars.len() && is_url_char(chars[j]) {
                        j += 1;
                    }
                    // Trim trailing punctuation
                    while j > host_end
                        && matches!(chars[j - 1], '.' | ',' | ';' | ')' | ']' | '}' | '\'')
                    {
                        j -= 1;
                    }
                    // Require path/port after hostname (avoid linking bare "example.com")
                    let after_host: String = chars[host_end..j].iter().collect();
                    if after_host.starts_with('/') || after_host.starts_with(':') {
                        let url: String = chars[i..j].iter().collect();
                        results.push((i, url));
                        i = j;
                        continue;
                    }
                }
            }
            i += 1;
        }
    }
    results
}

/// Characters that are valid inside a URL path.
pub(crate) fn is_url_char(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(
            c,
            '/' | ':'
                | '.'
                | '-'
                | '_'
                | '~'
                | '?'
                | '#'
                | '['
                | ']'
                | '@'
                | '!'
                | '$'
                | '&'
                | '\''
                | '('
                | ')'
                | '*'
                | '+'
                | ','
                | ';'
                | '='
                | '%'
        )
}

/// Open a URL in the platform's default browser.
///
/// Uses `open` on macOS, `xdg-open` on Linux, `start` on Windows.
pub fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";
    #[cfg(windows)]
    let program = "cmd";

    #[cfg(windows)]
    {
        let _ = std::process::Command::new(program)
            .args(["/C", "start", url])
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new(program).arg(url).spawn();
    }
}

/// Detect a file path at the given position in a line.
///
/// Returns the path string and its start column if a file path pattern
/// is found. Supports:
/// - Relative paths: `src/main.rs`, `./lib/utils.ts`
/// - Absolute paths: `/usr/local/bin/foo`
/// - Paths with line numbers: `src/main.rs:42`, `file.go:10:5`
/// - Home-relative paths: `~/projects/foo/src/lib.rs:100`
///
/// The path must contain at least one `/` or start with `~` to avoid
/// false positives on plain filenames.
pub fn find_file_path(line: &str, col: usize) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    if col >= chars.len() {
        return None;
    }

    // File path characters: letters, digits, path separators, dots, dashes, etc.
    let is_path_char = |c: char| {
        c.is_alphanumeric() || matches!(c, '/' | '.' | '-' | '_' | '~' | '+' | ':' | '\\')
    };

    // If cursor is on whitespace, nothing to detect.
    if !is_path_char(chars[col]) {
        return None;
    }

    // Scan left for the start of the token.
    let mut start = col;
    while start > 0 && is_path_char(chars[start - 1]) {
        start -= 1;
    }

    // Scan right for the end.
    let mut end = col;
    while end + 1 < chars.len() && is_path_char(chars[end + 1]) {
        end += 1;
    }

    let token: String = chars[start..=end].iter().collect();

    // Must contain '/' or start with '~' to be a plausible path.
    // Bare filenames like "main.rs" are too ambiguous.
    // Exception: paths with `:line:col` suffix that look like compiler output
    // (e.g., "main.rs:42:10" or "lib.rs:15") — these are common in build output.
    let has_slash = token.contains('/');
    let has_tilde = token.starts_with('~');
    let has_line_suffix = token.matches(':').count() >= 1
        && token
            .split(':')
            .nth(1)
            .is_some_and(|s| s.parse::<u32>().is_ok());

    if !has_slash && !has_tilde && !has_line_suffix {
        return None;
    }

    // Must have a plausible file extension or be a directory path.
    // Strip line:col suffix for validation.
    let path_part = token.split(':').next().unwrap_or(&token);
    if path_part.is_empty() {
        return None;
    }

    Some(token)
}

/// Open a file path in the user's editor.
///
/// Parses `path:line:col` format and opens the file using `$VISUAL`,
/// `$EDITOR`, or a platform default.
pub fn open_file_path(path_spec: &str) {
    // Split path:line:col
    let parts: Vec<&str> = path_spec.splitn(3, ':').collect();
    let file_path = parts[0];
    let line = parts.get(1).and_then(|s| s.parse::<u32>().ok());
    let col = parts.get(2).and_then(|s| s.parse::<u32>().ok());

    // Expand ~ to home directory.
    let expanded = if file_path.starts_with('~') {
        if let Some(home) = std::env::var_os("HOME") {
            format!(
                "{}{}",
                home.to_string_lossy(),
                file_path.strip_prefix('~').unwrap_or(file_path)
            )
        } else {
            file_path.to_string()
        }
    } else {
        file_path.to_string()
    };

    // Determine editor command.
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| {
            if cfg!(target_os = "macos") {
                "nano".to_string()
            } else {
                "vi".to_string()
            }
        });

    // Split editor command (may contain args like "code --wait").
    let mut cmd_parts = editor.split_whitespace();
    let cmd = cmd_parts.next().unwrap_or("vi");
    let extra_args: Vec<&str> = cmd_parts.collect();

    // Build the command.
    let mut command = std::process::Command::new(cmd);
    command.args(&extra_args);

    // Add +line:col argument for editors that support it (vim, nano, etc.)
    // For GUI editors (code, subl, zed), use -g line:col or --goto format.
    let is_gui_editor = matches!(
        cmd,
        "code" | "code-insiders" | "cursor" | "subl" | "zed" | "mate" | "atom"
    );

    if is_gui_editor {
        // GUI editors: VS Code uses `file:line:col` with --goto.
        if let Some(ln) = line {
            let goto_arg = if let Some(cn) = col {
                format!("{}:{}:{}", expanded, ln, cn)
            } else {
                format!("{}:{}", expanded, ln)
            };
            if cmd == "code" || cmd == "code-insiders" || cmd == "cursor" {
                command.arg("--goto").arg(goto_arg);
            } else if cmd == "subl" {
                command.arg(format!("{}:{}:{}", expanded, ln, col.unwrap_or(1)));
            } else {
                command.arg(&expanded);
            }
        } else {
            command.arg(&expanded);
        }
    } else {
        // CLI editors: vim, nano, etc. use +line format.
        if let Some(ln) = line {
            let arg = if let Some(cn) = col {
                format!("+{}:{}", ln, cn)
            } else {
                format!("+{}", ln)
            };
            command.arg(arg);
        }
        command.arg(&expanded);
    }

    // On macOS: GUI editors launch directly; CLI editors open in Terminal.app.
    #[cfg(target_os = "macos")]
    {
        if is_gui_editor {
            let _ = command.spawn();
        } else {
            // Open CLI editor in a new Terminal.app window.
            let mut full_cmd = editor.clone();
            if let Some(ln) = line {
                full_cmd.push_str(&format!(" +{}", ln));
            }
            full_cmd.push_str(&format!(" \"{}\"", expanded));
            let script = format!(
                "tell application \"Terminal\" to do script \"{}\"",
                full_cmd.replace('\\', "\\\\").replace('"', "\\\"")
            );
            let _ = std::process::Command::new("osascript")
                .args(["-e", &script])
                .spawn();
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = command.spawn();
    }

    log::info!(
        "Opening file: {} (editor: {}, line: {:?})",
        expanded,
        cmd,
        line
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── SGR encoding ──────────────────────────────────────────────────

    #[test]
    fn test_sgr_press_left_click() {
        let ev = MouseEvent {
            button: MouseButton::Left,
            x: 5,
            y: 10,
            mods: MouseModifiers::default(),
        };
        let s = encode_sgr_press(&ev);
        assert_eq!(s, "\x1b[<0;6;11M");
    }

    #[test]
    fn test_sgr_release_left_click() {
        let ev = MouseEvent {
            button: MouseButton::Left,
            x: 5,
            y: 10,
            mods: MouseModifiers::default(),
        };
        let s = encode_sgr_release(&ev);
        assert_eq!(s, "\x1b[<0;6;11m");
    }

    #[test]
    fn test_sgr_with_modifiers() {
        let ev = MouseEvent {
            button: MouseButton::Right,
            x: 0,
            y: 0,
            mods: MouseModifiers {
                shift: true,
                ctrl: true,
                alt: false,
            },
        };
        // Right = 2, shift = 4, ctrl = 16 → 2 + 4 + 16 = 22
        let s = encode_sgr_press(&ev);
        assert_eq!(s, "\x1b[<22;1;1M");
    }

    #[test]
    fn test_sgr_wheel_up() {
        let ev = MouseEvent {
            button: MouseButton::WheelUp,
            x: 20,
            y: 30,
            mods: MouseModifiers::default(),
        };
        // WheelUp = 64
        let s = encode_sgr_press(&ev);
        assert_eq!(s, "\x1b[<64;21;31M");
    }

    #[test]
    fn test_sgr_wheel_down() {
        let ev = MouseEvent {
            button: MouseButton::WheelDown,
            x: 0,
            y: 0,
            mods: MouseModifiers::default(),
        };
        let s = encode_sgr_press(&ev);
        assert_eq!(s, "\x1b[<65;1;1M");
    }

    // ── Motion encoding ───────────────────────────────────────────────

    #[test]
    fn test_sgr_motion_pressed() {
        let ev = MouseEvent {
            button: MouseButton::Left,
            x: 3,
            y: 7,
            mods: MouseModifiers::default(),
        };
        // Motion adds bit 32: 0 + 32 = 32
        let s = encode_sgr_motion(&ev, true);
        assert_eq!(s, "\x1b[<32;4;8M");
    }

    #[test]
    fn test_sgr_motion_released() {
        let ev = MouseEvent {
            button: MouseButton::Left,
            x: 3,
            y: 7,
            mods: MouseModifiers::default(),
        };
        let s = encode_sgr_motion(&ev, false);
        assert_eq!(s, "\x1b[<32;4;8m");
    }

    // ── Motion filtering ──────────────────────────────────────────────

    #[test]
    fn test_should_report_any_event() {
        assert!(should_report_motion(true, false, false));
        assert!(should_report_motion(true, false, true));
    }

    #[test]
    fn test_should_report_button_event_with_held() {
        assert!(should_report_motion(false, true, true));
        assert!(!should_report_motion(false, true, false));
    }

    #[test]
    fn test_should_report_neither_mode() {
        assert!(!should_report_motion(false, false, true));
    }

    // ── Legacy encoding ───────────────────────────────────────────────

    #[test]
    fn test_legacy_encoding() {
        let ev = MouseEvent {
            button: MouseButton::Left,
            x: 0,
            y: 0,
            mods: MouseModifiers::default(),
        };
        let bytes = encode_legacy(&ev).unwrap();
        assert_eq!(bytes, vec![0x1b, b'[', 32, 32, 32, b'M']);
    }

    #[test]
    fn test_legacy_encoding_out_of_range() {
        let ev = MouseEvent {
            button: MouseButton::Left,
            x: 300,
            y: 0,
            mods: MouseModifiers::default(),
        };
        assert!(encode_legacy(&ev).is_none());
    }

    // ── URXVT encoding ────────────────────────────────────────────────

    #[test]
    fn test_urxvt_encoding() {
        let ev = MouseEvent {
            button: MouseButton::Left,
            x: 5,
            y: 10,
            mods: MouseModifiers::default(),
        };
        let s = encode_urxvt(&ev, true);
        assert!(s.starts_with("\x1b["));
        assert!(s.ends_with('M'));
    }

    // ── encode_mouse_event dispatcher ─────────────────────────────────

    #[test]
    fn test_encode_mouse_event_sgr() {
        let ev = MouseEvent {
            button: MouseButton::Left,
            x: 0,
            y: 0,
            mods: MouseModifiers::default(),
        };
        let bytes = encode_mouse_event(&ev, true, false, true).unwrap();
        assert_eq!(bytes, b"\x1b[<0;1;1M");
    }

    #[test]
    fn test_encode_mouse_event_legacy_none() {
        let ev = MouseEvent {
            button: MouseButton::Left,
            x: 300,
            y: 0,
            mods: MouseModifiers::default(),
        };
        // Legacy + out of range → None
        assert!(encode_mouse_event(&ev, false, false, true).is_none());
    }

    // ── Selection ─────────────────────────────────────────────────────

    #[test]
    fn test_selection_basic() {
        let mut sel = MouseSelection::default();
        assert!(!sel.is_active());

        sel.start(5, 3);
        assert!(sel.dragging);
        assert!(sel.is_active());

        sel.extend(10, 3);
        let ((sx, sy), (ex, ey)) = sel.normalized().unwrap();
        assert_eq!((sx, sy), (5, 3));
        assert_eq!((ex, ey), (10, 3));

        sel.finish();
        assert!(!sel.dragging);
        assert!(sel.is_active());

        sel.clear();
        assert!(!sel.is_active());
    }

    #[test]
    fn test_selection_reversed() {
        let mut sel = MouseSelection::default();
        sel.start(10, 5);
        sel.extend(3, 2);

        let ((sx, sy), (ex, ey)) = sel.normalized().unwrap();
        // Start should be earlier (top-left) even though user dragged backward
        assert_eq!((sx, sy), (3, 2));
        assert_eq!((ex, ey), (10, 5));
    }

    #[test]
    fn test_selection_single_cell() {
        let mut sel = MouseSelection::default();
        sel.start(5, 5);
        sel.finish();
        // Single-cell selection is active while not dragging? No — single cell
        // with same start/end and not dragging means click without drag.
        assert!(!sel.is_active());
    }

    #[test]
    fn test_selection_single_cell_dragging() {
        let mut sel = MouseSelection::default();
        sel.start(5, 5);
        // While dragging, even a same-cell "selection" is active
        assert!(sel.is_active());
        sel.finish();
        // After release, single cell is not a selection
        assert!(!sel.is_active());
    }

    // ── Pixel conversion ──────────────────────────────────────────────

    #[test]
    fn test_pixel_to_cell() {
        assert_eq!(pixel_to_cell(0.0, 0.0, 8.0, 16.0), (0, 0));
        assert_eq!(pixel_to_cell(7.9, 15.9, 8.0, 16.0), (0, 0));
        assert_eq!(pixel_to_cell(8.0, 16.0, 8.0, 16.0), (1, 1));
        assert_eq!(pixel_to_cell(80.0, 160.0, 8.0, 16.0), (10, 10));
    }

    // ── URL detection (P17-C) ───────────────────────────────────────────

    #[test]
    fn test_detect_url_in_text() {
        let line = "Check https://example.com/path?q=1 for details";
        // URL starts at char column 6
        let result = detect_url_at_position(line, 8);
        assert!(result.is_some());
        let (start, end, url) = result.unwrap();
        assert_eq!(start, 6);
        assert!(url.contains("https://example.com/path?q=1"));
        assert!(end > start);
    }

    #[test]
    fn test_detect_url_multiple() {
        let line = "Visit http://a.com and https://b.com";
        let urls = find_urls(line);
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0].1, "http://a.com");
        assert_eq!(urls[1].1, "https://b.com");
    }

    #[test]
    fn test_detect_url_at_position_inside() {
        let line = "See https://rust-lang.org/crates for more";
        // Position inside the URL
        assert!(detect_url_at_position(line, 15).is_some());
        // Position at URL start
        assert!(detect_url_at_position(line, 4).is_some());
    }

    #[test]
    fn test_detect_url_at_position_outside() {
        let line = "See https://rust-lang.org/crates for more";
        // Position before the URL
        assert!(detect_url_at_position(line, 0).is_none());
        // Position after the URL
        assert!(detect_url_at_position(line, 38).is_none());
    }

    #[test]
    fn test_no_url_in_plain_text() {
        assert!(find_urls("just some regular text").is_empty());
        assert!(find_urls("").is_empty());
        assert!(detect_url_at_position("hello world", 3).is_none());
    }

    #[test]
    fn test_url_trailing_punctuation_trimmed() {
        let urls = find_urls("Link: https://example.com.");
        assert_eq!(urls.len(), 1);
        // Trailing dot should be trimmed
        assert_eq!(urls[0].1, "https://example.com");

        let urls2 = find_urls("See (https://example.com) here");
        assert_eq!(urls2.len(), 1);
        // Trailing ) should be trimmed
        assert_eq!(urls2[0].1, "https://example.com");
    }

    #[test]
    fn test_www_url_detected() {
        let urls = find_urls("Go to www.rust-lang.org now");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].1, "www.rust-lang.org");
    }

    #[test]
    fn test_bare_hostname_with_path() {
        let urls = find_urls("Clone from github.com/user/repo");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].1, "github.com/user/repo");
    }

    #[test]
    fn test_bare_hostname_with_port() {
        let urls = find_urls("Connect to example.com:8080/api");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].1, "example.com:8080/api");
    }

    #[test]
    fn test_bare_hostname_no_path_not_linked() {
        // Bare hostname without path or port should NOT be detected as URL
        let urls = find_urls("Welcome to example.com today");
        assert!(
            urls.is_empty(),
            "bare hostname without path should not be URL: {urls:?}"
        );
    }

    #[test]
    fn test_git_ssh_urls_detected() {
        let urls = find_urls("Clone: git://github.com/rust-lang/rust.git");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].1, "git://github.com/rust-lang/rust.git");

        let urls2 = find_urls("SSH: ssh://user@host:22/path");
        assert_eq!(urls2.len(), 1);
        assert_eq!(urls2[0].1, "ssh://user@host:22/path");
    }

    #[test]
    fn test_pixel_to_cell_negative() {
        // Negative coords clamp to 0
        assert_eq!(pixel_to_cell(-5.0, -5.0, 8.0, 16.0), (0, 0));
    }

    // ── File path detection ──────────────────────────────────────────

    #[test]
    fn test_find_file_path_relative() {
        let line = "error in src/main.rs:42:10";
        let path = find_file_path(line, 14).unwrap(); // cursor on 'm' in main.rs
        assert_eq!(path, "src/main.rs:42:10");
    }

    #[test]
    fn test_find_file_path_absolute() {
        let line = "Error: /usr/local/bin/foo.go:10";
        let path = find_file_path(line, 10).unwrap(); // cursor in path
        assert_eq!(path, "/usr/local/bin/foo.go:10");
    }

    #[test]
    fn test_find_file_path_home() {
        let line = "See ~/projects/ggterm/src/lib.rs:100";
        let path = find_file_path(line, 10).unwrap(); // cursor on 'p'
        assert!(path.starts_with("~/projects/"));
        assert!(path.contains("lib.rs:100"));
    }

    #[test]
    fn test_find_file_path_no_slash_no_line_rejected() {
        // Bare filename without directory or line number should not be detected.
        let line = "error in main.rs here";
        assert!(find_file_path(line, 10).is_none());
    }

    #[test]
    fn test_find_file_path_whitespace_rejected() {
        let line = "some text with spaces";
        assert!(find_file_path(line, 5).is_none());
    }

    #[test]
    fn test_find_file_path_line_only() {
        let line = "  at ./lib/parser.rs";
        let path = find_file_path(line, 8).unwrap(); // cursor on 'l' in lib
        assert_eq!(path, "./lib/parser.rs");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Block selection tests
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn test_block_selection_start_and_extend() {
        let mut sel = MouseSelection::default();
        sel.start_block(5, 2);
        assert!(sel.block_mode);
        assert!(sel.dragging);
        assert_eq!(sel.start, Some((5, 2)));
        assert_eq!(sel.end, Some((5, 2)));

        sel.extend(10, 6);
        assert_eq!(sel.end, Some((10, 6)));
    }

    #[test]
    fn test_block_rect_normal() {
        let mut sel = MouseSelection::default();
        sel.start_block(3, 1);
        sel.extend(8, 5);
        let (x0, y0, x1, y1) = sel.block_rect().unwrap();
        assert_eq!((x0, y0), (3, 1));
        assert_eq!((x1, y1), (8, 5));
    }

    #[test]
    fn test_block_rect_reversed_drag() {
        // User drags from bottom-right to top-left.
        let mut sel = MouseSelection::default();
        sel.start_block(10, 8);
        sel.extend(2, 1);
        let (x0, y0, x1, y1) = sel.block_rect().unwrap();
        assert_eq!((x0, y0), (2, 1));
        assert_eq!((x1, y1), (10, 8));
    }

    #[test]
    fn test_block_selection_clear_resets_mode() {
        let mut sel = MouseSelection::default();
        sel.start_block(5, 5);
        assert!(sel.block_mode);
        sel.clear();
        assert!(!sel.block_mode);
        assert!(!sel.is_active());
    }

    #[test]
    fn test_normal_selection_does_not_set_block() {
        let mut sel = MouseSelection::default();
        sel.start(0, 0);
        assert!(!sel.block_mode);
    }

    #[test]
    fn test_find_file_path_rust_compiler_format() {
        // Rust compiler format: --> src/main.rs:15:5
        let line = "  --> src/main.rs:15:5";
        let col = line.find("main").unwrap();
        assert_eq!(
            find_file_path(line, col),
            Some("src/main.rs:15:5".to_string())
        );
    }
}
