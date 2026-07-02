/// SSH connection screen — form for host/port/user/password + key file.
///
/// On successful connection, navigates to [TerminalScreen].

import 'package:flutter/material.dart';

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
  final Future<bool> Function(ConnectionParams params) onConnect;

  /// Called when the user taps Echo Test.
  final VoidCallback? onEchoTest;

  const ConnectionScreen({
    super.key,
    required this.onConnect,
    this.onEchoTest,
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

  bool _connecting = false;
  bool _obscurePassword = true;
  bool _useKeyFile = false;

  @override
  void dispose() {
    _hostController.dispose();
    _portController.dispose();
    _userController.dispose();
    _passController.dispose();
    _keyController.dispose();
    super.dispose();
  }

  Future<void> _connect() async {
    if (!_formKey.currentState!.validate()) return;

    setState(() => _connecting = true);

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
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Error: $e')),
        );
      }
    } finally {
      if (mounted) setState(() => _connecting = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('GGTerm — Connect'),
        backgroundColor: Colors.grey.shade900,
        foregroundColor: Colors.white,
      ),
      body: Container(
        color: Colors.grey.shade950,
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

              // ── Connect button ──
              FilledButton.icon(
                onPressed: _connecting ? null : _connect,
                icon: _connecting
                    ? const SizedBox(
                        width: 18,
                        height: 18,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      )
                    : const Icon(Icons.power_plug),
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
                  label: const Text('Echo Test (No SSH)'),
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
              ListTile(
                leading: const Icon(Icons.history, color: Colors.grey),
                title: Text(
                  'No saved connections',
                  style: TextStyle(color: Colors.grey.shade600),
                ),
                dense: true,
              ),
            ],
          ),
        ),
      ),
    );
  }
}
