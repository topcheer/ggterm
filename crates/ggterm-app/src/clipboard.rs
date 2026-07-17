//! Clipboard integration — read from and write to the system clipboard.
//!
//! Platform support:
//! - **macOS**: `pbpaste` / `pbcopy`
//! - **Linux (X11)**: `xclip` or `xsel`
//! - **Linux (Wayland)**: `wl-copy` / `wl-paste`
//! - **Windows**: `powershell Get-Clipboard` / `clip`
//! - **Other**: stub (returns `None` / `false`)

// ══════════════════════════════════════════════════════════════════
//  Platform detection
// ══════════════════════════════════════════════════════════════════

/// Detected display server type for clipboard access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Only some variants constructed per platform
enum DisplayServer {
    Macos,
    Wayland,
    X11,
    Windows,
    Unsupported,
}

/// Detect the current display server by checking environment variables.
fn detect_display_server() -> DisplayServer {
    #[cfg(target_os = "macos")]
    {
        DisplayServer::Macos
    }

    #[cfg(target_os = "windows")]
    {
        DisplayServer::Windows
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            return DisplayServer::Wayland;
        }
        if std::env::var("DISPLAY").is_ok() {
            return DisplayServer::X11;
        }
        DisplayServer::Unsupported
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", unix)))]
    {
        DisplayServer::Unsupported
    }
}

// ══════════════════════════════════════════════════════════════════
//  Public API
// ══════════════════════════════════════════════════════════════════

/// Source for paste operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PasteSource {
    /// Ctrl+V / menu paste — always uses CLIPBOARD.
    #[default]
    Clipboard,
    /// Middle-click paste — uses PRIMARY on Linux, CLIPBOARD elsewhere.
    MiddleClick,
}

/// Read text appropriate for the given paste source.
pub fn read_for_paste(source: PasteSource) -> Option<String> {
    match source {
        PasteSource::Clipboard => read_clipboard(),
        PasteSource::MiddleClick => read_primary_selection().or_else(read_clipboard),
    }
}

/// Read text from the system clipboard.
///
/// Returns `None` if the clipboard is empty or unavailable.
pub fn read_clipboard() -> Option<String> {
    match detect_display_server() {
        DisplayServer::Macos => read_macos(),
        DisplayServer::Windows => read_windows(),
        DisplayServer::Wayland => read_wayland(),
        DisplayServer::X11 => read_x11(),
        DisplayServer::Unsupported => {
            log::debug!("Clipboard read: unsupported platform");
            None
        }
    }
}

