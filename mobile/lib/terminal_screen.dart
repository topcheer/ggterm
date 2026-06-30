/// Main terminal screen with custom Canvas-based cell renderer.
///
/// Receives [SessionManager] (from flutter_rust_bridge) and renders
/// terminal cells via [CustomPaint] + [_TerminalPainter].
/// Touch gestures: pan to scroll scrollback, pinch to zoom font.

import 'dart:math' as math;
import 'package:flutter/material.dart';
import 'package:flutter/gestures.dart';

import 'theme.dart';

// ── Data types bridged from Rust ──────────────────────────────────────

/// A single terminal cell with character + color info.
class TerminalCell {
  final String char;
  final int fgIndex; // ANSI 0-15, or -1 for default fg
  final int bgIndex; // ANSI 0-15, or -1 for default bg
  final bool bold;
  final bool italic;
  final bool underline;

  const TerminalCell({
    this.char = ' ',
    this.fgIndex = -1,
    this.bgIndex = -1,
    this.bold = false,
    this.italic = false,
    this.underline = false,
  });
}

/// Snapshot of the terminal screen for rendering.
class ScreenData {
  final List<TerminalCell> cells; // flat array, row-major
  final int cursorCol;
  final int cursorRow;
  final int cols;
  final int rows;
  final int scrollOffset; // scrollback offset (0 = bottom/latest)

  const ScreenData({
    required this.cells,
    this.cursorCol = 0,
    this.cursorRow = 0,
    this.cols = 80,
    this.rows = 24,
    this.scrollOffset = 0,
  });
}

// ── TerminalScreen widget ─────────────────────────────────────────────

/// Placeholder for the Rust-bridged session manager.
typedef SessionManager = dynamic;

class TerminalScreen extends StatefulWidget {
  final SessionManager session;
  final TerminalTheme theme;

  const TerminalScreen({
    super.key,
    required this.session,
    this.theme = darkTheme,
  });

  @override
  State<TerminalScreen> createState() => _TerminalScreenState();
}

class _TerminalScreenState extends State<TerminalScreen> {
  double _fontSize = 14.0;
  double _scrollOffset = 0.0;
  ScreenData _screen = const ScreenData(cells: []);

  // Cell dimensions derived from font size (monospace ratio ~0.6).
  double get _cellWidth => _fontSize * 0.6;
  double get _cellHeight => _fontSize * 1.2;

  @override
  void initState() {
    super.initState();
    _updateScreen();
  }

  void _updateScreen() {
    // TODO: Poll ScreenData from SessionManager via flutter_rust_bridge.
    // For now, generate a placeholder grid.
    final cols = 80;
    final rows = 24;
    final cells = List<TerminalCell>.filled(
      cols * rows,
      const TerminalCell(char: ' '),
    );
    // Write a welcome message on the first line.
    const welcome = 'GGTerm Mobile — Ready';
    for (var i = 0; i < welcome.length && i < cols; i++) {
      cells[i] = TerminalCell(char: welcome[i], fgIndex: 14); // bright cyan
    }
    setState(() {
      _screen = ScreenData(
        cells: cells,
        cols: cols,
        rows: rows,
        cursorCol: welcome.length,
        cursorRow: 0,
      );
    });
  }

  // ── Gesture handlers ──

  void _onScale(ScaleUpdateDetails details) {
    // Pinch to zoom font.
    if ((details.scale - 1.0).abs() > 0.01) {
      setState(() {
        _fontSize = (_fontSize * details.scale).clamp(8.0, 32.0);
      });
    }
  }

  void _onPan(DragUpdateDetails details) {
    // Pan to scroll scrollback.
    setState(() {
      _scrollOffset = (_scrollOffset - details.delta.dy)
          .clamp(0.0, _screen.rows * _cellHeight);
    });
  }

  void _onTapUp(TapUpDetails details, BoxConstraints constraints) {
    // Convert tap position to cell coordinates.
    final col = (details.localPosition.dx / _cellWidth).floor();
    final row = (details.localPosition.dy / _cellHeight).floor();
    debugPrint('Tap at col=$col row=$row');
    // TODO: Forward tap to session for cursor positioning or mouse events.
  }

