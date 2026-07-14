/// Share screen — host mode for P2P terminal sharing.
///
/// Generates an Iroh NodeTicket via FFI and displays it as a QR code.
/// A remote peer (another GGTerm instance) scans the code to connect.
///
/// Flow:
/// 1. Call `ggterm_p2p_generate_ticket()` to get a ticket string
/// 2. Render QR code from the ticket using `qr_flutter`
/// 3. Poll `ggterm_p2p_is_connected()` to show connection status
/// 4. On connection, optionally navigate to terminal or show status

library;
import 'dart:async';
import 'package:flutter/material.dart';
import 'package:qr_flutter/qr_flutter.dart';

import '../ffi/p2p_bindings.dart';
import '../theme.dart';

class ShareScreen extends StatefulWidget {
  final P2pBindings p2p;
  final TerminalTheme theme;

  /// Called when a peer connects (optional).
  final void Function(int sessionId)? onPeerConnected;

  const ShareScreen({
    super.key,
    required this.p2p,
    this.theme = darkTheme,
    this.onPeerConnected,
  });

  @override
  State<ShareScreen> createState() => _ShareScreenState();
}

enum _ShareState {
  generating,
  waiting,
  connected,
  error,
}

class _ShareScreenState extends State<ShareScreen> {
  _ShareState _state = _ShareState.generating;
  String? _ticket;
  String? _errorMessage;
  Timer? _pollTimer;

  @override
  void initState() {
    super.initState();
    _generateTicket();
  }

  @override
  void dispose() {
    _pollTimer?.cancel();
    super.dispose();
  }

  Future<void> _generateTicket() async {
    if (!widget.p2p.isAvailable) {
      setState(() {
        _state = _ShareState.error;
        _errorMessage = 'P2P is not available in this build.\n'
            'Make sure ggterm_ffi is compiled with the p2p feature.';
      });
      return;
    }

    final ticket = widget.p2p.generateTicket();

    if (ticket == null || ticket.isEmpty) {
      setState(() {
        _state = _ShareState.error;
        _errorMessage = 'Failed to generate P2P ticket.\n'
            'Check your network connection and try again.';
      });
      return;
    }

    setState(() {
      _ticket = ticket;
      _state = _ShareState.waiting;
    });

    // Start polling for connection status.
    _pollTimer = Timer.periodic(const Duration(seconds: 1), (_) {
      _checkConnection();
    });
  }

  void _checkConnection() {
    // Use session ID 0 for the host-side ticket check.
    // The Rust side tracks connection state for the generated ticket.
    if (widget.p2p.isConnected(0)) {
      _pollTimer?.cancel();
      setState(() => _state = _ShareState.connected);

      // Notify callback if provided.
      widget.onPeerConnected?.call(0);
    }
  }

