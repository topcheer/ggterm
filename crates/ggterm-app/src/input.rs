//! Input encoding: converts keyboard events to ANSI byte sequences
//! for the PTY.
//!
//! This module does NOT depend on winit — it takes simple key descriptions
//! and produces bytes. The winit→InputKey mapping happens in the app layer.

/// A keyboard input event, independent of any windowing toolkit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputKey {
    /// The key code (e.g., 'a', 'A', '\r', '\t').
    pub key: char,
    /// Modifier flags.
    pub modifiers: KeyModifiers,
}

/// Keyboard modifier flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// Encodes keyboard input into ANSI byte sequences for the PTY.
#[derive(Default)]
pub struct InputEncoder {
    /// Whether cursor keys should send application-mode sequences.
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
        let k = key.key;

        // Special keys
        match k {
            '\r' | '\n' => return b"\r".to_vec(),
            '\t' => return b"\t".to_vec(),
            '\x08' => return b"\x7f".to_vec(), // Backspace → DEL
            '\x1b' => return b"\x1b".to_vec(), // Escape
            _ => {}
        }

        // Ctrl+letter → control character
        if key.modifiers.ctrl && k.is_ascii_alphabetic() {
            let byte = (k.to_ascii_lowercase() as u8) & 0x1f;
            return vec![byte];
        }

        // Alt+key → ESC prefix
        if key.modifiers.alt {
            let mut out = vec![0x1b];
            let mut buf = [0u8; 4];
            out.extend_from_slice(k.encode_utf8(&mut buf).as_bytes());
            return out;
        }

        // Regular printable character
        let mut buf = [0u8; 4];
        k.encode_utf8(&mut buf).as_bytes().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enter_key() {
        let enc = InputEncoder::new();
        let key = InputKey { key: '\r', modifiers: KeyModifiers::default() };
        assert_eq!(enc.encode(&key), b"\r");
    }

    #[test]
    fn test_tab_key() {
        let enc = InputEncoder::new();
        let key = InputKey { key: '\t', modifiers: KeyModifiers::default() };
        assert_eq!(enc.encode(&key), b"\t");
    }

    #[test]
    fn test_backspace() {
        let enc = InputEncoder::new();
        let key = InputKey { key: '\x08', modifiers: KeyModifiers::default() };
        assert_eq!(enc.encode(&key), b"\x7f");
    }

    #[test]
    fn test_ctrl_c() {
        let enc = InputEncoder::new();
        let key = InputKey { key: 'c', modifiers: KeyModifiers { ctrl: true, ..Default::default() } };
        assert_eq!(enc.encode(&key), b"\x03");
    }

    #[test]
    fn test_ctrl_d() {
        let enc = InputEncoder::new();
        let key = InputKey { key: 'd', modifiers: KeyModifiers { ctrl: true, ..Default::default() } };
        assert_eq!(enc.encode(&key), b"\x04");

        // Ctrl+D should be 0x04 regardless of case
        let key_upper = InputKey { key: 'D', modifiers: KeyModifiers { ctrl: true, ..Default::default() } };
        assert_eq!(enc.encode(&key_upper), b"\x04");
    }

    #[test]
    fn test_regular_char() {
        let enc = InputEncoder::new();
        let key = InputKey { key: 'A', modifiers: KeyModifiers::default() };
        assert_eq!(enc.encode(&key), b"A");
    }

    #[test]
    fn test_alt_char() {
        let enc = InputEncoder::new();
        let key = InputKey { key: 'a', modifiers: KeyModifiers { alt: true, ..Default::default() } };
        assert_eq!(enc.encode(&key), b"\x1ba");
    }

    #[test]
    fn test_escape_key() {
        let enc = InputEncoder::new();
        let key = InputKey { key: '\x1b', modifiers: KeyModifiers::default() };
        assert_eq!(enc.encode(&key), b"\x1b");
    }

    #[test]
    fn test_unicode_char() {
        let enc = InputEncoder::new();
        let key = InputKey { key: '你', modifiers: KeyModifiers::default() };
        assert_eq!(enc.encode(&key), "你".as_bytes());
    }
}
