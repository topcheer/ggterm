//! Input encoding: converts keyboard events to ANSI byte sequences
//! for the PTY.
//!
//! This module does NOT depend on winit — it takes simple key descriptions
//! and produces bytes. The winit→InputKey mapping happens in the app layer.

/// Special keys that don't map to a single Unicode char.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialKey {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

/// Keyboard modifier flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// A keyboard input event, independent of any windowing toolkit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputKey {
    /// A regular character key (possibly with modifiers).
    Char(char, KeyModifiers),
    /// A special key (arrows, function keys, etc.).
    Special(SpecialKey, KeyModifiers),
}

impl InputKey {
    /// Convenience: create a Char key with no modifiers.
    pub fn char(c: char) -> Self {
        InputKey::Char(c, KeyModifiers::default())
    }

    /// Convenience: create a Char key with modifiers.
    pub fn char_mod(c: char, mods: KeyModifiers) -> Self {
        InputKey::Char(c, mods)
    }

    /// Convenience: create a Special key with no modifiers.
    pub fn special(k: SpecialKey) -> Self {
        InputKey::Special(k, KeyModifiers::default())
    }

    /// Convenience: create a Special key with modifiers.
    pub fn special_mod(k: SpecialKey, mods: KeyModifiers) -> Self {
        InputKey::Special(k, mods)
    }
}

impl InputKey {
    /// Convenience: create a Char key with no modifiers.
    pub fn char(c: char) -> Self {
        InputKey::Char(c, KeyModifiers::default())
    }

    /// Convenience: create a Char key with modifiers.
    pub fn char_with(c: char, mods: KeyModifiers) -> Self {
        InputKey::Char(c, mods)
    }

    /// Convenience: create a Special key with no modifiers.
    pub fn special(k: SpecialKey) -> Self {
        InputKey::Special(k, KeyModifiers::default())
    }

    /// Convenience: create a Special key with modifiers.
    pub fn special_with(k: SpecialKey, mods: KeyModifiers) -> Self {
        InputKey::Special(k, mods)
    }
}

impl InputKey {
    /// Convenience: create a Char key with no modifiers.
    pub fn char(c: char) -> Self {
        InputKey::Char(c, KeyModifiers::default())
    }

    /// Convenience: create a Char key with modifiers.
    pub fn char_mod(c: char, mods: KeyModifiers) -> Self {
        InputKey::Char(c, mods)
    }

    /// Convenience: create a Special key with no modifiers.
    pub fn special(k: SpecialKey) -> Self {
        InputKey::Special(k, KeyModifiers::default())
    }

    /// Convenience: create a Special key with modifiers.
    pub fn special_mod(k: SpecialKey, mods: KeyModifiers) -> Self {
        InputKey::Special(k, mods)
    }
}

/// Encodes keyboard input into ANSI byte sequences for the PTY.
#[derive(Default)]
pub struct InputEncoder {
    /// Whether cursor keys should send application-mode sequences (DECCKM).
    cursor_app_mode: bool,
}

impl InputEncoder {
    /// Create a new input encoder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set cursor application mode (DECCKM).
    pub fn set_cursor_app_mode(&mut self, on: bool) {
        self.cursor_app_mode = on;
    }

    /// Encode a key press into bytes to write to the PTY.
    pub fn encode(&self, key: &InputKey) -> Vec<u8> {
        match key {
            InputKey::Char(ch, mods) => self.encode_char(*ch, mods),
            InputKey::Special(sk, mods) => self.encode_special(*sk, mods),
        }
    }

    // ── Character keys ──────────────────────────────────────────

    fn encode_char(&self, ch: char, mods: &KeyModifiers) -> Vec<u8> {
        // Named single-char keys
        match ch {
            '\r' | '\n' => return b"\r".to_vec(),
            '\t' => return b"\t".to_vec(),
            '\x08' => return b"\x7f".to_vec(), // Backspace → DEL
            '\x1b' => return b"\x1b".to_vec(), // Escape
            _ => {}
        }

        // Ctrl+letter → control character (0x01–0x1A)
        if mods.ctrl && ch.is_ascii_alphabetic() {
            let byte = (ch.to_ascii_lowercase() as u8) & 0x1f;
            return vec![byte];
        }

        // Alt+key → ESC prefix
        if mods.alt {
            let mut out = vec![0x1b];
            let mut buf = [0u8; 4];
            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            return out;
        }

        // Regular printable character
        let mut buf = [0u8; 4];
        ch.encode_utf8(&mut buf).as_bytes().to_vec()
    }

