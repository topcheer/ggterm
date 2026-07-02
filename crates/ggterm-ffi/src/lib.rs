//! # GGTerm FFI — C-ABI bindings for mobile integration
//!
//! Exposes the terminal core engine via C-ABI functions so that Flutter
//! (via dart:ffi or flutter_rust_bridge) and other languages can drive
//! the terminal without depending on Rust's type system.

pub mod api;
pub mod transport;

use ggterm_core::{Cell, Color, Grid, Parser, Terminal};
use std::ffi::c_int;
use std::ptr;

// Re-export core types for api.rs
pub use ggterm_core;

// ── FFI Handle ─────────────────────────────────────────────────────────

/// Opaque handle to a terminal session (Terminal + Parser + input buffer).
pub struct TerminalHandle {
    pub terminal: Terminal,
    pub parser: Parser,
    pub input_buffer: Vec<u8>,
}

impl TerminalHandle {
    /// Create a new terminal session.
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            terminal: Terminal::new(cols, rows),
            parser: Parser::new(),
            input_buffer: Vec::new(),
        }
    }

    /// Process raw bytes from the transport (PTY/SSH output).
    pub fn process_bytes(&mut self, data: &[u8]) {
        self.parser.feed(data, &mut self.terminal);
    }

    /// Queue input bytes (keystrokes from the user).
    pub fn send_input(&mut self, data: &[u8]) {
        self.input_buffer.extend_from_slice(data);
    }

    /// Take pending input bytes for the host to send to PTY/SSH.
    pub fn take_input(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.input_buffer)
    }
}

// ── FFI Cell ───────────────────────────────────────────────────────────

/// A single terminal cell serialized for C/Flutter consumption.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct GGTermCell {
    /// Unicode codepoint (0 = empty/space).
    pub char_code: u32,
    /// CellFlags bits (bold, italic, underline, etc.).
    pub flags: u16,
    /// Foreground color: 0=default, 0x01XX0000=indexed, 0x00RRGGBB=RGB.
    pub fg: u32,
    /// Background color: same packing as fg.
    pub bg: u32,
}

impl GGTermCell {
    /// Convert a core `Cell` to the FFI representation.
    pub fn from_cell(cell: &Cell) -> Self {
        Self {
            char_code: if cell.ch == ' ' { 0 } else { cell.ch as u32 },
            flags: cell.flags.bits(),
            fg: pack_color(cell.fg),
            bg: pack_color(cell.bg),
        }
    }
}

/// Pack a `Color` into a u32 for FFI transfer.
fn pack_color(color: Color) -> u32 {
    match color {
        Color::Default => 0,
        Color::Indexed(i) => 0x0100_0000 | (i as u32),
        Color::Rgb(r, g, b) => ((r as u32) << 16) | ((g as u32) << 8) | (b as u32),
    }
}

/// Convert an entire grid to a flat Vec<GGTermCell>.
pub fn grid_to_ffi(grid: &Grid) -> Vec<GGTermCell> {
    let cols = grid.width();
    let rows = grid.height();
    let mut cells = Vec::with_capacity(cols * rows);
    for row_idx in 0..rows {
        if let Some(row) = grid.row(row_idx) {
            for cell in &row.cells {
                cells.push(GGTermCell::from_cell(cell));
            }
        } else {
            cells.resize(cells.len() + cols, GGTermCell::default());
        }
    }
    cells
}

// ═══════════════════════════════════════════════════════════════════════
//  C-ABI Functions
// ═══════════════════════════════════════════════════════════════════════

/// Create a new terminal session. Returns an opaque handle.
#[unsafe(no_mangle)]
pub extern "C" fn ggterm_new(cols: usize, rows: usize) -> *mut TerminalHandle {
    Box::into_raw(Box::new(TerminalHandle::new(cols, rows)))
}

/// Destroy a terminal session and free its memory.
///
/// # Safety
/// `handle` must be a valid pointer from `ggterm_new`, or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_free(handle: *mut TerminalHandle) {
    if !handle.is_null() {
        unsafe { drop(Box::from_raw(handle)) };
    }
}

/// Feed raw bytes (PTY/SSH output) into the terminal for processing.
///
/// # Safety
/// `handle` must be valid. `data` must point to at least `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_process_bytes(
    handle: *mut TerminalHandle,
    data: *const u8,
    len: usize,
) {
    if handle.is_null() || data.is_null() {
        return;
    }
    unsafe {
        let h = &mut *handle;
        let slice = std::slice::from_raw_parts(data, len);
        h.process_bytes(slice);
    }
}

