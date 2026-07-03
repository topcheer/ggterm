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
/// mgr.sendInput(id, 'ls\n'.codeUnits);
/// ```
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
  final bool hasBell;

  const ScreenSnapshot({
    required this.cols,
    required this.rows,
    required this.cells,
    this.cursorCol = 0,
    this.cursorRow = 0,
    this.hasBell = false,
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
  final int flags;
  final int fg;
  final int bg;

  const GGTermCellData({
    this.charCode = 0,
    this.flags = 0,
    this.fg = 0,
    this.bg = 0,
  });

  String get char => charCode == 0 ? ' ' : String.fromCharCode(charCode);
  bool get bold => (flags & CellFlags.bold) != 0;
  bool get italic => (flags & CellFlags.italic) != 0;
  bool get underline => (flags & CellFlags.underline) != 0;
  bool get strikethrough => (flags & CellFlags.strikethrough) != 0;

  /// Resolved foreground RGB (0xRRGGBB).
  int get fgRgb => AnsiPalette.resolve(fg);

  /// Resolved background RGB (0xRRGGBB).
  int get bgRgb => AnsiPalette.resolve(bg, defaultFg: 0x1E1E2E, defaultBg: 0x1E1E2E);
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

  /// Send a string as input.
  void sendString(int id, String text) {
    sendInput(id, text.codeUnits);
  }

  /// Get terminal dimensions.
  (int, int) getDimensions(int id) {
    final colsPtr = malloc<IntPtr>();
    final rowsPtr = malloc<IntPtr>();
    try {
      _ffi.sessionDimensions(id, colsPtr, rowsPtr);
      return (colsPtr.value, rowsPtr.value);
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
    final ptr = malloc<GGTermCell>(cellCount);
    try {
      final n = _ffi.sessionReadCells(id, ptr, cellCount);
      if (n == 0) return ScreenSnapshot.empty;

      final cells = <GGTermCellData>[];
      for (var i = 0; i < n; i++) {
        final ffiCell = ptr[i];
        cells.add(GGTermCellData(
          charCode: ffiCell.charCode,
          flags: ffiCell.flags,
          fg: ffiCell.fg,
          bg: ffiCell.bg,
        ));
      }

      final (cursorCol, cursorRow) = getCursor(id);
      final bell = _ffi.sessionTakeBell(id) != 0;

      return ScreenSnapshot(
        cols: cols,
        rows: rows,
        cells: cells,
        cursorCol: cursorCol,
        cursorRow: cursorRow,
        hasBell: bell,
      );
    } finally {
      malloc.free(ptr);
    }
  }

  // ── Transport operations ──

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

  /// One-step pump + flush cycle.
  /// Call this in a timer loop for the render cycle.
  void pumpAndFlush(int id) {
    pump(id);
    flush(id);
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
