//! Transport-level FFI for mobile integration.
//!
//! Extends the C-ABI with session management that pairs terminal handles
//! with transports (SSH or local PTY). This allows Flutter/Dart to:
//!
//! 1. Create sessions with terminal + transport
//! 2. Connect via SSH (password or key auth)
//! 3. Pump data: read from transport → process into terminal
//! 4. Flush input: take terminal input → write to transport
//! 5. Query transport status

// FFI unsafe functions have simple null-safety contracts documented inline.
#![allow(clippy::missing_safety_doc)]

use crate::{GGTermCell, TerminalHandle};
use std::collections::HashMap;
#[cfg(feature = "ssh")]
use std::ffi::CStr;
use std::ffi::c_char;
use std::sync::{Mutex, OnceLock};

use ggterm_core::TerminalTransport;

/// A session pairs a terminal handle with an optional transport.
pub struct MobileSession {
    pub handle: TerminalHandle,
    pub transport: Option<Box<dyn TerminalTransport>>,
}

/// Global session registry.
static SESSIONS: OnceLock<Mutex<HashMap<u32, MobileSession>>> = OnceLock::new();
static NEXT_ID: OnceLock<Mutex<u32>> = OnceLock::new();
static LAST_ERROR: OnceLock<Mutex<String>> = OnceLock::new();

/// Create a new session and return its ID (used by p2p module).
#[allow(dead_code)]
pub(crate) fn create_session(cols: usize, rows: usize) -> u32 {
    let id = next_id();
    let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    map.insert(
        id,
        MobileSession {
            handle: TerminalHandle::new(cols.max(1), rows.max(1)),
            transport: None,
        },
    );
    id
}

/// Access the last error storage (for p2p module).
#[allow(dead_code)]
pub(crate) fn last_error_storage() -> &'static Mutex<String> {
    LAST_ERROR.get_or_init(|| Mutex::new(String::new()))
}

pub(crate) fn sessions() -> &'static Mutex<HashMap<u32, MobileSession>> {
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn next_id() -> u32 {
    let counter = NEXT_ID.get_or_init(|| Mutex::new(1));
    let mut id = counter.lock().unwrap_or_else(|e| e.into_inner());
    let val = *id;
    *id += 1;
    val
}

pub(crate) fn set_error(msg: impl Into<String>) {
    let storage = LAST_ERROR.get_or_init(|| Mutex::new(String::new()));
    *storage.lock().unwrap_or_else(|e| e.into_inner()) = msg.into();
}

// ── Session Lifecycle ──────────────────────────────────────────────────

/// Create a new session with a terminal at the given dimensions.
/// Returns a session ID > 0 on success, 0 on failure.
#[unsafe(no_mangle)]
pub extern "C" fn ggterm_session_create(cols: usize, rows: usize) -> u32 {
    let id = next_id();
    let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    map.insert(
        id,
        MobileSession {
            handle: TerminalHandle::new(cols.max(1), rows.max(1)),
            transport: None,
        },
    );
    id
}

/// Destroy a session, dropping its terminal and transport.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_session_destroy(id: u32) {
    // Remove from map while holding lock, but drop the value after releasing
    // the lock to prevent deadlock if the transport's Drop impl calls back
    // into the FFI layer (e.g., SSH channel close).
    let removed = {
        let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
        map.remove(&id)
    };
    // removed is dropped here, after the mutex guard is released.
    drop(removed);
}

/// Get the number of active sessions.
#[unsafe(no_mangle)]
pub extern "C" fn ggterm_session_count() -> usize {
    sessions().lock().unwrap_or_else(|e| e.into_inner()).len()
}

// ── Terminal Operations ────────────────────────────────────────────────

/// Feed raw bytes (from transport output) into the terminal for processing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_session_process_bytes(id: u32, data: *const u8, len: usize) {
    if data.is_null() || len == 0 {
        return;
    }
    let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = map.get_mut(&id) {
        unsafe {
            let slice = std::slice::from_raw_parts(data, len);
            s.handle.process_bytes(slice);
        }
    }
}

/// Queue input bytes (keystrokes) for the host.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_session_send_input(id: u32, data: *const u8, len: usize) {
    if data.is_null() || len == 0 {
        return;
    }
    let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = map.get_mut(&id) {
        unsafe {
            let slice = std::slice::from_raw_parts(data, len);
            s.handle.send_input(slice);
        }
    }
}

