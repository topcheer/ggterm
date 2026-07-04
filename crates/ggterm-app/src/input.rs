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
    /// Convenience: Char with no modifiers.
    pub fn char(c: char) -> Self {
        InputKey::Char(c, KeyModifiers::default())
    }

    /// Convenience: Char with modifiers.
    pub fn char_mod(c: char, mods: KeyModifiers) -> Self {
        InputKey::Char(c, mods)
    }

    /// Convenience: Special key with no modifiers.
    pub fn special(k: SpecialKey) -> Self {
        InputKey::Special(k, KeyModifiers::default())
    }

    /// Convenience: Special key with modifiers.
    pub fn special_mod(k: SpecialKey, mods: KeyModifiers) -> Self {
        InputKey::Special(k, mods)
    }
}

/// Encodes keyboard input into ANSI byte sequences for the PTY.
#[derive(Default)]
pub struct InputEncoder {
    /// Whether cursor keys should send application-mode sequences (DECCKM).
    cursor_app_mode: bool,
    /// Whether keypad keys should send application-mode sequences (DECPAM).
    keypad_app_mode: bool,
    /// modifyOtherKeys mode (0=off, 1=mode1, 2=mode2).
    modify_other_keys: u8,
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

    /// Set keypad application mode (DECPAM/DECPNM).
    pub fn set_keypad_app_mode(&mut self, on: bool) {
        self.keypad_app_mode = on;
    }

    /// Set modifyOtherKeys mode (0=off, 1=mode1, 2=mode2).
    ///
    /// When active, modified special keys (Ctrl+Arrow, Shift+Tab, etc.)
    /// are encoded with explicit modifier parameters per xterm spec.
    pub fn set_modify_other_keys(&mut self, mode: u8) {
        self.modify_other_keys = mode;
    }

    /// Encode a key press into bytes to write to the PTY.
    pub fn encode(&self, key: &InputKey) -> Vec<u8> {
        match key {
            InputKey::Char(ch, mods) => self.encode_char(*ch, mods),
            InputKey::Special(sk, mods) => self.encode_special(*sk, mods),
        }
    }

    fn encode_char(&self, ch: char, mods: &KeyModifiers) -> Vec<u8> {
        // ── Named control characters ──
        // These are intercepted first so that modifiers can override them.
        match ch {
            '\r' | '\n' => {
                // Ctrl+Enter → ^J (LF); plain Enter → CR
                if mods.ctrl {
                    return b"\n".to_vec();
                }
                return b"\r".to_vec();
            }
            '\t' => {
                // Ctrl+Tab is rare; most terminals send a plain tab.
                // Shift+Tab sends the reverse-tab (CSI Z) sequence.
                if mods.shift {
                    return b"\x1b[Z".to_vec();
                }
                return b"\t".to_vec();
            }
            '\x08' | '\x7f' => {
                // Ctrl+Backspace → ^W (delete word, common in shells)
                // Plain Backspace → DEL (0x7f)
                if mods.ctrl {
                    return b"\x17".to_vec(); // ^W
                }
                return b"\x7f".to_vec();
            }
            '\x1b' => return b"\x1b".to_vec(),
            _ => {}
        }

        // ── Ctrl combinations ──
        if mods.ctrl {
            // Ctrl+Space → NUL
            if ch == ' ' {
                return vec![0x00];
            }
            // Ctrl+letter → 0x01..0x1A
            if ch.is_ascii_alphabetic() {
                return vec![(ch.to_ascii_lowercase() as u8) & 0x1f];
            }
            // Ctrl+non-alpha control characters (punctuation + digits)
            if let Some(b) = ctrl_char(ch) {
                return vec![b];
            }
        }

        // ── Alt (Meta) combinations ──
        if mods.alt {
            let mut out = vec![0x1b];
            let mut buf = [0u8; 4];
            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            return out;
        }

        // ── Plain character ──
        let mut buf = [0u8; 4];
        ch.encode_utf8(&mut buf).as_bytes().to_vec()
    }

