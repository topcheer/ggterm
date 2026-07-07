/// SSH connection screen — form for host/port/user/password + key file.
///
/// On successful connection, navigates to [TerminalScreen].

library;
import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:path_provider/path_provider.dart';
import 'theme.dart';

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

  /// Currently selected terminal theme name (read by main.dart).
  /// Updated whenever the user picks a theme in the connection screen.
  static String currentThemeName = 'dark';

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
  int _connectElapsed = 0;
  Timer? _connectTimer;
  bool _obscurePassword = true;
  bool _useKeyFile = false;
  String? _errorMessage;
  String _selectedTheme = 'dark'; // persisted theme name

  static const _saveFile = 'last_connection.json';
  static const _historyFile = 'connection_history.json';
  List<Map<String, dynamic>> _history = [];
  final int _maxHistory = 10;

  @override
  void initState() {
    super.initState();
    _loadSavedConnection();
    _loadHistory();
    _loadSavedTheme();
  }

  Future<void> _loadSavedTheme() async {
    try {
      final dir = await getApplicationDocumentsDirectory();
      final file = File('${dir.path}/theme.txt');
      if (await file.exists()) {
        final name = (await file.readAsString()).trim();
        if (builtinThemeNames.contains(name)) {
          setState(() {
            _selectedTheme = name;
            ConnectionScreen.currentThemeName = name;
          });
        }
      }
    } catch (_) {}
  }

  Future<void> _saveTheme(String name) async {
    try {
      final dir = await getApplicationDocumentsDirectory();
      final file = File('${dir.path}/theme.txt');
      await file.writeAsString(name);
    } catch (_) {}
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
    await _saveHistoryFile();
  }

  /// Save history to file without modifying state.
  Future<void> _saveHistoryFile() async {
    try {
      final dir = await getApplicationDocumentsDirectory();
      final file = File('${dir.path}/$_historyFile');
      await file.writeAsString(jsonEncode(_history));
    } catch (_) {}
  }

  /// Export connection history as JSON to the system share sheet.
  /// Passwords are never stored in history, so this is safe to share.
  Future<void> _exportHistory() async {
    if (_history.isEmpty) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('No history to export')),
        );
      }
      return;
    }
    try {
      final dir = await getApplicationDocumentsDirectory();
      final exportFile = File('${dir.path}/ggterm_connections_export.json');
      await exportFile.writeAsString(
        const JsonEncoder.withIndent('  ').convert(_history),
      );
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text('Exported ${_history.length} connections'),
          action: SnackBarAction(
            label: 'Copy JSON',
            onPressed: () {
              Clipboard.setData(ClipboardData(text: jsonEncode(_history)));
            },
          ),
        ),
      );
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Export failed: $e')),
        );
      }
    }
  }

  /// Import connection history from clipboard JSON.
  Future<void> _importHistory() async {
    final data = await Clipboard.getData('text/plain');
    if (data?.text == null || data!.text!.trim().isEmpty) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('Clipboard is empty — copy JSON first')),
      );
      return;
    }
    try {
      final parsed = jsonDecode(data.text!);
      if (parsed is! List) {
        if (mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(content: Text('Invalid format — expected JSON array')),
          );
        }
        return;
      }
      final imported = parsed
          .whereType<Map<String, dynamic>>()
          .where((e) => e.containsKey('host') && e.containsKey('user'))
          .toList();
      if (imported.isEmpty) {
        if (mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(content: Text('No valid entries found')),
          );
        }
        return;
      }
      // Merge: skip entries with same host+user already in history.
      final existing = _history
          .map((e) => '${e['host']}:${e['user']}')
          .toSet();
      int added = 0;
      for (final entry in imported) {
        final key = '${entry['host']}:${entry['user']}';
        if (!existing.contains(key)) {
          _history.add(entry);
          added++;
        }
      }
      if (_history.length > _maxHistory) {
        _history = _history.sublist(0, _maxHistory);
      }
      await _saveHistoryFile();
      setState(() {});
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Imported $added connection(s)')),
        );
      }
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Import failed: $e')),
        );
      }
    }
  }

  @override
  void dispose() {
    _connectTimer?.cancel();
    _hostController.dispose();
    _portController.dispose();
    _userController.dispose();
    _passController.dispose();
    _keyController.dispose();
    _ticketController.dispose();
    super.dispose();
  }

  /// Opens a dialog to paste SSH private key content (PEM format).
  /// Saves the pasted key to the app's documents directory and sets the
  /// key file path automatically. This is essential on iOS where the
  /// filesystem isn't directly accessible to users.
  Future<void> _pasteKeyContent() async {
    final keyContent = await showDialog<String>(
      context: context,
      builder: (context) {
        final controller = TextEditingController();
        return AlertDialog(
          title: const Text('Paste SSH Private Key'),
          content: SizedBox(
            width: double.maxFinite,
            child: TextField(
              controller: controller,
              maxLines: 12,
              minLines: 6,
              decoration: const InputDecoration(
                hintText: '-----BEGIN OPENSSH PRIVATE KEY-----\n...\n-----END OPENSSH PRIVATE KEY-----',
                border: OutlineInputBorder(),
                labelText: 'Key content',
              ),
              style: const TextStyle(fontFamily: 'monospace', fontSize: 12),
            ),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(context),
              child: const Text('Cancel'),
            ),
            FilledButton(
              onPressed: () => Navigator.pop(context, controller.text.trim()),
              child: const Text('Save'),
            ),
          ],
        );
      },
    );

    if (keyContent == null || keyContent.isEmpty) return;
    if (!keyContent.contains('PRIVATE KEY')) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('Invalid key — must contain "PRIVATE KEY" header')),
        );
      }
      return;
    }

    try {
      final dir = await getApplicationDocumentsDirectory();
      final keyFile = File('${dir.path}/user_ssh_key');
      await keyFile.writeAsString(keyContent);
      setState(() {
        _keyController.text = keyFile.path;
      });
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('Key saved to app storage')),
        );
      }
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Failed to save key: $e')),
        );
      }
    }
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

  /// Convert raw SSH error strings into user-friendly messages.
  String _friendlyError(String raw) {
    final lower = raw.toLowerCase();
    if (lower.contains('connect timeout')) {
      return 'Connection timed out. Check the host and port, or try again.';
    }
    if (lower.contains('auth')) {
      return 'Authentication failed. Check your username and password/key.';
    }
    if (lower.contains('connection refused') || lower.contains('connection reset')) {
      return 'Connection refused. The server may be down or SSH is not running.';
    }
    if (lower.contains('dns') || lower.contains('resolve') || lower.contains('nodename')) {
      return 'Cannot resolve hostname. Check the host address.';
    }
    if (lower.contains('network') || lower.contains('unreachable')) {
      return 'Network is unreachable. Check your internet connection.';
    }
    if (lower.contains('channel') || lower.contains('pty') || lower.contains('shell')) {
      return 'Session setup failed. The server may refuse PTY allocation.';
    }
    return raw;
  }

  Future<void> _connect() async {
    if (!_formKey.currentState!.validate()) return;

    setState(() {
      _connecting = true;
      _connectElapsed = 0;
      _connectTimer?.cancel();
      _connectTimer = Timer.periodic(const Duration(seconds: 1), (_) {
        if (mounted && _connecting) {
          setState(() => _connectElapsed++);
        }
      });
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
      // Success: light haptic feedback.
      HapticFeedback.lightImpact();
    } catch (e) {
      // Failure: heavier haptic pattern for error awareness.
      HapticFeedback.heavyImpact();
      if (mounted) {
        setState(() {
          _errorMessage = _friendlyError(e.toString());
        });
      }
    } finally {
      _connectTimer?.cancel();
      _connectTimer = null;
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
        actions: [
          PopupMenuButton<String>(
            icon: const Icon(Icons.more_vert),
            onSelected: (value) async {
              switch (value) {
                case 'export':
                  await _exportHistory();
                  break;
                case 'import':
                  await _importHistory();
                  break;
                case 'clear':
                  setState(() {
                    _history.clear();
                  });
                  await _saveHistoryFile();
                  if (mounted) {
                    ScaffoldMessenger.of(context).showSnackBar(
                      const SnackBar(content: Text('History cleared')),
                    );
                  }
                  break;
              }
            },
            itemBuilder: (context) => [
              const PopupMenuItem(
                value: 'export',
                child: Row(children: [
                  Icon(Icons.upload, size: 20),
                  SizedBox(width: 12),
                  Text('Export history'),
                ]),
              ),
              const PopupMenuItem(
                value: 'import',
                child: Row(children: [
                  Icon(Icons.download, size: 20),
                  SizedBox(width: 12),
                  Text('Import history'),
                ]),
              ),
              const PopupMenuItem(
                value: 'clear',
                child: Row(children: [
                  Icon(Icons.clear_all, size: 20),
                  SizedBox(width: 12),
                  Text('Clear all history'),
                ]),
              ),
            ],
          ),
        ],
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
                autofocus: true,
                autofillHints: const [AutofillHints.url],
                textInputAction: TextInputAction.next,
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
                textInputAction: TextInputAction.next,
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
              const SizedBox(height: 8),
              // Common SSH port presets for quick selection
              Wrap(
                spacing: 8,
                runSpacing: 4,
                children: ['22', '2222', '8022', '443'].map((port) {
                  final isSelected = _portController.text == port;
                  return ChoiceChip(
                    label: Text(port),
                    selected: isSelected,
                    onSelected: (_) {
                      setState(() => _portController.text = port);
                    },
                    visualDensity: VisualDensity.compact,
                  );
                }).toList(),
              ),
              const SizedBox(height: 16),

              // ── Username ──
              TextFormField(
                controller: _userController,
                autofillHints: const [AutofillHints.username],
                textInputAction: TextInputAction.next,
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
                const SizedBox(height: 8),
                // Quick-fill buttons for common SSH key locations
                Wrap(
                  spacing: 8,
                  runSpacing: 4,
                  children: [
                    '~/.ssh/id_rsa',
                    '~/.ssh/id_ed25519',
                    '~/.ssh/id_ecdsa',
                  ].map((path) {
                    final name = path.split('/').last;
                    return ActionChip(
                      label: Text(name),
                      avatar: const Icon(Icons.folder_open, size: 16),
                      onPressed: () => setState(() {
                        _keyController.text = path;
                      }),
                      visualDensity: VisualDensity.compact,
                    );
                  }).toList(),
                ),
                const SizedBox(height: 8),
                // Paste key content — for users who can't easily type a file path
                // on mobile (e.g. iOS where the filesystem isn't accessible).
                // Opens a dialog to paste PEM key content, saves to app docs dir.
                TextButton.icon(
                  onPressed: _pasteKeyContent,
                  icon: const Icon(Icons.content_paste, size: 18),
                  label: const Text('Paste key content instead'),
                ),
                const SizedBox(height: 8),
              ] else ...[
                TextFormField(
                  controller: _passController,
                  obscureText: _obscurePassword,
                  autofillHints: const [AutofillHints.password],
                  textInputAction: TextInputAction.go,
                  onFieldSubmitted: (_) {
                    // Press "Go" on keyboard to connect immediately
                    if (!_connecting) _connect();
                  },
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

              // ── Theme selector ──
              Row(
                children: [
                  const Icon(Icons.palette_outlined, size: 18, color: Colors.grey),
                  const SizedBox(width: 8),
                  Expanded(
                    child: DropdownButtonFormField<String>(
                      initialValue: _selectedTheme,
                      decoration: const InputDecoration(
                        labelText: 'Terminal Theme',
                        isDense: true,
                        border: OutlineInputBorder(),
                        contentPadding: EdgeInsets.symmetric(horizontal: 12, vertical: 8),
                      ),
                      items: builtinThemeNames.map((name) {
                        return DropdownMenuItem(
                          value: name,
                          child: Text(name),
                        );
                      }).toList(),
                      onChanged: (value) {
                        if (value != null) {
                          setState(() {
                            _selectedTheme = value;
                            ConnectionScreen.currentThemeName = value;
                          });
                          _saveTheme(value);
                        }
                      },
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 12),

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
                label: Text(_connecting
                    ? 'Connecting… ${_connectElapsed}s'
                    : 'Connect'),
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
                  return Dismissible(
                    key: ValueKey('history-$idx-$host-$port-$user'),
                    direction: DismissDirection.endToStart,
                    background: Container(
                      alignment: Alignment.centerRight,
                      padding: const EdgeInsets.only(right: 20),
                      color: Colors.red.withValues(alpha: 0.8),
                      child: const Icon(Icons.delete, color: Colors.white),
                    ),
                    onDismissed: (_) => _removeFromHistory(idx),
                    child: ListTile(
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
                      onLongPress: () {
                        // Long-press: fill form and attempt direct connect
                        _fillFromHistory(e);
                        if (_formKey.currentState?.validate() ?? false) {
                          HapticFeedback.mediumImpact();
                          _connect();
                        }
                      },
                    ),
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
