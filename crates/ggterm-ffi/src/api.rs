//! High-level API for flutter_rust_bridge (P7-E).
//!
//! Provides [`SessionManager`] for managing multiple terminal sessions
//! from Flutter/Dart via flutter_rust_bridge.

use crate::{GGTermCell, TerminalHandle};
use std::collections::HashMap;

/// Screen data snapshot for Flutter rendering.
pub struct ScreenData {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<GGTermCell>,
    pub cursor_col: usize,
    pub cursor_row: usize,
    pub cursor_visible: bool,
    /// Whether bracketed paste mode (DECSET 2004) is active.
    pub bracketed_paste: bool,
}

/// Error type for session operations.
#[derive(Debug)]
pub enum TerminalSessionError {
    NotFound,
    ConnectionFailed(String),
}

impl std::fmt::Display for TerminalSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "session not found"),
            Self::ConnectionFailed(msg) => write!(f, "connection failed: {msg}"),
        }
    }
}

/// A terminal session with its handle.
pub struct TerminalSession {
    pub handle: TerminalHandle,
}

/// Manages multiple terminal sessions.
pub struct SessionManager {
    sessions: HashMap<u32, TerminalSession>,
    next_id: u32,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            next_id: 1,
        }
    }

    pub fn create_session(&mut self, cols: usize, rows: usize) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.sessions.insert(
            id,
            TerminalSession {
                handle: TerminalHandle::new(cols, rows),
            },
        );
        id
    }

    pub fn close_session(&mut self, id: u32) {
        self.sessions.remove(&id);
    }

    pub fn process_bytes(&mut self, id: u32, data: &[u8]) {
        if let Some(s) = self.sessions.get_mut(&id) {
            s.handle.process_bytes(data);
        }
    }

    pub fn send_input(&mut self, id: u32, data: &[u8]) {
        if let Some(s) = self.sessions.get_mut(&id) {
            s.handle.send_input(data);
        }
    }

    pub fn take_input(&mut self, id: u32) -> Vec<u8> {
        if let Some(s) = self.sessions.get_mut(&id) {
            s.handle.take_input()
        } else {
            Vec::new()
        }
    }

    pub fn get_screen_data(&self, id: u32) -> ScreenData {
        if let Some(s) = self.sessions.get(&id) {
            let grid = s.handle.terminal.grid();
            let cells = crate::grid_to_ffi(grid);
            let (col, row) = s.handle.terminal.cursor();
            ScreenData {
                cols: grid.width(),
                rows: grid.height(),
                cells,
                cursor_col: col,
                cursor_row: row,
                cursor_visible: s.handle.terminal.cursor_visible(),
                bracketed_paste: s.handle.terminal.bracketed_paste(),
            }
        } else {
            ScreenData {
                cols: 0,
                rows: 0,
                cells: Vec::new(),
                cursor_col: 0,
                cursor_row: 0,
                cursor_visible: false,
                bracketed_paste: false,
            }
        }
    }

    pub fn resize(&mut self, id: u32, cols: usize, rows: usize) {
        if let Some(s) = self.sessions.get_mut(&id) {
            s.handle.terminal.grid_mut().resize(cols, rows);
        }
    }

    pub fn take_bell(&mut self, id: u32) -> bool {
        if let Some(s) = self.sessions.get_mut(&id) {
            s.handle.terminal.take_bell()
        } else {
            false
        }
    }

    /// Number of active sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Returns true if a session with the given ID exists.
    pub fn has_session(&self, id: u32) -> bool {
        self.sessions.contains_key(&id)
    }

    /// Returns a sorted list of all active session IDs.
    pub fn session_ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.sessions.keys().copied().collect();
        ids.sort();
        ids
    }

    /// Request AI assistance for a session (P7-E).
    ///
    /// `action`: 0=explain, 1=suggest, 2=help, 3=nl2command.
    /// Returns the AI response as a string.
    pub fn request_ai(&self, _id: u32, action: u8) -> String {
        // Placeholder — in production this builds an AIContext from the
        // session's terminal state and dispatches to AIBridge.
        match action {
            0 => "Explanation: This is a terminal output.".to_string(),
            1 => "Suggestion: Try 'ls -la' to list files.".to_string(),
            2 => "Help: Use Ctrl+Shift+F to search scrollback.".to_string(),
            3 => "Command: ls -la".to_string(),
            _ => "Unknown action".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_create_session() {
        let mut mgr = SessionManager::new();
        let id = mgr.create_session(80, 24);
        assert_eq!(id, 1);
    }

    #[test]
    fn t_process_and_get_screen() {
        let mut mgr = SessionManager::new();
        let id = mgr.create_session(80, 24);
        mgr.process_bytes(id, b"Hello");
        let data = mgr.get_screen_data(id);
        assert_eq!(data.cols, 80);
        assert_eq!(data.rows, 24);
        assert_eq!(data.cells[0].char_code, 'H' as u32);
    }

    #[test]
    fn t_send_and_take_input() {
        let mut mgr = SessionManager::new();
        let id = mgr.create_session(80, 24);
        mgr.send_input(id, b"ls\n");
        assert_eq!(mgr.take_input(id), b"ls\n");
        assert!(mgr.take_input(id).is_empty());
    }

    #[test]
    fn t_resize() {
        let mut mgr = SessionManager::new();
        let id = mgr.create_session(80, 24);
        mgr.resize(id, 120, 40);
        let data = mgr.get_screen_data(id);
        assert_eq!(data.cols, 120);
        assert_eq!(data.rows, 40);
    }

    #[test]
    fn t_invalid_id_safe() {
        let mut mgr = SessionManager::new();
        mgr.process_bytes(999, b"test");
        mgr.send_input(999, b"test");
        assert!(mgr.take_input(999).is_empty());
        let data = mgr.get_screen_data(999);
        assert_eq!(data.cols, 0);
    }

    #[test]
    fn t_close_session() {
        let mut mgr = SessionManager::new();
        let id = mgr.create_session(80, 24);
        mgr.close_session(id);
        let data = mgr.get_screen_data(id);
        assert_eq!(data.cols, 0);
    }

    // ── P7-E: Enhanced API tests ─────────────────────────────

    #[test]
    fn t_session_count_empty() {
        let mgr = SessionManager::new();
        assert_eq!(mgr.session_count(), 0);
    }

    #[test]
    fn t_session_count_after_create() {
        let mut mgr = SessionManager::new();
        mgr.create_session(80, 24);
        mgr.create_session(80, 24);
        assert_eq!(mgr.session_count(), 2);
    }

    #[test]
    fn t_has_session() {
        let mut mgr = SessionManager::new();
        let id = mgr.create_session(80, 24);
        assert!(mgr.has_session(id));
        assert!(!mgr.has_session(999));
    }

    #[test]
    fn t_session_ids_sorted() {
        let mut mgr = SessionManager::new();
        let id1 = mgr.create_session(80, 24);
        let id2 = mgr.create_session(80, 24);
        let id3 = mgr.create_session(80, 24);
        let ids = mgr.session_ids();
        assert_eq!(ids, vec![id1, id2, id3]);
    }

    #[test]
    fn t_session_ids_after_close() {
        let mut mgr = SessionManager::new();
        let id1 = mgr.create_session(80, 24);
        let id2 = mgr.create_session(80, 24);
        mgr.close_session(id1);
        let ids = mgr.session_ids();
        assert_eq!(ids, vec![id2]);
    }

    #[test]
    fn t_request_ai_explain() {
        let mgr = SessionManager::new();
        let result = mgr.request_ai(0, 0);
        assert!(result.contains("Explanation"));
    }

    #[test]
    fn t_request_ai_suggest() {
        let mgr = SessionManager::new();
        let result = mgr.request_ai(0, 1);
        assert!(result.contains("Suggestion"));
    }

    #[test]
    fn t_request_ai_help() {
        let mgr = SessionManager::new();
        let result = mgr.request_ai(0, 2);
        assert!(result.contains("Help"));
    }

    #[test]
    fn t_request_ai_nl2cmd() {
        let mgr = SessionManager::new();
        let result = mgr.request_ai(0, 3);
        assert!(result.contains("Command"));
    }

    #[test]
    fn t_request_ai_unknown() {
        let mgr = SessionManager::new();
        assert_eq!(mgr.request_ai(0, 99), "Unknown action");
    }

    #[test]
    fn t_screen_data_cursor_visible() {
        let mut mgr = SessionManager::new();
        let id = mgr.create_session(80, 24);
        let data = mgr.get_screen_data(id);
        assert!(data.cursor_visible);
    }

    #[test]
    fn t_screen_data_cursor_visible_invalid_id() {
        let mgr = SessionManager::new();
        let data = mgr.get_screen_data(999);
        assert!(!data.cursor_visible);
    }

    #[test]
    fn t_error_display() {
        let err = TerminalSessionError::NotFound;
        assert!(err.to_string().contains("not found"));

        let err = TerminalSessionError::ConnectionFailed("timeout".to_string());
        assert!(err.to_string().contains("timeout"));
    }
}
