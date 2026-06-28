//! Input encoding: converts keyboard events to ANSI byte sequences
//! for the PTY.
//!
//! This module does NOT depend on winit — it takes simple key descriptions
//! and produces bytes. The winit→InputKey mapping happens in the app layer.

/// Modifier flags for a key press.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl KeyModifiers {
    pub fn new() -> Self {
        Self::default()
    }

    /// Compute xterm modifier parameter (1 = none, 2 = shift, 3 = alt, 4 = shift+alt, 5 = ctrl, ...)
    fn xterm_code(&self) -> u32 {
        let mut m = 1u32;
        if self.shift {
            m += 1;
        }
        if self.alt {
            m += 2;
        }
        if self.ctrl {
            m += 4;
        }
        m
    }

    fn has_any(&self) -> bool {
        self.shift || self.ctrl || self.alt
    }
}

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

/// Encodes keyboard events into ANSI byte sequences for PTY input.
pub struct InputEncoder {
    /// DECCKM — when true, cursor keys send SS3 sequences instead of CSI.
    cursor_app_mode: bool,
}

impl Default for InputEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl InputEncoder {
    pub fn new() -> Self {
        Self {
            cursor_app_mode: false,
        }
    }

    /// Enable/disable DECCKM (Application Cursor Keys Mode).
    /// When enabled, arrow keys send ESC O A/B/C/D instead of ESC [ A/B/C/D.
    pub fn set_cursor_app_mode(&mut self, enabled: bool) {
        self.cursor_app_mode = enabled;
    }

    /// Encode a key press into bytes to write to the PTY.
    pub fn encode(&self, key: &InputKey) -> Vec<u8> {
        match key {
            InputKey::Char(ch, mods) => self.encode_char(*ch, mods),
            InputKey::Special(sk, mods) => self.encode_special(*sk, mods),
        }
    }

    fn encode_char(&self, ch: char, mods: &KeyModifiers) -> Vec<u8> {
        // Special characters
        match ch {
            '\r' | '\n' => return b"\r".to_vec(),
            '\t' => return b"\t".to_vec(),
            '\x08' | '\x7f' => return b"\x7f".to_vec(), // Backspace → DEL
            '\x1b' => return b"\x1b".to_vec(),           // Escape
            _ => {}
        }

        // Ctrl+letter → control character (0x01-0x1A)
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
                if mods.has_any() {
                    return format!("\x1b[1;{}{}", mods.xterm_code(), suffix).into_bytes();
                }
                if self.cursor_app_mode {
                    return format!("\x1bO{}", suffix).into_bytes();
                }
                format!("\x1b[{}", suffix).into_bytes()
            }

            // Home/End: CSI H/F (or SS3 H/F in app mode)
            SpecialKey::Home | SpecialKey::End => {
                let suffix = match sk {
                    SpecialKey::Home => 'H',
                    SpecialKey::End => 'F',
                    _ => unreachable!(),
                };
                if mods.has_any() {
                    return format!("\x1b[1;{}{}", mods.xterm_code(), suffix).into_bytes();
                }
                if self.cursor_app_mode {
                    return format!("\x1bO{}", suffix).into_bytes();
                }
                format!("\x1b[{}", suffix).into_bytes()
            }

            // CSI ~ keys
            SpecialKey::Insert => self.encode_csi_tilde('2', mods),
            SpecialKey::Delete => self.encode_csi_tilde('3', mods),
            SpecialKey::PageUp => self.encode_csi_tilde('5', mods),
            SpecialKey::PageDown => self.encode_csi_tilde('6', mods),

            // F1-F4: SS3 when unmodified, CSI 1 P/Q/R/S when modified
            SpecialKey::F1 => {
                if mods.has_any() {
                    return self.encode_csi_letter('1', 'P', mods);
                }
                b"\x1bOP".to_vec()
            }
            SpecialKey::F2 => {
                if mods.has_any() {
                    return self.encode_csi_letter('1', 'Q', mods);
                }
                b"\x1bOQ".to_vec()
            }
            SpecialKey::F3 => {
                if mods.has_any() {
                    return self.encode_csi_letter('1', 'R', mods);
                }
                b"\x1bOR".to_vec()
            }
            SpecialKey::F4 => {
                if mods.has_any() {
                    return self.encode_csi_letter('1', 'S', mods);
                }
                b"\x1bOS".to_vec()
            }

