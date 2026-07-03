/// GGTerm mobile entry point.
///
/// Flow: ConnectionScreen → (connect) → TerminalScreen.
/// Uses dart:ffi to bridge to the Rust ggterm_ffi library.

import 'dart:io' show Platform;
import 'package:flutter/material.dart';

import 'ffi/session_manager.dart';
import 'connection_screen.dart';
import 'terminal_screen.dart';
import 'theme.dart';

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
      // SSH connect (blocking — in production, run on background isolate)
      connected = _sessionManager.sshConnect(sessionId, SshConnectionParams(
        host: params.host,
        port: params.port,
        user: params.username,
        password: params.password,
        keyFilePath: params.keyFilePath,
      ));
      title = '${params.username}@${params.host}';
    }

    if (connected && mounted) {
      Navigator.of(context).push(
        MaterialPageRoute(
          builder: (_) => TerminalScreen(
            sessionManager: _sessionManager,
            sessionId: sessionId,
            title: title,
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
    );
  }
}
