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
import 'dart:convert';
import 'dart:io';
import 'dart:math' as math;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:wakelock_plus/wakelock_plus.dart';
import 'package:path_provider/path_provider.dart';

import 'ffi/session_manager.dart';
import 'keyboard_bar.dart';
import 'theme.dart';
export 'theme.dart' show TerminalTheme, darkTheme;

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

class _TerminalScreenState extends State<TerminalScreen>
    with WidgetsBindingObserver {
  static final _wordCharRe = RegExp(r'[A-Za-z0-9/._\-~]');
  double _fontSize = 13.0;
  static const _fontSizeFile = 'font_size.json';
  static const _themeFile = 'terminal_theme.json';
  ScreenSnapshot _screen = ScreenSnapshot.empty;
  final _modifiers = ModifierState();
  bool _showKeyboardBar = true;
  bool _showInputBar = false;
  Timer? _renderTimer;
  Timer? _durationTimer; // updates AppBar duration every second
  bool _transportAlive = true;
  bool _sizeInitialized = false;
  bool _isPaused = false; // true when app is in background
  DateTime? _connectedAt; // when the session connected (for duration display)
  TerminalTheme _currentTheme = darkTheme;
  int _themeIndex = 0; // index into builtinThemeNames

  /// Last long-press position for URL extraction.
  Offset _lastLongPressPos = Offset.zero;
  // Frame hash for change detection — avoids setState when nothing changed.
  int _lastFrameHash = 0;
  // Cursor blink state.
  bool _cursorVisible = true;
  Timer? _blinkTimer;
  // Scroll position tracking.
  bool _isScrolledUp = false;
  // Tap tracking for triple-tap line select.
  int _tapCount = 0;
  DateTime _lastTapTime = DateTime.fromMillisecondsSinceEpoch(0);

  // Visible input bar for typing.
  final FocusNode _inputFocusNode = FocusNode();
  final TextEditingController _inputController = TextEditingController();
  String _lastInputText = '';

  // Local command history for the input bar.
  final List<String> _inputHistory = [];
  int _inputHistoryIndex = -1; // -1 = not browsing history

  /// Navigate input history (up = older, down = newer).
  void _navigateInputHistory(bool up) {
    if (_inputHistory.isEmpty) return;
    if (up) {
      if (_inputHistoryIndex < _inputHistory.length - 1) {
        _inputHistoryIndex++;
      }
    } else {
      if (_inputHistoryIndex > 0) {
        _inputHistoryIndex--;
      } else {
        _inputHistoryIndex = -1;
        _inputController.text = _lastInputText;
        _inputController.selection = TextSelection.fromPosition(
          TextPosition(offset: _inputController.text.length),
        );
        return;
      }
    }
    _inputController.text = _inputHistory[_inputHistory.length - 1 - _inputHistoryIndex];
    _inputController.selection = TextSelection.fromPosition(
      TextPosition(offset: _inputController.text.length),
    );
  }

  // Cell dimensions derived from font size (monospace ratio ~0.6).
  double get _cellWidth => _fontSize * 0.6;
  double get _cellHeight => _fontSize * 1.3;

  /// Persist input history to app storage so it survives app restarts.
  Future<void> _saveInputHistory() async {
    try {
      final dir = await getApplicationDocumentsDirectory();
      final file = File('${dir.path}/input_history.json');
      await file.writeAsString(jsonEncode(_inputHistory));
    } catch (_) {}
  }

  /// Load persisted input history on startup.
  Future<void> _loadInputHistory() async {
    try {
      final dir = await getApplicationDocumentsDirectory();
      final file = File('${dir.path}/input_history.json');
      if (await file.exists()) {
        final list = jsonDecode(await file.readAsString()) as List;
        _inputHistory.clear();
        _inputHistory.addAll(list.cast<String>());
      }
    } catch (_) {}
  }

  /// Human-readable scroll position label (e.g., "↓ 45%", "↓ 1.2k lines").
  String get _scrollPercentLabel {
    final offset = widget.sessionManager.displayOffset(widget.sessionId);
    if (offset <= 0) return '↓';
    final total = widget.sessionManager.scrollbackLen(widget.sessionId);
    if (total <= 0) return '↓ $offset';
    final pct = ((offset / total) * 100).round().clamp(1, 99);
    if (offset > 999) {
      return '↓ ${offset ~/ 1000}.${(offset % 1000) ~/ 100}k';
    }
    return '↓ $pct%';
  }

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _connectedAt = DateTime.now();
    // Keep screen awake while terminal is active — prevents screen timeout
    // during long-running commands, log monitoring, etc.
    WakelockPlus.enable();
    _loadInputHistory();
    // Up/Down arrow keys on the input bar navigate command history.
    _inputFocusNode.onKeyEvent = (node, event) {
      if (event is KeyDownEvent) {
        if (event.logicalKey == LogicalKeyboardKey.arrowUp) {
          _navigateInputHistory(true);
          return KeyEventResult.handled;
        }
        if (event.logicalKey == LogicalKeyboardKey.arrowDown) {
          _navigateInputHistory(false);
          return KeyEventResult.handled;
        }
      }
      return KeyEventResult.ignored;
    };
    _loadFontSize();
    _loadTheme();
    _startRenderLoop();
    _startCursorBlink();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.paused ||
        state == AppLifecycleState.inactive) {
      // App went to background — pause rendering to save battery.
      _isPaused = true;
      _renderTimer?.cancel();
      _blinkTimer?.cancel();
    } else if (state == AppLifecycleState.resumed) {
      // App came back to foreground — resume rendering.
      if (_isPaused) {
        _isPaused = false;
        _startRenderLoop();
        _startCursorBlink();
        _lastFrameHash = 0; // Force full refresh on resume
      }
    }
  }

  Future<void> _loadFontSize() async {
    try {
      final dir = await getApplicationDocumentsDirectory();
      final file = File('${dir.path}/$_fontSizeFile');
      if (await file.exists()) {
        final size = double.tryParse(await file.readAsString());
        if (size != null && size >= 8.0 && size <= 32.0) {
          setState(() {
            _fontSize = size;
          });
        }
      }
    } catch (_) {}
  }

  Future<void> _saveFontSize() async {
    try {
      final dir = await getApplicationDocumentsDirectory();
      final file = File('${dir.path}/$_fontSizeFile');
      await file.writeAsString(_fontSize.toStringAsFixed(1));
    } catch (_) {}
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

  /// Reset cursor to visible and restart blink timer.
  /// Called when user sends input, so the cursor is always visible
  /// during active typing (matches desktop and iTerm2 behavior).
  void _resetCursorBlink() {
    if (!_cursorVisible) {
      _cursorVisible = true;
    }
    // Restart the timer so the cursor stays visible for at least
    // one full blink cycle after the last keystroke.
    _startCursorBlink();
  }

  void _startRenderLoop() {
    _renderTimer?.cancel();
    // Duration update timer: refreshes AppBar every second so the
    // connection duration stays live even when idle (no setState storms).
    _durationTimer?.cancel();
    _durationTimer = Timer.periodic(const Duration(seconds: 1), (_) {
      if (mounted && _connectedAt != null) {
        setState(() {});
      }
    });
    // ~60fps pump + render cycle for low-latency echo
    _renderTimer = Timer.periodic(const Duration(milliseconds: 16), (_) {
      if (!mounted) return;
      final mgr = widget.sessionManager;
      final id = widget.sessionId;

      // Pump transport data — returns bytes read.
      final bytesPumped = mgr.pumpAndFlush(id);

      // Check alive
      final alive = mgr.isAlive(id);
      if (alive != _transportAlive) {
        final wasConnected = _transportAlive;
        _transportAlive = alive;
        if (!alive) {
          // Session disconnected — show duration toast.
          if (wasConnected && _connectedAt != null) {
            final dur = DateTime.now().difference(_connectedAt!);
            final mins = dur.inMinutes;
            final secs = dur.inSeconds % 60;
            final durStr = mins > 0 ? '${mins}m ${secs}s' : '${secs}s';
            _showCopiedSnackBar('Disconnected after $durStr');
          } else {
            _showCopiedSnackBar('Disconnected');
          }
          HapticFeedback.heavyImpact();
          _durationTimer?.cancel();
          // Auto-hide keyboard bar and input bar on disconnect so the
          // disconnect overlay is not obscured by keyboard UI.
          if (_showKeyboardBar || _showInputBar) {
            _showKeyboardBar = false;
            _showInputBar = false;
            _inputFocusNode.unfocus();
          }
        }
      }

      // Apply inertial scrolling (continues scrolling after finger lift).
      _applyInertialScroll();

      // Get screen snapshot (needed for bell check even when idle).
      final snapshot = mgr.getScreenSnapshot(id);

      // Skip hash comparison when no data was pumped AND no state change.
      // This avoids the expensive _computeFrameHash() + setState() on idle
      // terminals, saving CPU and battery at 60fps.
      final scrolledUp = widget.sessionManager.displayOffset(id) > 0;
      if (bytesPumped == 0 &&
          alive == _transportAlive &&
          scrolledUp == _isScrolledUp) {
        if (snapshot.hasBell) {
          HapticFeedback.mediumImpact();
        }
        return;
      }

      // Only rebuild if content changed (cursor blink handled separately).
      final hash = _computeFrameHash(snapshot);
      if (hash != _lastFrameHash || alive != _transportAlive || scrolledUp != _isScrolledUp) {
        _lastFrameHash = hash;
        _isScrolledUp = scrolledUp;
        setState(() {
          _screen = snapshot;
        });
      }

      // Bell feedback — vibrate when terminal emits BEL (0x07).
      // This alerts the user when a long-running command finishes,
      // a tab-completion error occurs, or any program rings the bell.
      if (snapshot.hasBell) {
        HapticFeedback.mediumImpact();
        _bellFlashFrames = 8; // ~130ms flash at 60fps
      }

      // Decay bell flash counter — forces redraw while flashing.
      if (_bellFlashFrames > 0) {
        _bellFlashFrames--;
        _lastFrameHash = 0; // Force repaint during flash
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
    WidgetsBinding.instance.removeObserver(this);
    WakelockPlus.disable();
    _renderTimer?.cancel();
    _blinkTimer?.cancel();
    _durationTimer?.cancel();
    _inputFocusNode.dispose();
    _inputController.dispose();
    super.dispose();
  }

  void _sendKey(String keyName) {
    final codes = _keyNameToBytes(keyName);
    _sendInput(codes);
  }

  /// Send input bytes and auto-scroll to bottom if scrolled up.
  void _sendInput(List<int> codes) {
    widget.sessionManager.sendInput(widget.sessionId, codes);
    // Auto-scroll to bottom on input (standard terminal behavior).
    if (widget.sessionManager.displayOffset(widget.sessionId) > 0) {
      widget.sessionManager.resetViewport(widget.sessionId);
      _lastFrameHash = 0; // Force refresh
    }
    // Reset cursor blink so it's visible during active typing.
    _resetCursorBlink();
  }

  /// Paste text from system clipboard into the terminal.
  /// This is critical for mobile: users need to paste passwords,
  /// commands, and file paths from other apps.
  Future<void> _pasteFromClipboard() async {
    final data = await Clipboard.getData('text/plain');
    if (data?.text == null || data!.text!.isEmpty) {
      _showCopiedSnackBar('Clipboard is empty');
      return;
    }

    final text = data.text!;

    // Safety: confirm before pasting multi-line content.
    // Multi-line paste can execute multiple commands accidentally.
    final lineCount = text.trim().split('\n').length;
    if (lineCount > 1 && mounted) {
      final confirmed = await showDialog<bool>(
        context: context,
        builder: (ctx) => AlertDialog(
          title: const Text('Multi-line paste'),
          content: Text(
            'Clipboard contains $lineCount lines.\n'
            'Pasting may execute multiple commands.\n\n'
            'First line: ${text.split('\n').first.length > 60 ? '${text.split('\n').first.substring(0, 60)}...' : text.split('\n').first}',
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(ctx, false),
              child: const Text('Cancel'),
            ),
            TextButton(
              onPressed: () => Navigator.pop(ctx, true),
              child: const Text('Paste'),
            ),
          ],
        ),
      );
      if (confirmed != true) return;
    }
    if (!mounted) return;

    // Send text as UTF-8 bytes to the PTY.
    final bytes = utf8.encode(text);
    _sendInput(bytes);

    _showCopiedSnackBar(
      lineCount > 1 ? 'Pasted $lineCount lines' : 'Pasted ${text.length} chars',
    );
  }

  /// Handle text input from the hidden TextField.
  /// Computes the delta between last and current text, then sends it.
  void _onInputChanged(String newText) {
    final oldText = _lastInputText;

    // Detect backspace (text got shorter).
    if (newText.length < oldText.length) {
      final deletedCount = oldText.length - newText.length;
      for (var i = 0; i < deletedCount; i++) {
        _sendInput([0x7F]); // DEL
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
            codes.addAll(utf8.encode(char));
          }
        } else if (_modifiers.alt) {
          codes.add(0x1B); // ESC prefix
          codes.addAll(utf8.encode(char));
        } else {
          codes.addAll(utf8.encode(char));
        }
      }

      if (codes.isNotEmpty) {
        _sendInput(codes);
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
      case 'CtrlBackslash':
        return [0x1C]; // SIGQUIT (Ctrl+\)
      case 'CtrlU':
        return [0x15]; // Ctrl+U — clear line
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
      // F1-F4 use SS3 (ESC O P/Q/R/S)
      case 'F1':
        return [0x1B, 0x4F, 0x50]; // ESC O P
      case 'F2':
        return [0x1B, 0x4F, 0x51]; // ESC O Q
      case 'F3':
        return [0x1B, 0x4F, 0x52]; // ESC O R
      case 'F4':
        return [0x1B, 0x4F, 0x53]; // ESC O S
      // F5-F12 use CSI with parameter
      case 'F5':
        return [0x1B, 0x5B, 0x31, 0x35, 0x7E]; // ESC [ 1 5 ~
      case 'F6':
        return [0x1B, 0x5B, 0x31, 0x37, 0x7E]; // ESC [ 1 7 ~
      case 'F7':
        return [0x1B, 0x5B, 0x31, 0x38, 0x7E]; // ESC [ 1 8 ~
      case 'F8':
        return [0x1B, 0x5B, 0x31, 0x39, 0x7E]; // ESC [ 1 9 ~
      case 'F9':
        return [0x1B, 0x5B, 0x32, 0x30, 0x7E]; // ESC [ 2 0 ~
      case 'F10':
        return [0x1B, 0x5B, 0x32, 0x31, 0x7E]; // ESC [ 2 1 ~
      case 'F11':
        return [0x1B, 0x5B, 0x32, 0x33, 0x7E]; // ESC [ 2 3 ~
      case 'F12':
        return [0x1B, 0x5B, 0x32, 0x34, 0x7E]; // ESC [ 2 4 ~
      case 'CtrlL':
        return [0x0C]; // Ctrl+L — clear screen
      case 'CtrlR':
        return [0x12]; // Ctrl+R — reverse search
      case 'CtrlW':
        return [0x17]; // Ctrl+W — delete word
      case 'CtrlO':
        return [0x0F]; // Ctrl+O — vim open file
      case 'CtrlY':
        return [0x19]; // Ctrl+Y — redo / scroll up
      case 'CtrlG':
        return [0x07]; // Ctrl+G — bell / cancel
      case 'CtrlS':
        return [0x13]; // Ctrl+S — XOFF (pause output)
      case 'CtrlQ':
        return [0x11]; // Ctrl+Q — XON (resume output)
      case 'CtrlV':
        return [0x16]; // Ctrl+V — literal next (verbatim insert)
      case 'CtrlA':
        return [0x01]; // Ctrl+A — start of line
      case 'CtrlE':
        return [0x05]; // Ctrl+E — end of line
      case 'CtrlSpace':
        return [0x00]; // Ctrl+Space — NUL (vim autocomplete, emacs set-mark)
      case 'CtrlB':
        return [0x02]; // Ctrl+B — tmux prefix / backward char
      case 'CtrlK':
        return [0x0B]; // Ctrl+K — kill to end of line
      case 'CtrlF':
        return [0x06]; // Ctrl+F — forward char
      case 'CtrlN':
        return [0x0E]; // Ctrl+N — next history
      case 'CtrlP':
        return [0x10]; // Ctrl+P — previous history
      default:
        return utf8.encode(name);
    }
  }

  // Track accumulated scroll from two-finger drag.
  double _scrollAccumulator = 0.0;
  // Inertial scrolling: velocity in rows/frame, decays over time.
  double _scrollVelocity = 0.0;
  DateTime _lastScrollTime = DateTime.fromMillisecondsSinceEpoch(0);
  // Visual bell: counts down frames for red border flash.
  int _bellFlashFrames = 0;

  void _onScale(ScaleUpdateDetails details) {
    // Pinch to zoom (scale change).
    if ((details.scale - 1.0).abs() > 0.01) {
      final newSize = (_fontSize * details.scale).clamp(8.0, 32.0);
      // Show font size toast when it changes by at least 0.5pt
      if ((newSize - _fontSize).abs() >= 0.5) {
        _showCopiedSnackBar('Font size: ${newSize.toStringAsFixed(1)}pt');
      }
      setState(() {
        _fontSize = newSize;
      });
      _saveFontSize();
      return;
    }

    // Two-finger vertical drag = scroll terminal scrollback.
    // focalPointDelta.y < 0 = dragging up = scroll up (toward older).
    final dy = details.focalPointDelta.dy;
    if (dy.abs() > 0.5) {
      _scrollAccumulator += dy;
      // Track velocity for inertial scrolling.
      // Normalize to rows/sec based on cell height.
      final now = DateTime.now();
      final dtMs = now.difference(_lastScrollTime).inMilliseconds;
      if (dtMs > 0 && dtMs < 200) {
        // Exponential moving average of velocity (rows per 16ms frame).
        final instVel = dy / _cellHeight * (16 / dtMs);
        _scrollVelocity = _scrollVelocity * 0.5 + instVel * 0.5;
      }
      _lastScrollTime = now;

      // Scroll roughly one row per cell height of drag.
      final threshold = _cellHeight;
      while (_scrollAccumulator.abs() >= threshold) {
        if (_scrollAccumulator < 0) {
          // Dragging up → scroll toward older scrollback.
          widget.sessionManager.scrollUp(widget.sessionId, 1);
        } else {
          // Dragging down → scroll toward newer content.
          widget.sessionManager.scrollDown(widget.sessionId, 1);
        }
        _scrollAccumulator -= threshold * (_scrollAccumulator.sign);
      }
      // Force a screen refresh to show scrolled content.
      _lastFrameHash = 0;
    }
  }

  /// Apply inertial scrolling: called each frame to continue scrolling
  /// after the user lifts their fingers, with exponential decay.
  void _applyInertialScroll() {
    if (_scrollVelocity.abs() < 0.1) return;
    // Accumulate velocity into pixel space.
    final delta = _scrollVelocity * _cellHeight;
    _scrollAccumulator += delta;
    // Apply scroll.
    final threshold = _cellHeight;
    while (_scrollAccumulator.abs() >= threshold) {
      if (_scrollAccumulator < 0) {
        widget.sessionManager.scrollUp(widget.sessionId, 1);
      } else {
        widget.sessionManager.scrollDown(widget.sessionId, 1);
      }
      _scrollAccumulator -= threshold * (_scrollAccumulator.sign);
    }
    _lastFrameHash = 0;
    // Decay velocity (0.92 = lose 8% per frame ≈ 0.5s to ~stop).
    _scrollVelocity *= 0.92;
    if (_scrollVelocity.abs() < 0.1) {
      _scrollVelocity = 0;
    }
  }

  /// Stop inertial scrolling immediately (e.g. on touch).
  void _cancelInertia() {
    _scrollVelocity = 0;
  }

  void _onTapUp(TapUpDetails details) {
    // Track tap count for multi-tap detection.
    // We handle ALL tap logic here (including double-tap word select)
    // because Flutter's onDoubleTapDown doesn't increment our counter,
    // which breaks triple-tap detection.
    final now = DateTime.now();
    if (now.difference(_lastTapTime).inMilliseconds < 400) {
      _tapCount++;
    } else {
      _tapCount = 1;
    }
    _lastTapTime = now;

    // Triple-tap → select and copy entire line.
    if (_tapCount >= 3) {
      _selectLineAt(details.localPosition);
      _tapCount = 0;
      return;
    }

    // Double-tap → select and copy word.
    if (_tapCount == 2) {
      _selectWordAt(details.localPosition);
      return;
    }

    // Single tap shows input bar for typing.
    if (!_showInputBar) {
      setState(() {
        _showInputBar = true;
      });
    }
    _inputFocusNode.requestFocus();
  }

  /// Double-tap → select and copy the word at the tap position.
  void _selectWordAt(Offset position) {
    final col = (position.dx / _cellWidth).floor();
    final row = (position.dy / _cellHeight).floor();
    if (row < 0 || row >= _screen.rows || col < 0 || col >= _screen.cols) {
      return;
    }

    final idx = row * _screen.cols + col;
    if (idx >= _screen.cells.length) return;
    final cell = _screen.cells[idx];
    if (cell.charCode == 0) return;

    // Scan left for word start.
    var startCol = col;
    while (startCol > 0) {
      final i = row * _screen.cols + (startCol - 1);
      if (i >= _screen.cells.length) break;
      final c = _screen.cells[i];
      if (c.charCode == 0) break;
      final ch = c.char;
      if (!_wordCharRe.hasMatch(ch)) break;
      startCol--;
    }

    // Scan right for word end.
    var endCol = col;
    while (endCol < _screen.cols - 1) {
      final i = row * _screen.cols + (endCol + 1);
      if (i >= _screen.cells.length) break;
      final c = _screen.cells[i];
      if (c.charCode == 0) break;
      final ch = c.char;
      if (!_wordCharRe.hasMatch(ch)) break;
      endCol++;
    }

    // Extract word text.
    final buf = StringBuffer();
    for (var c = startCol; c <= endCol; c++) {
      final i = row * _screen.cols + c;
      if (i < _screen.cells.length) {
        final cell = _screen.cells[i];
        if (cell.charCode != 0) buf.write(cell.char);
      }
    }

    final word = buf.toString();
    if (word.isEmpty) return;

    Clipboard.setData(ClipboardData(text: word));
    _showCopiedSnackBar('Copied: $word');
  }

  /// Triple-tap → select and copy entire line at position.
  void _selectLineAt(Offset position) {
    final row = (position.dy / _cellHeight).floor();
    if (row < 0 || row >= _screen.rows) return;

    // Extract all non-null characters on this row.
    final buf = StringBuffer();
    for (var col = 0; col < _screen.cols; col++) {
      final idx = row * _screen.cols + col;
      if (idx < _screen.cells.length) {
        final cell = _screen.cells[idx];
        if (cell.charCode != 0) {
          buf.write(cell.char);
        }
      }
    }

    final line = buf.toString().trimRight();
    if (line.isEmpty) return;

    Clipboard.setData(ClipboardData(text: line));
    _showCopiedSnackBar('Copied line: ${line.length > 40 ? '${line.substring(0, 40)}...' : line}');
  }

  /// Double-tap → select and copy the word at the tap position.
  /// Scans left and right for word boundaries (alphanumeric + / . _ -).
  /// Long-press → copy all visible terminal text to clipboard.
  /// This is the simplest and most useful copy action on mobile:
  /// user presses and holds, gets immediate "Copied N lines" feedback.
  void _onLongPress(LongPressStartDetails details) {
    _lastLongPressPos = details.localPosition;
    showModalBottomSheet<void>(
      context: context,
      backgroundColor: Colors.grey.shade900,
      shape: const RoundedRectangleBorder(
        borderRadius: BorderRadius.vertical(top: Radius.circular(12)),
      ),
      builder: (context) {
        return SafeArea(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const SizedBox(height: 4),
              // Handle bar.
              Container(
                width: 36,
                height: 4,
                margin: const EdgeInsets.only(bottom: 8),
                decoration: BoxDecoration(
                  color: Colors.grey.shade600,
                  borderRadius: BorderRadius.circular(2),
                ),
              ),
              ListTile(
                leading: const Icon(Icons.copy, color: Colors.white70),
                title: const Text('Copy all visible text',
                    style: TextStyle(color: Colors.white)),
                onTap: () {
                  Navigator.pop(context);
                  final text = _extractVisibleText();
                  if (text.trim().isEmpty) {
                    _showCopiedSnackBar('Nothing to copy');
                    return;
                  }
                  final lineCount = text.trim().split('\n').length;
                  Clipboard.setData(ClipboardData(text: text));
                  _showCopiedSnackBar(
                      lineCount > 1 ? 'Copied $lineCount lines' : 'Copied');
                },
              ),
              ListTile(
                leading: const Icon(Icons.code, color: Colors.white70),
                title: const Text('Copy screen as Markdown',
                    style: TextStyle(color: Colors.white)),
                onTap: () {
                  Navigator.pop(context);
                  _copyScreenAsMarkdown();
                },
              ),
              ListTile(
                leading: const Icon(Icons.paste, color: Colors.white70),
                title: const Text('Paste from clipboard',
                    style: TextStyle(color: Colors.white)),
                onTap: () {
                  Navigator.pop(context);
                  _pasteFromClipboard();
                },
              ),
              ListTile(
                leading:
                    const Icon(Icons.select_all, color: Colors.white70),
                title: const Text('Select word',
                    style: TextStyle(color: Colors.white)),
                onTap: () {
                  Navigator.pop(context);
                  _selectWordAt(details.localPosition);
                },
              ),
              ListTile(
                leading:
                    const Icon(Icons.cleaning_services, color: Colors.white70),
                title: const Text('Clear screen',
                    style: TextStyle(color: Colors.white)),
                onTap: () {
                  Navigator.pop(context);
                  // Ctrl+L = clear screen
                  widget.sessionManager
                      .sendInput(widget.sessionId, [0x0C]);
                  HapticFeedback.selectionClick();
                },
              ),
              ListTile(
                leading:
                    const Icon(Icons.keyboard_tab, color: Colors.white70),
                title: const Text('Send Tab (autocomplete)',
                    style: TextStyle(color: Colors.white)),
                onTap: () {
                  Navigator.pop(context);
                  widget.sessionManager
                      .sendInput(widget.sessionId, [0x09]);
                  HapticFeedback.selectionClick();
                },
              ),
              ListTile(
                leading:
                    const Icon(Icons.open_in_browser, color: Colors.white70),
                title: const Text('Open URL at tap position',
                    style: TextStyle(color: Colors.white)),
                onTap: () {
                  Navigator.pop(context);
                  _openUrlAt(_lastLongPressPos);
                  HapticFeedback.selectionClick();
                },
              ),
              const SizedBox(height: 8),
            ],
          ),
        );
      },
    );
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

  /// Extract text from the current visible terminal screen and copy to clipboard.
  /// Useful for sharing terminal output without a selection.
  Future<void> _copyScreenText() async {
    final snap = widget.sessionManager.getScreenSnapshot(widget.sessionId);
    if (snap.cells.isEmpty) {
      _showCopiedSnackBar('Screen is empty');
      return;
    }

    // Build text from cell data, row by row.
    final lines = <String>[];
    for (var row = 0; row < snap.rows; row++) {
      final sb = StringBuffer();
      for (var col = 0; col < snap.cols; col++) {
        final idx = row * snap.cols + col;
        if (idx < snap.cells.length) {
          final cell = snap.cells[idx];
          // Skip wide-char spacers (second half of CJK/emoji)
          if (cell.flags & 0x100 != 0) continue; // WIDE_SPACER flag
          if (cell.charCode != 0) {
            sb.writeCharCode(cell.charCode);
          } else {
            sb.write(' ');
          }
        }
      }
      final line = sb.toString().trimRight();
      lines.add(line);
    }

    // Trim trailing empty lines
    while (lines.isNotEmpty && lines.last.isEmpty) {
      lines.removeLast();
    }

    if (lines.isEmpty) {
      _showCopiedSnackBar('Screen is empty');
      return;
    }

    final text = lines.join('\n');
    await Clipboard.setData(ClipboardData(text: text));
    _showCopiedSnackBar('Copied ${text.length} chars (${lines.length} lines)');
    HapticFeedback.selectionClick();
  }

  /// Copy screen text as a Markdown fenced code block.
  /// Useful for pasting terminal output into Slack, Discord, GitHub, etc.
  Future<void> _copyScreenAsMarkdown() async {
    final snap = widget.sessionManager.getScreenSnapshot(widget.sessionId);
    if (snap.cells.isEmpty) {
      _showCopiedSnackBar('Screen is empty');
      return;
    }

    final lines = <String>[];
    for (var row = 0; row < snap.rows; row++) {
      final sb = StringBuffer();
      for (var col = 0; col < snap.cols; col++) {
        final idx = row * snap.cols + col;
        if (idx < snap.cells.length) {
          final cell = snap.cells[idx];
          if (cell.flags & 0x100 != 0) continue; // WIDE_SPACER flag
          if (cell.charCode != 0) {
            sb.writeCharCode(cell.charCode);
          } else {
            sb.write(' ');
          }
        }
      }
      final line = sb.toString().trimRight();
      lines.add(line);
    }

    while (lines.isNotEmpty && lines.last.isEmpty) {
      lines.removeLast();
    }

    if (lines.isEmpty) {
      _showCopiedSnackBar('Screen is empty');
      return;
    }

    final text = lines.join('\n');
    final markdown = '```\n$text\n```';
    await Clipboard.setData(ClipboardData(text: markdown));
    _showCopiedSnackBar(
        'Copied ${lines.length} lines as Markdown code block');
    HapticFeedback.selectionClick();
  }

  /// Format a Duration as human-readable string (e.g. "3m 42s", "1h 5m").
  String _formatDuration(Duration d) {
    final h = d.inHours;
    final m = d.inMinutes % 60;
    final s = d.inSeconds % 60;
    if (h > 0) return '${h}h ${m}m';
    if (m > 0) return '${m}m ${s}s';
    return '${s}s';
  }

  /// Build a thin scrollbar indicator showing scroll position in scrollback.
  Widget _buildScrollbar(BuildContext context) {
    final mgr = widget.sessionManager;
    final id = widget.sessionId;
    final scrollLen = mgr.scrollbackLen(id);
    final offset = mgr.displayOffset(id);
    if (scrollLen <= 0) return const SizedBox.shrink();

    // Calculate scrollbar thumb position.
    final visibleRows = mgr.getScreenSnapshot(id).rows;
    final totalRows = scrollLen + visibleRows;
    final thumbFraction = visibleRows / totalRows;
    final thumbHeight = (thumbFraction * 1000).clamp(30.0, 1000.0);
    final scrollFraction = offset / scrollLen;
    final thumbTop = (scrollFraction * (1000 - thumbHeight)).clamp(0.0, 1000.0 - thumbHeight);

    return LayoutBuilder(
      builder: (context, constraints) {
        final maxH = constraints.maxHeight;
        if (maxH <= 0) return const SizedBox.shrink();
        final ratio = maxH / 1000;
        return Align(
          alignment: Alignment.topCenter,
          child: Container(
            width: 3,
            height: thumbHeight * ratio,
            margin: EdgeInsets.only(top: thumbTop * ratio),
            decoration: BoxDecoration(
              color: Colors.white.withValues(alpha: 0.25),
              borderRadius: BorderRadius.circular(1.5),
            ),
          ),
        );
      },
    );
  }

  Future<void> _saveTheme() async {
    try {
      final dir = await getApplicationDocumentsDirectory();
      final file = File('${dir.path}/$_themeFile');
      await file.writeAsString(builtinThemeNames[_themeIndex]);
    } catch (_) {}
  }

  Future<void> _loadTheme() async {
    try {
      final dir = await getApplicationDocumentsDirectory();
      final file = File('${dir.path}/$_themeFile');
      if (await file.exists()) {
        final name = (await file.readAsString()).trim();
        final idx = builtinThemeNames.indexOf(name);
        if (idx >= 0) {
          setState(() {
            _themeIndex = idx;
            _currentTheme = themeByName(name);
          });
        }
      }
    } catch (_) {}
  }

  /// Extract the URL at the given position and open it in the system browser.
  void _openUrlAt(Offset position) {
    final col = (position.dx / _cellWidth).floor();
    final row = (position.dy / _cellHeight).floor();
    if (row < 0 || row >= _screen.rows || col < 0 || col >= _screen.cols) {
      _showCopiedSnackBar('No URL found');
      return;
    }

    // Extract the full line at the tap position.
    final lineBuf = StringBuffer();
    for (var c = 0; c < _screen.cols; c++) {
      final idx = row * _screen.cols + c;
      if (idx < _screen.cells.length) {
        final cell = _screen.cells[idx];
        lineBuf.write(cell.charCode != 0 ? cell.char : ' ');
      }
    }
    final line = lineBuf.toString().trim();

    // Find URL in the line using regex.
    final urlRegex = RegExp(r'https?://[^\s]+');
    final match = urlRegex.firstMatch(line);
    if (match != null) {
      final url = match.group(0)!;
      _launchUrl(url);
    } else {
      // Also check for bare URLs (e.g., github.com/user/repo).
      final bareUrlRegex = RegExp(r'[a-z0-9-]+\.[a-z]{2,}/?[^\s]*');
      final bareMatch = bareUrlRegex.firstMatch(line);
      if (bareMatch != null) {
        _launchUrl('https://${bareMatch.group(0)}');
      } else {
        _showCopiedSnackBar('No URL found at tap position');
      }
    }
  }

  /// Launch a URL using the platform's default browser.
  Future<void> _launchUrl(String url) async {
    try {
      // Use our custom platform channel (backed by native AppDelegate / MainActivity).
      const channel = MethodChannel('dev.ggterm/share');
      final success = await channel.invokeMethod<bool>('shareUrl', {'url': url});
      if (success == true) {
        _showCopiedSnackBar('Opening: $url');
      } else {
        throw Exception('platform returned false');
      }
    } catch (_) {
      // Fallback: copy to clipboard so user can paste in browser.
      await Clipboard.setData(ClipboardData(text: url));
      _showCopiedSnackBar('URL copied: $url');
    }
  }

  void _showCopiedSnackBar(String message) {
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(
        content: Text(message),
        duration: const Duration(seconds: 2),
      ),
    );
  }

  /// Handle hardware keyboard events (iPad/Android tablet with physical keyboard).
  /// Maps physical keys to terminal escape sequences, enabling Tab, Esc, F-keys,
  /// arrow keys, and Ctrl combos without needing the on-screen keyboard bar.
  KeyEventResult _onHardwareKeyEvent(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent) return KeyEventResult.ignored;

    final key = event.logicalKey;
    final ctrl = HardwareKeyboard.instance.isControlPressed;
    final alt = HardwareKeyboard.instance.isAltPressed;

    // Map common special keys using Flutter LogicalKeyboardKey constants.
    switch (key) {
      case LogicalKeyboardKey.tab:
        _sendInput([0x09]);
        return KeyEventResult.handled;
      case LogicalKeyboardKey.escape:
        _sendInput([0x1B]);
        return KeyEventResult.handled;
      case LogicalKeyboardKey.backspace:
        _sendInput([0x7F]);
        return KeyEventResult.handled;
      case LogicalKeyboardKey.enter:
        _sendInput([0x0D]);
        return KeyEventResult.handled;
      // Arrow keys
      case LogicalKeyboardKey.arrowUp:
        _sendInput([0x1B, 0x5B, 0x41]);
        return KeyEventResult.handled;
      case LogicalKeyboardKey.arrowDown:
        _sendInput([0x1B, 0x5B, 0x42]);
        return KeyEventResult.handled;
      case LogicalKeyboardKey.arrowRight:
        _sendInput([0x1B, 0x5B, 0x43]);
        return KeyEventResult.handled;
      case LogicalKeyboardKey.arrowLeft:
        _sendInput([0x1B, 0x5B, 0x44]);
        return KeyEventResult.handled;
      // Navigation keys
      case LogicalKeyboardKey.home:
        _sendInput([0x1B, 0x5B, 0x48]);
        return KeyEventResult.handled;
      case LogicalKeyboardKey.end:
        _sendInput([0x1B, 0x5B, 0x46]);
        return KeyEventResult.handled;
      case LogicalKeyboardKey.pageUp:
        _sendInput([0x1B, 0x5B, 0x35, 0x7E]);
        return KeyEventResult.handled;
      case LogicalKeyboardKey.pageDown:
        _sendInput([0x1B, 0x5B, 0x36, 0x7E]);
        return KeyEventResult.handled;
      case LogicalKeyboardKey.delete:
        _sendInput([0x1B, 0x5B, 0x33, 0x7E]);
        return KeyEventResult.handled;
      // F1-F12
      case LogicalKeyboardKey.f1:
        _sendInput([0x1B, 0x4F, 0x50]); // ESC O P
        return KeyEventResult.handled;
      case LogicalKeyboardKey.f2:
        _sendInput([0x1B, 0x4F, 0x51]); // ESC O Q
        return KeyEventResult.handled;
      case LogicalKeyboardKey.f3:
        _sendInput([0x1B, 0x4F, 0x52]); // ESC O R
        return KeyEventResult.handled;
      case LogicalKeyboardKey.f4:
        _sendInput([0x1B, 0x4F, 0x53]); // ESC O S
        return KeyEventResult.handled;
      case LogicalKeyboardKey.f5:
        _sendInput([0x1B, 0x5B, 0x31, 0x35, 0x7E]); // ESC [ 1 5 ~
        return KeyEventResult.handled;
      case LogicalKeyboardKey.f6:
        _sendInput([0x1B, 0x5B, 0x31, 0x37, 0x7E]); // ESC [ 1 7 ~
        return KeyEventResult.handled;
      case LogicalKeyboardKey.f7:
        _sendInput([0x1B, 0x5B, 0x31, 0x38, 0x7E]); // ESC [ 1 8 ~
        return KeyEventResult.handled;
      case LogicalKeyboardKey.f8:
        _sendInput([0x1B, 0x5B, 0x31, 0x39, 0x7E]); // ESC [ 1 9 ~
        return KeyEventResult.handled;
      case LogicalKeyboardKey.f9:
        _sendInput([0x1B, 0x5B, 0x32, 0x30, 0x7E]); // ESC [ 2 0 ~
        return KeyEventResult.handled;
      case LogicalKeyboardKey.f10:
        _sendInput([0x1B, 0x5B, 0x32, 0x31, 0x7E]); // ESC [ 2 1 ~
        return KeyEventResult.handled;
      case LogicalKeyboardKey.f11:
        _sendInput([0x1B, 0x5B, 0x32, 0x33, 0x7E]); // ESC [ 2 3 ~
        return KeyEventResult.handled;
      case LogicalKeyboardKey.f12:
        _sendInput([0x1B, 0x5B, 0x32, 0x34, 0x7E]); // ESC [ 2 4 ~
        return KeyEventResult.handled;
    }

    // Ctrl+letter combos (a-z)
    if (ctrl) {
      final char = key.keyLabel.toLowerCase();
      if (char.length == 1 && char.codeUnitAt(0) >= 0x61 && char.codeUnitAt(0) <= 0x7A) {
        _sendInput([char.codeUnitAt(0) - 0x60]); // Ctrl+a = 0x01, etc.
        return KeyEventResult.handled;
      }
      // Ctrl+Space = NUL
      if (key == LogicalKeyboardKey.space) {
        _sendInput([0x00]);
        return KeyEventResult.handled;
      }
    }

    // Regular printable characters — let TextField handle them
    // so that the onChanged handler picks them up.
    if (!ctrl && !alt) {
      return KeyEventResult.ignored;
    }

    // Alt+letter = ESC + letter (meta prefix)
    if (alt && !ctrl) {
      final char = key.keyLabel;
      if (char.isNotEmpty) {
        _sendInput([0x1B, ...utf8.encode(char)]);
        return KeyEventResult.handled;
      }
    }

    return KeyEventResult.ignored;
  }

  @override
  Widget build(BuildContext context) {
    final theme = _currentTheme;

    return PopScope(
      canPop: false,
      onPopInvokedWithResult: (didPop, result) {
        if (didPop) return;
        // If keyboard bar or input bar is visible, hide them first.
        if (_showKeyboardBar || _showInputBar) {
          setState(() {
            _showKeyboardBar = false;
            _showInputBar = false;
          });
          return;
        }
        // Otherwise, disconnect and go back.
        Navigator.of(context).pop();
      },
      child: Focus(
        autofocus: true,
        onKeyEvent: _onHardwareKeyEvent,
        child: Scaffold(
      backgroundColor: theme.background,
      appBar: AppBar(
        title: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(
              _screen.title.isNotEmpty ? _screen.title : widget.title,
              overflow: TextOverflow.ellipsis,
              style: const TextStyle(fontSize: 16),
            ),
            // Subtitle: show SSH connection info + duration when available
            if (widget.title != 'Echo Test' && widget.title != 'P2P Session')
              Text(
                _connectedAt != null
                    ? '${widget.title}  •  ${_formatDuration(DateTime.now().difference(_connectedAt!))}'
                    : widget.title,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontSize: 11,
                  color: Colors.grey.shade400,
                  fontWeight: FontWeight.normal,
                ),
              ),
          ],
        ),
        backgroundColor: Colors.grey.shade900,
        foregroundColor: Colors.white,
        actions: [
          // Scroll-to-bottom button with line count (visible when scrolled up).
          if (_isScrolledUp)
            TextButton.icon(
              onPressed: () {
                widget.sessionManager.resetViewport(widget.sessionId);
                _lastFrameHash = 0; // Force refresh
              },
              icon: const Icon(Icons.vertical_align_bottom,
                  color: Colors.white70, size: 18),
              label: Text(
                _scrollPercentLabel,
                style: const TextStyle(color: Colors.white70, fontSize: 12),
              ),
            ),
          // Transport status indicator (green dot=connected, red pulsing=disconnected)
          Tooltip(
            message: _transportAlive ? 'Connected' : 'Disconnected',
            child: Padding(
              padding: const EdgeInsets.only(right: 8),
              child: Center(
                child: _transportAlive
                    ? Container(
                        width: 10,
                        height: 10,
                        decoration: BoxDecoration(
                          shape: BoxShape.circle,
                          color: Colors.green,
                          boxShadow: [
                            BoxShadow(
                              color: Colors.green.withValues(alpha: 0.4),
                              blurRadius: 6,
                              spreadRadius: 1,
                            ),
                          ],
                        ),
                      )
                    : TweenAnimationBuilder<double>(
                        tween: Tween(begin: 0.5, end: 1.0),
                        duration: const Duration(milliseconds: 800),
                        curve: Curves.easeInOut,
                        builder: (context, scale, child) {
                          return Transform.scale(
                            scale: 0.8 + scale * 0.4,
                            child: Container(
                              width: 10,
                              height: 10,
                              decoration: BoxDecoration(
                                shape: BoxShape.circle,
                                color: Colors.red.withValues(alpha: 0.6 + scale * 0.4),
                                boxShadow: [
                                  BoxShadow(
                                    color: Colors.red.withValues(alpha: 0.3 + scale * 0.3),
                                    blurRadius: 4 + scale * 4,
                                    spreadRadius: scale,
                                  ),
                                ],
                              ),
                            ),
                          );
                        },
                      ),
              ),
            ),
          ),
          // Export screen text to clipboard
          IconButton(
            icon: const Icon(Icons.content_copy, color: Colors.white70, size: 20),
            tooltip: 'Copy screen text',
            onPressed: _copyScreenText,
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
          // Overflow menu
          PopupMenuButton<String>(
            icon: const Icon(Icons.more_vert),
            color: Colors.grey.shade900,
            onSelected: (value) async {
              switch (value) {
                case 'clear':
                  // Clear screen: send Ctrl+L
                  _sendInput([0x0C]);
                  widget.sessionManager.flush(widget.sessionId);
                  break;
                case 'copy_markdown':
                  _copyScreenAsMarkdown();
                  break;
                case 'font_increase':
                  setState(() {
                    _fontSize = (_fontSize + 1.0).clamp(8.0, 32.0);
                    _saveFontSize();
                  });
                  HapticFeedback.selectionClick();
                  break;
                case 'font_decrease':
                  setState(() {
                    _fontSize = (_fontSize - 1.0).clamp(8.0, 32.0);
                    _saveFontSize();
                  });
                  HapticFeedback.selectionClick();
                  break;
                case 'font_reset':
                  setState(() {
                    _fontSize = 13.0;
                    _saveFontSize();
                  });
                  HapticFeedback.selectionClick();
                  break;
                case 'theme_cycle':
                  setState(() {
                    _themeIndex = (_themeIndex + 1) % builtinThemeNames.length;
                    _currentTheme = themeByName(builtinThemeNames[_themeIndex]);
                  });
                  _saveTheme();
                  _showCopiedSnackBar('Theme: ${builtinThemeNames[_themeIndex]}');
                  HapticFeedback.selectionClick();
                  break;
                case 'copy_all':
                  final text = widget.sessionManager.getTerminalText(widget.sessionId);
                  if (text.isEmpty) {
                    _showCopiedSnackBar('No text to copy');
                  } else {
                    Clipboard.setData(ClipboardData(text: text));
                    final lines = '\n'.allMatches(text).length + 1;
                    _showCopiedSnackBar('Copied $lines lines');
                  }
                  HapticFeedback.selectionClick();
                  break;
                case 'share_text':
                  final text = widget.sessionManager.getTerminalText(widget.sessionId);
                  if (text.isEmpty) {
                    _showCopiedSnackBar('No text to share');
                  } else {
                    // Use platform channel to share text via system share sheet.
                    try {
                      const channel = MethodChannel('dev.ggterm/share');
                      await channel.invokeMethod('shareText', {'text': text, 'subject': 'GGTerm output'});
                    } catch (_) {
                      // Fallback: copy to clipboard if share fails.
                      Clipboard.setData(ClipboardData(text: text));
                      _showCopiedSnackBar('Copied to clipboard (share unavailable)');
                    }
                  }
                  HapticFeedback.selectionClick();
                  break;
                case 'disconnect':
                  widget.sessionManager.destroySession(widget.sessionId);
                  if (mounted) Navigator.of(context).pop();
                  break;
              }
            },
            itemBuilder: (context) => [
              const PopupMenuItem(
                value: 'clear',
                child: Row(children: [
                  Icon(Icons.clear, color: Colors.white70, size: 20),
                  SizedBox(width: 12),
                  Text('Clear screen', style: TextStyle(color: Colors.white)),
                ]),
              ),
              const PopupMenuItem(
                value: 'copy_markdown',
                child: Row(children: [
                  Icon(Icons.code, color: Colors.white70, size: 20),
                  SizedBox(width: 12),
                  Text('Copy as Markdown',
                      style: TextStyle(color: Colors.white)),
                ]),
              ),
              const PopupMenuItem(
                value: 'font_increase',
                child: Row(children: [
                  Icon(Icons.text_increase, color: Colors.white70, size: 20),
                  SizedBox(width: 12),
                  Text('Font size +', style: TextStyle(color: Colors.white)),
                ]),
              ),
              const PopupMenuItem(
                value: 'font_decrease',
                child: Row(children: [
                  Icon(Icons.text_decrease, color: Colors.white70, size: 20),
                  SizedBox(width: 12),
                  Text('Font size −', style: TextStyle(color: Colors.white)),
                ]),
              ),
              const PopupMenuItem(
                value: 'font_reset',
                child: Row(children: [
                  Icon(Icons.refresh, color: Colors.white70, size: 20),
                  SizedBox(width: 12),
                  Text('Reset font size', style: TextStyle(color: Colors.white)),
                ]),
              ),
              const PopupMenuItem(
                value: 'theme_cycle',
                child: Row(children: [
                  Icon(Icons.palette, color: Colors.white70, size: 20),
                  SizedBox(width: 12),
                  Text('Switch theme', style: TextStyle(color: Colors.white)),
                ]),
              ),
              const PopupMenuItem(
                value: 'copy_all',
                child: Row(children: [
                  Icon(Icons.copy_all, color: Colors.white70, size: 20),
                  SizedBox(width: 12),
                  Text('Copy all visible text',
                      style: TextStyle(color: Colors.white)),
                ]),
              ),
              const PopupMenuItem(
                value: 'share_text',
                child: Row(children: [
                  Icon(Icons.share, color: Colors.white70, size: 20),
                  SizedBox(width: 12),
                  Text('Share terminal text',
                      style: TextStyle(color: Colors.white)),
                ]),
              ),
              const PopupMenuItem(
                value: 'disconnect',
                child: Row(children: [
                  Icon(Icons.logout, color: Colors.redAccent, size: 20),
                  SizedBox(width: 12),
                  Text('Disconnect',
                      style: TextStyle(color: Colors.redAccent)),
                ]),
              ),
            ],
          ),
        ],
      ),
      body: SafeArea(
        child: Column(
          children: [
            // ── Disconnected banner ──
            if (!_transportAlive)
              Material(
                color: const Color(0xFFB71C1C),
                child: Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 16,
                    vertical: 8,
                  ),
                  child: Row(
                    children: [
                      const Icon(Icons.wifi_off, color: Colors.white, size: 18),
                      const SizedBox(width: 8),
                      const Expanded(
                        child: Text(
                          'Connection lost',
                          style: TextStyle(color: Colors.white, fontSize: 13),
                        ),
                      ),
                      TextButton(
                        onPressed: () => Navigator.of(context).pop(),
                        style: TextButton.styleFrom(
                          foregroundColor: Colors.white,
                          padding: const EdgeInsets.symmetric(horizontal: 12),
                          minimumSize: const Size(0, 32),
                        ),
                        child: const Text('Reconnect'),
                      ),
                    ],
                  ),
                ),
              ),
            // ── Terminal canvas with hidden text input ──
            Expanded(
              child: Stack(
                children: [
                  LayoutBuilder(
                builder: (context, constraints) {
                  // Compute grid dimensions from available space.
                  final availH = constraints.maxHeight;
                  final newCols = (constraints.maxWidth / _cellWidth).floor().clamp(10, 300);
                  final newRows = (availH / _cellHeight).floor().clamp(3, 100);

                  // Resize terminal when dimensions change by ≥1 row/col.
                  // This handles keyboard open/close gracefully — the grid
                  // preserves scrollback content on resize.
                  WidgetsBinding.instance.addPostFrameCallback((_) {
                    if (!_sizeInitialized || newCols != _screen.cols || newRows != _screen.rows) {
                      _sizeInitialized = true;
                      widget.sessionManager.resize(widget.sessionId, newCols, newRows);
                      _lastFrameHash = 0; // Force screen refresh
                    }
                  });

                  return GestureDetector(
                    onScaleStart: (_) => _cancelInertia(),
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
                        // Visual bell flash — red border when BEL fires.
                        if (_bellFlashFrames > 0)
                          Positioned.fill(
                            child: IgnorePointer(
                              child: Container(
                                decoration: BoxDecoration(
                                  border: Border.all(
                                    color: Colors.red
                                        .withValues(alpha: _bellFlashFrames / 8.0 * 0.8),
                                    width: 3,
                                  ),
                                  borderRadius: BorderRadius.circular(4),
                                ),
                              ),
                            ),
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
                  // Scrollbar indicator (thin bar on right edge when scrolled up).
                  if (_isScrolledUp)
                    Positioned(
                      top: 0,
                      bottom: 0,
                      right: 2,
                      child: _buildScrollbar(context),
                    ),
                  // Floating scroll-to-bottom button (visible when scrolled up).
                  if (_isScrolledUp)
                    Positioned(
                      bottom: 16,
                      right: 16,
                      child: FloatingActionButton(
                        mini: true,
                        backgroundColor: Colors.blue.withValues(alpha: 0.85),
                        foregroundColor: Colors.white,
                        elevation: 4,
                        onPressed: () {
                          widget.sessionManager.resetViewport(widget.sessionId);
                          _lastFrameHash = 0;
                          _cancelInertia();
                        },
                        child: const Icon(Icons.arrow_downward, size: 20),
                      ),
                    ),
                ],
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
                        textInputAction: TextInputAction.send,
                        onChanged: _onInputChanged,
                        onSubmitted: (text) {
                          final cmd = text.trim();
                          if (cmd.isNotEmpty) {
                            _inputHistory.remove(cmd);
                            _inputHistory.add(cmd);
                            if (_inputHistory.length > 100) {
                              _inputHistory.removeAt(0);
                            }
                            _saveInputHistory();
                          }
                          _inputHistoryIndex = -1;
                          _sendInput([0x0D]);
                          widget.sessionManager.flush(widget.sessionId);
                          _inputController.clear();
                          _lastInputText = '';
                          _inputFocusNode.requestFocus();
                        },
                        keyboardType: TextInputType.visiblePassword,
                        onEditingComplete: () {},
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
                      ),
                    ),
                    IconButton(
                      icon: const Icon(Icons.keyboard_return, color: Colors.white),
                      onPressed: () {
                        _sendInput([0x0D]);
                        widget.sessionManager.flush(widget.sessionId);
                        _inputController.clear();
                        _lastInputText = '';
                      },
                    ),
                  ],
                ),
              ),

            // ── Keyboard bar (animated show/hide) ──
            AnimatedSize(
              duration: const Duration(milliseconds: 200),
              curve: Curves.easeInOut,
              child: _showKeyboardBar
                  ? KeyboardBar(
                      modifiers: _modifiers,
                      onKey: _sendKey,
                      onPaste: _pasteFromClipboard,
                    )
                  : const SizedBox.shrink(),
            ),
          ],
        ), // Column
      ), // SafeArea
      ), // Scaffold
      ), // Focus
    ); // PopScope
  }
}