    fn encode_special(&self, sk: SpecialKey, mods: &KeyModifiers) -> Vec<u8> {
        match sk {
            SpecialKey::Up => self.cursor_key('A', mods),
            SpecialKey::Down => self.cursor_key('B', mods),
            SpecialKey::Right => self.cursor_key('C', mods),
            SpecialKey::Left => self.cursor_key('D', mods),
            SpecialKey::Home => self.cursor_key('H', mods),
            SpecialKey::End => self.cursor_key('F', mods),
            SpecialKey::PageUp => csi_tilde("5", mods),
            SpecialKey::PageDown => csi_tilde("6", mods),
            SpecialKey::Insert => csi_tilde("2", mods),
            SpecialKey::Delete => csi_tilde("3", mods),
            SpecialKey::F1 => {
                if has_mod(mods) {
                    csi_modified("1", 'P', mods)
                } else {
                    b"\x1bOP".to_vec()
                }
            }
            SpecialKey::F2 => {
                if has_mod(mods) {
                    csi_modified("1", 'Q', mods)
                } else {
                    b"\x1bOQ".to_vec()
                }
            }
            SpecialKey::F3 => {
                if has_mod(mods) {
                    csi_modified("1", 'R', mods)
                } else {
                    b"\x1bOR".to_vec()
                }
            }
            SpecialKey::F4 => {
                if has_mod(mods) {
                    csi_modified("1", 'S', mods)
                } else {
                    b"\x1bOS".to_vec()
                }
            }
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

    fn cursor_key(&self, suffix: char, mods: &KeyModifiers) -> Vec<u8> {
        // modifyOtherKeys mode 2: always send CSI with explicit params,
        // even without modifiers. Mode 1: only for modified keys.
        if self.modify_other_keys == 2 {
            return csi_modified_with_mode("1", suffix, mods, 2);
        }
        if has_mod(mods) {
            return csi_modified_with_mode("1", suffix, mods, self.modify_other_keys);
        }
        if self.cursor_app_mode {
            return format!("\x1bO{}", suffix).into_bytes();
        }
        format!("\x1b[{}", suffix).into_bytes()
    }
}

// ── Helpers ────────────────────────────────────────────────────

/// Map non-alpha Ctrl+key combinations to their control codes.
///
/// These cover the standard ASCII control character mappings used by
/// US-layout terminals. Returns `None` for characters without a
/// well-known Ctrl mapping.
fn ctrl_char(ch: char) -> Option<u8> {
    match ch {
        // Punctuation → control chars
        '[' => Some(0x1b),                   // ESC
        '\\' => Some(0x1c),                  // FS
        ']' => Some(0x1d),                   // GS
        '^' | '6' => Some(0x1e),             // RS
        '_' | '/' | '-' | '=' => Some(0x1f), // US
        '?' => Some(0x7f),                   // DEL
        '@' | '`' | '2' => Some(0x00),       // NUL
        '3' => Some(0x1b),                   // ESC
        '4' => Some(0x1c),                   // FS
        '5' => Some(0x1d),                   // GS
        '7' => Some(0x1f),                   // US
        '8' => Some(0x7f),                   // DEL
        _ => None,
    }
}

fn has_mod(mods: &KeyModifiers) -> bool {
    mods.shift || mods.ctrl || mods.alt
}

fn mod_code(mods: &KeyModifiers) -> u8 {
    let mut m = 1u8;
    if mods.shift {
        m += 1;
    }
    if mods.alt {
        m += 2;
    }
    if mods.ctrl {
        m += 4;
    }
    m
}

fn csi_modified(param: &str, suffix: char, mods: &KeyModifiers) -> Vec<u8> {
    format!("\x1b[{};{}{}", param, mod_code(mods), suffix).into_bytes()
}

/// Encode a modified cursor key with modifyOtherKeys mode awareness.
///
/// - mode 0: standard `\x1b[1;{mod}{suffix}` (only when modifiers present)
/// - mode 1: standard `\x1b[1;{mod}{suffix}` (only when modifiers present)
/// - mode 2: `\x1b[1;{mod};1{suffix}` with trailing `;1` per xterm spec
fn csi_modified_with_mode(param: &str, suffix: char, mods: &KeyModifiers, mode: u8) -> Vec<u8> {
    let m = mod_code(mods);
    if mode >= 2 {
        // Mode 2: append ;1 after the modifier code.
        format!("\x1b[{param};{m};1{suffix}").into_bytes()
    } else {
        format!("\x1b[{param};{m}{suffix}").into_bytes()
    }
}

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

    // ── Character keys ─────────────────────────────────────────

    #[test]
    fn test_enter_key() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::char('\r')), b"\r");
    }

    #[test]
    fn test_tab_key() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::char('\t')), b"\t");
    }

    #[test]
    fn test_backspace() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::char('\x08')), b"\x7f");
    }

    #[test]
    fn test_ctrl_c() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            'c',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x03");
    }

    #[test]
    fn test_ctrl_d() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            'd',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x04");
    }

    #[test]
    fn test_regular_char() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::char('A')), b"A");
    }

    #[test]
    fn test_alt_char() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            'a',
            KeyModifiers {
                alt: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1ba");
    }

    #[test]
    fn test_escape_key() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::char('\x1b')), b"\x1b");
    }

    #[test]
    fn test_unicode_char() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::char('你')), "你".as_bytes());
    }

    // ── Arrow keys ─────────────────────────────────────────────

    #[test]
    fn test_arrow_up() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::Up)), b"\x1b[A");
    }

    #[test]
    fn test_arrow_down() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::Down)), b"\x1b[B");
    }

    #[test]
    fn test_arrow_right() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::Right)), b"\x1b[C");
    }

    #[test]
    fn test_arrow_left() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::Left)), b"\x1b[D");
    }

    #[test]
    fn test_arrow_app_mode() {
        let mut enc = InputEncoder::new();
        enc.set_cursor_app_mode(true);
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::Up)), b"\x1bOA");
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::Down)), b"\x1bOB");
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::Right)), b"\x1bOC");
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::Left)), b"\x1bOD");
    }

    #[test]
    fn test_shift_arrow_up() {
        let enc = InputEncoder::new();
        let key = InputKey::special_mod(
            SpecialKey::Up,
            KeyModifiers {
                shift: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[1;2A");
    }

    #[test]
    fn test_ctrl_arrow_right() {
        let enc = InputEncoder::new();
        let key = InputKey::special_mod(
            SpecialKey::Right,
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[1;5C");
    }

    #[test]
    fn test_alt_arrow_down() {
        let enc = InputEncoder::new();
        let key = InputKey::special_mod(
            SpecialKey::Down,
            KeyModifiers {
                alt: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[1;3B");
    }

    // ── Home / End ─────────────────────────────────────────────

    #[test]
    fn test_home() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::Home)), b"\x1b[H");
    }

    #[test]
    fn test_end() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::End)), b"\x1b[F");
    }

    #[test]
    fn test_home_app_mode() {
        let mut enc = InputEncoder::new();
        enc.set_cursor_app_mode(true);
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::Home)), b"\x1bOH");
        assert_eq!(enc.encode(&InputKey::special(SpecialKey::End)), b"\x1bOF");
    }

    // ── PgUp / PgDn / Ins / Del ────────────────────────────────

    #[test]
    fn test_page_up() {
        let enc = InputEncoder::new();
        assert_eq!(
            enc.encode(&InputKey::special(SpecialKey::PageUp)),
            b"\x1b[5~"
        );
    }

    #[test]
    fn test_page_down() {
        let enc = InputEncoder::new();
        assert_eq!(
            enc.encode(&InputKey::special(SpecialKey::PageDown)),
            b"\x1b[6~"
        );
    }

    #[test]
    fn test_insert() {
        let enc = InputEncoder::new();
        assert_eq!(
            enc.encode(&InputKey::special(SpecialKey::Insert)),
            b"\x1b[2~"
        );
    }

    #[test]
    fn test_delete() {
        let enc = InputEncoder::new();
        assert_eq!(
            enc.encode(&InputKey::special(SpecialKey::Delete)),
            b"\x1b[3~"
        );
    }

    #[test]
    fn test_ctrl_delete() {
        let enc = InputEncoder::new();
        let key = InputKey::special_mod(
            SpecialKey::Delete,
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[3;5~");
    }

    // ── F1–F12 ─────────────────────────────────────────────────

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
        let key = InputKey::special_mod(
            SpecialKey::F5,
            KeyModifiers {
                shift: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[15;2~");
    }

    #[test]
    fn test_ctrl_f1() {
        let enc = InputEncoder::new();
        let key = InputKey::special_mod(
            SpecialKey::F1,
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[1;5P");
    }

    // ── Helpers ────────────────────────────────────────────────

    #[test]
    fn test_has_mod() {
        assert!(!has_mod(&nomod()));
        assert!(has_mod(&KeyModifiers {
            shift: true,
            ..Default::default()
        }));
    }

    #[test]
    fn test_mod_code() {
        assert_eq!(mod_code(&nomod()), 1);
        assert_eq!(
            mod_code(&KeyModifiers {
                shift: true,
                ..Default::default()
            }),
            2
        );
        assert_eq!(
            mod_code(&KeyModifiers {
                alt: true,
                ..Default::default()
            }),
            3
        );
        assert_eq!(
            mod_code(&KeyModifiers {
                ctrl: true,
                ..Default::default()
            }),
            5
        );
        assert_eq!(
            mod_code(&KeyModifiers {
                shift: true,
                ctrl: true,
                ..Default::default()
            }),
            6
        );
        assert_eq!(
            mod_code(&KeyModifiers {
                shift: true,
                alt: true,
                ctrl: true
            }),
            8
        );
    }

    #[test]
    fn test_all_special_keys_produce_output() {
        let enc = InputEncoder::new();
        let all = [
            SpecialKey::Up,
            SpecialKey::Down,
            SpecialKey::Left,
            SpecialKey::Right,
            SpecialKey::Home,
            SpecialKey::End,
            SpecialKey::PageUp,
            SpecialKey::PageDown,
            SpecialKey::Insert,
            SpecialKey::Delete,
            SpecialKey::F1,
            SpecialKey::F2,
            SpecialKey::F3,
            SpecialKey::F4,
            SpecialKey::F5,
            SpecialKey::F6,
            SpecialKey::F7,
            SpecialKey::F8,
            SpecialKey::F9,
            SpecialKey::F10,
            SpecialKey::F11,
            SpecialKey::F12,
        ];
        for k in all {
            let result = enc.encode(&InputKey::special(k));
            assert!(!result.is_empty(), "{:?} produced empty output", k);
            assert!(result.starts_with(b"\x1b"), "{:?} should start with ESC", k);
        }
    }

    // ════════════════════════════════════════════════════════════════
    //  P9-F: Keyboard Input Refinement — comprehensive tests
    // ════════════════════════════════════════════════════════════════

    // ── Ctrl+non-alpha → control characters ──

    #[test]
    fn test_ctrl_space_nul() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            ' ',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x00");
    }

    #[test]
    fn test_ctrl_enter_lf() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            '\r',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\n");
    }

    #[test]
    fn test_ctrl_backspace_word_delete() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            '\x08',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x17"); // ^W
    }

    #[test]
    fn test_shift_tab_reverse_tab() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            '\t',
            KeyModifiers {
                shift: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[Z"); // CSI Z
    }

    #[test]
    fn test_ctrl_open_bracket_esc() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            '[',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b");
    }

    #[test]
    fn test_ctrl_close_bracket_gs() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            ']',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1d");
    }

    #[test]
    fn test_ctrl_backslash_fs() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            '\\',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1c");
    }

    #[test]
    fn test_ctrl_slash_us() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            '/',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1f");
    }

    #[test]
    fn test_ctrl_backtick_nul() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            '`',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x00");
    }

    #[test]
    fn test_ctrl_caret_rs() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            '^',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1e");
    }

    #[test]
    fn test_ctrl_underscore_us() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            '_',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1f");
    }

    #[test]
    fn test_ctrl_minus_us() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            '-',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1f");
    }

    #[test]
    fn test_ctrl_equal_us() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            '=',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1f");
    }

    // ── Ctrl+digits (US keyboard) ──

    #[test]
    fn test_ctrl_2_nul() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            '2',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x00");
    }

    #[test]
    fn test_ctrl_3_esc() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            '3',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b");
    }

    #[test]
    fn test_ctrl_4_fs() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            '4',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1c");
    }

    #[test]
    fn test_ctrl_5_gs() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            '5',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1d");
    }

    #[test]
    fn test_ctrl_6_rs() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            '6',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1e");
    }

    #[test]
    fn test_ctrl_7_us() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            '7',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1f");
    }

    #[test]
    fn test_ctrl_8_del() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            '8',
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x7f");
    }

    // ── Ctrl+letter full alphabet ──

    #[test]
    fn test_ctrl_a_through_z() {
        let enc = InputEncoder::new();
        for c in 'a'..='z' {
            let key = InputKey::char_mod(
                c,
                KeyModifiers {
                    ctrl: true,
                    ..Default::default()
                },
            );
            let expected = vec![(c as u8) & 0x1f];
            assert_eq!(enc.encode(&key), expected, "Ctrl+{}", c);
        }
        // Uppercase should produce the same control codes
        for c in 'A'..='Z' {
            let key = InputKey::char_mod(
                c,
                KeyModifiers {
                    ctrl: true,
                    ..Default::default()
                },
            );
            let expected = vec![(c.to_ascii_lowercase() as u8) & 0x1f];
            assert_eq!(enc.encode(&key), expected, "Ctrl+Shift+{}", c);
        }
    }

    // ── Alt+digit ──

    #[test]
    fn test_alt_digits() {
        let enc = InputEncoder::new();
        for n in '0'..='9' {
            let key = InputKey::char_mod(
                n,
                KeyModifiers {
                    alt: true,
                    ..Default::default()
                },
            );
            let mut expected = vec![0x1b];
            expected.push(n as u8);
            assert_eq!(enc.encode(&key), expected, "Alt+{}", n);
        }
    }

    // ── Alt+letter ──

    #[test]
    fn test_alt_lowercase_letter() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            'x',
            KeyModifiers {
                alt: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1bx");
    }

    #[test]
    fn test_alt_uppercase_letter() {
        let enc = InputEncoder::new();
        let key = InputKey::char_mod(
            'X',
            KeyModifiers {
                alt: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1bX");
    }

    // ── Shift+arrows (modified cursor keys) ──

    #[test]
    fn test_shift_arrow_left() {
        let enc = InputEncoder::new();
        let key = InputKey::special_mod(
            SpecialKey::Left,
            KeyModifiers {
                shift: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[1;2D");
    }

    #[test]
    fn test_shift_arrow_right() {
        let enc = InputEncoder::new();
        let key = InputKey::special_mod(
            SpecialKey::Right,
            KeyModifiers {
                shift: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[1;2C");
    }

    #[test]
    fn test_shift_arrow_down() {
        let enc = InputEncoder::new();
        let key = InputKey::special_mod(
            SpecialKey::Down,
            KeyModifiers {
                shift: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[1;2B");
    }

    // ── Ctrl+Shift+arrows ──

    #[test]
    fn test_ctrl_shift_arrow_right() {
        let enc = InputEncoder::new();
        let key = InputKey::special_mod(
            SpecialKey::Right,
            KeyModifiers {
                shift: true,
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[1;6C"); // mod=6 (shift+ctrl)
    }

    #[test]
    fn test_ctrl_shift_arrow_left() {
        let enc = InputEncoder::new();
        let key = InputKey::special_mod(
            SpecialKey::Left,
            KeyModifiers {
                shift: true,
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[1;6D");
    }

    // ── Ctrl+Shift+Alt (all modifiers) ──

    #[test]
    fn test_all_modifiers_arrow() {
        let enc = InputEncoder::new();
        let key = InputKey::special_mod(
            SpecialKey::Up,
            KeyModifiers {
                shift: true,
                ctrl: true,
                alt: true,
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[1;8A"); // mod=8 (shift+alt+ctrl)
    }

    // ── Modified navigation keys ──

    #[test]
    fn test_ctrl_home() {
        let enc = InputEncoder::new();
        let key = InputKey::special_mod(
            SpecialKey::Home,
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[1;5H");
    }

    #[test]
    fn test_ctrl_end() {
        let enc = InputEncoder::new();
        let key = InputKey::special_mod(
            SpecialKey::End,
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[1;5F");
    }

    #[test]
    fn test_shift_page_up() {
        let enc = InputEncoder::new();
        let key = InputKey::special_mod(
            SpecialKey::PageUp,
            KeyModifiers {
                shift: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[5;2~");
    }

    #[test]
    fn test_alt_insert() {
        let enc = InputEncoder::new();
        let key = InputKey::special_mod(
            SpecialKey::Insert,
            KeyModifiers {
                alt: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[2;3~");
    }

    // ── Modified function keys ──

    #[test]
    fn test_alt_f1() {
        let enc = InputEncoder::new();
        let key = InputKey::special_mod(
            SpecialKey::F1,
            KeyModifiers {
                alt: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[1;3P");
    }

    #[test]
    fn test_shift_ctrl_f5() {
        let enc = InputEncoder::new();
        let key = InputKey::special_mod(
            SpecialKey::F5,
            KeyModifiers {
                shift: true,
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[15;6~");
    }

    #[test]
    fn test_ctrl_f11() {
        let enc = InputEncoder::new();
        let key = InputKey::special_mod(
            SpecialKey::F11,
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[23;5~");
    }

    #[test]
    fn test_alt_f12() {
        let enc = InputEncoder::new();
        let key = InputKey::special_mod(
            SpecialKey::F12,
            KeyModifiers {
                alt: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\x1b[24;3~");
    }

    // ── ctrl_char helper ──

    #[test]
    fn test_ctrl_char_mappings() {
        assert_eq!(ctrl_char('['), Some(0x1b));
        assert_eq!(ctrl_char(']'), Some(0x1d));
        assert_eq!(ctrl_char('\\'), Some(0x1c));
        assert_eq!(ctrl_char('/'), Some(0x1f));
        assert_eq!(ctrl_char('^'), Some(0x1e));
        assert_eq!(ctrl_char('_'), Some(0x1f));
        assert_eq!(ctrl_char('`'), Some(0x00));
        assert_eq!(ctrl_char('@'), Some(0x00));
        assert_eq!(ctrl_char('?'), Some(0x7f));
        assert_eq!(ctrl_char('a'), None); // letters handled separately
        assert_eq!(ctrl_char('!'), None); // no mapping
    }

    // ── No modifier regressions ──

    #[test]
    fn test_plain_backspace_still_del() {
        let enc = InputEncoder::new();
        // Plain Backspace → DEL (0x7f)
        assert_eq!(enc.encode(&InputKey::char('\x08')), b"\x7f");
        assert_eq!(enc.encode(&InputKey::char('\x7f')), b"\x7f");
    }

    #[test]
    fn test_plain_enter_still_cr() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::char('\r')), b"\r");
    }

    #[test]
    fn test_plain_tab_still_tab() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::char('\t')), b"\t");
    }

    #[test]
    fn test_alt_enter() {
        let enc = InputEncoder::new();
        // Alt+Enter should still send CR (the named char check comes first)
        let key = InputKey::char_mod(
            '\r',
            KeyModifiers {
                alt: true,
                ..Default::default()
            },
        );
        assert_eq!(enc.encode(&key), b"\r");
    }

    // ── modifyOtherKeys ─────────────────────────────────────────

    #[test]
    fn test_modify_other_keys_mode1_ctrl_arrow() {
        let mut enc = InputEncoder::new();
        enc.set_modify_other_keys(1);
        let key = InputKey::Special(
            SpecialKey::Right,
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        // Ctrl modifier = 5, so: ESC[1;5C
        assert_eq!(enc.encode(&key), b"\x1b[1;5C");
    }

    #[test]
    fn test_modify_other_keys_mode2_unmodified_arrow() {
        let mut enc = InputEncoder::new();
        enc.set_modify_other_keys(2);
        let key = InputKey::Special(SpecialKey::Up, KeyModifiers::default());
        // Mode 2: even unmodified keys get explicit modifier code + trailing ;1
        // modifier code 1 (no mods), so: ESC[1;1;1A
        assert_eq!(enc.encode(&key), b"\x1b[1;1;1A");
    }

    #[test]
    fn test_modify_other_keys_mode2_shift_arrow() {
        let mut enc = InputEncoder::new();
        enc.set_modify_other_keys(2);
        let key = InputKey::Special(
            SpecialKey::Left,
            KeyModifiers {
                shift: true,
                ..Default::default()
            },
        );
        // Shift = 2, so: ESC[1;2;1D
        assert_eq!(enc.encode(&key), b"\x1b[1;2;1D");
    }

    #[test]
    fn test_modify_other_keys_off_normal_arrow() {
        let enc = InputEncoder::new();
        // modifyOtherKeys = 0 (off), unmodified arrow → normal sequence
        let key = InputKey::Special(SpecialKey::Down, KeyModifiers::default());
        assert_eq!(enc.encode(&key), b"\x1b[B");
    }

    #[test]
    fn test_modify_other_keys_mode1_unmodified_arrow() {
        let mut enc = InputEncoder::new();
        enc.set_modify_other_keys(1);
        // Mode 1: unmodified keys are NOT affected, only modified ones
        let key = InputKey::Special(SpecialKey::Down, KeyModifiers::default());
        assert_eq!(enc.encode(&key), b"\x1b[B");
    }
}
