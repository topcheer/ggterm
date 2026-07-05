/// QR code scanning screen for P2P connections.
///
/// Uses `mobile_scanner` to scan a QR code containing an Iroh NodeTicket.
/// On successful scan, calls [P2pBindings.connect] and navigates to
/// [TerminalScreen] on success.

library;
import 'dart:async';
import 'package:flutter/material.dart';
import 'package:mobile_scanner/mobile_scanner.dart';
import 'package:permission_handler/permission_handler.dart';

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
  bool _permissionGranted = false;
  String? _lastError;

  @override
  void initState() {
    super.initState();
    debugPrint('[QR] initState');
    WidgetsBinding.instance.addObserver(this);
    _checkPermission();
  }

  Future<void> _checkPermission() async {
    var status = await Permission.camera.status;
    debugPrint('[QR] permission status: $status');

    if (!status.isGranted) {
      debugPrint('[QR] requesting permission...');
      status = await Permission.camera.request();
      debugPrint('[QR] request result: $status');
    }

    if (mounted) {
      setState(() {
        _permissionGranted = status.isGranted;
        if (!status.isGranted) {
          _lastError = 'Camera permission denied. Grant in Settings > Apps > GGTerm > Permissions.';
        }
      });
    }
  }

  @override
  void dispose() {
    debugPrint('[QR] dispose');
    WidgetsBinding.instance.removeObserver(this);
    _controller.dispose();
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    debugPrint('[QR] lifecycle: $state');
  }

  void _handleBarcode(BarcodeCapture capture) {
    if (_isConnecting) return;

    final List<Barcode> barcodes = capture.barcodes;
    if (barcodes.isEmpty) return;

    final String? rawValue = barcodes.first.rawValue;
    if (rawValue == null || rawValue.isEmpty) return;

    debugPrint('[QR] barcode: ${rawValue.substring(0, rawValue.length > 30 ? 30 : null)}...');
    _attemptConnection(rawValue);
  }

  Future<void> _attemptConnection(String ticket) async {
    debugPrint('[QR] attempt connect, len=${ticket.length}');
    setState(() {
      _isConnecting = true;
      _lastError = null;
    });

    // Stop camera while connecting.
    await _controller.stop();

    debugPrint('[QR] calling p2p.connect (non-blocking)...');
    final sessionId = widget.p2p.connect(ticket);
    debugPrint('[QR] sessionId=$sessionId (connect started in background)');

    if (sessionId == 0) {
      final err = widget.p2p.lastError();
      debugPrint('[QR] immediate failure: $err');
      if (mounted) {
        setState(() {
          _isConnecting = false;
          _lastError = 'Failed to start: ${err ?? "unknown"}';
        });
      }
      return;
    }

    // Poll connect status for up to 60 seconds (non-blocking connect).
    debugPrint('[QR] polling connect status...');
    bool connected = false;
    for (int i = 0; i < 120; i++) {
      await Future.delayed(const Duration(milliseconds: 500));
      if (!mounted) return;

      final status = widget.p2p.connectStatus(sessionId);
      if (i % 10 == 0) debugPrint('[QR] poll #$i: status=$status');

      if (status == 1) {
        connected = true;
        break;
      }
      if (status == -1) {
        final err = widget.p2p.lastError();
        debugPrint('[QR] connect failed: $err');
        if (mounted) {
          setState(() {
            _isConnecting = false;
            _lastError = 'Connection failed: ${err ?? "unknown"}';
          });
          // Restart camera so user can scan again.
          _controller.start();
        }
        return;
      }
    }

    debugPrint('[QR] final: connected=$connected');

    if (connected && mounted) {
      debugPrint('[QR] navigating to terminal');
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
        _lastError = 'Connection timed out (30s). Host may be offline.';
      });
      // Restart camera so user can scan again.
      _controller.start();
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Colors.black,
      appBar: AppBar(
        title: const Text('Scan QR Code'),
        backgroundColor: Colors.black,
        foregroundColor: Colors.white,
        leading: IconButton(
          icon: const Icon(Icons.close),
          onPressed: () => Navigator.of(context).pop(),
        ),
        actions: [
          ValueListenableBuilder(
            valueListenable: _controller,
            builder: (context, MobileScannerState state, _) {
              if (state.torchState == TorchState.off) {
                return const SizedBox.shrink();
              }
              return IconButton(
                icon: Icon(
                  state.torchState == TorchState.on ? Icons.flash_on : Icons.flash_off,
                  color: state.torchState == TorchState.on ? Colors.amber : Colors.white54,
                ),
                onPressed: () => _controller.toggleTorch(),
              );
            },
          ),
        ],
      ),
      body: Stack(
        children: [
          // MobileScanner is ALWAYS in the tree — it manages its own lifecycle.
          // We overlay other widgets on top instead of conditionally hiding it.
          if (_permissionGranted && !_isConnecting)
            MobileScanner(
              controller: _controller,
              onDetect: _handleBarcode,
            ),

          // Scan reticle overlay.
          if (_permissionGranted && !_isConnecting)
            const _ScanOverlay(),

          // Permission not granted / requesting state.
          if (!_permissionGranted)
            Center(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Icon(
                    _lastError != null ? Icons.camera_alt_outlined : Icons.hourglass_empty,
                    size: 64,
                    color: Colors.white38,
                  ),
                  const SizedBox(height: 16),
                  Text(
                    _lastError ?? 'Requesting camera permission...',
                    textAlign: TextAlign.center,
                    style: const TextStyle(color: Colors.white54, fontSize: 16),
                  ),
                  if (_lastError != null) ...[
                    const SizedBox(height: 24),
                    ElevatedButton.icon(
                      onPressed: () {
                        setState(() {
                          _lastError = null;
                          _permissionGranted = false;
                        });
                        _checkPermission();
                      },
                      icon: const Icon(Icons.refresh),
                      label: const Text('Retry'),
                    ),
                  ],
                ],
              ),
            ),

          // Connecting overlay.
          if (_isConnecting)
            Container(
              color: Colors.black87,
              child: const Center(
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    CircularProgressIndicator(color: Colors.blue, strokeWidth: 3),
                    SizedBox(height: 24),
                    Text('Connecting...', style: TextStyle(color: Colors.white, fontSize: 18)),
                  ],
                ),
              ),
            ),

          // Instructions at bottom.
          if (_permissionGranted && !_isConnecting && _lastError == null)
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

          // Error toast.
          if (_lastError != null && !_isConnecting && _permissionGranted)
            Positioned(
              bottom: 80,
              left: 24,
              right: 24,
              child: Container(
                padding: const EdgeInsets.all(16),
                decoration: BoxDecoration(
                  color: Colors.red.shade900.withValues(alpha: 0.9),
                  borderRadius: BorderRadius.circular(12),
                ),
                child: Row(
                  children: [
                    const Icon(Icons.error_outline, color: Colors.white),
                    const SizedBox(width: 12),
                    Expanded(child: Text(_lastError!, style: const TextStyle(color: Colors.white))),
                  ],
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// Reticle overlay for the scanner.
class _ScanOverlay extends StatelessWidget {
  const _ScanOverlay();

  @override
  Widget build(BuildContext context) {
    return ColorFiltered(
      colorFilter: ColorFilter.mode(
        Colors.black.withValues(alpha: 0.4),
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
