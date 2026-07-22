/// Low-level dart:ffi bindings for the ggterm_ffi C-ABI.
///
/// Maps all Rust extern "C" functions to Dart equivalents.
/// Usage:
/// ```dart
/// final ffi = GgtermFfi();
/// final sessionId = ffi.sessionCreate(80, 24);
/// ffi.sessionProcessBytes(sessionId, data);
/// ffi.sessionDestroy(sessionId);
/// ```
library;
import 'dart:ffi';
import 'dart:io';
import 'package:ffi/ffi.dart';

import 'types.dart';

// ── Function type definitions ─────────────────────────────────────────

// usize on 64-bit platforms is 8 bytes; use IntPtr for correct ABI.
typedef _SessionCreateC = Uint32 Function(IntPtr cols, IntPtr rows);
typedef _SessionCreateDart = int Function(int cols, int rows);

typedef _SessionDestroyC = Void Function(Uint32 id);
typedef _SessionDestroyDart = void Function(int id);

typedef _SessionCountC = IntPtr Function();
typedef _SessionCountDart = int Function();

typedef _ProcessBytesC = Void Function(
    Uint32 id, Pointer<Uint8> data, IntPtr len);
typedef _ProcessBytesDart = void Function(
    int id, Pointer<Uint8> data, int len);

typedef _SendInputC = Void Function(
    Uint32 id, Pointer<Uint8> data, IntPtr len);
typedef _SendInputDart = void Function(
    int id, Pointer<Uint8> data, int len);

typedef _TakeInputC = IntPtr Function(
    Uint32 id, Pointer<Uint8> buf, IntPtr maxLen);
typedef _TakeInputDart = int Function(
    int id, Pointer<Uint8> buf, int maxLen);

typedef _ReadCellsC = IntPtr Function(
    Uint32 id, Pointer<GGTermCell> buf, IntPtr maxCells);
typedef _ReadCellsDart = int Function(
    int id, Pointer<GGTermCell> buf, int maxCells);

typedef _DimensionsC = Void Function(
    Uint32 id, Pointer<IntPtr> cols, Pointer<IntPtr> rows);
typedef _DimensionsDart = void Function(
    int id, Pointer<IntPtr> cols, Pointer<IntPtr> rows);

typedef _CursorC = Void Function(
    Uint32 id, Pointer<IntPtr> col, Pointer<IntPtr> row);
typedef _CursorDart = void Function(
    int id, Pointer<IntPtr> col, Pointer<IntPtr> row);

typedef _ResizeC = Void Function(Uint32 id, IntPtr cols, IntPtr rows);
typedef _ResizeDart = void Function(int id, int cols, int rows);

typedef _TakeBellC = Int32 Function(Uint32 id);
typedef _TakeBellDart = int Function(int id);

typedef _PumpC = IntPtr Function(Uint32 id);
typedef _PumpDart = int Function(int id);

typedef _FlushC = Void Function(Uint32 id);
typedef _FlushDart = void Function(int id);

typedef _IsAliveC = Int32 Function(Uint32 id);
typedef _IsAliveDart = int Function(int id);
typedef _ScrollC = Void Function(Uint32 id, UintPtr lines);
typedef _ScrollDart = void Function(int id, int lines);
typedef _ResetViewportC = Void Function(Uint32 id);
typedef _ResetViewportDart = void Function(int id);
typedef _DisplayOffsetC = UintPtr Function(Uint32 id);
typedef _DisplayOffsetDart = int Function(int id);

typedef _SshConnectC = Int32 Function(
    Uint32 id, Pointer<Utf8> host, Uint16 port, Pointer<Utf8> user, Pointer<Utf8> password);
typedef _SshConnectDart = int Function(
    int id, Pointer<Utf8> host, int port, Pointer<Utf8> user, Pointer<Utf8> password);

typedef _SshConnectKeyC = Int32 Function(
    Uint32 id, Pointer<Utf8> host, Uint16 port, Pointer<Utf8> user, Pointer<Utf8> keyPath);
typedef _SshConnectKeyDart = int Function(
    int id, Pointer<Utf8> host, int port, Pointer<Utf8> user, Pointer<Utf8> keyPath);

typedef _EchoConnectC = Int32 Function(Uint32 id);
typedef _EchoConnectDart = int Function(int id);

typedef _LocalShellConnectC = Int32 Function(Uint32 id);
typedef _LocalShellConnectDart = int Function(int id);

// Title and CWD functions
typedef _SessionStringC = IntPtr Function(
    Uint32 id, Pointer<Int8> buf, IntPtr maxLen);
typedef _SessionStringDart = int Function(
    int id, Pointer<Int8> buf, int maxLen);

typedef _LastErrorC = Pointer<Utf8> Function();
typedef _LastErrorDart = Pointer<Utf8> Function();

// ── FFI bindings class ────────────────────────────────────────────────

/// Provides all C-ABI bindings to the ggterm_ffi shared library.
class GgtermFfi {
  late final DynamicLibrary _lib;