  @override
  Widget build(BuildContext context) {
    final theme = widget.theme;

    return Scaffold(
      backgroundColor: theme.background,
      body: SafeArea(
        child: Column(
          children: [
            // ── Terminal canvas ──
            Expanded(
              child: LayoutBuilder(
                builder: (context, constraints) {
                  return GestureDetector(
                    onScaleUpdate: _onScale,
                    onVerticalDragUpdate: _onPan,
                    onTapUp: (details) =>
                        _onTapUp(details, constraints),
                    child: CustomPaint(
                      painter: _TerminalPainter(
                        screen: _screen,
                        theme: theme,
                        cellWidth: _cellWidth,
                        cellHeight: _cellHeight,
                        scrollPixelOffset: _scrollOffset,
                      ),
                      child: Container(),
                    ),
                  );
                },
              ),
            ),
          ],
        ),
      ),
    );
  }
}

// ── Custom painter ────────────────────────────────────────────────────

class _TerminalPainter extends CustomPainter {
  final ScreenData screen;
  final TerminalTheme theme;
  final double cellWidth;
  final double cellHeight;
  final double scrollPixelOffset;

  _TerminalPainter({
    required this.screen,
    required this.theme,
    required this.cellWidth,
    required this.cellHeight,
    this.scrollPixelOffset = 0.0,
  });

  @override
  void paint(Canvas canvas, Size size) {
    // Fill background.
    final bgPaint = Paint()..color = theme.background;
    canvas.drawRect(Offset.zero & size, bgPaint);

    final fgDefault = theme.foreground;
    final bgDefault = theme.background;

    // Clip to visible area.
    canvas.clipRect(Offset.zero & size);

    final cols = screen.cols;
    final rows = screen.rows;
    final maxVisibleRows =
        math.min(rows, (size.height / cellHeight).floor());

    for (var row = 0; row < maxVisibleRows; row++) {
      final y = row * cellHeight;

      for (var col = 0; col < cols; col++) {
        final x = col * cellWidth;
        final idx = row * cols + col;
        if (idx >= screen.cells.length) break;

        final cell = screen.cells[idx];

        // Resolve background.
        final cellBg = cell.bgIndex >= 0 && cell.bgIndex < theme.palette.length
            ? theme.palette[cell.bgIndex]
            : bgDefault;

        // Draw cell background.
        final bgRect = Rect.fromLTWH(x, y, cellWidth, cellHeight);
        canvas.drawRect(bgRect, Paint()..color = cellBg);

        // Resolve foreground.
        final cellFg = cell.fgIndex >= 0 && cell.fgIndex < theme.palette.length
            ? theme.palette[cell.fgIndex]
            : fgDefault;

        // Draw character.
        if (cell.char.isNotEmpty && cell.char != ' ') {
          final textStyle = TextStyle(
            color: cellFg,
            fontSize: cellHeight * 0.85,
            fontFamily: 'monospace',
            fontWeight: cell.bold ? FontWeight.bold : FontWeight.normal,
            fontStyle: cell.italic ? FontStyle.italic : FontStyle.normal,
            decoration:
                cell.underline ? TextDecoration.underline : TextDecoration.none,
          );

          final tp = TextPainter(
            text: TextSpan(text: cell.char, style: textStyle),
            textDirection: TextDirection.ltr,
          )..layout();

          // Center character in cell.
          final dx = x + (cellWidth - tp.width) / 2;
          final dy = y + (cellHeight - tp.height) / 2;
          tp.paint(canvas, Offset(dx, dy));
        }

        // Draw cursor (block style).
        if (col == screen.cursorCol && row == screen.cursorRow) {
          final cursorPaint = Paint()
            ..color = theme.cursor.withOpacity(0.6)
            ..blendMode = BlendMode.srcOver;
          canvas.drawRect(bgRect, cursorPaint);
        }
      }
    }
  }

  @override
  bool shouldRepaint(covariant _TerminalPainter old) {
    return old.screen != screen ||
        old.cellWidth != cellWidth ||
        old.cellHeight != cellHeight ||
        old.scrollPixelOffset != scrollPixelOffset;
  }
}