/// Read pending input bytes to send to the transport. Returns bytes written.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_session_take_input(id: u32, buf: *mut u8, max_len: usize) -> usize {
    if buf.is_null() || max_len == 0 {
        return 0;
    }
    let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = map.get_mut(&id) {
        let input = s.handle.take_input();
        let n = input.len().min(max_len);
        if n > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(input.as_ptr(), buf, n);
            }
        }
        // Re-queue any bytes that didn't fit in the caller's buffer.
        if input.len() > n {
            s.handle.send_input(&input[n..]);
        }
        n
    } else {
        0
    }
}

/// Read terminal cells into a flat array for rendering.
/// Returns the number of cells written.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_session_read_cells(
    id: u32,
    buf: *mut GGTermCell,
    max_cells: usize,
) -> usize {
    if buf.is_null() || max_cells == 0 {
        return 0;
    }
    let map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = map.get(&id) {
        let grid = s.handle.terminal.grid();
        let cols = grid.width();
        let rows = grid.height();
        let total = cols * rows;
        let n = total.min(max_cells);

        // Write directly to the output buffer — avoids allocating a Vec.
        // Iterate row-by-row to avoid per-cell division/modulo.
        let mut written = 0usize;
        for row_idx in 0..rows {
            if written >= n {
                break;
            }
            let row = grid.display_row(row_idx);
            for col in 0..cols {
                if written >= n {
                    break;
                }
                unsafe {
                    *buf.add(written) = match row {
                        Some(r) => match r.cells.get(col) {
                            Some(c) => GGTermCell::from_cell(c),
                            None => GGTermCell::default(),
                        },
                        None => GGTermCell::default(),
                    };
                }
                written += 1;
            }
        }
        n
    } else {
        0
    }
}

/// Get terminal dimensions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_session_dimensions(id: u32, cols: *mut usize, rows: *mut usize) {
    if cols.is_null() || rows.is_null() {
        return;
    }
    let map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = map.get(&id) {
        let grid = s.handle.terminal.grid();
        unsafe {
            *cols = grid.width();
            *rows = grid.height();
        }
    }
}

/// Get cursor position.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_session_cursor(id: u32, col: *mut usize, row: *mut usize) {
    if col.is_null() || row.is_null() {
        return;
    }
    let map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = map.get(&id) {
        let (c, r) = s.handle.terminal.cursor();
        unsafe {
            *col = c;
            *row = r;
        }
    }
}

/// Resize the terminal grid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_session_resize(id: u32, cols: usize, rows: usize) {
    let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = map.get_mut(&id) {
        s.handle
            .terminal
            .grid_mut()
            .resize(cols.max(1), rows.max(1));
        // Also resize transport if present (clamp to >= 1 for PTY safety).
        if let Some(t) = s.transport.as_mut() {
            t.resize(cols.max(1), rows.max(1));
        }
    }
}

/// Get the terminal title (OSC 0/2). Writes up to `max_len` bytes (including NUL)
/// into `buf`. Returns the number of bytes written (excluding NUL), or 0 if
/// the session doesn't exist or the title is empty.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_session_title(id: u32, buf: *mut c_char, max_len: usize) -> usize {
    if buf.is_null() || max_len == 0 {
        return 0;
    }
    let map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = map.get(&id) else {
        return 0;
    };
    let title = s.handle.terminal.title();
    let bytes = title.as_bytes();
    let copy_len = bytes.len().min(max_len - 1); // -1 for NUL
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, copy_len);
        *buf.add(copy_len) = 0; // NUL terminate
    }
    copy_len
}

/// Get the current working directory (OSC 7). Writes up to `max_len` bytes
/// (including NUL) into `buf`. Returns bytes written (excl. NUL), or 0 if
/// the session has no cwd.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_session_cwd(id: u32, buf: *mut c_char, max_len: usize) -> usize {
    if buf.is_null() || max_len == 0 {
        return 0;
    }
    let map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = map.get(&id) else {
        return 0;
    };
    let Some(cwd) = s.handle.terminal.cwd() else {
        return 0;
    };
    let path_str = cwd.to_string_lossy();
    let bytes = path_str.as_bytes();
    let copy_len = bytes.len().min(max_len - 1);
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, copy_len);
        *buf.add(copy_len) = 0;
    }
    copy_len
}