  // Bound functions
  late final int Function(int, int) sessionCreate;
  late final void Function(int) sessionDestroy;
  late final int Function() sessionCount;
  late final void Function(int, Pointer<Uint8>, int) sessionProcessBytes;
  late final void Function(int, Pointer<Uint8>, int) sessionSendInput;
  late final int Function(int, Pointer<Uint8>, int) sessionTakeInput;
  late final int Function(int, Pointer<GGTermCell>, int) sessionReadCells;
  late final void Function(int, Pointer<IntPtr>, Pointer<IntPtr>) sessionDimensions;
  late final void Function(int, Pointer<IntPtr>, Pointer<IntPtr>) sessionCursor;
  late final void Function(int, int, int) sessionResize;
  late final int Function(int) sessionTakeBell;
  late final int Function(int) sessionCursorVisible;
  late final int Function(int) sessionCursorStyle;
  late final int Function(int) sessionAltScreen;
  late final int Function(int) sessionBracketedPaste;
  late final int Function(int) sessionAltScroll;
  late final int Function(int) sessionCursorKeysApp;
  late final int Function(int) transportPump;
  late final void Function(int) transportFlush;
  late final int Function(int) transportIsAlive;
  late final void Function(int, int) sessionScrollUp;
  late final void Function(int, int) sessionScrollDown;
  late final void Function(int) sessionResetViewport;
  late final int Function(int) sessionDisplayOffset;
  late final int Function(int) sessionScrollbackLen;
  late final int Function(int, Pointer<Utf8>, int, Pointer<Utf8>, Pointer<Utf8>) sshConnect;
  late final int Function(int, Pointer<Utf8>, int, Pointer<Utf8>, Pointer<Utf8>) sshConnectKey;
  late final int Function(int) echoConnect;
  late final int Function(int) localShellConnect;
  late final int Function(int, Pointer<Int8>, int) sessionTitle;
  late final int Function(int, Pointer<Int8>, int) sessionCwd;
  late final Pointer<Utf8> Function() lastError;

  /// Load the ggterm_ffi library.
  ///
  /// On desktop: loads from build directory or system path.
  /// On mobile: loads from the bundled library in the app bundle.
  GgtermFfi({String? libraryPath}) {
    if (libraryPath != null) {
      _lib = DynamicLibrary.open(libraryPath);
    } else {
      _lib = _loadLibrary();
    }
    _bindFunctions();
  }

  DynamicLibrary _loadLibrary() {
    if (Platform.isMacOS || Platform.isLinux) {
      // Try several common locations
      const candidates = [
        'libggterm_ffi.dylib',
        'libggterm_ffi.so',
        './libggterm_ffi.dylib',
        '../target/debug/libggterm_ffi.dylib',
        '../target/debug/libggterm_ffi.so',
        '../target/release/libggterm_ffi.dylib',
      ];
      for (final path in candidates) {
        try {
          return DynamicLibrary.open(path);
        } catch (_) {
          continue;
        }
      }
    }
    if (Platform.isIOS) {
      // In debug builds, the actual code lives in Runner.debug.dylib.
      // Try opening it first before falling back to process.
      try {
        return DynamicLibrary.open('Runner.debug.dylib');
      } catch (_) {}
      return DynamicLibrary.process();
    }
    if (Platform.isAndroid) {
      // Shared library packaged in jniLibs
      return DynamicLibrary.open('libggterm_ffi.so');
    }
    throw UnsupportedError('Unsupported platform for ggterm_ffi');
  }

