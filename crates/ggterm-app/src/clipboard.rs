//! Clipboard integration — read from and write to the system clipboard.
//!
//! On macOS, uses `pbpaste`/`pbcopy`.
//! On other platforms, returns `None` (stub implementation).

/// Read text from the system clipboard.
///
/// Returns `None` if the clipboard is empty or unavailable.
pub fn read_clipboard() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        let result = Command::new("pbpaste").output();
        match result {
            Ok(output) if output.status.success() => {
                let text = String::from_utf8_lossy(&output.stdout).to_string();
                if text.is_empty() { None } else { Some(text) }
            }
            _ => None,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        log::debug!("Clipboard read not implemented on this platform");
        None
    }
}

/// Write text to the system clipboard.
///
/// Returns `true` if successful.
pub fn write_clipboard(text: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        let result = Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(stdin) = child.stdin.as_mut() {
                    stdin.write_all(text.as_bytes())?;
                }
                child.wait()
            });
        result.is_ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = text;
        log::debug!("Clipboard write not implemented on this platform");
        false
    }
}

/// Write raw bytes to the system clipboard (for OSC 52).
pub fn set_clipboard_bytes(data: &[u8]) {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = write_clipboard(text);
    } else {
        log::warn!("OSC 52 clipboard: invalid UTF-8, ignoring");
    }
}

/// Wrap text in bracketed paste escape sequences if `bracketed` is true.
///
/// When bracketed paste mode (DEC 2004) is active, the terminal wraps
/// pasted text in `\x1b[200~` ... `\x1b[201~` markers so applications
/// can distinguish pasted text from typed input.
pub fn bracket_paste(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let mut bytes = Vec::with_capacity(text.len() + 12);
        bytes.extend_from_slice(b"\x1b[200~");
        bytes.extend_from_slice(text.as_bytes());
        bytes.extend_from_slice(b"\x1b[201~");
        bytes
    } else {
        text.as_bytes().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_clipboard_does_not_panic() {
        let _ = read_clipboard();
    }

    #[test]
    fn test_write_clipboard_returns_bool() {
        let _ = write_clipboard("test");
    }

    #[test]
    fn test_bracket_paste_with_brackets() {
        let result = bracket_paste("hello", true);
        assert_eq!(result, b"\x1b[200~hello\x1b[201~");
    }

    #[test]
    fn test_bracket_paste_without_brackets() {
        let result = bracket_paste("hello", false);
        assert_eq!(result, b"hello");
    }

    #[test]
    fn test_bracket_paste_empty() {
        let result = bracket_paste("", true);
        assert_eq!(result, b"\x1b[200~\x1b[201~");
    }
}
