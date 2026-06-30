/// GGTerm mobile entry point.
///
/// Flow: ConnectionScreen → (connect) → TerminalScreen.
///
/// The session manager will be provided by flutter_rust_bridge once the
/// Rust core library is wired up. For now, the UI shell is self-contained.

import 'package:flutter/material.dart';

import 'connection_screen.dart';
import 'keyboard_bar.dart';
import 'ai_toolbar.dart';
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
      home: const ConnectionScreen(
        onConnect: _placeholderConnect,
      ),
    );
  }
}

/// Placeholder connection handler.
///
/// In production this will:
/// 1. Initialize the Rust session via flutter_rust_bridge.
/// 2. Establish SSH connection.
/// 3. Return true on success.
Future<bool> _placeholderConnect(ConnectionParams params) async {
  // Simulate connection attempt.
  await Future.delayed(const Duration(milliseconds: 500));
  debugPrint(
      'Connect: ${params.username}@${params.host}:${params.port} '
      '(key: ${params.keyFilePath ?? "password"})');
  return true;
}