            // F5-F12: CSI 1n ~ format
            // F5=15, F6=17, F7=18, F8=19, F9=20, F10=21, F11=23, F12=24
            SpecialKey::F5 => self.encode_csi_tilde_str("15", mods),
            SpecialKey::F6 => self.encode_csi_tilde_str("17", mods),
            SpecialKey::F7 => self.encode_csi_tilde_str("18", mods),
            SpecialKey::F8 => self.encode_csi_tilde_str("19", mods),
            SpecialKey::F9 => self.encode_csi_tilde_str("20", mods),
            SpecialKey::F10 => self.encode_csi_tilde_str("21", mods),
            SpecialKey::F11 => self.encode_csi_tilde_str("23", mods),
            SpecialKey::F12 => self.encode_csi_tilde_str("24", mods),
        }
    }

    /// CSI {n}~ or CSI {n};{mod}~ (single-digit parameter)
    fn encode_csi_tilde(&self, num: char, mods: &KeyModifiers) -> Vec<u8> {
        if mods.has_any() {
            format!("\x1b[{};{}~", num, mods.xterm_code()).into_bytes()
        } else {
            format!("\x1b[{}~", num).into_bytes()
        }
    }

    /// CSI {n}~ or CSI {n};{mod}~ (multi-digit parameter)
    fn encode_csi_tilde_str(&self, num: &str, mods: &KeyModifiers) -> Vec<u8> {
        if mods.has_any() {
            format!("\x1b[{};{}~", num, mods.xterm_code()).into_bytes()
        } else {
            format!("\x1b[{}~", num).into_bytes()
        }
    }

    /// CSI {n}{letter} or CSI {n};{mod}{letter} (used for modified F1-F4)
    fn encode_csi_letter(&self, num: char, letter: char, mods: &KeyModifiers) -> Vec<u8> {
        format!("\x1b[{};{}{}", num, mods.xterm_code(), letter).into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mod_none() -> KeyModifiers {
        KeyModifiers::default()
    }

    fn mod_ctrl() -> KeyModifiers {
        KeyModifiers {
            ctrl: true,
            ..Default::default()
        }
    }

    fn mod_alt() -> KeyModifiers {
        KeyModifiers {
            alt: true,
            ..Default::default()
        }
    }

    fn mod_shift() -> KeyModifiers {
        KeyModifiers {
            shift: true,
            ..Default::default()
        }
    }

    // === Char tests ===

    #[test]
    fn test_regular_char() {
        let enc = InputEncoder::new();
        let key = InputKey::Char('a', mod_none());
        assert_eq!(enc.encode(&key), b"a");
    }

    #[test]
    fn test_enter_key() {
        let enc = InputEncoder::new();
        let key = InputKey::Char('\r', mod_none());
        assert_eq!(enc.encode(&key), b"\r");
    }

    #[test]
    fn test_tab_key() {
        let enc = InputEncoder::new();
        let key = InputKey::Char('\t', mod_none());
        assert_eq!(enc.encode(&key), b"\t");
    }

    #[test]
    fn test_backspace() {
        let enc = InputEncoder::new();
        let key = InputKey::Char('\x08', mod_none());
        assert_eq!(enc.encode(&key), b"\x7f");
    }

    #[test]
    fn test_ctrl_c() {
        let enc = InputEncoder::new();
        let key = InputKey::Char('c', mod_ctrl());
        assert_eq!(enc.encode(&key), b"\x03");
    }

    #[test]
    fn test_ctrl_d() {
        let enc = InputEncoder::new();
        let key = InputKey::Char('d', mod_ctrl());
        assert_eq!(enc.encode(&key), b"\x04");
    }

    #[test]
    fn test_escape_key() {
        let enc = InputEncoder::new();
        let key = InputKey::Char('\x1b', mod_none());
        assert_eq!(enc.encode(&key), b"\x1b");
    }

    #[test]
    fn test_alt_char() {
        let enc = InputEncoder::new();
        let key = InputKey::Char('a', mod_alt());
        assert_eq!(enc.encode(&key), b"\x1ba");
    }

    #[test]
    fn test_unicode_char() {
        let enc = InputEncoder::new();
        let key = InputKey::Char('你', mod_none());
        assert_eq!(enc.encode(&key), "你".as_bytes());
    }

    // === Arrow key tests ===

    #[test]
    fn test_arrow_up() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::Up, mod_none());
        assert_eq!(enc.encode(&key), b"\x1b[A");
    }

    #[test]
    fn test_arrow_down() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::Down, mod_none());
        assert_eq!(enc.encode(&key), b"\x1b[B");
    }

    #[test]
    fn test_arrow_right() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::Right, mod_none());
        assert_eq!(enc.encode(&key), b"\x1b[C");
    }

    #[test]
    fn test_arrow_left() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::Left, mod_none());
        assert_eq!(enc.encode(&key), b"\x1b[D");
    }

    #[test]
    fn test_arrow_app_mode() {
        let mut enc = InputEncoder::new();
        enc.set_cursor_app_mode(true);
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::Up, mod_none())), b"\x1bOA");
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::Down, mod_none())), b"\x1bOB");
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::Right, mod_none())), b"\x1bOC");
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::Left, mod_none())), b"\x1bOD");
    }

    #[test]
    fn test_arrow_with_shift() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::Up, mod_shift());
        assert_eq!(enc.encode(&key), b"\x1b[1;2A");
    }

    #[test]
    fn test_arrow_with_ctrl() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::Right, mod_ctrl());
        assert_eq!(enc.encode(&key), b"\x1b[1;5C");
    }

    #[test]
    fn test_arrow_with_alt() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::Down, mod_alt());
        assert_eq!(enc.encode(&key), b"\x1b[1;3B");
    }

    #[test]
    fn test_arrow_ctrl_shift() {
        let enc = InputEncoder::new();
        let key = InputKey::Special(SpecialKey::Up, KeyModifiers { ctrl: true, shift: true, alt: false });
        assert_eq!(enc.encode(&key), b"\x1b[1;6A");
    }

    // === Home/End tests ===

    #[test]
    fn test_home() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::Home, mod_none())), b"\x1b[H");
    }

    #[test]
    fn test_end() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::End, mod_none())), b"\x1b[F");
    }

    #[test]
    fn test_home_app_mode() {
        let mut enc = InputEncoder::new();
        enc.set_cursor_app_mode(true);
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::Home, mod_none())), b"\x1bOH");
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::End, mod_none())), b"\x1bOF");
    }

    // === Page/Insert/Delete tests ===

    #[test]
    fn test_page_up() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::PageUp, mod_none())), b"\x1b[5~");
    }

    #[test]
    fn test_page_down() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::PageDown, mod_none())), b"\x1b[6~");
    }

    #[test]
    fn test_insert() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::Insert, mod_none())), b"\x1b[2~");
    }

    #[test]
    fn test_delete() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::Delete, mod_none())), b"\x1b[3~");
    }

    #[test]
    fn test_delete_with_ctrl() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::Delete, mod_ctrl())), b"\x1b[3;5~");
    }

    // === Function key tests ===

    #[test]
    fn test_f1_f4_unmodified() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::F1, mod_none())), b"\x1bOP");
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::F2, mod_none())), b"\x1bOQ");
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::F3, mod_none())), b"\x1bOR");
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::F4, mod_none())), b"\x1bOS");
    }

    #[test]
    fn test_f1_with_shift() {
        let enc = InputEncoder::new();
        // Modified F1-F4: CSI 1 ; {mod} P
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::F1, mod_shift())), b"\x1b[1;2P");
    }

    #[test]
    fn test_f5_to_f12() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::F5, mod_none())), b"\x1b[15~");
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::F6, mod_none())), b"\x1b[17~");
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::F7, mod_none())), b"\x1b[18~");
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::F8, mod_none())), b"\x1b[19~");
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::F9, mod_none())), b"\x1b[20~");
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::F10, mod_none())), b"\x1b[21~");
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::F11, mod_none())), b"\x1b[23~");
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::F12, mod_none())), b"\x1b[24~");
    }

    #[test]
    fn test_f5_with_ctrl() {
        let enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::F5, mod_ctrl())), b"\x1b[15;5~");
    }

    // === Cursor app mode toggle ===

    #[test]
    fn test_cursor_app_mode_toggle() {
        let mut enc = InputEncoder::new();
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::Up, mod_none())), b"\x1b[A");
        enc.set_cursor_app_mode(true);
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::Up, mod_none())), b"\x1bOA");
        enc.set_cursor_app_mode(false);
        assert_eq!(enc.encode(&InputKey::Special(SpecialKey::Up, mod_none())), b"\x1b[A");
    }

    // === xterm modifier code ===

    #[test]
    fn test_xterm_mod_codes() {
        assert_eq!(mod_none().xterm_code(), 1);
        assert_eq!(mod_shift().xterm_code(), 2);
        assert_eq!(mod_alt().xterm_code(), 3);
        assert_eq!(mod_ctrl().xterm_code(), 5);
        assert_eq!(
            KeyModifiers { shift: true, alt: true, ctrl: false }.xterm_code(),
            4
        );
        assert_eq!(
            KeyModifiers { shift: true, alt: false, ctrl: true }.xterm_code(),
            6
        );
        assert_eq!(
            KeyModifiers { shift: false, alt: true, ctrl: true }.xterm_code(),
            7
        );
        assert_eq!(
            KeyModifiers { shift: true, alt: true, ctrl: true }.xterm_code(),
            8
        );
    }
}
