P9-F Keyboard Input Refinement complete (commit 6f6b745):
- input.rs encode_char(): Ctrl+Space→NUL, Ctrl+Enter→LF, Ctrl+Backspace→^W, Shift+Tab→CSI Z
- ctrl_char() helper maps Ctrl+punctuation/digits to ASCII control codes
- keymap.rs: Backspace now sends \x08 (not \x7f) so encoder can distinguish Ctrl+Backspace
- NumpadEnter mapping added
- App::terminal_mut() added (for P9-D mouse scroll)
- 136 input+keymap tests (up from 73), 1102 total tests
- Also fixed P9-D build issues: mouse.rs clippy, window.rs winit API mismatches