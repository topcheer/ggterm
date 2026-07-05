/// Main terminal screen with FFI-backed Canvas renderer.
///
/// Receives [SessionManager] and session ID from the connection flow.
/// A timer loop pumps transport data and flushes input at ~60fps,
/// providing near-instant echo from the remote terminal.
/// then repaints the terminal via [CustomPaint].
///
/// Touch gestures: pan to scroll scrollback, pinch to zoom font,
/// tap to position cursor.

library;
import 'dart:async';
import 'dart:math' as math;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

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
  bool _showInputBar = false;
  Timer? _renderTimer;
  bool _transportAlive = true;
  bool _sizeInitialized = false;
  // Frame hash for change detection — avoids setState when nothing changed.
  int _lastFrameHash = 0;
  // Cursor blink state.
  bool _cursorVisible = true;
  Timer? _blinkTimer;

  // Visible input bar for typing.
  final FocusNode _inputFocusNode = FocusNode();
  final TextEditingController _inputController = TextEditingController();
  String _lastInputText = '';

  // Cell dimensions derived from font size (monospace ratio ~0.6).
  double get _cellWidth => _fontSize * 0.6;
  double get _cellHeight => _fontSize * 1.3;

  @override
  void initState() {
    super.initState();
    _startRenderLoop();
    _startCursorBlink();
  }

  void _startCursorBlink() {
    _blinkTimer?.cancel();
    // Blink cursor at ~1Hz (530ms on, 530ms off — standard terminal rate).
    _blinkTimer = Timer.periodic(const Duration(milliseconds: 530), (_) {
      if (!mounted || !_transportAlive) return;
      _cursorVisible = !_cursorVisible;
      // Only trigger repaint, not full setState — no layout change needed.
      setState(() {});
    });
  }

  void _startRenderLoop() {
    _renderTimer?.cancel();
    // ~60fps pump + render cycle for low-latency echo
    _renderTimer = Timer.periodic(const Duration(milliseconds: 16), (_) {
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

      // Only rebuild if content changed (cursor blink handled separately).
      final hash = _computeFrameHash(snapshot);
      if (hash != _lastFrameHash || alive != _transportAlive) {
        _lastFrameHash = hash;
        setState(() {
          _screen = snapshot;
        });
      }
    });
  }

  /// Fast hash of visible screen content for change detection.
  /// Compares cells + cursor position — skips when nothing changed
  /// to avoid 60fps setState storms on idle terminals.
  int _computeFrameHash(ScreenSnapshot snap) {
    var h = snap.cursorCol ^ (snap.cursorRow << 16);
    for (var i = 0; i < snap.cells.length; i++) {
      final c = snap.cells[i];
      h = (h * 31 + c.charCode ^ (c.flags << 8) ^ c.fg ^ (c.bg << 16)) & 0x7FFFFFFF;
    }
    return h;
  }

  @override
  void dispose() {
    _renderTimer?.cancel();
    _blinkTimer?.cancel();
    _inputFocusNode.dispose();
    _inputController.dispose();
    super.dispose();
  }

  void _sendKey(String keyName) {
    final codes = _keyNameToBytes(keyName);
    widget.sessionManager.sendInput(widget.sessionId, codes);
  }

  /// Handle text input from the hidden TextField.
  /// Computes the delta between last and current text, then sends it.
  void _onInputChanged(String newText) {
    final oldText = _lastInputText;

    // Detect backspace (text got shorter).
    if (newText.length < oldText.length) {
      final deletedCount = oldText.length - newText.length;
      for (var i = 0; i < deletedCount; i++) {
        widget.sessionManager.sendInput(widget.sessionId, [0x7F]); // DEL
      }
      _lastInputText = newText;
      return;
    }

    // Detect new characters typed.
    if (newText.length > oldText.length) {
      final added = newText.substring(oldText.length);
      final codes = <int>[];

      for (final char in added.characters) {
        if (_modifiers.ctrl) {
          // Ctrl+letter → control character
          final c = char.toLowerCase().codeUnitAt(0);
          if (c >= 0x61 && c <= 0x7A) {
            codes.add(c - 0x60); // a=1, b=2, ...
          } else {
            codes.addAll(char.codeUnits);
          }
        } else if (_modifiers.alt) {
          codes.add(0x1B); // ESC prefix
          codes.addAll(char.codeUnits);
        } else {
          codes.addAll(char.codeUnits);
        }
      }

      if (codes.isNotEmpty) {
        widget.sessionManager.sendInput(widget.sessionId, codes);
        // Flush immediately for low-latency echo — don't wait for next
        // 16ms render cycle.
        widget.sessionManager.flush(widget.sessionId);
      }
      _modifiers.releaseAll();
    }

    _lastInputText = newText;
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
      case 'CtrlL':
        return [0x0C]; // Ctrl+L — clear screen
      case 'CtrlR':
        return [0x12]; // Ctrl+R — reverse search
      case 'CtrlW':
        return [0x17]; // Ctrl+W — delete word
      case 'CtrlA':
        return [0x01]; // Ctrl+A — start of line
      case 'CtrlE':
        return [0x05]; // Ctrl+E — end of line
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
    // Double-tap toggles the input bar.
    if (!_showInputBar) {
      setState(() {
        _showInputBar = true;
      });
    }
    _inputFocusNode.requestFocus();
  }

  /// Long-press → copy all visible terminal text to clipboard.
  /// This is the simplest and most useful copy action on mobile:
  /// user presses and holds, gets immediate "Copied N lines" feedback.
  void _onLongPress(LongPressStartDetails details) {
    final text = _extractVisibleText();
    if (text.trim().isEmpty) {
      _showCopiedSnackBar('Nothing to copy');
      return;
    }

    final lineCount = text.trim().split('\n').length;
    Clipboard.setData(ClipboardData(text: text));
    _showCopiedSnackBar('Copied $lineCount lines');
  }

  /// Extract all visible terminal text as a string.
  String _extractVisibleText() {
    final buf = StringBuffer();
    for (var row = 0; row < _screen.rows; row++) {
      var lineText = '';
      for (var col = 0; col < _screen.cols; col++) {
        final idx = row * _screen.cols + col;
        if (idx < _screen.cells.length) {
          final cell = _screen.cells[idx];
          lineText += cell.char;
        }
      }
      // Trim trailing spaces but keep the line.
      buf.writeln(lineText.trimRight());
    }
    return buf.toString().trimRight();
  }

  void _showCopiedSnackBar(String message) {
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(
        content: Text(message),
        duration: const Duration(seconds: 2),
      ),
    );
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
          // Toggle input bar
          IconButton(
            icon: Icon(_showInputBar ? Icons.edit_off : Icons.edit),
            onPressed: () {
              setState(() => _showInputBar = !_showInputBar);
              if (_showInputBar) {
                _inputFocusNode.requestFocus();
              }
            },
          ),
        ],
      ),
      body: SafeArea(
        child: Column(
          children: [
            // ── Terminal canvas with hidden text input ──
            Expanded(
              child: LayoutBuilder(
                builder: (context, constraints) {
                  // Compute grid dimensions from available space.
                  // Lock to initial size — don't resize when keyboard opens,
                  // as that would clear the terminal grid.
                  final availH = constraints.maxHeight;
                  final cols = (constraints.maxWidth / _cellWidth).floor().clamp(10, 300);
                  final rows = (availH / _cellHeight).floor().clamp(3, 100);

                  // Only resize once on first layout (or if cols changed).
                  WidgetsBinding.instance.addPostFrameCallback((_) {
                    if (!_sizeInitialized) {
                      _sizeInitialized = true;
                      widget.sessionManager.resize(widget.sessionId, cols, rows);
                    }
                  });

                  return GestureDetector(
                    onScaleUpdate: _onScale,
                    onTapUp: _onTapUp,
                    onLongPressStart: _onLongPress,
                    child: Stack(
                      children: [
                        // Terminal canvas
                        CustomPaint(
                          painter: _TerminalPainter(
                            screen: _screen,
                            theme: theme,
                            cellWidth: _cellWidth,
                            cellHeight: _cellHeight,
                            cursorVisible: _cursorVisible,
                          ),
                          child: Container(),
                        ),
                        // Disconnect overlay
                        if (!_transportAlive)
                          Positioned.fill(
                            child: Container(
                              color: Colors.black54,
                              child: Center(
                                child: Column(
                                  mainAxisSize: MainAxisSize.min,
                                  children: [
                                    const Icon(
                                      Icons.cloud_off,
                                      color: Colors.white70,
                                      size: 48,
                                    ),
                                    const SizedBox(height: 12),
                                    const Text(
                                      'Connection closed',
                                      style: TextStyle(
                                        color: Colors.white,
                                        fontSize: 16,
                                        fontWeight: FontWeight.w500,
                                      ),
                                    ),
                                    const SizedBox(height: 16),
                                    FilledButton(
                                      onPressed: () =>
                                          Navigator.of(context).pop(),
                                      child: const Text('Back to Connections'),
                                    ),
                                  ],
                                ),
                              ),
                            ),
                          ),
                      ],
                    ),
                  );
                },
              ),
            ),

            // ── Visible input bar ──
            if (_showInputBar)
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
                color: Colors.grey.shade900,
                child: Row(
                  children: [
                    Expanded(
                      child: TextField(
                        controller: _inputController,
                        focusNode: _inputFocusNode,
                        onChanged: _onInputChanged,
                        onSubmitted: (_) {
                          widget.sessionManager
                              .sendInput(widget.sessionId, [0x0D]);
                          widget.sessionManager.flush(widget.sessionId);
                          _inputController.clear();
                          _lastInputText = '';
                          _inputFocusNode.requestFocus();
                        },
                        autofocus: true,
                        style: const TextStyle(
                          fontSize: 14,
                          color: Colors.white,
                          fontFamily: 'monospace',
                        ),
                        decoration: const InputDecoration(
                          hintText: 'Type here...',
                          hintStyle: TextStyle(color: Colors.grey),
                          border: OutlineInputBorder(),
                          isDense: true,
                          contentPadding: EdgeInsets.symmetric(
                            horizontal: 8,
                            vertical: 8,
                          ),
                        ),
                        autocorrect: false,
                        enableSuggestions: false,
                        keyboardType: TextInputType.visiblePassword,
                      ),
                    ),
                    IconButton(
                      icon: const Icon(Icons.keyboard_return, color: Colors.white),
                      onPressed: () {
                        widget.sessionManager
                            .sendInput(widget.sessionId, [0x0D]);
                        widget.sessionManager.flush(widget.sessionId);
                        _inputController.clear();
                        _lastInputText = '';
                      },
                    ),
                  ],
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
  final bool cursorVisible;

  _TerminalPainter({
    required this.screen,
    required this.theme,
    required this.cellWidth,
    required this.cellHeight,
    this.cursorVisible = true,
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

        // Draw cursor (block style) — only when visible (blink).
        if (cursorVisible && col == screen.cursorCol && row == screen.cursorRow) {
          final cursorPaint = Paint()
            ..color = theme.cursor.withValues(alpha: 0.5)
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
