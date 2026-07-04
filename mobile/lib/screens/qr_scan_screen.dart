/// QR code scanning screen for P2P connections.
///
/// Uses `mobile_scanner` to scan a QR code containing an Iroh NodeTicket.
/// On successful scan, calls [P2pBindings.connect] and navigates to
/// [TerminalScreen] on success.
///
/// Flow:
/// 1. Camera permission → scan QR code
/// 2. Extract ticket string from scanned data
/// 3. Call FFI `ggterm_p2p_connect(ticket)`
/// 4. If session ID > 0, navigate to terminal screen
/// 5. If failed, show error and allow re-scan

import 'dart:async';
import 'package:flutter/material.dart';
import 'package:mobile_scanner/mobile_scanner.dart';

import '../ffi/p2p_bindings.dart';
import '../ffi/session_manager.dart';
import '../terminal_screen.dart';
import '../theme.dart';

class QrScanScreen extends StatefulWidget {
  final P2pBindings p2p;
  final SessionManager sessionManager;
  final TerminalTheme theme;

  const QrScanScreen({
    super.key,
    required this.p2p,
    required this.sessionManager,
    this.theme = darkTheme,
  });

  @override
  State<QrScanScreen> createState() => _QrScanScreenState();
}

class _QrScanScreenState extends State<QrScanScreen> with WidgetsBindingObserver {
  final MobileScannerController _controller = MobileScannerController(
    detectionSpeed: DetectionSpeed.noDuplicates,
    facing: CameraFacing.back,
    torchEnabled: false,
  );