// ── Custom painter with real FFI data ────────────────────────────────

class _TerminalPainter extends CustomPainter {
  final ScreenSnapshot screen;
  final TerminalTheme theme;
  final double cellWidth;
  final double cellHeight;
  final bool cursorVisible;

  // Reusable Paint objects to avoid per-frame allocation.
  static final _cellBgPaint = Paint();
  static final _cursorRectPaint = Paint();
  static final _cursorBorderPaint = Paint()
    ..style = PaintingStyle.stroke
    ..strokeWidth = 1.5;

  _TerminalPainter({
    required this.screen,
    required this.theme,
    required this.cellWidth,
    required this.cellHeight,
    this.cursorVisible = true,
  });

  @override
  bool shouldRepaint(covariant _TerminalPainter old) {
    // Fast path: if dimensions and cursor state are the same, only
    // repaint if the screen snapshot identity changed.
    // The render timer already checks content hash before calling
    // setState, so identical(screen) is a reliable cheap check.
    if (cellWidth != old.cellWidth ||
        cellHeight != old.cellHeight ||
        cursorVisible != old.cursorVisible) {
      return true;
    }
    // Screen is a different object each frame; compare by reference.
    return !identical(screen, old.screen);
  }

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
    final fontSize = cellHeight * 0.85;

