/// High-level Dart session manager wrapping the FFI bindings.
///
/// Provides a clean Dart API for:
/// - Creating/destroying terminal sessions
/// - Connecting to SSH or echo transport
/// - Pumping data and flushing input
/// - Reading screen data for rendering
///
/// Usage:
/// ```dart
/// final mgr = SessionManager();
/// final id = mgr.createSession(80, 24);
/// mgr.echoConnect(id); // for testing
/// final screen = mgr.getScreenData(id);
/// mgr.sendInput(id, utf8.encode('ls\n'));
/// ```
library;
import 'dart:convert';
import 'dart:ffi';
import 'package:ffi/ffi.dart';

import 'ffi_bindings.dart';
import 'types.dart';

/// Screen data snapshot for rendering.
class ScreenSnapshot {
  final int cols;
  final int rows;
  final List<GGTermCellData> cells;
  final int cursorCol;
  final int cursorRow;
  final bool cursorVisible;
  final int cursorStyle; // 0=Default, 1=BlinkBlock, 2=SteadyBlock, 3=BlinkUnderline, 4=SteadyUnderline, 5=BlinkBar, 6=SteadyBar
  final bool hasBell;
  /// Whether bracketed paste mode (DECSET 2004) is active.
  final bool bracketedPaste;
  /// Terminal title (OSC 0/2). Empty if not set.
  final String title;
  /// Current working directory (OSC 7). Empty if not reported.
  final String cwd;

  const ScreenSnapshot({
    required this.cols,
    required this.rows,
    required this.cells,
    this.cursorCol = 0,
    this.cursorRow = 0,
    this.cursorVisible = true,
    this.cursorStyle = 0,
    this.hasBell = false,
    this.bracketedPaste = false,
    this.title = '',
    this.cwd = '',
  });

  static const empty = ScreenSnapshot(
    cols: 0,
    rows: 0,
    cells: [],
  );
}

/// Dart-side cell data (decoded from FFI struct).
class GGTermCellData {
  final int charCode;
  final int combiningChar;
  final int flags;
  final int fg;
  final int bg;

  const GGTermCellData({
    this.charCode = 0,
    this.combiningChar = 0,
    this.flags = 0,
    this.fg = 0,
    this.bg = 0,
  });

  String get char => charCode == 0 ? ' ' : String.fromCharCode(charCode);
  /// Character with combining mark appended (for rendering accented chars).
  String get charWithCombining {
    if (charCode == 0) return ' ';
    final base = String.fromCharCode(charCode);
    if (combiningChar != 0) {
      return '$base${String.fromCharCode(combiningChar)}';
    }
    return base;
  }
  bool get bold => (flags & CellFlags.bold) != 0;
  bool get italic => (flags & CellFlags.italic) != 0;
  bool get underline => (flags & CellFlags.underline) != 0;
  bool get strikethrough => (flags & CellFlags.strikethrough) != 0;
  bool get blink => (flags & CellFlags.blink) != 0;
  bool get dim => (flags & CellFlags.faint) != 0;
  bool get hidden => (flags & CellFlags.hidden) != 0;
  bool get reverse => (flags & CellFlags.reverse) != 0;
  bool get underlineDouble => (flags & CellFlags.underlineDouble) != 0;
  bool get underlineCurly => (flags & CellFlags.underlineCurly) != 0;
  bool get underlineDotted => (flags & CellFlags.underlineDotted) != 0;
  bool get underlineDashed => (flags & CellFlags.underlineDashed) != 0;
  bool get overline => (flags & CellFlags.overline) != 0;
  bool get wide => (flags & CellFlags.wide) != 0;

  /// Resolved foreground RGB (0xRRGGBB).
  int get fgRgb => AnsiPalette.resolve(fg, isBackground: false);

  /// Resolved background RGB (0xRRGGBB).
  int get bgRgb => AnsiPalette.resolve(bg, isBackground: true);
}

/// Connection parameters for SSH.
class SshConnectionParams {
  final String host;
  final int port;
  final String user;
  final String? password;
  final String? keyFilePath;

  const SshConnectionParams({
    required this.host,
    this.port = 22,
    required this.user,
    this.password,
    this.keyFilePath,
  });

  bool get usesKey => keyFilePath != null;
}

/// Manages terminal sessions via FFI.
class SessionManager {
  final GgtermFfi _ffi;
  final Set<int> _activeSessions = {};

  SessionManager({GgtermFfi? ffi}) : _ffi = ffi ?? GgtermFfi();

  /// Create a new terminal session.
  /// Returns the session ID (> 0).
  int createSession(int cols, int rows) {
    final id = _ffi.sessionCreate(cols, rows);
    if (id > 0) {
      _activeSessions.add(id);
    }
    return id;
  }

  /// Destroy a session and free resources.
  void destroySession(int id) {
    _ffi.sessionDestroy(id);
    _activeSessions.remove(id);
  }