  bool _isConnecting = false;
  String? _lastError;
  String? _scannedTicket;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _controller.dispose();
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    // Pause/resume camera when app goes background/foreground.
    if (_controller.value.isRunning) {
      switch (state) {
        case AppLifecycleState.detached:
        case AppLifecycleState.paused:
        case AppLifecycleState.inactive:
          _controller.stop();
          break;
        case AppLifecycleState.resumed:
          _controller.start();
          break;
        case AppLifecycleState.hidden:
          break;
      }
    }
  }

  void _handleBarcode(BarcodeCapture capture) {
    if (_isConnecting) return;

    final List<Barcode> barcodes = capture.barcodes;
    if (barcodes.isEmpty) return;

    final String? rawValue = barcodes.first.rawValue;
    if (rawValue == null || rawValue.isEmpty) return;

    // Iroh NodeTicket is a base32 string (~130 chars).
    // Accept any non-empty string as a potential ticket.
    _scannedTicket = rawValue;
    _attemptConnection(rawValue);
  }

  Future<void> _attemptConnection(String ticket) async {
    setState(() {
      _isConnecting = true;
      _lastError = null;
    });

    // Stop scanner while connecting.
    await _controller.stop();

    // Call FFI on main isolate (P2P connect should be fast).
    final sessionId = widget.p2p.connect(ticket);

    if (sessionId > 0 && mounted) {
      // Wait for QUIC connection to establish.
      final connected = await _waitForConnection(sessionId, timeoutSeconds: 15);

      if (connected && mounted) {
        Navigator.of(context).pushReplacement(
          MaterialPageRoute(
            builder: (_) => TerminalScreen(
              sessionManager: widget.sessionManager,
              sessionId: sessionId,
              title: 'P2P Session',
              theme: widget.theme,
            ),
          ),
        );
      } else if (mounted) {
        setState(() {
          _isConnecting = false;
          _lastError = 'Connection timed out. The host may be offline.';
        });
        // Resume scanner for retry.
        await _controller.start();
      }
    } else if (mounted) {
      setState(() {
        _isConnecting = false;
        _lastError = 'Invalid ticket or P2P not available.';
      });
      await _controller.start();
    }
  }

  /// Poll [P2pBindings.isConnected] until it returns true or timeout.
  Future<bool> _waitForConnection(int sessionId,
      {int timeoutSeconds = 15}) async {
    final deadline = DateTime.now().add(Duration(seconds: timeoutSeconds));
    while (DateTime.now().isBefore(deadline)) {
      if (widget.p2p.isConnected(sessionId)) {
        return true;
      }
      await Future.delayed(const Duration(milliseconds: 500));
    }
    return false;
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Colors.black,
      appBar: AppBar(
        title: const Text('Scan QR Code'),
        backgroundColor: Colors.black,
        foregroundColor: Colors.white,
        actions: [
          // Torch toggle.
          IconButton(
            icon: ValueListenableBuilder(
              valueListenable: _controller,
              builder: (context, state, _) {
                return Icon(
                  state.torchState == TorchState.on
                      ? Icons.flash_on
                      : Icons.flash_off,
                  color: state.torchState == TorchState.on
                      ? Colors.amber
                      : Colors.white54,
                );
              },
            ),
            onPressed: () => _controller.toggleTorch(),
          ),
          // Camera flip.
          IconButton(
            icon: const Icon(Icons.cameraswitch, color: Colors.white54),
            onPressed: () => _controller.switchCamera(),
          ),
        ],
      ),
      body: Stack(
        children: [
          // ── Camera scanner ──
          if (!_isConnecting)
            MobileScanner(
              controller: _controller,
              onDetect: _handleBarcode,
            ),

          // ── Scan overlay (reticle frame) ──
          if (!_isConnecting)
            _ScanOverlay(
              color: widget.theme.cursor,
            ),

          // ── Connecting indicator ──
          if (_isConnecting)
            Container(
              color: Colors.black87,
              child: Center(
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    const CircularProgressIndicator(
                      color: Colors.blue,
                      strokeWidth: 3,
                    ),
                    const SizedBox(height: 24),
                    Text(
                      'Connecting...',
                      style: TextStyle(
                        color: Colors.white,
                        fontSize: 18,
                        decoration: TextDecoration.none,
                      ),
                    ),
                    const SizedBox(height: 8),
                    if (_scannedTicket != null)
                      Padding(
                        padding: const EdgeInsets.symmetric(horizontal: 32),
                        child: Text(
                          'Ticket: ${_scannedTicket!.substring(0, _scannedTicket!.length > 40 ? 40 : _scannedTicket!.length)}...',
                          style: TextStyle(
                            color: Colors.white54,
                            fontSize: 12,
                            decoration: TextDecoration.none,
                          ),
                          textAlign: TextAlign.center,
                        ),
                      ),
                  ],
                ),
              ),
            ),

          // ── Error message ──
          if (_lastError != null && !_isConnecting)
            Positioned(
              bottom: 80,
              left: 24,
              right: 24,
              child: Container(
                padding: const EdgeInsets.all(16),
                decoration: BoxDecoration(
                  color: Colors.red.shade900.withOpacity(0.9),
                  borderRadius: BorderRadius.circular(12),
                ),
                child: Row(
                  children: [
                    const Icon(Icons.error_outline, color: Colors.white),
                    const SizedBox(width: 12),
                    Expanded(
                      child: Text(
                        _lastError!,
                        style: const TextStyle(color: Colors.white),
                      ),
                    ),
                  ],
                ),
              ),
            ),

          // ── Instructions ──
          if (!_isConnecting && _lastError == null)
            Positioned(
              bottom: 80,
              left: 24,
              right: 24,
              child: Container(
                padding: const EdgeInsets.all(16),
                decoration: BoxDecoration(
                  color: Colors.black54,
                  borderRadius: BorderRadius.circular(12),
                ),
                child: const Row(
                  children: [
                    Icon(Icons.qr_code_scanner, color: Colors.white70),
                    SizedBox(width: 12),
                    Expanded(
                      child: Text(
                        'Point your camera at the GGTerm QR code on the desktop',
                        style: TextStyle(color: Colors.white70, fontSize: 14),
                      ),
                    ),
                  ],
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// Animated reticle overlay for the scanner.
class _ScanOverlay extends StatelessWidget {
  final Color color;

  const _ScanOverlay({required this.color});

  @override
  Widget build(BuildContext context) {
    return ColorFiltered(
      colorFilter: ColorFilter.mode(
        Colors.black.withOpacity(0.4),
        BlendMode.srcOut,
      ),
      child: Stack(
        children: [
          Container(
            decoration: const BoxDecoration(
              color: Colors.black,
              backgroundBlendMode: BlendMode.dstOut,
            ),
          ),
          Center(
            child: Container(
              height: 250,
              width: 250,
              decoration: BoxDecoration(
                color: Colors.red,
                borderRadius: BorderRadius.circular(16),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