    // Cache Paint objects to avoid per-cell allocation.
    final cellBgPaint = _cellBgPaint;

    // First pass: draw all cell backgrounds (batch fillRect).
    for (var row = 0; row < maxVisibleRows; row++) {
      final y = row * cellHeight;
      for (var col = 0; col < cols; col++) {
        final idx = row * cols + col;
        if (idx >= screen.cells.length) break;
        final cell = screen.cells[idx];
        final x = col * cellWidth;

        // Skip hidden cells (SGR 8).
        if (cell.hidden) continue;

        // For reverse video (SGR 7), draw fg color as background.
        final effectiveBg = cell.reverse ? cell.fgRgb : cell.bgRgb;
        if (effectiveBg != 0) {
          cellBgPaint.color = Color(0xFF000000 | effectiveBg);
          canvas.drawRect(
            Rect.fromLTWH(x, y, cellWidth, cellHeight),
            cellBgPaint,
          );
        }
      }
    }

    // Second pass: batch render text by grouping consecutive cells with
    // identical style (fg color, bold, italic, underline, strikethrough).
    for (var row = 0; row < maxVisibleRows; row++) {
      final y = row * cellHeight;
      var runStart = -1;
      var runText = StringBuffer();
      var runFg = 0;
      var runBold = false;
      var runItalic = false;
      var runUnderline = false;
      var runStrikethrough = false;
      var runDim = false;

      for (var col = 0; col <= cols; col++) {
        final idx = row * cols + col;
        final cell = col < cols && idx < screen.cells.length
            ? screen.cells[idx]
            : null;

        final isEmpty = cell == null || cell.charCode == 0 || cell.hidden;

        if (!isEmpty) {
          // For reverse video, use bg color as text color.
          final effectiveFg = cell.reverse ? cell.bgRgb : cell.fgRgb;
          if (runStart < 0) {
            // Start a new run.
            runStart = col;
            runText.clear();
            runText.write(cell.char);
            runFg = effectiveFg;
            runBold = cell.bold;
            runItalic = cell.italic;
            runUnderline = cell.underline;
            runStrikethrough = cell.strikethrough;
            runDim = cell.dim;
            // Wide chars (CJK, emoji) are 2 cells wide — flush immediately
            // so subsequent text starts at the correct column.
            if (cell.wide) {
              _paintRun(canvas, runText.toString(), runStart, y,
                  runFg, runBold, runItalic, runUnderline,
                  runStrikethrough, runDim, cellWidth, cellHeight, fontSize);
              runStart = -1;
            }
          } else if (effectiveFg == runFg &&
              cell.bold == runBold &&
              cell.italic == runItalic &&
              cell.underline == runUnderline &&
              cell.strikethrough == runStrikethrough &&
              cell.dim == runDim) {
            // Continue current run.
            runText.write(cell.char);
            // Wide chars (CJK, emoji) take 2 cells. Flush immediately
            // so subsequent text starts at the correct column.
            if (cell.wide) {
              _paintRun(canvas, runText.toString(), runStart, y,
                  runFg, runBold, runItalic, runUnderline,
                  runStrikethrough, runDim, cellWidth, cellHeight, fontSize);
              runStart = -1;
            }
          } else {
            // Flush current run, start new one.
            _paintRun(canvas, runText.toString(), runStart, y,
                runFg, runBold, runItalic, runUnderline,
                runStrikethrough, runDim, cellWidth, cellHeight, fontSize);
            runStart = col;
            runText.clear();
            runText.write(cell.char);
            runFg = effectiveFg;
            runBold = cell.bold;
            runItalic = cell.italic;
            runUnderline = cell.underline;
            runStrikethrough = cell.strikethrough;
            runDim = cell.dim;
          }
        } else if (runStart >= 0) {
          // Empty cell — flush current run.
          _paintRun(canvas, runText.toString(), runStart, y,
              runFg, runBold, runItalic, runUnderline,
              runStrikethrough, runDim, cellWidth, cellHeight, fontSize);
          runStart = -1;
        }
      }
    }