    // ── Special keys ────────────────────────────────────────────

    fn encode_special(&self, sk: SpecialKey, mods: &KeyModifiers) -> Vec<u8> {
        match sk {
            // Arrow keys: CSI A/B/C/D (or SS3 in app mode)
            SpecialKey::Up | SpecialKey::Down | SpecialKey::Left | SpecialKey::Right => {
                let suffix = match sk {
                    SpecialKey::Up => 'A',
                    SpecialKey::Down => 'B',
                    SpecialKey::Right => 'C',
                    SpecialKey::Left => 'D',
                    _ => unreachable!(),
                };
                if has_mod(mods) {
                    return csi_modified("1", suffix, mods);
                }
                if self.cursor_app_mode {
                    return format!("\x1bO{}", suffix).into_bytes();
                }
                format!("\x1b[{}", suffix).into_bytes()
            }

            SpecialKey::Home => {
                if has_mod(mods) {
                    return csi_modified("1", 'H', mods);
                }
                if self.cursor_app_mode {
                    return b"\x1bOH".to_vec();
                }
                b"\x1b[H".to_vec()
            }
            SpecialKey::End => {
                if has_mod(mods) {
                    return csi_modified("1", 'F', mods);
                }
                if self.cursor_app_mode {
                    return b"\x1bOF".to_vec();
                }
                b"\x1b[F".to_vec()
            }

            SpecialKey::PageUp => csi_tilde("5", mods),
            SpecialKey::PageDown => csi_tilde("6", mods),
            SpecialKey::Insert => csi_tilde("2", mods),
            SpecialKey::Delete => csi_tilde("3", mods),

            // F1–F4: SS3 when unmodified, CSI 1 P/Q/R/S when modified
            SpecialKey::F1 => {
                if has_mod(mods) { csi_modified("1", 'P', mods) } else { b"\x1bOP".to_vec() }
            }
            SpecialKey::F2 => {
                if has_mod(mods) { csi_modified("1", 'Q', mods) } else { b"\x1bOQ".to_vec() }
            }
            SpecialKey::F3 => {
                if has_mod(mods) { csi_modified("1", 'R', mods) } else { b"\x1bOR".to_vec() }
            }
            SpecialKey::F4 => {
                if has_mod(mods) { csi_modified("1", 'S', mods) } else { b"\x1bOS".to_vec() }
            }

            // F5–F12: CSI nn~
            SpecialKey::F5 => csi_tilde("15", mods),
            SpecialKey::F6 => csi_tilde("17", mods),
            SpecialKey::F7 => csi_tilde("18", mods),
            SpecialKey::F8 => csi_tilde("19", mods),
            SpecialKey::F9 => csi_tilde("20", mods),
            SpecialKey::F10 => csi_tilde("21", mods),
            SpecialKey::F11 => csi_tilde("23", mods),
            SpecialKey::F12 => csi_tilde("24", mods),
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────

/// True if any modifier is active.
fn has_mod(mods: &KeyModifiers) -> bool {
    mods.shift || mods.ctrl || mods.alt
}

/// Compute the xterm modifier code (1 = none, 2 = shift, 3 = alt, 4 = shift+alt, …)
fn mod_code(mods: &KeyModifiers) -> u8 {
    let mut m = 1u8;
    if mods.shift { m += 1; }
    if mods.alt { m += 2; }
    if mods.ctrl { m += 4; }
    m
}

/// CSI {param};{mod} {suffix}  — used for modified cursor keys and F1-F4.
fn csi_modified(param: &str, suffix: char, mods: &KeyModifiers) -> Vec<u8> {
    format!("\x1b[{};{}{}", param, mod_code(mods), suffix).into_bytes()
}

/// CSI {num}~ or CSI {num};{mod}~ — used for PgUp/PgDn/Ins/Del/F5-F12.
fn csi_tilde(num: &str, mods: &KeyModifiers) -> Vec<u8> {
    if has_mod(mods) {
        return format!("\x1b[{};{}~", num, mod_code(mods)).into_bytes();
    }
    format!("\x1b[{}~", num).into_bytes()
}

// ═══════════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn nomod() -> KeyModifiers {
        KeyModifiers::default()
    }

    // ── Character keys (backward compatible) ───────────────────

    #[test]
    fn test_enter_key() {
        let enc = InputEncoder::new();
        let key = InputKey::Char('\r', nomod());
        assert_eq!(enc.encode(&key), b"\r");
    }

    #[test]
    fn test_tab_key() {
        let enc = InputEncoder::new();
        let key = InputKey::Char('\t', nomod());
        assert_eq!(enc.encode(&key), b"\t");
    }

    #[test]
    fn test_backspace() {
        let enc = InputEncoder::new();
        let key = InputKey::Char('\x08', nomod());
        assert_eq!(enc.encode(&key), b"\x7f");
    }

    #[test]
    fn test_ctrl_c() {
        let enc = InputEncoder::new();
        let key = InputKey::Char('c', KeyModifiers { ctrl: true, ..Default::default() });
        assert_eq!(enc.encode(&key), b"\x03");
    }

    #[test]
    fn test_ctrl_d() {
        let enc = InputEncoder::new();
        let key = InputKey::Char('d', KeyModifiers { ctrl: true, ..Default::default() });
        assert_eq!(enc.encode(&key), b"\x04");
        // Ctrl+D should be 0x04 regardless of case
        let key_upper = InputKey::Char('D', KeyModifiers { ctrl: true, ..Default::default() });
        assert_eq!(enc.encode(&key_upper), b"\x04");
    }

    #[test]
    fn test_regular_char() {
        let enc = InputEncoder::new();
        let key = InputKey::Char('A', nomod());
        assert_eq!(enc.encode(&key), b"A");
    }

    #[test]
    fn test_alt_char() {
        let enc = InputEncoder::new();
        let key = InputKey::Char('a', KeyModifiers { alt: true, ..Default::default() });
        assert_eq!(enc.encode(&key), b"\x1ba");
    }

    #[test]
    fn test_escape_key() {
        let enc = InputEncoder::new();
        let key = InputKey::Char('\x1b', nomod());
        assert_eq!(enc.encode(&key), b"\x1b");
    }

    #[test]
    fn test_unicode_char() {
        let enc = InputEncoder::new();
        let key = InputKey::Char('你', nomod());
        assert_eq!(enc.encode(&key), "你".as_bytes());
    }

    // ── Arrow keys ─────────────────────────────────────────────

    #[test]
    fn test_arrow_up() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::Up, nomod());
        assert_eq!(enc.encode(&key), b"\x1b[A");
    }

    #[test]
    fn test_arrow_down() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::Down, nomod());
        assert_eq!(enc.encode(&key), b"\x1b[B");
    }

    #[test]
    fn test_arrow_right() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::Right, nomod());
        assert_eq!(enc.encode(&key), b"\x1b[C");
    }

    #[test]
    fn test_arrow_left() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::Left, nomod());
        assert_eq!(enc.encode(&key), b"\x1b[D");
    }

    // ── Arrow keys in app cursor mode ──────────────────────────

    #[test]
    fn test_arrow_app_mode() {
        let mut enc = InputEncoder::new();
        enc.set_cursor_app_mode(true);
        let key = InputKey::Special(SpecialKey::Up, nomod());
        assert_eq!(enc.encode(&key), b"\x1bOA"); // SS3
    }

    #[test]
    fn test_home_app_mode() {
        let mut enc = InputEncoder::new();
        enc.set_cursor_app_mode(true);
        let key = InputKey::Special(SpecialKey::Home, nomod());
        assert_eq!(enc.encode(&key), b"\x1bOH");
    }

    #[test]
    fn test_end_app_mode() {
        let mut enc = InputEncoder::new();
        enc.set_cursor_app_mode(true);
        let key = InputKey::Special(SpecialKey::End, nomod());
        assert_eq!(enc.encode(&key), b"\x1bOF");
    }

    // ── Modified arrows ────────────────────────────────────────

    #[test]
    fn test_shift_arrow_up() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::Up, KeyModifiers { shift: true, ..Default::default() });
        assert_eq!(enc.encode(&key), b"\x1b[1;2A");
    }

    #[test]
    fn test_ctrl_arrow_right() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::Right, KeyModifiers { ctrl: true, ..Default::default() });
        assert_eq!(enc.encode(&key), b"\x1b[1;5C");
    }

    #[test]
    fn test_alt_arrow_down() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::Down, KeyModifiers { alt: true, ..Default::default() });
        assert_eq!(enc.encode(&key), b"\x1b[1;3B");
    }

    // ── Home / End ─────────────────────────────────────────────

    #[test]
    fn test_home_normal() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::Home, nomod());
        assert_eq!(enc.encode(&key), b"\x1b[H");
    }

    #[test]
    fn test_end_normal() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::End, nomod());
        assert_eq!(enc.encode(&key), b"\x1b[F");
    }

    // ── PgUp / PgDn / Ins / Del ────────────────────────────────

    #[test]
    fn test_page_up() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::PageUp, nomod());
        assert_eq!(enc.encode(&key), b"\x1b[5~");
    }

    #[test]
    fn test_page_down() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::PageDown, nomod());
        assert_eq!(enc.encode(&key), b"\x1b[6~");
    }

    #[test]
    fn test_insert() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::Insert, nomod());
        assert_eq!(enc.encode(&key), b"\x1b[2~");
    }

    #[test]
    fn test_delete() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::Delete, nomod());
        assert_eq!(enc.encode(&key), b"\x1b[3~");
    }

    #[test]
    fn test_ctrl_delete() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::Delete, KeyModifiers { ctrl: true, ..Default::default() });
        assert_eq!(enc.encode(&key), b"\x1b[3;5~");
    }

    // ── Function keys F1–F12 ───────────────────────────────────

    #[test]
    fn test_f1() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::F1)), b"\x1bOP");
    }

    #[test]
    fn test_f2() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::F2)), b"\x1bOQ");
    }

    #[test]
    fn test_f3() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::F3)), b"\x1bOR");
    }

    #[test]
    fn test_f4() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::F4)), b"\x1bOS");
    }

    #[test]
    fn test_f5() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::F5)), b"\x1b[15~");
    }

    #[test]
    fn test_f6() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::F6)), b"\x1b[17~");
    }

    #[test]
    fn test_f7() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::F7)), b"\x1b[18~");
    }

    #[test]
    fn test_f8() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::F8)), b"\x1b[19~");
    }

    #[test]
    fn test_f9() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::F9)), b"\x1b[20~");
    }

    #[test]
    fn test_f10() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::F10)), b"\x1b[21~");
    }

    #[test]
    fn test_f11() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::F11)), b"\x1b[23~");
    }

    #[test]
    fn test_f12() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::F12)), b"\x1b[24~");
    }

    // ── Modified function keys ─────────────────────────────────

    #[test]
    fn test_shift_f5() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::F5, KeyModifiers { shift: true, ..Default::default() });
        assert_eq!(enc.encode(&key), b"\x1b[15;2~");
    }

    #[test]
    fn test_ctrl_f1() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::F1, KeyModifiers { ctrl: true, ..Default::default() });
        assert_eq!(enc.encode(&key), b"\x1b[1;5P");
    }

    // ── Helper tests ───────────────────────────────────────────

    #[test]
    fn test_has_mod() {
        assert!(!has_mod(&nomod()));
        assert!(has_mod(&KeyModifiers { shift: true, ..Default::default() }));
        assert!(has_mod(&KeyModifiers { ctrl: true, ..Default::default() }));
    }

    #[test]
    fn test_mod_code() {
        assert_eq!(mod_code(&nomod()), 1);
        assert_eq!(mod_code(&KeyModifiers { shift: true, ..Default::default() }), 2);
        assert_eq!(mod_code(&KeyModifiers { alt: true, ..Default::default() }), 3);
        assert_eq!(mod_code(&KeyModifiers { shift: true, alt: true, ..Default::default() }), 4);
        assert_eq!(mod_code(&KeyModifiers { ctrl: true, ..Default::default() }), 5);
        assert_eq!(mod_code(&KeyModifiers { shift: true, ctrl: true, ..Default::default() }), 6);
    }

    // ── Convenience constructors ───────────────────────────────

    #[test]
    fn test_input_key_char_constructor() {
        let key = InputKey::char('x');
        assert_eq!(key, InputKey::Char('x', KeyModifiers::default()));
    }

    #[test]
    fn test_input_key_special_constructor() {
        let key = InputKey::special(SpecialKey::F1);
        assert_eq!(key, InputKey::Special(SpecialKey::F1, KeyModifiers::default()));
    }
}
