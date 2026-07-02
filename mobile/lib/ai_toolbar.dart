/// AI toolbar with 4 action buttons and response overlay.
///
/// Buttons: Explain, Suggest, Help, NL→Command.
/// Each triggers an AI request via the session manager and displays
/// the response in a dismissible overlay panel.

import 'package:flutter/material.dart';

/// AI action types matching the desktop `ggterm_ai::Action` enum.
enum AiAction {
  explain('Explain'),
  suggest('Suggest'),
  help('Help'),
  nlToCommand('NL→Cmd');

  final String label;
  const AiAction(this.label);
}

/// Callback for triggering an AI request.
typedef AiRequestCallback = void Function(AiAction action);

class AiToolbar extends StatefulWidget {
  final AiRequestCallback onRequest;

  const AiToolbar({
    super.key,
    required this.onRequest,
  });

  @override
  State<AiToolbar> createState() => _AiToolbarState();
}

class _AiToolbarState extends State<AiToolbar> {
  bool _loading = false;
  String? _response;
  String? _error;

  void _handleAction(AiAction action) {
    setState(() {
      _loading = true;
      _response = null;
      _error = null;
    });

    widget.onRequest(action);

    // In a real implementation, the caller would update this widget
    // via a stream or callback. For now, simulate a response.
    Future.delayed(const Duration(milliseconds: 300), () {
      if (mounted) {
        setState(() {
          _loading = false;
          _response = 'AI response for ${action.label}...';
        });
      }
    });
  }

  /// Set response text from external source (e.g. flutter_rust_bridge stream).
  void setResponse(String text) {
    setState(() {
      _loading = false;
      _response = text;
      _error = null;
    });
  }

  /// Set error message.
  void setError(String message) {
    setState(() {
      _loading = false;
      _error = message;
    });
  }

  void _dismiss() {
    setState(() {
      _response = null;
      _error = null;
    });
  }

  Widget _actionButton(AiAction action, IconData icon) {
    return Expanded(
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 2),
        child: FilledButton.tonalIcon(
          onPressed: _loading ? null : () => _handleAction(action),
          icon: Icon(icon, size: 18),
          label: Text(
            action.label,
            style: const TextStyle(fontSize: 12),
          ),
          style: FilledButton.styleFrom(
            padding: const EdgeInsets.symmetric(vertical: 8),
            minimumSize: const Size(0, 36),
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        // ── Action button row ──
        Container(
          padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 4),
          color: Colors.grey.shade900,
          child: Row(
            children: [
              _actionButton(AiAction.explain, Icons.lightbulb_outline),
              _actionButton(AiAction.suggest, Icons.auto_fix_high),
              _actionButton(AiAction.help, Icons.help_outline),
              _actionButton(AiAction.nlToCommand, Icons.terminal),
            ],
          ),
        ),

        // ── Loading indicator ──
        if (_loading)
          LinearProgressIndicator(
            backgroundColor: Colors.grey.shade800,
            minHeight: 2,
          ),

        // ── Error banner ──
        if (_error != null)
          Container(
            width: double.infinity,
            padding: const EdgeInsets.all(8),
            color: Colors.red.shade900,
            child: Row(
              children: [
                Icon(Icons.error_outline, color: Colors.red.shade200, size: 18),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    _error!,
                    style: TextStyle(color: Colors.red.shade100, fontSize: 13),
                  ),
                ),
                GestureDetector(
                  onTap: _dismiss,
                  child: Icon(Icons.close,
                      color: Colors.red.shade200, size: 18),
                ),
              ],
            ),
          ),

        // ── Response overlay panel ──
        if (_response != null)
          Container(
            width: double.infinity,
            constraints: const BoxConstraints(maxHeight: 200),
            padding: const EdgeInsets.all(12),
            decoration: BoxDecoration(
              color: Colors.grey.shade800,
              border: Border(
                top: BorderSide(color: Colors.blue.withOpacity(0.3)),
              ),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    Icon(Icons.smart_toy,
                        color: Colors.blue.shade300, size: 18),
                    const SizedBox(width: 8),
                    Text(
                      'AI Response',
                      style: TextStyle(
                        color: Colors.blue.shade300,
                        fontSize: 13,
                        fontWeight: FontWeight.bold,
                      ),
                    ),
                    const Spacer(),
                    GestureDetector(
                      onTap: _dismiss,
                      child: Icon(Icons.close,
                          color: Colors.grey.shade400, size: 18),
                    ),
                  ],
                ),
                const SizedBox(height: 8),
                Expanded(
                  child: SingleChildScrollView(
                    child: SelectableText(
                      _response!,
                      style: const TextStyle(
                        color: Colors.white70,
                        fontSize: 14,
                        fontFamily: 'monospace',
                        height: 1.4,
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ),
      ],
    );
  }
}
