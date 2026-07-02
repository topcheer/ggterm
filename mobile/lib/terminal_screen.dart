/// Main terminal screen with FFI-backed Canvas renderer.
///
/// Receives [SessionManager] and session ID from the connection flow.
/// A timer loop pumps transport data and flushes input at ~30fps,
/// then repaints the terminal via [CustomPaint].
///
/// Touch gestures: pan to scroll scrollback, pinch to zoom font,
/// tap to position cursor.

import 'dart:async';
import 'dart:math' as math;
import 'package:flutter/material.dart';
import 'package:flutter/gestures.dart';

import 'ffi/session_manager.dart';
import 'keyboard_bar.dart';
import 'theme.dart';

class TerminalScreen extends StatefulWidget {
  final SessionManager sessionManager;
  final int sessionId;
  final String title;
  final TerminalTheme theme;

  const TerminalScreen({
    super.key,
    required this.sessionManager,
    required this.sessionId,
    this.title = 'Terminal',
    this.theme = darkTheme,
  });

  @override
  State<TerminalScreen> createState() => _TerminalScreenState();
}

class _TerminalScreenState extends State<TerminalScreen> {
  double _fontSize = 13.0;
  ScreenSnapshot _screen = ScreenSnapshot.empty;
  final _modifiers = ModifierState();
  bool _showKeyboardBar = true;
  Timer? _renderTimer;
  bool _transportAlive = true;

  // Cell dimensions derived from font size (monospace ratio ~0.6).
  double get _cellWidth => _fontSize * 0.6;
  double get _cellHeight => _fontSize * 1.3;

  @override
  void initState() {
    super.initState();
    _startRenderLoop();
  }

  void _startRenderLoop() {
    _renderTimer?.cancel();
    // ~30fps pump + render cycle
    _renderTimer = Timer.periodic(const Duration(milliseconds: 33), (_) {
      if (!mounted) return;
      final mgr = widget.sessionManager;
      final id = widget.sessionId;

      // Pump transport data
      mgr.pumpAndFlush(id);

      // Check alive
      final alive = mgr.isAlive(id);
      if (alive != _transportAlive) {
        _transportAlive = alive;
      }

      // Get screen snapshot
      final snapshot = mgr.getScreenSnapshot(id);
      setState(() {
        _screen = snapshot;
      });
    });
  }

  @override
  void dispose() {
    _renderTimer?.cancel();
    super.dispose();
  }

  void _sendKey(String keyName) {
    final codes = _keyNameToBytes(keyName);
    widget.sessionManager.sendInput(widget.sessionId, codes);
  }

  void _sendChar(String char) {
    if (char.isEmpty) return;
    // Apply modifier prefix
    final prefix = _modifiers.prefix;
    final codes = <int>[];

    if (prefix.contains('Ctrl')) {
      // Ctrl+letter → control character
      final c = char.toLowerCase().codeUnitAt(0);
      if (c >= 0x61 && c <= 0x7A) {
        codes.add(c - 0x60); // a=1, b=2, ...
      }
    } else if (prefix.contains('Alt')) {
      codes.add(0x1B); // ESC prefix
      codes.addAll(char.codeUnits);
    } else {
      codes.addAll(char.codeUnits);
    }

    widget.sessionManager.sendInput(widget.sessionId, codes);
    _modifiers.releaseAll();
  }

  /// Convert special key names to terminal escape sequences.
  List<int> _keyNameToBytes(String name) {
    switch (name) {
      case 'Enter':
        return [0x0D];
      case 'Tab':
        return [0x09];
      case 'Escape':
        return [0x1B];
      case 'Backspace':
        return [0x7F];
      case 'CtrlC':
        return [0x03]; // SIGINT
      case 'CtrlD':
        return [0x04]; // EOF
      case 'CtrlZ':
        return [0x1A]; // SIGTSTP
      case 'Up':
        return [0x1B, 0x5B, 0x41]; // ESC [ A
      case 'Down':
        return [0x1B, 0x5B, 0x42];
      case 'Right':
        return [0x1B, 0x5B, 0x43];
      case 'Left':
        return [0x1B, 0x5B, 0x44];
      case 'Home':
        return [0x1B, 0x5B, 0x48]; // ESC [ H
      case 'End':
        return [0x1B, 0x5B, 0x46]; // ESC [ F
      case 'PageUp':
        return [0x1B, 0x5B, 0x35, 0x7E]; // ESC [ 5 ~
      case 'PageDown':
        return [0x1B, 0x5B, 0x36, 0x7E]; // ESC [ 6 ~
      default:
        return name.codeUnits;
    }
  }

  void _onScale(ScaleUpdateDetails details) {
    if ((details.scale - 1.0).abs() > 0.01) {
      setState(() {
        _fontSize = (_fontSize * details.scale).clamp(8.0, 32.0);
      });
    }
  }

  void _onTapUp(TapUpDetails details) {
    final col = (details.localPosition.dx / _cellWidth).floor();
    final row = (details.localPosition.dy / _cellHeight).floor();
    debugPrint('Tap at col=$col row=$row');
    // TODO: Forward tap for cursor positioning or mouse events
  }

