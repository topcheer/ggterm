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

  /// Get the pixel position of a cell index (top-left corner).
  Offset cellPosition(int index) {
    final row = index ~/ cols;
    final col = index % cols;
    return Offset(col * cellWidth, row * cellHeight);
  }

  /// Get the center-bottom of a cell (where handles sit).
  Offset cellHandlePosition(int index) {
    final pos = cellPosition(index);
    return Offset(pos.dx + cellWidth / 2, pos.dy + cellHeight);
  }

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

/// A draggable selection handle widget — a small teardrop shape
/// positioned at the start or end of the selection.
class SelectionHandle extends StatelessWidget {
  final Offset position;
  final bool isStart;
  final ValueChanged<Offset> onDrag;

  const SelectionHandle({
    super.key,
    required this.position,
    required this.isStart,
    required this.onDrag,
  });

  @override
  Widget build(BuildContext context) {
    return Positioned(
      left: position.dx - 7,
      top: position.dy - 2,
      child: GestureDetector(
        onPanUpdate: (details) {
          onDrag(Offset(
            position.dx + details.delta.dx,
            position.dy + details.delta.dy,
          ));
        },
        child: Container(
          width: 14,
          height: 14,
          decoration: BoxDecoration(
            color: Colors.blue,
            shape: BoxShape.circle,
            border: Border.all(color: Colors.white, width: 2),
            boxShadow: [
              BoxShadow(
                color: Colors.black.withValues(alpha: 0.3),
                blurRadius: 4,
                offset: const Offset(0, 1),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