/// Consume the bell flag. Returns 1 if bell was rung, 0 otherwise.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_session_take_bell(id: u32) -> i32 {
    let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = map.get_mut(&id)
        && s.handle.terminal.take_bell()
    {
        return 1;
    }
    0
}

// ── Transport: Data Pump ───────────────────────────────────────────────

/// Read from the transport, feed bytes into the terminal.
/// Returns the number of bytes read and processed.
#[unsafe(no_mangle)]
pub extern "C" fn ggterm_transport_pump(id: u32) -> usize {
    let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = map.get_mut(&id) else {
        return 0;
    };

    // Read from transport
    if let Some(t) = s.transport.as_mut() {
        let data = t.read();
        let n = data.len();
        if n > 0 {
            s.handle.process_bytes(&data);
        }
        return n;
    }

    0
}

/// Flush queued input to the transport (send keystrokes to remote host).
#[unsafe(no_mangle)]
pub extern "C" fn ggterm_transport_flush(id: u32) {
    let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = map.get_mut(&id) else {
        return;
    };

    let input = s.handle.take_input();
    if input.is_empty() {
        return;
    }

    if let Some(t) = s.transport.as_mut()
        && t.is_alive()
    {
        t.write(&input);
    }
}

/// Check if the transport is alive.
#[unsafe(no_mangle)]
pub extern "C" fn ggterm_transport_is_alive(id: u32) -> i32 {
    let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = map.get_mut(&id) else {
        return 0;
    };

    if let Some(t) = s.transport.as_mut() {
        if t.is_alive() {
            return 1;
        }
    } else {
        // No transport = local mode, always "alive"
        return 1;
    }
    0
}

/// Scroll the terminal viewport up (toward older scrollback).
/// `lines` is the number of rows to scroll.
#[unsafe(no_mangle)]
pub extern "C" fn ggterm_session_scroll_up(id: u32, lines: usize) {
    let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = map.get_mut(&id) {
        s.handle.terminal.grid_mut().scroll_up_viewport(lines);
    }
}

/// Scroll the terminal viewport down (toward newer content).
#[unsafe(no_mangle)]
pub extern "C" fn ggterm_session_scroll_down(id: u32, lines: usize) {
    let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = map.get_mut(&id) {
        s.handle.terminal.grid_mut().scroll_down_viewport(lines);
    }
}

/// Reset viewport to the bottom (most recent content).
#[unsafe(no_mangle)]
pub extern "C" fn ggterm_session_reset_viewport(id: u32) {
    let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = map.get_mut(&id) {
        s.handle.terminal.grid_mut().reset_viewport();
    }
}

/// Get the current display offset (0 = at bottom, >0 = scrolled up).
#[unsafe(no_mangle)]
pub extern "C" fn ggterm_session_display_offset(id: u32) -> usize {
    let map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = map.get(&id) {
        s.handle.terminal.grid().display_offset()
    } else {
        0
    }
}

/// Get the total number of scrollback lines (off-screen history).
#[unsafe(no_mangle)]
pub extern "C" fn ggterm_session_scrollback_len(id: u32) -> usize {
    let map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = map.get(&id) {
        s.handle.terminal.grid().scrollback_len()
    } else {
        0
    }
}

// ── Transport: SSH ─────────────────────────────────────────────────────

/// Connect to an SSH host with password authentication.
/// Returns 0 on success, -1 on failure (use ggterm_last_error for details).
#[cfg(feature = "ssh")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_ssh_connect(
    id: u32,
    host: *const c_char,
    port: u16,
    user: *const c_char,
    password: *const c_char,
) -> i32 {
    let (host_s, user_s, pass_s) = unsafe {
        if host.is_null() || user.is_null() || password.is_null() {
            set_error("null argument to ggterm_ssh_connect");
            return -1;
        }
        let host_s = match CStr::from_ptr(host).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => {
                set_error("invalid UTF-8 in host");
                return -1;
            }
        };
        let user_s = match CStr::from_ptr(user).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => {
                set_error("invalid UTF-8 in user");
                return -1;
            }
        };
        let pass_s = match CStr::from_ptr(password).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => {
                set_error("invalid UTF-8 in password");
                return -1;
            }
        };
        (host_s, user_s, pass_s)
    };

    // Read cols/rows from session to set PTY size
    let (cols, rows) = {
        let map = sessions().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = map.get(&id) {
            let g = s.handle.terminal.grid();
            (g.width(), g.height())
        } else {
            set_error("session not found");
            return -1;
        }
    };

    // Connect (this is blocking)
    match ggterm_ssh::SshSession::connect(&host_s, port, &user_s, &pass_s) {
        Ok(session) => {
            let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
            if let Some(s) = map.get_mut(&id) {
                // We need to resize the session to match our terminal size
                let mut transport: Box<dyn TerminalTransport> = Box::new(session);
                transport.resize(cols, rows);
                s.transport = Some(transport);
                0
            } else {
                set_error("session disappeared during connect");
                -1
            }
        }
        Err(e) => {
            set_error(format!("SSH connection failed: {e}"));
            -1
        }
    }
}