  /// Process raw bytes (transport output) into the terminal.
  void processBytes(int id, List<int> bytes) {
    if (bytes.isEmpty) return;
    final ptr = malloc<Uint8>(bytes.length);
    try {
      for (var i = 0; i < bytes.length; i++) {
        ptr[i] = bytes[i];
      }
      _ffi.sessionProcessBytes(id, ptr, bytes.length);
    } finally {
      malloc.free(ptr);
    }
  }

  /// Send input bytes (keystrokes) to the terminal.
  void sendInput(int id, List<int> bytes) {
    if (bytes.isEmpty) return;

    final ptr = malloc<Uint8>(bytes.length);
    try {
      for (var i = 0; i < bytes.length; i++) {
        ptr[i] = bytes[i];
      }
      _ffi.sessionSendInput(id, ptr, bytes.length);
    } finally {
      malloc.free(ptr);
    }
  }

  /// Send a string as input (UTF-8 encoded).
  void sendString(int id, String text) {
    sendInput(id, utf8.encode(text));
  }

  /// Get terminal dimensions.
  /// Returns (0, 0) if session not found or invalid.
  (int, int) getDimensions(int id) {
    final colsPtr = malloc<IntPtr>();
    final rowsPtr = malloc<IntPtr>();
    try {
      // Zero-initialize to detect unfound sessions.
      colsPtr.value = 0;
      rowsPtr.value = 0;
      _ffi.sessionDimensions(id, colsPtr, rowsPtr);
      final cols = colsPtr.value;
      final rows = rowsPtr.value;
      // Sanity check: dimensions must be positive and reasonable.
      if (cols <= 0 || cols > 500 || rows <= 0 || rows > 200) {
        return (0, 0);
      }
      return (cols, rows);
    } finally {
      malloc.free(colsPtr);
      malloc.free(rowsPtr);
    }
  }

  /// Get cursor position.
  (int, int) getCursor(int id) {
    final colPtr = malloc<IntPtr>();
    final rowPtr = malloc<IntPtr>();
    try {
      colPtr.value = 0;
      rowPtr.value = 0;
      _ffi.sessionCursor(id, colPtr, rowPtr);
      return (colPtr.value, rowPtr.value);
    } finally {
      malloc.free(colPtr);
      malloc.free(rowPtr);
    }
  }

  /// Resize the terminal.
  void resize(int id, int cols, int rows) {
    _ffi.sessionResize(id, cols, rows);
  }

  /// Get a complete screen snapshot for rendering.
  ScreenSnapshot getScreenSnapshot(int id) {
    final (cols, rows) = getDimensions(id);
    if (cols == 0 || rows == 0) return ScreenSnapshot.empty;

    final cellCount = cols * rows;
    // Safety: cap allocation to prevent crashes from garbage dimensions.
    if (cellCount <= 0 || cellCount > 100000) return ScreenSnapshot.empty;
    final ptr = malloc<GGTermCell>(cellCount);
    try {
      final n = _ffi.sessionReadCells(id, ptr, cellCount);
      if (n == 0) return ScreenSnapshot.empty;

      final cells = <GGTermCellData>[];
      for (var i = 0; i < n; i++) {
        final ffiCell = ptr[i];
        cells.add(GGTermCellData(
          charCode: ffiCell.charCode,
          combiningChar: ffiCell.combiningChar,
          flags: ffiCell.flags,
          fg: ffiCell.fg,
          bg: ffiCell.bg,
        ));
      }

      final (cursorCol, cursorRow) = getCursor(id);
      final cursorVisible = _ffi.sessionCursorVisible(id) != 0;
      final cursorStyle = _ffi.sessionCursorStyle(id);
      final bell = _ffi.sessionTakeBell(id) != 0;
      final bracketedPaste = _ffi.sessionBracketedPaste(id) != 0;
      final title = _ffi.getSessionTitle(id);
      final cwd = _ffi.getSessionCwd(id);

      return ScreenSnapshot(
        cols: cols,
        rows: rows,
        cells: cells,
        cursorCol: cursorCol,
        cursorRow: cursorRow,
        cursorVisible: cursorVisible,
        cursorStyle: cursorStyle,
        hasBell: bell,
        bracketedPaste: bracketedPaste,
        title: title,
        cwd: cwd,
      );
    } finally {
      malloc.free(ptr);
    }
  }

  // ── Transport operations ──

  /// Returns true if the alternate screen buffer is active (vim/less).
  bool isAltScreen(int id) => _ffi.sessionAltScreen(id) != 0;

  /// Returns true if cursor keys are in application mode (DECCKM).
  /// When active (vim/less/htop), arrow keys must use SS3 sequences.
  bool cursorKeysApp(int id) => _ffi.sessionCursorKeysApp(id) != 0;

  /// Returns true if alternate scroll mode is enabled (DECSET 7727).
  /// When true, scroll in alt screen should send arrow keys, not scroll viewport.
  bool altScrollEnabled(int id) => _ffi.sessionAltScroll(id) != 0;

