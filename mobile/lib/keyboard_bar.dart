/// Virtual modifier key bar displayed above the system soft keyboard.
///
/// Modifier keys (Ctrl, Alt, Shift, Tab, Esc, arrows) are shown as
/// toggleable buttons. When active, a modifier stays highlighted until
/// the next regular key press, then auto-releases.

library;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

/// Callback when a special key is pressed.
typedef KeyCallback = void Function(String keyName);

/// Callback when paste is requested.
typedef PasteCallback = void Function();

/// State of active modifiers.
class ModifierState extends ChangeNotifier {
  bool _ctrl = false;
  bool _alt = false;
  bool _shift = false;

  bool get ctrl => _ctrl;
  bool get alt => _alt;
  bool get shift => _shift;

  void toggleCtrl() {
    _ctrl = !_ctrl;
    notifyListeners();
  }

  void toggleAlt() {
    _alt = !_alt;
    notifyListeners();
  }

  void toggleShift() {
    _shift = !_shift;
    notifyListeners();
  }

  /// Release all modifiers (called after a regular key press).
  void releaseAll() {
    _ctrl = false;
    _alt = false;
    _shift = false;
    notifyListeners();
  }

  /// Build a modifier prefix string (e.g. "Ctrl+Alt+").
  String get prefix {
    final parts = <String>[];
    if (_ctrl) parts.add('Ctrl');
    if (_alt) parts.add('Alt');
    if (_shift) parts.add('Shift');
    return parts.isEmpty ? '' : '${parts.join('+')}+';
  }
}

class KeyboardBar extends StatefulWidget {
  final ModifierState modifiers;
  final KeyCallback onKey;
  final PasteCallback? onPaste;

  const KeyboardBar({
    super.key,
    required this.modifiers,
    required this.onKey,
    this.onPaste,
  });

  @override
  State<KeyboardBar> createState() => _KeyboardBarState();
}

class _KeyboardBarState extends State<KeyboardBar> {
  @override
  void initState() {
    super.initState();
    widget.modifiers.addListener(_onModChange);
  }

  @override
  void dispose() {
    widget.modifiers.removeListener(_onModChange);
    super.dispose();
  }

  void _onModChange() => setState(() {});

  void _sendKey(String name) {
    if (name == '__paste__') {
      widget.onPaste?.call();
      return;
    }
    widget.onKey(name);
    // Auto-release modifiers after a non-modifier key.
    widget.modifiers.releaseAll();
  }

  Widget _modifierButton({
    required String label,
    required bool active,
    required VoidCallback onTap,
  }) {
    return GestureDetector(
      onTap: () {
        HapticFeedback.selectionClick();
        onTap();
      },
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
        decoration: BoxDecoration(
          color: active ? Colors.blue.withValues(alpha: 0.3) : Colors.grey.shade800,
          borderRadius: BorderRadius.circular(6),
          border: active
              ? Border.all(color: Colors.blue, width: 1.5)
              : Border.all(color: Colors.grey.shade600, width: 0.5),
        ),
        child: Text(
          label,
          style: TextStyle(
            color: active ? Colors.blue : Colors.grey.shade300,
            fontSize: 13,
            fontWeight: active ? FontWeight.bold : FontWeight.normal,
          ),
        ),
      ),
    );
  }

  Widget _keyButton(String label, String keyName) {
    return GestureDetector(
      onTap: () {
        HapticFeedback.selectionClick();
        _sendKey(keyName);
      },
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
        decoration: BoxDecoration(
          color: Colors.grey.shade800,
          borderRadius: BorderRadius.circular(6),
          border: Border.all(color: Colors.grey.shade600, width: 0.5),
        ),
        child: Text(
          label,
          style: TextStyle(
            color: Colors.grey.shade300,
            fontSize: 13,
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final m = widget.modifiers;

    return Container(
      height: 44,
      color: Colors.grey.shade900,
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 6),
        child: Row(
          spacing: 4,
          children: [
            // Modifier toggles
            _modifierButton(
              label: 'Ctrl',
              active: m.ctrl,
              onTap: m.toggleCtrl,
            ),
            _modifierButton(
              label: 'Alt',
              active: m.alt,
              onTap: m.toggleAlt,
            ),
            _modifierButton(
              label: 'Shift',
              active: m.shift,
              onTap: m.toggleShift,
            ),
            // Paste button — reads system clipboard and sends to terminal.
            if (widget.onPaste != null)
              _keyButton('Paste', '__paste__'),
            // Separator
            Container(
              width: 1,
              height: 24,
              color: Colors.grey.shade700,
            ),
            // Special keys
            _keyButton('Enter', 'Enter'),
            _keyButton('Esc', 'Escape'),
            _keyButton('Tab', 'Tab'),
            _keyButton('^C', 'CtrlC'), // SIGINT
            _keyButton('^D', 'CtrlD'), // EOF
            _keyButton('^Z', 'CtrlZ'), // SIGTSTP
            _keyButton(r'^\', r'CtrlBackslash'), // SIGQUIT
            _keyButton('^U', 'CtrlU'), // clear line
            // Separator
            Container(
              width: 1,
              height: 24,
              color: Colors.grey.shade700,
            ),
            // Arrow keys
            _keyButton('←', 'Left'),
            _keyButton('↓', 'Down'),
            _keyButton('↑', 'Up'),
            _keyButton('→', 'Right'),
            // Separator
            Container(
              width: 1,
              height: 24,
              color: Colors.grey.shade700,
            ),
            // Page keys
            _keyButton('PgUp', 'PageUp'),
            _keyButton('PgDn', 'PageDown'),
            _keyButton('Home', 'Home'),
            _keyButton('End', 'End'),
            // Separator
            Container(
              width: 1,
              height: 24,
              color: Colors.grey.shade700,
            ),
            // Quick-access terminal symbols (painful to type on mobile keyboard)
            _keyButton('/', '/'),
            _keyButton('~', '~'),
            _keyButton('|', '|'),
            _keyButton('-', '-'),
            _keyButton('..', '..'),
            _keyButton('*', '*'),
            _keyButton('\$', '\$'),
            _keyButton('&&', '&&'),
            // Programming symbols
            _keyButton('{', '{'),
            _keyButton('}', '}'),
            _keyButton('[', '['),
            _keyButton(']', ']'),
            _keyButton(';', ';'),
            _keyButton('>', '>'),
            _keyButton('#', '#'),
            _keyButton('=', '='),
            // Separator
            Container(
              width: 1,
              height: 24,
              color: Colors.grey.shade700,
            ),
            // Common Ctrl combos
            _keyButton('^L', 'CtrlL'), // clear screen
            _keyButton('^R', 'CtrlR'), // reverse search
            _keyButton('^W', 'CtrlW'), // delete word
            _keyButton('^A', 'CtrlA'), // start of line
            _keyButton('^E', 'CtrlE'), // end of line
            // Separator
            Container(
              width: 1,
              height: 24,
              color: Colors.grey.shade700,
            ),
            // F-keys (essential for vim, htop, man)
            _keyButton('F1', 'F1'),
            _keyButton('F2', 'F2'),
            _keyButton('F3', 'F3'),
            _keyButton('F4', 'F4'),
            _keyButton('F5', 'F5'),
            _keyButton('F6', 'F6'),
            _keyButton('F7', 'F7'),
            _keyButton('F8', 'F8'),
            _keyButton('F9', 'F9'),
            _keyButton('F10', 'F10'),
            _keyButton('F11', 'F11'),
            _keyButton('F12', 'F12'),
          ],
        ),
      ),
    );
  }
}
