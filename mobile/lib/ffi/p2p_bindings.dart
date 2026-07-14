/// P2P FFI bindings for ggterm_ffi P2P functions.
///
/// Maps the C-ABI P2P functions for Iroh-based QUIC connections.
/// These functions are optional (behind the "p2p" feature in Rust).
///
/// C API (from ggcxf_dev):
/// ```c
/// uint32_t ggterm_p2p_connect(const char* ticket);
/// const char* ggterm_p2p_host_ticket(uint32_t session_id);
/// bool ggterm_p2p_is_connected(uint32_t session_id);
/// const char* ggterm_p2p_generate_ticket(void);
/// void ggterm_p2p_free_string(const char* ptr);
/// ```
///
/// Usage:
/// ```dart
/// final p2p = P2pBindings();
/// if (!p2p.isAvailable) {
///   print('P2P not compiled into this build');
///   return;
/// }
/// final ticket = p2p.generateTicket();
/// // Display ticket as QR code...
/// ```
library;
import 'dart:ffi';
import 'dart:io';
import 'package:ffi/ffi.dart';

// ── Function type definitions ─────────────────────────────────────────

typedef _P2pConnectC = Uint32 Function(Pointer<Utf8> ticket);
typedef _P2pConnectDart = int Function(Pointer<Utf8> ticket);

typedef _P2pHostTicketC = Pointer<Utf8> Function(Uint32 sessionId);
typedef _P2pHostTicketDart = Pointer<Utf8> Function(int sessionId);

typedef _P2pIsConnectedC = Bool Function(Uint32 sessionId);
typedef _P2pIsConnectedDart = bool Function(int sessionId);

typedef _P2pGenerateTicketC = Pointer<Utf8> Function();
typedef _P2pGenerateTicketDart = Pointer<Utf8> Function();

typedef _P2pFreeStringC = Void Function(Pointer<Utf8> ptr);
typedef _P2pFreeStringDart = void Function(Pointer<Utf8> ptr);

typedef _P2pConnectStatusC = Int32 Function(Uint32 sessionId);
typedef _P2pConnectStatusDart = int Function(int sessionId);

typedef _LastErrorC = Pointer<Utf8> Function();
typedef _LastErrorDart = Pointer<Utf8> Function();

// ── P2P bindings class ────────────────────────────────────────────────

/// Provides bindings to the P2P C-ABI functions in ggterm_ffi.
///
/// All functions are optional — if the Rust binary was compiled without
/// the `p2p` feature, [isAvailable] returns false and all calls become no-ops.
class P2pBindings {
  final DynamicLibrary _lib;

  /// Whether P2P functions were found in the loaded library.
  late final bool isAvailable;

  // Bound functions (nullable when not available)
  late final int Function(Pointer<Utf8>) _p2pConnect;
  late final Pointer<Utf8> Function(int) _p2pHostTicket;
  late final bool Function(int) _p2pIsConnected;
  late final Pointer<Utf8> Function() _p2pGenerateTicket;
  late final void Function(Pointer<Utf8>)? _p2pFreeString;
  late final Pointer<Utf8> Function() _lastError;
  late final int Function(int) _p2pConnectStatus;

  /// Load P2P bindings from the given [DynamicLibrary].
  ///
  /// Pass the same library used by [GgtermFfi]. If P2P symbols are not
  /// found, [isAvailable] is set to false.
  P2pBindings(this._lib) {
    _bindFunctions();
  }

  /// Convenience constructor that loads the library automatically.
  P2pBindings.autoload({String? libraryPath})
      : _lib = _loadLibrary(libraryPath) {
    _bindFunctions();
  }

  static DynamicLibrary _loadLibrary(String? libraryPath) {
    if (libraryPath != null) {
      return DynamicLibrary.open(libraryPath);
    }
    if (Platform.isMacOS || Platform.isLinux) {
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
      return DynamicLibrary.process();
    }
    if (Platform.isAndroid) {
      return DynamicLibrary.open('libggterm_ffi.so');
    }
    throw UnsupportedError('Unsupported platform for ggterm_ffi');
  }