/// Connect to an SSH host with public key authentication.
#[cfg(feature = "ssh")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_ssh_connect_key(
    id: u32,
    host: *const c_char,
    port: u16,
    user: *const c_char,
    key_path: *const c_char,
) -> i32 {
    let (host_s, user_s, key_s) = unsafe {
        if host.is_null() || user.is_null() || key_path.is_null() {
            set_error("null argument to ggterm_ssh_connect_key");
            return -1;
        }
        let host_s = match CStr::from_ptr(host).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => {
                set_error("invalid UTF-8 in host");
                return -1;
            }
        };
        let user_s = match CStr::from_ptr(user).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => {
                set_error("invalid UTF-8 in user");
                return -1;
            }
        };
        let key_s = match CStr::from_ptr(key_path).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => {
                set_error("invalid UTF-8 in key_path");
                return -1;
            }
        };
        (host_s, user_s, key_s)
    };

    let (cols, rows) = {
        let map = sessions().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = map.get(&id) {
            let g = s.handle.terminal.grid();
            (g.width(), g.height())
        } else {
            set_error("session not found");
            return -1;
        }
    };

    match ggterm_ssh::SshSession::connect_with_key(
        &host_s,
        port,
        &user_s,
        std::path::Path::new(&key_s),
    ) {
        Ok(session) => {
            let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
            if let Some(s) = map.get_mut(&id) {
                let mut transport: Box<dyn TerminalTransport> = Box::new(session);
                transport.resize(cols, rows);
                s.transport = Some(transport);
                0
            } else {
                set_error("session disappeared during connect");
                -1
            }
        }
        Err(e) => {
            set_error(format!("SSH connection failed: {e}"));
            -1
        }
    }
}

// ── Transport: Echo (for testing without SSH) ──────────────────────────

/// An echo transport that simply echoes input back as output.
/// Useful for testing the mobile app without a real SSH server.
pub struct EchoTransport {
    pending_output: Vec<u8>,
    alive: bool,
}

impl Default for EchoTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl EchoTransport {
    pub fn new() -> Self {
        let mut s = Self {
            pending_output: Vec::new(),
            alive: true,
        };
        s.pending_output.extend_from_slice(
            b"GGTerm Echo Mode\r\nType commands and they will be echoed back.\r\n",
        );
        s
    }
}

impl TerminalTransport for EchoTransport {
    fn read(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending_output)
    }

    fn write(&mut self, data: &[u8]) {
        // Echo back with CR before LF for terminal display.
        for &b in data {
            if b == b'\n' {
                self.pending_output.push(b'\r');
            }
            self.pending_output.push(b);
        }
    }

    fn resize(&mut self, _cols: usize, _rows: usize) {}

    fn is_alive(&mut self) -> bool {
        self.alive
    }
}

/// Start an echo transport for testing (no real connection needed).
/// Returns 0 on success.
#[unsafe(no_mangle)]
pub extern "C" fn ggterm_echo_connect(id: u32) -> i32 {
    let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = map.get_mut(&id) {
        s.transport = Some(Box::new(EchoTransport::new()));
        0
    } else {
        set_error("session not found");
        -1
    }
}

