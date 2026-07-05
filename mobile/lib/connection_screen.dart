/// SSH connection screen — form for host/port/user/password + key file.
///
/// On successful connection, navigates to [TerminalScreen].

library;
import 'dart:convert';
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:path_provider/path_provider.dart';

/// SSH connection parameters (passed to SessionManager).
class ConnectionParams {
  final String host;
  final int port;
  final String username;
  final String password;
  final String? keyFilePath;

  const ConnectionParams({
    required this.host,
    this.port = 22,
    required this.username,
    this.password = '',
    this.keyFilePath,
  });
}

class ConnectionScreen extends StatefulWidget {
  /// Called when the user taps Connect.
  final Future<void> Function(ConnectionParams params) onConnect;

  /// Called when the user taps Echo Test.
  final VoidCallback? onEchoTest;

  /// Called when the user taps Local Shell (Android only).
  final VoidCallback? onLocalShell;

  /// Called when the user wants to scan a P2P QR code.
  final VoidCallback? onScanQr;

  /// Called when the user wants to host a P2P session.
  final VoidCallback? onShare;

  const ConnectionScreen({
    super.key,
    required this.onConnect,
    this.onEchoTest,
    this.onLocalShell,
    this.onScanQr,
    this.onShare,
  });

  @override
  State<ConnectionScreen> createState() => _ConnectionScreenState();
}

class _ConnectionScreenState extends State<ConnectionScreen> {
  final _formKey = GlobalKey<FormState>();
  final _hostController = TextEditingController();
  final _portController = TextEditingController(text: '22');
  final _userController = TextEditingController();
  final _passController = TextEditingController();
  final _keyController = TextEditingController();
  final _ticketController = TextEditingController();

  bool _connecting = false;
  bool _obscurePassword = true;
  bool _useKeyFile = false;
  String? _errorMessage;

  static const _saveFile = 'last_connection.json';
  static const _historyFile = 'connection_history.json';
  List<Map<String, dynamic>> _history = [];
  final int _maxHistory = 10;

  @override
  void initState() {
    super.initState();
    _loadSavedConnection();
    _loadHistory();
  }

  /// Load saved connection details (host, port, user — never password).
  Future<void> _loadSavedConnection() async {
    try {
      final dir = await getApplicationDocumentsDirectory();
      final file = File('${dir.path}/$_saveFile');
      if (await file.exists()) {
        final json = jsonDecode(await file.readAsString()) as Map<String, dynamic>;
        if (mounted) {
          setState(() {
            _hostController.text = json['host'] as String? ?? '';
            _portController.text = json['port'] as String? ?? '22';
            _userController.text = json['user'] as String? ?? '';
            _keyController.text = json['keyFile'] as String? ?? '';
            _useKeyFile = json['useKeyFile'] as bool? ?? false;
          });
        }
      }
    } catch (_) {
      // Ignore errors — non-critical feature.
    }
  }

  /// Save connection details (never password) for next launch.
  Future<void> _saveConnection() async {
    try {
      final dir = await getApplicationDocumentsDirectory();
      final file = File('${dir.path}/$_saveFile');
      final json = jsonEncode({
        'host': _hostController.text.trim(),
        'port': _portController.text.trim(),
        'user': _userController.text.trim(),
        'keyFile': _keyController.text.trim(),
        'useKeyFile': _useKeyFile,
      });
      await file.writeAsString(json);
    } catch (_) {
      // Ignore errors — non-critical feature.
    }
  }

  /// Load connection history list.
  Future<void> _loadHistory() async {
    try {
      final dir = await getApplicationDocumentsDirectory();
      final file = File('${dir.path}/$_historyFile');
      if (await file.exists()) {
        final list = jsonDecode(await file.readAsString()) as List;
        if (mounted) {
          setState(() {
            _history = list
                .map((e) => e as Map<String, dynamic>)
                .toList();
          });
        }
      }
    } catch (_) {}
  }

  /// Save connection to history (deduplicated by host+port+user).
  Future<void> _addToHistory() async {
    final entry = {
      'host': _hostController.text.trim(),
      'port': _portController.text.trim(),
      'user': _userController.text.trim(),
      'keyFile': _keyController.text.trim(),
      'useKeyFile': _useKeyFile,
      'timestamp': DateTime.now().millisecondsSinceEpoch,
    };
    if ((entry['host'] as String?)?.isEmpty ?? true) return;

    // Deduplicate by host+port+user.
    final key = '${entry['host']}:${entry['port']}@${entry['user']}';
    _history.removeWhere((e) {
      final ek = '${e['host']}:${e['port']}@${e['user']}';
      return ek == key;
    });
    _history.insert(0, entry);
    if (_history.length > _maxHistory) {
      _history = _history.sublist(0, _maxHistory);
    }

    try {
      final dir = await getApplicationDocumentsDirectory();
      final file = File('${dir.path}/$_historyFile');
      await file.writeAsString(jsonEncode(_history));
    } catch (_) {}
  }

