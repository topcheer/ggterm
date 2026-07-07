/// GGTerm mobile entry point.
///
/// Flow: ConnectionScreen → (connect) → TerminalScreen.
/// Uses dart:ffi to bridge to the Rust ggterm_ffi library.

library;
import 'dart:io' show Platform, Process, File;
import 'package:flutter/material.dart';

import 'ffi/session_manager.dart';
import 'ffi/p2p_bindings.dart';
import 'connection_screen.dart';
import 'terminal_screen.dart';
import 'theme.dart';
import 'screens/qr_scan_screen.dart';
import 'screens/share_screen.dart';

void main() {
  runApp(const GGTermApp());
}

class GGTermApp extends StatelessWidget {
  const GGTermApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'GGTerm',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        brightness: Brightness.dark,
        colorSchemeSeed: Colors.blue,
        useMaterial3: true,
      ),
      home: const _ConnectionEntry(),
    );
  }
}

class _ConnectionEntry extends StatefulWidget {
  const _ConnectionEntry();

  @override
  State<_ConnectionEntry> createState() => _ConnectionEntryState();
}

class _ConnectionEntryState extends State<_ConnectionEntry> {
  final _sessionManager = SessionManager();
  late final P2pBindings _p2p;

  @override
  void initState() {
    super.initState();
    _p2p = P2pBindings.autoload();
    // Check for auto-connect ticket file (pushed by adb for automation).
    WidgetsBinding.instance.addPostFrameCallback((_) => _checkAutoConnect());
  }

  /// Read ticket from /sdcard/ggterm_ticket.txt and auto-connect.
  void _checkAutoConnect() {
    Future(() async {
      try {
        // Try multiple paths — the one accessible depends on Android version.
        final paths = [
          '/data/data/com.example.ggterm_mobile/files/ggterm_ticket.txt',
          '/data/local/tmp/ggterm_ticket.txt',
        ];
        String? ticket;
        for (final path in paths) {
          final file = File(path);
          try {
            if (await file.exists()) {
              ticket = (await file.readAsString()).trim();
              debugPrint('[AUTO] Read ticket from $path (${ticket.length} chars)');
              break;
            }
          } catch (_) {}
        }

        if (ticket != null && ticket.length > 20) {
          debugPrint('[AUTO] Auto-connecting...');
          await _connectWithTicket(ticket);
        } else {
          debugPrint('[AUTO] No ticket found');
        }
      } catch (e) {
        debugPrint('[AUTO] Error: $e');
      }
    });
  }

  /// Connect directly using a P2P ticket string.
  Future<void> _connectWithTicket(String ticket) async {
    if (!_p2p.isAvailable) {
      debugPrint('[AUTO] P2P not available');
      return;
    }

    debugPrint('[AUTO] calling p2p.connect...');
    final sessionId = _p2p.connect(ticket);
    debugPrint('[AUTO] sessionId=$sessionId');

    if (sessionId == 0) {
      final err = _p2p.lastError();
      debugPrint('[AUTO] connect failed: $err');
      return;
    }

    // Poll connect status.
    bool connected = false;
    for (int i = 0; i < 120; i++) {
      await Future.delayed(const Duration(milliseconds: 500));
      final status = _p2p.connectStatus(sessionId);
      if (i % 10 == 0) debugPrint('[AUTO] poll #$i: status=$status');
      if (status == 1) {
        connected = true;
        break;
      }
      if (status == -1) {
        debugPrint('[AUTO] connect failed: ${_p2p.lastError()}');
        return;
      }
    }

    if (connected && mounted) {
      debugPrint('[AUTO] connected! navigating to terminal');
      // Delete the ticket file so it doesn't auto-connect again.
      try {
        await Process.run('rm', ['-f', '/sdcard/ggterm_ticket.txt']);
      } catch (_) {}

      if (!mounted) return;
      Navigator.of(context).push(
        MaterialPageRoute(
          builder: (_) => TerminalScreen(
            sessionManager: _sessionManager,
            sessionId: sessionId,
            title: 'P2P Session',
            theme: builtinThemes[ConnectionScreen.currentThemeName] ?? darkTheme,
          ),
        ),
      ).then((_) {
        _sessionManager.destroySession(sessionId);
      });
    }
  }

  @override
  void dispose() {
    _sessionManager.dispose();
    super.dispose();
  }

  Future<void> _onConnect(ConnectionParams params, {bool echo = false, bool localShell = false}) async {
    // Create session
    final sessionId = _sessionManager.createSession(80, 24);

    bool connected = false;
    String title = '';
    if (echo) {
      connected = _sessionManager.echoConnect(sessionId);
      title = 'Echo Mode';
    } else if (localShell) {
      connected = _sessionManager.localShellConnect(sessionId);
      title = 'Local Shell';
    } else {
      // SSH connect — use key auth if key file is provided, otherwise password.
      final sshParams = SshConnectionParams(
        host: params.host,
        port: params.port,
        user: params.username,
        password: params.password,
        keyFilePath: params.keyFilePath,
      );
      if (sshParams.usesKey) {
        connected = _sessionManager.sshConnectKey(sessionId, sshParams);
      } else {
        connected = _sessionManager.sshConnect(sessionId, sshParams);
      }
      title = '${params.username}@${params.host}';
    }

    if (connected && mounted) {
      Navigator.of(context).push(
        MaterialPageRoute(
          builder: (_) => TerminalScreen(
            sessionManager: _sessionManager,
            sessionId: sessionId,
            title: title,
            theme: builtinThemes[ConnectionScreen.currentThemeName] ?? darkTheme,
          ),
        ),
      ).then((_) {
        // Clean up session when terminal screen is popped
        _sessionManager.destroySession(sessionId);
      });
    } else {
      // Show error
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text('Connection failed: ${_sessionManager.lastError.isEmpty ? "Unknown error" : _sessionManager.lastError}'),
            backgroundColor: Colors.red.shade800,
          ),
        );
        _sessionManager.destroySession(sessionId);
      }
    }
  }

  void _onScanQr() {
    Navigator.of(context).push(
      MaterialPageRoute(
        builder: (_) => QrScanScreen(
          p2p: _p2p,
          sessionManager: _sessionManager,
        ),
      ),
    );
  }

  void _onShare() {
    Navigator.of(context).push(
      MaterialPageRoute(
        builder: (_) => ShareScreen(
          p2p: _p2p,
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return ConnectionScreen(
      onConnect: (params) async {
        await _onConnect(params);
      },
      onEchoTest: () => _onConnect(const ConnectionParams(
        host: '', username: 'echo',
      ), echo: true),
      onLocalShell: Platform.isAndroid
          ? () => _onConnect(const ConnectionParams(
              host: '', username: 'local',
            ), localShell: true)
          : null,
      onScanQr: _p2p.isAvailable ? _onScanQr : null,
      // Share Terminal (P2P host) only makes sense with a local terminal.
      // iOS has no local shell, so only show on Android.
      onShare: (_p2p.isAvailable && Platform.isAndroid) ? _onShare : null,
    );
  }
}