/// Start a local shell transport (Android only).
///
/// Uses forkpty() to spawn /system/bin/sh inside a PTY.
/// Returns 0 on success, -1 on failure.
///
/// On non-Android platforms, always returns -1.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn ggterm_local_shell_connect(id: u32) -> i32 {
    let (cols, rows) = {
        let map = sessions().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = map.get(&id) {
            let g = s.handle.terminal.grid();
            (g.width(), g.height())
        } else {
            set_error("session not found");
            return -1;
        }
    };

    match crate::local_shell::LocalShellTransport::connect(cols, rows) {
        Ok(transport) => {
            let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
            if let Some(s) = map.get_mut(&id) {
                let mut t: Box<dyn TerminalTransport> = Box::new(transport);
                t.resize(cols, rows);
                s.transport = Some(t);
                0
            } else {
                set_error("session disappeared during local shell connect");
                -1
            }
        }
        Err(e) => {
            set_error(format!("Local shell failed: {e}"));
            -1
        }
    }
}

/// Stub for non-Android platforms — always returns -1.
#[cfg(not(target_os = "android"))]
#[unsafe(no_mangle)]
pub extern "C" fn ggterm_local_shell_connect(_id: u32) -> i32 {
    set_error("local shell is only available on Android");
    -1
}

// ── Error Reporting ────────────────────────────────────────────────────