    // Third pass: draw cursor (over everything).
    // Semi-transparent so the character underneath remains visible.
    if (cursorVisible) {
      final cy = screen.cursorRow;
      final cx = screen.cursorCol;
      if (cy < maxVisibleRows && cx < cols) {
        final x = cx * cellWidth;
        final y = cy * cellHeight;
        // Draw cursor block with 50% opacity — text underneath remains readable.
        _cursorRectPaint.color = theme.cursor.withValues(alpha: 0.5);
        canvas.drawRect(
          Rect.fromLTWH(x, y, cellWidth, cellHeight),
          _cursorRectPaint,
        );
        // Draw a 1px border for crispness.
        _cursorBorderPaint.color = theme.cursor;
        _cursorBorderPaint.strokeWidth = 1.0;
        canvas.drawRect(
          Rect.fromLTWH(x, y, cellWidth, cellHeight),
          _cursorBorderPaint,
        );
      }
    }
  }

  /// Paint a run of text at the given column.
  void _paintRun(
    Canvas canvas,
    String text,
    int startCol,
    double rowY,
    int fg,
    bool bold,
    bool italic,
    bool underline,
    bool strikethrough,
    bool dim,
    double cellW,
    double cellH,
    double fontSize,
  ) {
    if (text.isEmpty) return;

    var textColor = Color(0xFF000000 | fg);
    // SGR 2 (dim/faint): render at 60% opacity.
    if (dim) textColor = textColor.withValues(alpha: 0.6);

    final style = TextStyle(
      color: textColor,
      fontSize: fontSize,
      fontFamily: 'monospace',
      fontWeight: bold ? FontWeight.bold : FontWeight.normal,
      fontStyle: italic ? FontStyle.italic : FontStyle.normal,
      decoration: TextDecoration.combine([
        if (underline) TextDecoration.underline,
        if (strikethrough) TextDecoration.lineThrough,
      ]),
    );

    final tp = TextPainter(
      text: TextSpan(text: text, style: style),
      textDirection: TextDirection.ltr,
    )..layout();

    final x = startCol * cellW;
    final dy = rowY + (cellH - tp.height) / 2;
    tp.paint(canvas, Offset(x, dy));
  }
}