  /// Get all visible terminal text as a string.
  /// Reads cells from the screen buffer, converting to text line by line.
  /// Strips trailing whitespace from each line and skips empty trailing lines.
  String getTerminalText(int id) {
    final (cols, rows) = getDimensions(id);
    if (cols == 0 || rows == 0) return '';

    final cellCount = cols * rows;
    if (cellCount <= 0 || cellCount > 100000) return '';
    final ptr = malloc<GGTermCell>(cellCount);
    try {
      final n = _ffi.sessionReadCells(id, ptr, cellCount);
      if (n == 0) return '';

      final lines = <String>[];
      for (var row = 0; row < rows; row++) {
        final buf = StringBuffer();
        for (var col = 0; col < cols; col++) {
          final idx = row * cols + col;
          if (idx >= n) break;
          final cell = ptr[idx];
          if (cell.charCode == 0) {
            buf.write(' ');
          } else {
            buf.writeCharCode(cell.charCode);
          }
        }
        lines.add(buf.toString().trimRight());
      }
      // Remove trailing empty lines.
      while (lines.isNotEmpty && lines.last.isEmpty) {
        lines.removeLast();
      }
      return lines.join('\n');
    } finally {
      malloc.free(ptr);
    }
  }

  /// Connect to an SSH host with password auth.
  /// Returns true on success.
  bool sshConnect(int id, SshConnectionParams params) {
    final hostPtr = params.host.toNativeUtf8();
    final userPtr = params.user.toNativeUtf8();
    final passPtr = (params.password ?? '').toNativeUtf8();
    try {
      final result = _ffi.sshConnect(
          id, hostPtr, params.port, userPtr, passPtr);
      if (result != 0) {
        _lastError = _ffi.getLastErrorString();
      }
      return result == 0;
    } finally {
      malloc.free(hostPtr);
      malloc.free(userPtr);
      malloc.free(passPtr);
    }
  }

  /// Connect to an SSH host with public key auth.
  bool sshConnectKey(int id, SshConnectionParams params) {
    final hostPtr = params.host.toNativeUtf8();
    final userPtr = params.user.toNativeUtf8();
    final keyPtr = (params.keyFilePath ?? '').toNativeUtf8();
    try {
      final result = _ffi.sshConnectKey(
          id, hostPtr, params.port, userPtr, keyPtr);
      if (result != 0) {
        _lastError = _ffi.getLastErrorString();
      }
      return result == 0;
    } finally {
      malloc.free(hostPtr);
      malloc.free(userPtr);
      malloc.free(keyPtr);
    }
  }

  /// Connect to echo transport (for testing without SSH).
  bool echoConnect(int id) {
    return _ffi.echoConnect(id) == 0;
  }

  /// Connect to a local shell (Android only).
  /// Uses forkpty() to spawn /system/bin/sh.
  bool localShellConnect(int id) {
    final result = _ffi.localShellConnect(id);
    if (result != 0) {
      _lastError = _ffi.getLastErrorString();
    }
    return result == 0;
  }

  /// Pump data: read from transport → feed into terminal.
  /// Returns bytes read.
  int pump(int id) {
    return _ffi.transportPump(id);
  }

  /// Flush queued input to the transport.
  void flush(int id) {
    _ffi.transportFlush(id);
  }

  /// Check if transport is alive.
  bool isAlive(int id) {
    return _ffi.transportIsAlive(id) != 0;
  }

  /// Scroll terminal viewport up (toward older scrollback).
  void scrollUp(int id, int lines) {
    _ffi.sessionScrollUp(id, lines);
  }

  /// Scroll terminal viewport down (toward newer content).
  void scrollDown(int id, int lines) {
    _ffi.sessionScrollDown(id, lines);
  }

  /// Reset viewport to bottom (most recent).
  void resetViewport(int id) {
    _ffi.sessionResetViewport(id);
  }

  /// Get current display offset (0 = at bottom).
  int displayOffset(int id) {
    return _ffi.sessionDisplayOffset(id);
  }

  /// Get total scrollback history length (off-screen lines).
  int scrollbackLen(int id) {
    try {
      return _ffi.sessionScrollbackLen(id);
    } catch (_) {
      return 0;
    }
  }

  /// One-step pump + flush cycle.
  /// Call this in a timer loop for the render cycle.
  int pumpAndFlush(int id) {
    try {
      final bytes = pump(id);
      flush(id);
      return bytes;
    } catch (_) {
      // Native call failed (panic, corrupted state, poisoned mutex).
      // Return 0 to signal no data — the render loop will skip this frame.
      return 0;
    }
  }

  String _lastError = '';
  String get lastError => _lastError;

  /// Clean up all sessions.
  void dispose() {
    for (final id in _activeSessions.toList()) {
      destroySession(id);
    }
  }
}