  /// Fill the form from a history entry.
  void _fillFromHistory(Map<String, dynamic> entry) {
    setState(() {
      _hostController.text = entry['host'] as String? ?? '';
      _portController.text = entry['port'] as String? ?? '22';
      _userController.text = entry['user'] as String? ?? '';
      _keyController.text = entry['keyFile'] as String? ?? '';
      _useKeyFile = entry['useKeyFile'] as bool? ?? false;
    });
  }

  /// Delete a history entry.
  Future<void> _removeFromHistory(int index) async {
    setState(() {
      _history.removeAt(index);
    });
    try {
      final dir = await getApplicationDocumentsDirectory();
      final file = File('${dir.path}/$_historyFile');
      await file.writeAsString(jsonEncode(_history));
    } catch (_) {}
  }

  @override
  void dispose() {
    _hostController.dispose();
    _portController.dispose();
    _userController.dispose();
    _passController.dispose();
    _keyController.dispose();
    _ticketController.dispose();
    super.dispose();
  }

  /// Direct P2P connect via pasted ticket.
  void _onDirectConnect(String ticket) {
    if (ticket.trim().isEmpty) return;
    // Navigate to terminal screen with the pasted ticket.
    // The QR scan screen handler accepts both scanned and pasted tickets.
    widget.onScanQr?.call();
  }

