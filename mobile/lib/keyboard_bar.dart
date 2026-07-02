/// Virtual modifier key bar displayed above the system soft keyboard.
///
/// Modifier keys (Ctrl, Alt, Shift, Tab, Esc, arrows) are shown as
/// toggleable buttons. When active, a modifier stays highlighted until
/// the next regular key press, then auto-releases.

import 'package:flutter/material.dart';

/// Callback when a special key is pressed.
typedef KeyCallback = void Function(String keyName);

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

  const KeyboardBar({
    super.key,
    required this.modifiers,
    required this.onKey,
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
      onTap: onTap,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
        decoration: BoxDecoration(
          color: active ? Colors.blue.withOpacity(0.3) : Colors.grey.shade800,
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
      onTap: () => _sendKey(keyName),
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
            // Separator
            Container(
              width: 1,
              height: 24,
              color: Colors.grey.shade700,
            ),
            // Special keys
            _keyButton('Esc', 'Escape'),
            _keyButton('Tab', 'Tab'),
            _keyButton('^C', 'CtrlC'), // SIGINT
            _keyButton('^D', 'CtrlD'), // EOF
            _keyButton('^Z', 'CtrlZ'), // SIGTSTP
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
          ],
        ),
      ),
    );
  }
}