/// Write text to the system clipboard.
///
/// Returns `true` if successful.
pub fn write_clipboard(text: &str) -> bool {
    match detect_display_server() {
        DisplayServer::Macos => write_macos(text),
        DisplayServer::Windows => write_windows(text),
        DisplayServer::Wayland => write_wayland(text),
        DisplayServer::X11 => write_x11(text),
        DisplayServer::Unsupported => {
            log::debug!("Clipboard write: unsupported platform");
            false
        }
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

// ══════════════════════════════════════════════════════════════════
//  Primary selection (X11 / Wayland middle-click paste)
// ══════════════════════════════════════════════════════════════════

/// Read text from the PRIMARY selection (X11 middle-click buffer).
///
/// On Linux, text selected with the mouse is placed in the PRIMARY
/// selection, and middle-click pastes from it. This is separate from
/// the CLIPBOARD selection (Ctrl+C / Ctrl+V).
///
/// Returns `None` on non-Linux platforms or if the PRIMARY selection
/// is empty / unavailable.
pub fn read_primary_selection() -> Option<String> {
    match detect_display_server() {
        DisplayServer::X11 => read_x11_primary(),
        DisplayServer::Wayland => read_wayland_primary(),
        _ => None,
    }
}

/// Write text to the PRIMARY selection (for copy-on-select on Linux).
///
/// On non-Linux platforms this writes to the regular clipboard instead.
pub fn write_primary_selection(text: &str) -> bool {
    match detect_display_server() {
        DisplayServer::X11 => write_x11_primary(text),
        DisplayServer::Wayland => write_wayland_primary(text),
        _ => write_clipboard(text),
    }
}

// ══════════════════════════════════════════════════════════════════
//  Platform implementations
// ══════════════════════════════════════════════════════════════════

// ── macOS ──────────────────────────────────────────────────────────

fn read_macos() -> Option<String> {
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

fn write_macos(text: &str) -> bool {
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

// ── Windows ──────────────────────────────────────────────────────────

fn read_windows() -> Option<String> {
    use std::process::Command;
    let result = Command::new("powershell")
        .args(["-NoProfile", "-Command", "Get-Clipboard"])
        .output();
    match result {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            // PowerShell Get-Clipboard returns CRLF line endings on Windows.
            // Normalize to LF so pasted text is consistent across platforms.
            let text = text.replace("\r\n", "\n");
            // Also strip any lone \r (rare, but possible from legacy apps).
            let text = text.trim_end_matches('\n').to_string();
            if text.is_empty() { None } else { Some(text) }
        }
        _ => None,
    }
}

fn write_windows(text: &str) -> bool {
    use std::process::Command;
    let result = Command::new("clip")
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

// ── Linux Wayland ──────────────────────────────────────────────────

fn read_wayland() -> Option<String> {
    use std::process::Command;
    // Try wl-paste first (wl-clipboard package)
    let result = Command::new("wl-paste").arg("--no-newline").output();
    match result {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            if text.is_empty() { None } else { Some(text) }
        }
        _ => None,
    }
}

fn write_wayland(text: &str) -> bool {
    use std::process::Command;
    let result = Command::new("wl-copy")
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

// ── Linux X11 ──────────────────────────────────────────────────────

fn read_x11() -> Option<String> {
    use std::process::Command;

    // Try xclip first
    let result = Command::new("xclip")
        .args(["-selection", "clipboard", "-o"])
        .output();
    if let Ok(output) = result
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        if !text.is_empty() {
            return Some(text);
        }
    }

    // Fall back to xsel
    let result = Command::new("xsel")
        .args(["--clipboard", "--output"])
        .output();
    match result {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            if text.is_empty() { None } else { Some(text) }
        }
        _ => None,
    }
}

fn write_x11(text: &str) -> bool {
    use std::process::Command;

    // Try xclip first
    let result = Command::new("xclip")
        .args(["-selection", "clipboard"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(stdin) = child.stdin.as_mut() {
                stdin.write_all(text.as_bytes())?;
            }
            child.wait()
        });
    if result.is_ok() {
        return true;
    }

    // Fall back to xsel
    let result = Command::new("xsel")
        .args(["--clipboard", "--input"])
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

// ── Linux X11 PRIMARY selection ───────────────────────────────────

fn read_x11_primary() -> Option<String> {
    use std::process::Command;

    // Try xclip with PRIMARY selection
    let result = Command::new("xclip")
        .args(["-selection", "primary", "-o"])
        .output();
    if let Ok(output) = result
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        if !text.is_empty() {
            return Some(text);
        }
    }

    // Fall back to xsel
    let result = Command::new("xsel")
        .args(["--primary", "--output"])
        .output();
    match result {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            if text.is_empty() { None } else { Some(text) }
        }
        _ => None,
    }
}

fn write_x11_primary(text: &str) -> bool {
    use std::process::Command;

    // Try xclip first
    let result = Command::new("xclip")
        .args(["-selection", "primary"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(stdin) = child.stdin.as_mut() {
                stdin.write_all(text.as_bytes())?;
            }
            child.wait()
        });
    if result.is_ok() {
        return true;
    }

    // Fall back to xsel
    let result = Command::new("xsel")
        .args(["--primary", "--input"])
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

// ── Linux Wayland PRIMARY selection ───────────────────────────────

fn read_wayland_primary() -> Option<String> {
    use std::process::Command;
    // wl-paste --primary
    let result = Command::new("wl-paste")
        .args(["--primary", "--no-newline"])
        .output();
    match result {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            if text.is_empty() { None } else { Some(text) }
        }
        _ => None,
    }
}

fn write_wayland_primary(text: &str) -> bool {
    use std::process::Command;
    let result = Command::new("wl-copy")
        .arg("--primary")
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

// ══════════════════════════════════════════════════════════════════
//  Tests
// ══════════════════════════════════════════════════════════════════

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

    #[test]
    fn test_detect_display_server() {
        let ds = detect_display_server();
        // On macOS this should always be Macos
        #[cfg(target_os = "macos")]
        assert_eq!(ds, DisplayServer::Macos);
        // On other platforms it should be one of the known variants
        #[cfg(not(target_os = "macos"))]
        assert!(matches!(
            ds,
            DisplayServer::Windows
                | DisplayServer::Wayland
                | DisplayServer::X11
                | DisplayServer::Unsupported
        ));
    }

    #[test]
    fn test_set_clipboard_bytes_valid_utf8() {
        // Should not panic on valid UTF-8
        set_clipboard_bytes(b"hello world");
    }

    #[test]
    fn test_set_clipboard_bytes_invalid_utf8() {
        // Should not panic on invalid UTF-8
        set_clipboard_bytes(&[0xff, 0xfe, 0xfd]);
    }

    #[test]
    fn test_read_primary_does_not_panic() {
        let _ = read_primary_selection();
    }

    #[test]
    fn test_write_primary_does_not_panic() {
        let _ = write_primary_selection("test");
    }

    #[test]
    fn test_read_for_paste_clipboard() {
        // Clipboard source should always return Some or None without panic.
        let _ = read_for_paste(PasteSource::Clipboard);
    }

    #[test]
    fn test_read_for_paste_middle_click() {
        // MiddleClick falls back to Clipboard on non-Linux.
        let _ = read_for_paste(PasteSource::MiddleClick);
    }

    #[test]
    fn test_paste_source_default() {
        assert_eq!(PasteSource::default(), PasteSource::Clipboard);
    }
}