/// Send input bytes (keystrokes) to the terminal's input buffer.
///
/// # Safety
/// `handle` must be valid. `data` must point to at least `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_send_input(
    handle: *mut TerminalHandle,
    data: *const u8,
    len: usize,
) {
    if handle.is_null() || data.is_null() {
        return;
    }
    unsafe {
        let h = &mut *handle;
        let slice = std::slice::from_raw_parts(data, len);
        h.send_input(slice);
    }
}

/// Read pending input bytes. Returns the number of bytes written to `buf`.
///
/// # Safety
/// `handle` must be valid. `buf` must point to at least `max_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_take_input(
    handle: *mut TerminalHandle,
    buf: *mut u8,
    max_len: usize,
) -> usize {
    if handle.is_null() || buf.is_null() || max_len == 0 {
        return 0;
    }
    unsafe {
        let h = &mut *handle;
        let input = h.take_input();
        let n = input.len().min(max_len);
        if n > 0 {
            ptr::copy_nonoverlapping(input.as_ptr(), buf, n);
        }
        n
    }
}

/// Read terminal cells into a flat array for rendering.
///
/// # Safety
/// `handle` must be valid. `buf` must point to at least `max_cells` GGTermCell.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_read_cells(
    handle: *mut TerminalHandle,
    buf: *mut GGTermCell,
    max_cells: usize,
) -> usize {
    if handle.is_null() || buf.is_null() || max_cells == 0 {
        return 0;
    }
    unsafe {
        let h = &*handle;
        let cells = grid_to_ffi(h.terminal.grid());
        let n = cells.len().min(max_cells);
        ptr::copy_nonoverlapping(cells.as_ptr(), buf, n);
        n
    }
}

/// Get terminal grid dimensions.
///
/// # Safety
/// All pointers must be valid or null (null is safe no-op).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_dimensions(
    handle: *mut TerminalHandle,
    cols: *mut usize,
    rows: *mut usize,
) {
    if handle.is_null() || cols.is_null() || rows.is_null() {
        return;
    }
    unsafe {
        let h = &*handle;
        let grid = h.terminal.grid();
        *cols = grid.width();
        *rows = grid.height();
    }
}

/// Get the cursor position.
///
/// # Safety
/// All pointers must be valid or null (null is safe no-op).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_cursor(
    handle: *mut TerminalHandle,
    col: *mut usize,
    row: *mut usize,
) {
    if handle.is_null() || col.is_null() || row.is_null() {
        return;
    }
    unsafe {
        let h = &*handle;
        let (c, r) = h.terminal.cursor();
        *col = c;
        *row = r;
    }
}

/// Resize the terminal grid.
///
/// # Safety
/// `handle` must be valid or null (null is safe no-op).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_resize(handle: *mut TerminalHandle, cols: usize, rows: usize) {
    if handle.is_null() {
        return;
    }
    unsafe {
        let h = &mut *handle;
        h.terminal.grid_mut().resize(cols, rows);
    }
}