  void _bindFunctions() {
    sessionCreate = _lib
        .lookupFunction<_SessionCreateC, _SessionCreateDart>(
            'ggterm_session_create');
    sessionDestroy = _lib
        .lookupFunction<_SessionDestroyC, _SessionDestroyDart>(
            'ggterm_session_destroy');
    sessionCount = _lib
        .lookupFunction<_SessionCountC, _SessionCountDart>(
            'ggterm_session_count');
    sessionProcessBytes = _lib
        .lookupFunction<_ProcessBytesC, _ProcessBytesDart>(
            'ggterm_session_process_bytes');
    sessionSendInput = _lib
        .lookupFunction<_SendInputC, _SendInputDart>(
            'ggterm_session_send_input');
    sessionTakeInput = _lib
        .lookupFunction<_TakeInputC, _TakeInputDart>(
            'ggterm_session_take_input');
    sessionReadCells = _lib
        .lookupFunction<_ReadCellsC, _ReadCellsDart>(
            'ggterm_session_read_cells');
    sessionDimensions = _lib
        .lookupFunction<_DimensionsC, _DimensionsDart>(
            'ggterm_session_dimensions');
    sessionCursor = _lib
        .lookupFunction<_CursorC, _CursorDart>(
            'ggterm_session_cursor');
    sessionResize = _lib
        .lookupFunction<_ResizeC, _ResizeDart>(
            'ggterm_session_resize');
    sessionTakeBell = _lib
        .lookupFunction<_TakeBellC, _TakeBellDart>(
            'ggterm_session_take_bell');
    sessionCursorVisible = _lib
        .lookupFunction<_TakeBellC, _TakeBellDart>(
            'ggterm_session_cursor_visible');
    sessionCursorStyle = _lib
        .lookupFunction<_TakeBellC, _TakeBellDart>(
            'ggterm_session_cursor_style');
    sessionAltScreen = _lib
        .lookupFunction<_TakeBellC, _TakeBellDart>(
            'ggterm_session_alt_screen');
    sessionBracketedPaste = _lib
        .lookupFunction<_TakeBellC, _TakeBellDart>(
            'ggterm_session_bracketed_paste');
    sessionAltScroll = _lib
        .lookupFunction<_TakeBellC, _TakeBellDart>(
            'ggterm_session_alt_scroll');
    sessionCursorKeysApp = _lib
        .lookupFunction<_TakeBellC, _TakeBellDart>(
            'ggterm_session_cursor_keys_app');
    transportPump = _lib
        .lookupFunction<_PumpC, _PumpDart>(
            'ggterm_transport_pump');
    transportFlush = _lib
        .lookupFunction<_FlushC, _FlushDart>(
            'ggterm_transport_flush');
    transportIsAlive = _lib
        .lookupFunction<_IsAliveC, _IsAliveDart>(
            'ggterm_transport_is_alive');
    sessionScrollUp = _lib
        .lookupFunction<_ScrollC, _ScrollDart>(
            'ggterm_session_scroll_up');
    sessionScrollDown = _lib
        .lookupFunction<_ScrollC, _ScrollDart>(
            'ggterm_session_scroll_down');
    sessionResetViewport = _lib
        .lookupFunction<_ResetViewportC, _ResetViewportDart>(
            'ggterm_session_reset_viewport');
    sessionDisplayOffset = _lib
        .lookupFunction<_DisplayOffsetC, _DisplayOffsetDart>(
            'ggterm_session_display_offset');
    try {
      sessionScrollbackLen = _lib
          .lookupFunction<_DisplayOffsetC, _DisplayOffsetDart>(
              'ggterm_session_scrollback_len');
    } catch (_) {
      sessionScrollbackLen = (_) => 0;
    }
    // SSH functions are optional (behind the "ssh" feature flag in Rust)
    try {
      sshConnect = _lib
          .lookupFunction<_SshConnectC, _SshConnectDart>(
              'ggterm_ssh_connect');
    } catch (_) {
      sshConnect = (_, __, ___, ____, _____) => -1;
    }
    try {
      sshConnectKey = _lib
          .lookupFunction<_SshConnectKeyC, _SshConnectKeyDart>(
              'ggterm_ssh_connect_key');
    } catch (_) {
      sshConnectKey = (_, __, ___, ____, _____) => -1;
    }
    echoConnect = _lib
        .lookupFunction<_EchoConnectC, _EchoConnectDart>(
            'ggterm_echo_connect');
    try {
      localShellConnect = _lib
          .lookupFunction<_LocalShellConnectC, _LocalShellConnectDart>(
              'ggterm_local_shell_connect');
    } catch (_) {
      // Not available on this platform (iOS / desktop).
      localShellConnect = (_) => -1;
    }
    lastError = _lib
        .lookupFunction<_LastErrorC, _LastErrorDart>(
            'ggterm_last_error');
    // Title and CWD are optional (older builds may not have them).
    try {
      sessionTitle = _lib
          .lookupFunction<_SessionStringC, _SessionStringDart>(
              'ggterm_session_title');
    } catch (_) {
      sessionTitle = (_, __, ___) => 0;
    }
    try {
      sessionCwd = _lib
          .lookupFunction<_SessionStringC, _SessionStringDart>(
              'ggterm_session_cwd');
    } catch (_) {
      sessionCwd = (_, __, ___) => 0;
    }
  }

  /// Get the last error message as a Dart string.
  String getLastErrorString() {
    final ptr = lastError();
    if (ptr == nullptr) return '';
    return ptr.toDartString();
  }

  /// Get the terminal title (OSC 0/2) as a Dart string.
  /// Returns empty string if no title is set or the function is unavailable.
  String getSessionTitle(int id, {int maxLen = 256}) {
    final buf = calloc.allocate<Int8>(maxLen);
    try {
      final n = sessionTitle(id, buf, maxLen);
      if (n <= 0) return '';
      return buf.cast<Utf8>().toDartString(length: n);
    } finally {
      calloc.free(buf);
    }
  }

  /// Get the current working directory (OSC 7) as a Dart string.
  /// Returns empty string if no cwd is set or the function is unavailable.
  String getSessionCwd(int id, {int maxLen = 1024}) {
    final buf = calloc.allocate<Int8>(maxLen);
    try {
      final n = sessionCwd(id, buf, maxLen);
      if (n <= 0) return '';
      return buf.cast<Utf8>().toDartString(length: n);
    } finally {
      calloc.free(buf);
    }
  }
}