/// Get the last error message as a C string.
/// The returned pointer is valid until the next FFI call.
#[unsafe(no_mangle)]
pub extern "C" fn ggterm_last_error() -> *const c_char {
    static ERROR_BUF: OnceLock<Mutex<std::ffi::CString>> = OnceLock::new();
    let buf = ERROR_BUF.get_or_init(|| Mutex::new(std::ffi::CString::new("").unwrap()));
    let storage = LAST_ERROR.get_or_init(|| Mutex::new(String::new()));
    let msg = storage.lock().unwrap_or_else(|e| e.into_inner());
    if msg.is_empty() {
        return c"".as_ptr();
    }
    // Update the buffer and return pointer in a single lock.
    let cstr = std::ffi::CString::new(msg.as_str())
        .unwrap_or_else(|_| std::ffi::CString::new("").unwrap());
    let mut b = buf.lock().unwrap_or_else(|e| e.into_inner());
    *b = cstr;
    b.as_ptr()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_session_create_and_destroy() {
        let id = ggterm_session_create(80, 24);
        assert!(id > 0);
        // Verify the session is functional
        let (mut cols, mut rows) = (0usize, 0usize);
        unsafe { ggterm_session_dimensions(id, &mut cols, &mut rows) };
        assert_eq!(cols, 80);
        assert_eq!(rows, 24);
        unsafe { ggterm_session_destroy(id) };
    }

    #[test]
    fn t_session_process_bytes() {
        let id = ggterm_session_create(80, 24);
        let data = b"Hello";
        unsafe {
            ggterm_session_process_bytes(id, data.as_ptr(), data.len());
        }

        // Read cells to verify
        let mut cells = vec![GGTermCell::default(); 80 * 24];
        let n = unsafe { ggterm_session_read_cells(id, cells.as_mut_ptr(), cells.len()) };
        assert_eq!(n, 80 * 24);
        assert_eq!(cells[0].char_code, 'H' as u32);
        assert_eq!(cells[1].char_code, 'e' as u32);

        unsafe { ggterm_session_destroy(id) };
    }

    #[test]
    fn t_session_send_and_take_input() {
        let id = ggterm_session_create(80, 24);
        let data = b"ls\n";
        unsafe {
            ggterm_session_send_input(id, data.as_ptr(), data.len());
        }

        let mut buf = [0u8; 64];
        let n = unsafe { ggterm_session_take_input(id, buf.as_mut_ptr(), buf.len()) };
        assert_eq!(n, 3);
        assert_eq!(&buf[..n], b"ls\n");

        unsafe { ggterm_session_destroy(id) };
    }

    #[test]
    fn t_session_dimensions() {
        let id = ggterm_session_create(100, 30);
        let (mut cols, mut rows) = (0usize, 0usize);
        unsafe { ggterm_session_dimensions(id, &mut cols, &mut rows) };
        assert_eq!(cols, 100);
        assert_eq!(rows, 30);
        unsafe { ggterm_session_destroy(id) };
    }

    #[test]
    fn t_session_resize() {
        let id = ggterm_session_create(80, 24);
        unsafe { ggterm_session_resize(id, 120, 40) };

        let (mut cols, mut rows) = (0usize, 0usize);
        unsafe { ggterm_session_dimensions(id, &mut cols, &mut rows) };
        assert_eq!(cols, 120);
        assert_eq!(rows, 40);
        unsafe { ggterm_session_destroy(id) };
    }

    #[test]
    fn t_session_cursor() {
        let id = ggterm_session_create(80, 24);
        let data = b"Hi";
        unsafe {
            ggterm_session_process_bytes(id, data.as_ptr(), data.len());
        }

        let (mut col, mut row) = (0usize, 0usize);
        unsafe { ggterm_session_cursor(id, &mut col, &mut row) };
        assert_eq!(col, 2);
        assert_eq!(row, 0);
        unsafe { ggterm_session_destroy(id) };
    }

    #[test]
    fn t_session_take_bell() {
        let id = ggterm_session_create(80, 24);
        let bell = b"\x07";
        unsafe {
            ggterm_session_process_bytes(id, bell.as_ptr(), bell.len());
        }
        assert_eq!(unsafe { ggterm_session_take_bell(id) }, 1);
        // Bell consumed
        assert_eq!(unsafe { ggterm_session_take_bell(id) }, 0);
        unsafe { ggterm_session_destroy(id) };
    }

    #[test]
    fn t_echo_transport_connect() {
        let id = ggterm_session_create(80, 24);
        assert_eq!(ggterm_echo_connect(id), 0);
        assert_eq!(ggterm_transport_is_alive(id), 1);
        unsafe { ggterm_session_destroy(id) };
    }

    #[test]
    fn t_echo_transport_pump() {
        let id = ggterm_session_create(80, 24);
        ggterm_echo_connect(id);

        // Pump should read the welcome message
        let n = ggterm_transport_pump(id);
        assert!(n > 0);

        // Read cells to verify text was processed
        let mut cells = vec![GGTermCell::default(); 80 * 24];
        let ncells = unsafe { ggterm_session_read_cells(id, cells.as_mut_ptr(), cells.len()) };
        assert!(ncells > 0);
        // First char should be 'G' from "GGTerm Echo Mode"
        assert_eq!(cells[0].char_code, 'G' as u32);

        unsafe { ggterm_session_destroy(id) };
    }

    #[test]
    fn t_echo_transport_flush() {
        let id = ggterm_session_create(80, 24);
        ggterm_echo_connect(id);

        // Send input
        let data = b"test";
        unsafe {
            ggterm_session_send_input(id, data.as_ptr(), data.len());
        }

        // Flush to transport (echo transport will queue it back)
        ggterm_transport_flush(id);

        // Pump should read the echoed input
        let n = ggterm_transport_pump(id);
        assert!(n > 0);

        unsafe { ggterm_session_destroy(id) };
    }

    #[test]
    fn t_transport_pump_no_transport() {
        let id = ggterm_session_create(80, 24);
        // No transport connected, pump should return 0
        assert_eq!(ggterm_transport_pump(id), 0);
        // is_alive should return 1 (local mode always alive)
        assert_eq!(ggterm_transport_is_alive(id), 1);
        unsafe { ggterm_session_destroy(id) };
    }

    #[test]
    fn t_destroy_cleans_up() {
        let id = ggterm_session_create(80, 24);
        // Verify the session was created (id > 0).
        assert!(id > 0, "session_create should return non-zero id");
        // Verify it is alive.
        assert_eq!(ggterm_transport_is_alive(id), 1);
        unsafe { ggterm_session_destroy(id) };
        // After destroy, the session should be gone.
        assert_eq!(ggterm_transport_is_alive(id), 0);
        assert_eq!(ggterm_transport_pump(id), 0);
        // Verify the session is gone by trying to get dimensions — should be 0,0.
        let (mut cols, mut rows) = (0usize, 0usize);
        unsafe { ggterm_session_dimensions(id, &mut cols, &mut rows) };
        // A destroyed session has no grid, so cols/rows should remain 0.
        assert_eq!(cols, 0);
        assert_eq!(rows, 0);
    }

    #[test]
    fn t_last_error_empty() {
        let ptr = ggterm_last_error();
        // Should return a valid pointer (empty string or message)
        assert!(!ptr.is_null());
    }

    #[test]
    fn t_session_title() {
        let id = ggterm_session_create(80, 24);
        // Set title via OSC 2.
        let osc = b"\x1b]2;My Terminal Title\x07";
        unsafe {
            ggterm_session_process_bytes(id, osc.as_ptr(), osc.len());
        }
        let mut buf = [0i8; 128];
        let n = unsafe { ggterm_session_title(id, buf.as_mut_ptr(), buf.len()) };
        assert!(n > 0);
        let title_bytes: Vec<u8> = buf[..n].iter().map(|&b| b as u8).collect();
        assert_eq!(String::from_utf8_lossy(&title_bytes), "My Terminal Title");
        unsafe { ggterm_session_destroy(id) };
    }

    #[test]
    fn t_session_title_empty() {
        let id = ggterm_session_create(80, 24);
        let mut buf = [0i8; 128];
        let n = unsafe { ggterm_session_title(id, buf.as_mut_ptr(), buf.len()) };
        // No title set — should be 0 or very short.
        assert_eq!(n, 0);
        unsafe { ggterm_session_destroy(id) };
    }

    #[test]
    fn t_session_title_truncated() {
        let id = ggterm_session_create(80, 24);
        let long_title = "A".repeat(200);
        let osc = format!("\x1b]2;{long_title}\x07");
        unsafe {
            ggterm_session_process_bytes(id, osc.as_ptr(), osc.len());
        }
        let mut buf = [0i8; 32];
        let n = unsafe { ggterm_session_title(id, buf.as_mut_ptr(), buf.len()) };
        assert_eq!(n, 31); // max_len - 1 = 31
        unsafe { ggterm_session_destroy(id) };
    }

    #[test]
    fn t_session_title_null_args() {
        let id = ggterm_session_create(80, 24);
        let n = unsafe { ggterm_session_title(id, std::ptr::null_mut(), 128) };
        assert_eq!(n, 0);
        let mut buf = [0i8; 128];
        let n = unsafe { ggterm_session_title(id, buf.as_mut_ptr(), 0) };
        assert_eq!(n, 0);
        unsafe { ggterm_session_destroy(id) };
    }

    #[test]
    fn t_multiple_sessions() {
        let id1 = ggterm_session_create(80, 24);
        let id2 = ggterm_session_create(120, 40);
        assert_ne!(id1, id2);

        let (mut cols1, mut rows1) = (0usize, 0usize);
        unsafe { ggterm_session_dimensions(id1, &mut cols1, &mut rows1) };
        assert_eq!(cols1, 80);

        let (mut cols2, mut rows2) = (0usize, 0usize);
        unsafe { ggterm_session_dimensions(id2, &mut cols2, &mut rows2) };
        assert_eq!(cols2, 120);

        unsafe {
            ggterm_session_destroy(id1);
            ggterm_session_destroy(id2);
        }
    }

    #[test]
    fn t_scroll_display_offset() {
        let id = ggterm_session_create(80, 24);
        // Initially at bottom — offset should be 0.
        assert_eq!(ggterm_session_display_offset(id), 0);

        // Process enough output to fill scrollback.
        for _ in 0..50 {
            let line = b"line of text\r\n";
            unsafe {
                ggterm_session_process_bytes(id, line.as_ptr(), line.len());
            }
        }

        // Scroll up 5 lines.
        ggterm_session_scroll_up(id, 5);
        assert_eq!(ggterm_session_display_offset(id), 5);

        // Scroll up more.
        ggterm_session_scroll_up(id, 3);
        assert_eq!(ggterm_session_display_offset(id), 8);

        // Scroll down 2.
        ggterm_session_scroll_down(id, 2);
        assert_eq!(ggterm_session_display_offset(id), 6);

        // Reset to bottom.
        ggterm_session_reset_viewport(id);
        assert_eq!(ggterm_session_display_offset(id), 0);

        unsafe { ggterm_session_destroy(id) };
    }

    #[test]
    fn t_scrollback_len() {
        let id = ggterm_session_create(80, 24);
        // Initially no scrollback.
        assert_eq!(ggterm_session_scrollback_len(id), 0);

        // Process enough output to create scrollback.
        for _ in 0..50 {
            let line = b"line of text\r\n";
            unsafe {
                ggterm_session_process_bytes(id, line.as_ptr(), line.len());
            }
        }

        // Should have scrollback lines.
        let len = ggterm_session_scrollback_len(id);
        assert!(len > 0, "scrollback should have content after 50 lines");

        unsafe { ggterm_session_destroy(id) };
    }

    #[test]
    fn t_scrollback_len_invalid_session() {
        assert_eq!(ggterm_session_scrollback_len(99999), 0);
    }
}