  void _regenerate() {
    _pollTimer?.cancel();
    setState(() {
      _state = _ShareState.generating;
      _ticket = null;
      _errorMessage = null;
    });
    _generateTicket();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: widget.theme.background,
      appBar: AppBar(
        title: const Text('Share Terminal'),
        backgroundColor: widget.theme.background,
        foregroundColor: widget.theme.foreground,
        actions: [
          if (_state == _ShareState.waiting)
            IconButton(
              icon: const Icon(Icons.refresh),
              tooltip: 'Regenerate ticket',
              onPressed: _regenerate,
            ),
        ],
      ),
      body: SafeArea(
        child: Center(
          child: Padding(
            padding: const EdgeInsets.all(24),
            child: switch (_state) {
              _ShareState.generating => _buildGenerating(),
              _ShareState.waiting => _buildWaiting(),
              _ShareState.connected => _buildConnected(),
              _ShareState.error => _buildError(),
            },
          ),
        ),
      ),
    );
  }

  // ── State views ──────────────────────────────────────────────────────

  Widget _buildGenerating() {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        const CircularProgressIndicator(color: Colors.blue),
        const SizedBox(height: 24),
        Text(
          'Generating ticket...',
          style: TextStyle(
            color: widget.theme.foreground,
            fontSize: 16,
            decoration: TextDecoration.none,
          ),
        ),
        const SizedBox(height: 8),
        Text(
          'Setting up Iroh endpoint for P2P connection',
          style: TextStyle(
            color: widget.theme.foreground.withValues(alpha: 0.5),
            fontSize: 13,
            decoration: TextDecoration.none,
          ),
        ),
      ],
    );
  }

  Widget _buildWaiting() {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        // ── QR code ──
        Container(
          padding: const EdgeInsets.all(20),
          decoration: BoxDecoration(
            color: Colors.white,
            borderRadius: BorderRadius.circular(20),
            boxShadow: [
              BoxShadow(
                color: Colors.black.withValues(alpha: 0.3),
                blurRadius: 20,
                offset: const Offset(0, 8),
              ),
            ],
          ),
          child: QrImageView(
            data: _ticket ?? '',
            version: QrVersions.auto,
            size: 250,
            gapless: true,
            errorStateBuilder: (context, err) {
              return Container(
                width: 250,
                height: 250,
                color: Colors.grey.shade200,
                child: Center(
                  child: Text(
                    'Ticket too long for QR code',
                    style: TextStyle(color: Colors.red.shade700, fontSize: 12),
                    textAlign: TextAlign.center,
                  ),
                ),
              );
            },
          ),
        ),

        const SizedBox(height: 32),

        // ── Status text ──
        Row(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            SizedBox(
              width: 12,
              height: 12,
              child: CircularProgressIndicator(
                strokeWidth: 2,
                color: widget.theme.cursor,
              ),
            ),
            const SizedBox(width: 12),
            Text(
              'Waiting for connection...',
              style: TextStyle(
                color: widget.theme.foreground,
                fontSize: 15,
                decoration: TextDecoration.none,
              ),
            ),
          ],
        ),

        const SizedBox(height: 8),

        Text(
          'Scan this QR code from another GGTerm instance',
          style: TextStyle(
            color: widget.theme.foreground.withValues(alpha: 0.5),
            fontSize: 13,
            decoration: TextDecoration.none,
          ),
        ),

        const SizedBox(height: 32),

        // ── Ticket info (for manual entry fallback) ──
        if (_ticket != null && _ticket!.isNotEmpty)
          ExpansionTile(
            title: Text(
              'Manual ticket (${_ticket!.length} chars)',
              style: TextStyle(
                color: widget.theme.foreground.withValues(alpha: 0.7),
                fontSize: 13,
              ),
            ),
            leading: Icon(
              Icons.info_outline,
              color: widget.theme.foreground.withValues(alpha: 0.5),
              size: 20,
            ),
            childrenPadding: const EdgeInsets.symmetric(horizontal: 16),
            children: [
              Container(
                width: double.infinity,
                padding: const EdgeInsets.all(12),
                decoration: BoxDecoration(
                  color: widget.theme.foreground.withValues(alpha: 0.05),
                  borderRadius: BorderRadius.circular(8),
                ),
                child: SelectableText(
                  _ticket!,
                  style: TextStyle(
                    color: widget.theme.foreground.withValues(alpha: 0.6),
                    fontSize: 11,
                    fontFamily: 'monospace',
                    height: 1.4,
                  ),
                ),
              ),
              const SizedBox(height: 8),
            ],
          ),
      ],
    );
  }

  Widget _buildConnected() {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        Container(
          width: 80,
          height: 80,
          decoration: BoxDecoration(
            color: Colors.green.shade400,
            shape: BoxShape.circle,
          ),
          child: const Icon(
            Icons.check,
            color: Colors.white,
            size: 48,
          ),
        ),
        const SizedBox(height: 24),
        Text(
          'Connected!',
          style: TextStyle(
            color: widget.theme.foreground,
            fontSize: 24,
            fontWeight: FontWeight.bold,
            decoration: TextDecoration.none,
          ),
        ),
        const SizedBox(height: 8),
        Text(
          'A peer has connected to your terminal',
          style: TextStyle(
            color: widget.theme.foreground.withValues(alpha: 0.6),
            fontSize: 14,
            decoration: TextDecoration.none,
          ),
        ),
        const SizedBox(height: 32),
        FilledButton.icon(
          onPressed: () => Navigator.of(context).maybePop(),
          icon: const Icon(Icons.close),
          label: const Text('Close'),
          style: FilledButton.styleFrom(
            minimumSize: const Size(200, 48),
          ),
        ),
      ],
    );
  }

  Widget _buildError() {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        Icon(
          Icons.error_outline,
          size: 64,
          color: Colors.red.shade400,
        ),
        const SizedBox(height: 24),
        Text(
          'Setup Failed',
          style: TextStyle(
            color: widget.theme.foreground,
            fontSize: 20,
            fontWeight: FontWeight.bold,
            decoration: TextDecoration.none,
          ),
        ),
        const SizedBox(height: 12),
        Text(
          _errorMessage ?? 'Unknown error',
          textAlign: TextAlign.center,
          style: TextStyle(
            color: widget.theme.foreground.withValues(alpha: 0.6),
            fontSize: 14,
            height: 1.5,
            decoration: TextDecoration.none,
          ),
        ),
        const SizedBox(height: 32),
        FilledButton.icon(
          onPressed: _regenerate,
          icon: const Icon(Icons.refresh),
          label: const Text('Try Again'),
          style: FilledButton.styleFrom(
            minimumSize: const Size(200, 48),
          ),
        ),
      ],
    );
  }
}
