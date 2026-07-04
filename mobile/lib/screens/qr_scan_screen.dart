/// QR code scanning screen for P2P connections.
///
/// Uses `mobile_scanner` to scan a QR code containing an Iroh NodeTicket.
/// On successful scan, calls [P2pBindings.connect] and navigates to
/// [TerminalScreen] on success.

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
  late MobileScannerController _controller;
  bool _isConnecting = false;
  bool _cameraStarted = false;
  String? _lastError;
  int _retryCount = 0;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _controller = MobileScannerController(
      detectionSpeed: DetectionSpeed.noDuplicates,
      facing: CameraFacing.back,
      torchEnabled: false,
    );
    // Start camera after first frame to ensure the widget is fully mounted
    // and the Android activity is ready to handle permission requests.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _startCamera();
    });
  }

  Future<void> _startCamera() async {
    if (_cameraStarted || _isConnecting) return;

    try {
      // mobile_scanner v5: start() internally requests camera permission
      // on Android. If permission is denied, it throws. We catch and
      // show a retry UI.
      await _controller.start();

      // Small delay to let the camera surface attach on some devices.
      await Future.delayed(const Duration(milliseconds: 300));

      if (mounted) {
        setState(() {
          _cameraStarted = true;
          _lastError = null;
          _retryCount = 0;
        });
      }
    } on MobileScannerException catch (e) {
      if (mounted) {
        setState(() {
          _lastError = e.errorCode == MobileScannerErrorCode.permissionDenied
              ? 'Camera permission denied. Please grant camera access in Settings.'
              : 'Camera error: ${e.errorCode.name}';
        });
      }
    } catch (e) {
      if (mounted) {
        setState(() {
          _lastError = 'Failed to start camera: $e';
        });
      }
    }
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _controller.dispose();
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    // Resume camera when app returns to foreground.
    if (state == AppLifecycleState.resumed && !_isConnecting) {
      _startCamera();
    } else if (state == AppLifecycleState.inactive && _cameraStarted) {
      _controller.stop();
      _cameraStarted = false;
    }
  }

  void _handleBarcode(BarcodeCapture capture) {
    if (_isConnecting) return;

    final List<Barcode> barcodes = capture.barcodes;
    if (barcodes.isEmpty) return;

    final String? rawValue = barcodes.first.rawValue;
    if (rawValue == null || rawValue.isEmpty) return;

    _attemptConnection(rawValue);
  }

  Future<void> _attemptConnection(String ticket) async {
    setState(() {
      _isConnecting = true;
      _lastError = null;
    });

    await _controller.stop();
    _cameraStarted = false;

    final sessionId = widget.p2p.connect(ticket);

    if (sessionId > 0 && mounted) {
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
        _startCamera();
      }
    } else if (mounted) {
      setState(() {
        _isConnecting = false;
        _lastError = 'Invalid ticket or P2P not available.';
      });
      _startCamera();
    }
  }

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

  /// Retry camera start — used by the Retry button.
  Future<void> _retryCamera() async {
    setState(() {
      _lastError = null;
      _retryCount++;
    });
    // Recreate controller on retry to reset internal state.
    _controller.dispose();
    _controller = MobileScannerController(
      detectionSpeed: DetectionSpeed.noDuplicates,
      facing: CameraFacing.back,
      torchEnabled: false,
    );
    _cameraStarted = false;
    await _startCamera();
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
          // Torch toggle — only show when camera is active.
          if (_cameraStarted)
            ValueListenableBuilder(
              valueListenable: _controller,
              builder: (context, MobileScannerState state, _) {
                return IconButton(
                  icon: Icon(
                    state.torchState == TorchState.on
                        ? Icons.flash_on
                        : Icons.flash_off,
                    color: state.torchState == TorchState.on
                        ? Colors.amber
                        : Colors.white54,
                  ),
                  onPressed: () => _controller.toggleTorch(),
                );
              },
            ),
        ],
      ),
      body: Stack(
        children: [
          // Camera scanner — only show when camera is active.
          if (!_isConnecting && _cameraStarted)
            MobileScanner(
              controller: _controller,
              onDetect: _handleBarcode,
            ),

          // Scan overlay reticle.
          if (!_isConnecting && _cameraStarted)
            _ScanOverlay(color: widget.theme.cursor),

          // Camera not started / error state.
          if (!_isConnecting && !_cameraStarted)
            _CameraStateView(
              error: _lastError,
              retryCount: _retryCount,
              onRetry: _retryCamera,
            ),

          // Connecting indicator.
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
                  ],
                ),
              ),
            ),

          // Error toast (non-fatal, e.g. connection timeout).
          if (_lastError != null && !_isConnecting && _cameraStarted)
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

          // Instructions.
          if (!_isConnecting && _lastError == null && _cameraStarted)
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

/// Shows camera starting/permission-denied state with a Retry button.
class _CameraStateView extends StatelessWidget {
  final String? error;
  final int retryCount;
  final VoidCallback onRetry;

  const _CameraStateView({
    this.error,
    required this.retryCount,
    required this.onRetry,
  });

  @override
  Widget build(BuildContext context) {
    final hasError = error != null;
    return Center(
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 32),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              hasError ? Icons.camera_alt_outlined : Icons.hourglass_empty,
              size: 64,
              color: Colors.white38,
            ),
            const SizedBox(height: 16),
            Text(
              error ?? 'Starting camera...',
              textAlign: TextAlign.center,
              style: TextStyle(
                color: Colors.white54,
                fontSize: 16,
                decoration: TextDecoration.none,
              ),
            ),
            if (hasError) ...[
              const SizedBox(height: 24),
              ElevatedButton.icon(
                onPressed: onRetry,
                icon: const Icon(Icons.refresh),
                label: Text(retryCount > 0 ? 'Retry ($retryCount)' : 'Retry'),
              ),
              const SizedBox(height: 16),
              Text(
                'If camera still fails, check:\n'
                'Settings > Apps > GGTerm > Permissions > Camera',
                textAlign: TextAlign.center,
                style: TextStyle(
                  color: Colors.white30,
                  fontSize: 13,
                  decoration: TextDecoration.none,
                ),
              ),
            ],
          ],
        ),
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
