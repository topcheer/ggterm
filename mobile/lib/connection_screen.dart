/// SSH connection screen — form for host/port/user/password + key file.
///
/// On successful connection, navigates to [TerminalScreen].

library;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

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
                decoration: const InputDecoration(
                  labelText: 'Host',
                  hintText: 'example.com',
                  prefixIcon: Icon(Icons.dns),
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

              // ── Recent connections ──
              Text(
                'Recent Connections',
                style: Theme.of(context).textTheme.titleSmall?.copyWith(
                      color: Colors.grey.shade400,
                    ),
              ),
              const SizedBox(height: 8),
              Material(
                color: Colors.transparent,
                child: ListTile(
                  leading: const Icon(Icons.history, color: Colors.grey),
                  title: Text(
                    'No saved connections',
                    style: TextStyle(color: Colors.grey.shade600),
                  ),
                  dense: true,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
