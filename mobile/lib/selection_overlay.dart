import 'package:flutter/material.dart';

/// Paints text selection highlight on the terminal canvas.
/// Used by terminal_screen.dart for drag-to-select functionality.
class TerminalSelectionOverlay extends CustomPainter {
  final int selStart;
  final int selEnd;
  final int cols;
  final int rows;
  final double cellWidth;
  final double cellHeight;

  TerminalSelectionOverlay({
    required this.selStart,
    required this.selEnd,
    required this.cols,
    required this.rows,
    required this.cellWidth,
    required this.cellHeight,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final lo = selStart < selEnd ? selStart : selEnd;
    final hi = selStart < selEnd ? selEnd : lo;
    final paint = Paint()..color = Colors.blue.withValues(alpha: 0.25);

    final startRow = lo ~/ cols;
    final startCol = lo % cols;
    final endRow = hi ~/ cols;
    final endCol = hi % cols;

    if (startRow == endRow) {
      canvas.drawRect(
        Rect.fromLTWH(startCol * cellWidth, startRow * cellHeight,
            (endCol - startCol + 1) * cellWidth, cellHeight),
        paint,
      );
    } else {
      // First row from startCol to end of line.
      canvas.drawRect(
        Rect.fromLTWH(startCol * cellWidth, startRow * cellHeight,
            (cols - startCol) * cellWidth, cellHeight),
        paint,
      );
      // Full rows in between.
      for (var r = startRow + 1; r < endRow; r++) {
        canvas.drawRect(
          Rect.fromLTWH(0, r * cellHeight, cols * cellWidth, cellHeight),
          paint,
        );
      }
      // Last row from start to endCol.
      canvas.drawRect(
        Rect.fromLTWH(0, endRow * cellHeight,
            (endCol + 1) * cellWidth, cellHeight),
        paint,
      );
    }
  }

  @override
  bool shouldRepaint(covariant TerminalSelectionOverlay old) =>
      selStart != old.selStart || selEnd != old.selEnd;
}