/// Check and consume the bell flag. Returns 1 if bell, 0 otherwise.
///
/// # Safety
/// `handle` must be valid or null (null is safe, returns 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_take_bell(handle: *mut TerminalHandle) -> c_int {
    if handle.is_null() {
        return 0;
    }
    unsafe {
        let h = &mut *handle;
        if h.terminal.take_bell() { 1 } else { 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_handle_create() {
        let h = TerminalHandle::new(80, 24);
        assert_eq!(h.terminal.grid().width(), 80);
        assert_eq!(h.terminal.grid().height(), 24);
    }

    #[test]
    fn t_handle_process_bytes() {
        let mut h = TerminalHandle::new(80, 24);
        h.process_bytes(b"Hello");
        let row = h.terminal.grid().row(0).unwrap();
        assert_eq!(row.cells[0].ch, 'H');
        assert_eq!(row.cells[1].ch, 'e');
    }

    #[test]
    fn t_handle_input_buffer() {
        let mut h = TerminalHandle::new(80, 24);
        h.send_input(b"ls\n");
        assert_eq!(h.take_input(), b"ls\n");
        assert!(h.take_input().is_empty());
    }

    #[test]
    fn t_pack_color_default() {
        assert_eq!(pack_color(Color::Default), 0);
    }

    #[test]
    fn t_pack_color_indexed() {
        assert_eq!(pack_color(Color::Indexed(3)), 0x01000003);
    }

    #[test]
    fn t_pack_color_rgb() {
        assert_eq!(pack_color(Color::Rgb(255, 128, 0)), 0x00FF8000);
    }

    #[test]
    fn t_ggterm_cell_from_char() {
        let cell = Cell::with_char('X');
        let ffi_cell = GGTermCell::from_cell(&cell);
        assert_eq!(ffi_cell.char_code, 'X' as u32);
    }

    #[test]
    fn t_ggterm_cell_from_space() {
        let cell = Cell::default();
        let ffi_cell = GGTermCell::from_cell(&cell);
        assert_eq!(ffi_cell.char_code, 0);
    }

    #[test]
    fn t_grid_to_ffi() {
        let mut h = TerminalHandle::new(80, 24);
        h.process_bytes(b"AB");
        let cells = grid_to_ffi(h.terminal.grid());
        assert_eq!(cells.len(), 80 * 24);
        assert_eq!(cells[0].char_code, 'A' as u32);
        assert_eq!(cells[1].char_code, 'B' as u32);
        assert_eq!(cells[2].char_code, 0);
    }

    #[test]
    fn t_ffi_new_and_free() {
        unsafe {
            let h = ggterm_new(80, 24);
            assert!(!h.is_null());
            ggterm_free(h);
        }
    }

    #[test]
    fn t_ffi_null_safety() {
        unsafe {
            ggterm_free(ptr::null_mut());
            ggterm_process_bytes(ptr::null_mut(), ptr::null(), 0);
            ggterm_send_input(ptr::null_mut(), ptr::null(), 0);
            assert_eq!(ggterm_take_input(ptr::null_mut(), ptr::null_mut(), 0), 0);
            assert_eq!(ggterm_read_cells(ptr::null_mut(), ptr::null_mut(), 0), 0);
            assert_eq!(ggterm_take_bell(ptr::null_mut()), 0);
        }
    }

    #[test]
    fn t_ffi_process_and_read_cells() {
        unsafe {
            let h = ggterm_new(80, 24);
            let data = b"Hi";
            ggterm_process_bytes(h, data.as_ptr(), data.len());

            let mut cells = vec![GGTermCell::default(); 80 * 24];
            let n = ggterm_read_cells(h, cells.as_mut_ptr(), cells.len());
            assert_eq!(n, 80 * 24);
            assert_eq!(cells[0].char_code, 'H' as u32);
            assert_eq!(cells[1].char_code, 'i' as u32);

            ggterm_free(h);
        }
    }

    #[test]
    fn t_ffi_send_and_take_input() {
        unsafe {
            let h = ggterm_new(80, 24);
            let data = b"ls\n";
            ggterm_send_input(h, data.as_ptr(), data.len());

            let mut buf = [0u8; 64];
            let n = ggterm_take_input(h, buf.as_mut_ptr(), buf.len());
            assert_eq!(n, 3);
            assert_eq!(&buf[..n], b"ls\n");

            ggterm_free(h);
        }
    }

    #[test]
    fn t_ffi_dimensions() {
        unsafe {
            let h = ggterm_new(100, 30);
            let (mut cols, mut rows) = (0usize, 0usize);
            ggterm_dimensions(h, &mut cols, &mut rows);
            assert_eq!(cols, 100);
            assert_eq!(rows, 30);
            ggterm_free(h);
        }
    }

    #[test]
    fn t_ffi_cursor() {
        unsafe {
            let h = ggterm_new(80, 24);
            let data = b"Hi";
            ggterm_process_bytes(h, data.as_ptr(), data.len());

            let (mut col, mut row) = (0usize, 0usize);
            ggterm_cursor(h, &mut col, &mut row);
            assert_eq!(col, 2);
            assert_eq!(row, 0);
            ggterm_free(h);
        }
    }

    #[test]
    fn t_ffi_resize() {
        unsafe {
            let h = ggterm_new(80, 24);
            ggterm_resize(h, 120, 40);

            let (mut cols, mut rows) = (0usize, 0usize);
            ggterm_dimensions(h, &mut cols, &mut rows);
            assert_eq!(cols, 120);
            assert_eq!(rows, 40);

            ggterm_free(h);
        }
    }
}