  void _bindFunctions() {
    isAvailable = true;

    try {
      _p2pConnect = _lib
          .lookupFunction<_P2pConnectC, _P2pConnectDart>('ggterm_p2p_connect');
    } catch (_) {
      isAvailable = false;
      _p2pConnect = (_) => 0;
    }

    try {
      _p2pHostTicket = _lib
          .lookupFunction<_P2pHostTicketC, _P2pHostTicketDart>(
              'ggterm_p2p_host_ticket');
    } catch (_) {
      _p2pHostTicket = (_) => nullptr;
    }

    try {
      _p2pIsConnected = _lib
          .lookupFunction<_P2pIsConnectedC, _P2pIsConnectedDart>(
              'ggterm_p2p_is_connected');
    } catch (_) {
      _p2pIsConnected = (_) => false;
    }

    try {
      _p2pGenerateTicket = _lib
          .lookupFunction<_P2pGenerateTicketC, _P2pGenerateTicketDart>(
              'ggterm_p2p_generate_ticket');
    } catch (_) {
      _p2pGenerateTicket = () => nullptr;
    }

    // Free function is optional — some implementations use thread-local buffers.
    try {
      _p2pFreeString = _lib
          .lookupFunction<_P2pFreeStringC, _P2pFreeStringDart>(
              'ggterm_p2p_free_string');
    } catch (_) {
      _p2pFreeString = null;
    }

    try {
      _lastError = _lib
          .lookupFunction<_LastErrorC, _LastErrorDart>('ggterm_last_error');
    } catch (_) {
      _lastError = () => nullptr;
    }

    try {
      _p2pConnectStatus = _lib.lookupFunction<_P2pConnectStatusC, _P2pConnectStatusDart>(
          'ggterm_p2p_connect_status');
    } catch (_) {
      _p2pConnectStatus = (_) => -1;
    }
  }

  // ── Public API ──────────────────────────────────────────────────────

  /// Connect to a remote host using an Iroh NodeTicket string.
  ///
  /// [ticket] is a base32-encoded NodeTicket (~130 chars), typically
  /// obtained by scanning a QR code.
  ///
  /// Returns a session ID (> 0) on success, or 0 on failure.
  /// Call [lastError] after a 0 return to get the error message.
  int connect(String ticket) {
    if (!isAvailable) return 0;
    final ticketPtr = ticket.toNativeUtf8();
    try {
      return _p2pConnect(ticketPtr);
    } finally {
      malloc.free(ticketPtr);
    }
  }

  /// Get the last error message from the FFI layer.
  String? lastError() {
    final ptr = _lastError();
    if (ptr == nullptr) return null;
    return ptr.toDartString();
  }

  /// Check connection status: 0=connecting, 1=connected, -1=failed.
  int connectStatus(int sessionId) {
    if (!isAvailable) return -1;
    return _p2pConnectStatus(sessionId);
  }

  /// Generate a host ticket for a given session.
  ///
  /// The returned string is an Iroh NodeTicket that can be encoded as a
  /// QR code for a remote peer to scan.
  ///
  /// Returns null if P2P is not available or ticket generation fails.
  String? hostTicket(int sessionId) {
    if (!isAvailable) return null;
    final ptr = _p2pHostTicket(sessionId);
    if (ptr == nullptr) return null;
    final result = ptr.toDartString();
    _p2pFreeString?.call(ptr);
    return result;
  }

  /// Check whether a P2P session is connected.
  ///
  /// Returns true once the QUIC connection is established.
  bool isConnected(int sessionId) {
    if (!isAvailable) return false;
    return _p2pIsConnected(sessionId);
  }

  /// Generate a standalone ticket string (not tied to a specific session).
  ///
  /// This creates a new Iroh endpoint and returns its NodeTicket.
  /// Useful for display in a QR code before a session is created.
  ///
  /// Returns null if P2P is not available or generation fails.
  String? generateTicket() {
    if (!isAvailable) return null;
    final ptr = _p2pGenerateTicket();
    if (ptr == nullptr) return null;
    final result = ptr.toDartString();
    _p2pFreeString?.call(ptr);
    return result;
  }
}