  @override
  Widget build(BuildContext context) {
    final theme = widget.theme;

    return Scaffold(
      backgroundColor: theme.background,
      appBar: AppBar(
        title: Text(widget.title),
        backgroundColor: Colors.grey.shade900,
        foregroundColor: Colors.white,
        actions: [
          // Transport status indicator
          Padding(
            padding: const EdgeInsets.only(right: 12),
            child: Center(
              child: Container(
                width: 10,
                height: 10,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  color: _transportAlive ? Colors.green : Colors.red,
                ),
              ),
            ),
          ),
          // Toggle keyboard bar
          IconButton(
            icon: Icon(_showKeyboardBar ? Icons.keyboard_hide : Icons.keyboard),
            onPressed: () => setState(() => _showKeyboardBar = !_showKeyboardBar),
          ),
        ],
      ),
      body: SafeArea(
        child: Column(
          children: [
            // ── Terminal canvas ──
            Expanded(
              child: LayoutBuilder(
                builder: (context, constraints) {
                  // Compute grid dimensions from available space
                  final cols = (constraints.maxWidth / _cellWidth).floor().clamp(10, 300);
                  final rows = ((constraints.maxHeight - (_showKeyboardBar ? 44 : 0)) / _cellHeight)
                      .floor()
                      .clamp(3, 100);

                  // Resize terminal if dimensions changed
                  WidgetsBinding.instance.addPostFrameCallback((_) {
                    if (_screen.cols != cols || _screen.rows != rows) {
                      widget.sessionManager.resize(widget.sessionId, cols, rows);
                    }
                  });

                  return GestureDetector(
                    onScaleUpdate: _onScale,
                    onTapUp: (details) => _onTapUp(details, constraints),
                    child: CustomPaint(
                      painter: _TerminalPainter(
                        screen: _screen,
                        theme: theme,
                        cellWidth: _cellWidth,
                        cellHeight: _cellHeight,
                      ),
                      child: Container(),
                    ),
                  );
                },
              ),
            ),

            // ── Keyboard bar ──
            if (_showKeyboardBar)
              KeyboardBar(
                modifiers: _modifiers,
                onKey: _sendKey,
              ),
          ],
        ),
      ),
    );
  }
}

// ── Custom painter with real FFI data ────────────────────────────────

class _TerminalPainter extends CustomPainter {
  final ScreenSnapshot screen;
  final TerminalTheme theme;
  final double cellWidth;
  final double cellHeight;

  _TerminalPainter({
    required this.screen,
    required this.theme,
    required this.cellWidth,
    required this.cellHeight,
  });

  @override
  void paint(Canvas canvas, Size size) {
    // Fill background.
    final bgPaint = Paint()..color = theme.background;
    canvas.drawRect(Offset.zero & size, bgPaint);

    if (screen.cells.isEmpty) return;

    canvas.clipRect(Offset.zero & size);

    final cols = screen.cols;
    final rows = screen.rows;
    final maxVisibleRows = math.min(rows, (size.height / cellHeight).floor());

    for (var row = 0; row < maxVisibleRows; row++) {
      final y = row * cellHeight;

      for (var col = 0; col < cols; col++) {
        final x = col * cellWidth;
        final idx = row * cols + col;
        if (idx >= screen.cells.length) break;

        final cell = screen.cells[idx];

        // Resolve background color.
        final cellBg = Color(0xFF000000 | cell.bgRgb);

        // Draw cell background.
        final bgRect = Rect.fromLTWH(x, y, cellWidth, cellHeight);
        canvas.drawRect(bgRect, Paint()..color = cellBg);

        // Resolve foreground color.
        final cellFg = Color(0xFF000000 | cell.fgRgb);

        // Draw character.
        if (cell.charCode != 0) {
          final textStyle = TextStyle(
            color: cellFg,
            fontSize: cellHeight * 0.85,
            fontFamily: 'monospace',
            fontWeight: cell.bold ? FontWeight.bold : FontWeight.normal,
            fontStyle: cell.italic ? FontStyle.italic : FontStyle.normal,
            decoration: cell.underline
                ? TextDecoration.underline
                : cell.strikethrough
                    ? TextDecoration.lineThrough
                    : TextDecoration.none,
          );

          final tp = TextPainter(
            text: TextSpan(text: cell.char, style: textStyle),
            textDirection: TextDirection.ltr,
          )..layout();

          final dx = x + (cellWidth - tp.width) / 2;
          final dy = y + (cellHeight - tp.height) / 2;
          tp.paint(canvas, Offset(dx, dy));
        }

        // Draw cursor (block style).
        if (col == screen.cursorCol && row == screen.cursorRow) {
          final cursorPaint = Paint()
            ..color = theme.cursor.withOpacity(0.5)
            ..blendMode = BlendMode.srcOver;
          canvas.drawRect(bgRect, cursorPaint);
        }
      }
    }
  }

  @override
  bool shouldRepaint(covariant _TerminalPainter old) {
    return true; // Always repaint — FFI data may have changed
  }
}