  /// Paste from clipboard and parse SSH connection string.
  /// Supports formats:
  ///   ssh://user@host:port
  ///   user@host:port
  ///   user@host
  ///   host:port
  Future<void> _pasteAndParseSshUrl() async {
    final data = await Clipboard.getData('text/plain');
    if (data?.text == null || data!.text!.trim().isEmpty) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('Clipboard is empty')),
        );
      }
      return;
    }

    var text = data.text!.trim();
    // Strip ssh:// prefix.
    if (text.startsWith('ssh://')) {
      text = text.substring(6);
    }
    // Strip trailing path.
    final slashIdx = text.indexOf('/');
    if (slashIdx >= 0) {
      text = text.substring(0, slashIdx);
    }

    String host = '';
    String port = '22';
    String user = '';

    // Split user@rest.
    final atIdx = text.indexOf('@');
    if (atIdx >= 0) {
      user = text.substring(0, atIdx);
      text = text.substring(atIdx + 1);
    }

    // Split host:port — handle IPv6 [::1]:22 and regular host:22.
    if (text.startsWith('[')) {
      // IPv6: [addr]:port
      final bracketEnd = text.indexOf(']');
      if (bracketEnd >= 0) {
        host = text.substring(1, bracketEnd);
        final after = text.substring(bracketEnd + 1);
        if (after.startsWith(':')) {
          port = after.substring(1);
        }
      } else {
        host = text;
      }
    } else {
      final colonIdx = text.lastIndexOf(':');
      if (colonIdx >= 0) {
        final maybePort = text.substring(colonIdx + 1);
        if (int.tryParse(maybePort) != null) {
          host = text.substring(0, colonIdx);
          port = maybePort;
        } else {
          host = text;
        }
      } else {
        host = text;
      }
    }

    setState(() {
      _hostController.text = host;
      _portController.text = port;
      if (user.isNotEmpty) _userController.text = user;
    });

    if (mounted) {
      final summary = user.isNotEmpty
          ? '$user@$host:$port'
          : '$host:$port';
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Filled: $summary')),
      );
    }
  }

  Future<void> _connect() async {
    if (!_formKey.currentState!.validate()) return;

    setState(() {
      _connecting = true;
      _errorMessage = null;
    });

    final params = ConnectionParams(
      host: _hostController.text.trim(),
      port: int.tryParse(_portController.text.trim()) ?? 22,
      username: _userController.text.trim(),
      password: _passController.text,
      keyFilePath: _useKeyFile && _keyController.text.isNotEmpty
          ? _keyController.text.trim()
          : null,
    );

    try {
      await widget.onConnect(params);
      // Save connection details for next launch (password excluded).
      await _saveConnection();
      await _addToHistory();
    } catch (e) {
      if (mounted) {
        setState(() {
          _errorMessage = e.toString();
        });
      }
    } finally {
      if (mounted) setState(() => _connecting = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Colors.grey.shade900,
      appBar: AppBar(
        title: const Text('GGTerm — Connect'),
        backgroundColor: Colors.grey.shade900,
        foregroundColor: Colors.white,
      ),
      body: Padding(
        padding: const EdgeInsets.all(24),
        child: Form(
          key: _formKey,
          child: ListView(
            children: [
              // ── Host ──
              TextFormField(
                controller: _hostController,
                decoration: InputDecoration(
                  labelText: 'Host',
                  hintText: 'example.com',
                  prefixIcon: const Icon(Icons.dns),
                  suffixIcon: IconButton(
                    icon: const Icon(Icons.content_paste),
                    tooltip: 'Paste SSH URL (user@host:port)',
                    onPressed: _pasteAndParseSshUrl,
                  ),
                ),
                validator: (v) =>
                    (v == null || v.trim().isEmpty) ? 'Enter host' : null,
              ),
              const SizedBox(height: 16),

              // ── Port ──
              TextFormField(
                controller: _portController,
                decoration: const InputDecoration(
                  labelText: 'Port',
                  prefixIcon: Icon(Icons.numbers),
                ),
                keyboardType: TextInputType.number,
                validator: (v) {
                  final port = int.tryParse(v ?? '');
                  if (port == null || port < 1 || port > 65535) {
                    return 'Enter valid port (1-65535)';
                  }
                  return null;
                },
              ),
              const SizedBox(height: 16),

              // ── Username ──
              TextFormField(
                controller: _userController,
                decoration: const InputDecoration(
                  labelText: 'Username',
                  hintText: 'root',
                  prefixIcon: Icon(Icons.person),
                ),
                validator: (v) =>
                    (v == null || v.trim().isEmpty) ? 'Enter username' : null,
              ),
              const SizedBox(height: 16),

              // ── Password / Key file toggle ──
              SwitchListTile(
                title: const Text('Use key file'),
                subtitle: const Text('Authenticate with SSH private key'),
                value: _useKeyFile,
                onChanged: (v) => setState(() => _useKeyFile = v),
              ),

              if (_useKeyFile) ...[
                TextFormField(
                  controller: _keyController,
                  decoration: const InputDecoration(
                    labelText: 'Key file path',
                    hintText: '~/.ssh/id_rsa',
                    prefixIcon: Icon(Icons.key),
                  ),
                ),
                const SizedBox(height: 16),
              ] else ...[
                TextFormField(
                  controller: _passController,
                  obscureText: _obscurePassword,
                  decoration: InputDecoration(
                    labelText: 'Password',
                    prefixIcon: const Icon(Icons.lock),
                    suffixIcon: IconButton(
                      icon: Icon(_obscurePassword
                          ? Icons.visibility
                          : Icons.visibility_off),
                      onPressed: () => setState(
                          () => _obscurePassword = !_obscurePassword),
                    ),
                  ),
                ),
                const SizedBox(height: 16),
              ],

              // ── Error message (inline, not transient SnackBar) ──
              if (_errorMessage != null) ...[
                Container(
                  padding: const EdgeInsets.all(12),
                  decoration: BoxDecoration(
                    color: Colors.red.withValues(alpha: 0.15),
                    borderRadius: BorderRadius.circular(8),
                    border: Border.all(color: Colors.red.shade700, width: 1),
                  ),
                  child: Row(
                    children: [
                      const Icon(Icons.error_outline, color: Colors.red, size: 20),
                      const SizedBox(width: 8),
                      Expanded(
                        child: Text(
                          _errorMessage!,
                          style: TextStyle(color: Colors.red.shade200, fontSize: 13),
                        ),
                      ),
                    ],
                  ),
                ),
                const SizedBox(height: 16),
              ],

              // ── Connect button ──
              FilledButton.icon(
                onPressed: _connecting ? null : _connect,
                icon: _connecting
                    ? const SizedBox(
                        width: 18,
                        height: 18,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      )
                    : const Icon(Icons.electrical_services),
                label: Text(_connecting ? 'Connecting...' : 'Connect'),
                style: FilledButton.styleFrom(
                  minimumSize: const Size.fromHeight(48),
                ),
              ),

              const SizedBox(height: 12),

              // ── Connection History ──
              if (_history.isNotEmpty) ...[
                const Divider(),
                Row(
                  children: [
                    const Icon(Icons.history, size: 16, color: Colors.grey),
                    const SizedBox(width: 6),
                    Text('Recent Connections',
                        style: Theme.of(context).textTheme.titleSmall),
                  ],
                ),
                const SizedBox(height: 4),
                ..._history.asMap().entries.map((entry) {
                  final idx = entry.key;
                  final e = entry.value;
                  final host = e['host'] as String? ?? '';
                  final port = e['port'] as String? ?? '22';
                  final user = e['user'] as String? ?? '';
                  final subtitle = user.isNotEmpty
                      ? '$user@$host${port != '22' ? ':$port' : ''}'
                      : '$host${port != '22' ? ':$port' : ''}';
                  return ListTile(
                    dense: true,
                    leading: const Icon(Icons.dns, size: 20),
                    title: Text(host,
                        style: const TextStyle(fontSize: 14)),
                    subtitle: Text(subtitle,
                        style: TextStyle(
                            fontSize: 12, color: Colors.grey.shade500)),
                    trailing: IconButton(
                      icon: const Icon(Icons.close, size: 16),
                      onPressed: () => _removeFromHistory(idx),
                      tooltip: 'Remove',
                    ),
                    onTap: () => _fillFromHistory(e),
                  );
                }),
              ],

              const SizedBox(height: 12),

              // ── Echo Test button (no SSH needed) ──
              if (widget.onEchoTest != null)
                OutlinedButton.icon(
                  onPressed: () => widget.onEchoTest!(),
                  icon: const Icon(Icons.terminal),
                  label: const Text('Echo Test'),
                  style: OutlinedButton.styleFrom(
                    minimumSize: const Size.fromHeight(44),
                  ),
                ),

              const SizedBox(height: 12),

              // ── Local Shell button (Android only) ──
              if (widget.onLocalShell != null)
                OutlinedButton.icon(
                  onPressed: () => widget.onLocalShell!(),
                  icon: const Icon(Icons.phone_android),
                  label: const Text('Local Shell'),
                  style: OutlinedButton.styleFrom(
                    minimumSize: const Size.fromHeight(44),
                  ),
                ),

              const SizedBox(height: 12),

              // ── P2P: Direct Ticket Input ──
              if (widget.onScanQr != null) ...[
                const Divider(),
                const SizedBox(height: 8),
                Text('P2P Direct Connect',
                    style: Theme.of(context).textTheme.titleSmall),
                const SizedBox(height: 8),
                TextField(
                  controller: _ticketController,
                  decoration: InputDecoration(
                    labelText: 'Paste Ticket',
                    hintText: 'Paste P2P ticket here...',
                    border: const OutlineInputBorder(),
                    suffixIcon: IconButton(
                      icon: const Icon(Icons.paste),
                      onPressed: () async {
                        final clip = await Clipboard.getData('text/plain');
                        if (clip?.text != null &&
                            clip!.text!.isNotEmpty) {
                          _ticketController.text = clip.text!;
                        }
                      },
                    ),
                  ),
                  maxLines: 2,
                  style: const TextStyle(
                      fontSize: 12, fontFamily: 'monospace'),
                ),
                const SizedBox(height: 8),
                FilledButton.icon(
                  onPressed: _ticketController.text.isEmpty
                      ? null
                      : () => _onDirectConnect(_ticketController.text),
                  icon: const Icon(Icons.link),
                  label: const Text('Connect via Ticket'),
                  style: FilledButton.styleFrom(
                    minimumSize: const Size.fromHeight(44),
                  ),
                ),
              ],

              const SizedBox(height: 12),

              // ── P2P: Scan QR ──
              if (widget.onScanQr != null)
                OutlinedButton.icon(
                  onPressed: () => widget.onScanQr!(),
                  icon: const Icon(Icons.qr_code_scanner),
                  label: const Text('Scan QR (P2P)'),
                  style: OutlinedButton.styleFrom(
                    minimumSize: const Size.fromHeight(44),
                  ),
                ),

              // ── P2P: Share Terminal ──
              if (widget.onShare != null)
                OutlinedButton.icon(
                  onPressed: () => widget.onShare!(),
                  icon: const Icon(Icons.qr_code),
                  label: const Text('Share Terminal (P2P)'),
                  style: OutlinedButton.styleFrom(
                    minimumSize: const Size.fromHeight(44),
                  ),
                ),

              const SizedBox(height: 24),

              // ── Quick tips ──
              Container(
                padding: const EdgeInsets.all(12),
                decoration: BoxDecoration(
                  color: Colors.blue.withValues(alpha: 0.08),
                  borderRadius: BorderRadius.circular(8),
                  border: Border.all(
                    color: Colors.blue.withValues(alpha: 0.2),
                    width: 1,
                  ),
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      children: [
                        const Icon(Icons.lightbulb_outline,
                            color: Colors.blue, size: 18),
                        const SizedBox(width: 6),
                        Text(
                          'Tips',
                          style: TextStyle(
                            color: Colors.blue.shade300,
                            fontWeight: FontWeight.w600,
                            fontSize: 13,
                          ),
                        ),
                      ],
                    ),
                    const SizedBox(height: 8),
                    Text(
                      '• Connect to any SSH server using host, port, and credentials\n'
                      '• Use Scan QR to connect to a GGTerm desktop sharing its terminal\n'
                      '• Long-press terminal text to copy it\n'
                      '• Double-tap a word to copy just that word\n'
                      '• Two-finger drag to scroll through history',
                      style: TextStyle(
                        color: Colors.grey.shade400,
                        fontSize: 12,
                        height: 1.5,
                      ),
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
