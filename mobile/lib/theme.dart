/// Terminal theme definitions for GGTerm mobile.
///
/// Mirrors the 9 built-in themes from the desktop renderer
/// (`crates/ggterm-render/src/theme.rs`).

library;
import 'dart:ui';

/// A complete terminal color theme.
class TerminalTheme {
  final String name;
  final Color foreground;
  final Color background;
  final Color cursor;
  final List<Color> palette; // 16 ANSI colors

  const TerminalTheme({
    required this.name,
    required this.foreground,
    required this.background,
    required this.cursor,
    required this.palette,
  });
}

// ── Color helper ──────────────────────────────────────────────────────

// _rgb removed — unused

// ── 9 built-in themes ────────────────────────────────────────────────

/// Default dark theme.
const darkTheme = TerminalTheme(
  name: 'dark',
  foreground: Color.fromARGB(255, 0xc0, 0xc0, 0xc0),
  background: Color.fromARGB(255, 0x00, 0x00, 0x00),
  cursor: Color.fromARGB(255, 0xff, 0xff, 0xff),
  palette: [
    Color.fromARGB(255, 0x00, 0x00, 0x00), // 0  black
    Color.fromARGB(255, 0xcc, 0x00, 0x00), // 1  red
    Color.fromARGB(255, 0x4e, 0x9a, 0x06), // 2  green
    Color.fromARGB(255, 0xc4, 0xa0, 0x00), // 3  yellow
    Color.fromARGB(255, 0x34, 0x65, 0xa4), // 4  blue
    Color.fromARGB(255, 0x75, 0x50, 0x7b), // 5  magenta
    Color.fromARGB(255, 0x06, 0x98, 0x9a), // 6  cyan
    Color.fromARGB(255, 0xd3, 0xd7, 0xcf), // 7  white
    Color.fromARGB(255, 0x55, 0x57, 0x53), // 8  bright black
    Color.fromARGB(255, 0xef, 0x29, 0x29), // 9  bright red
    Color.fromARGB(255, 0x8a, 0xe2, 0x34), // 10 bright green
    Color.fromARGB(255, 0xfc, 0xe9, 0x4f), // 11 bright yellow
    Color.fromARGB(255, 0x72, 0x9f, 0xcf), // 12 bright blue
    Color.fromARGB(255, 0xad, 0x7f, 0xa8), // 13 bright magenta
    Color.fromARGB(255, 0x34, 0xe2, 0xe2), // 14 bright cyan
    Color.fromARGB(255, 0xee, 0xee, 0xec), // 15 bright white
  ],
);

/// Light theme.
const lightTheme = TerminalTheme(
  name: 'light',
  foreground: Color.fromARGB(255, 0x28, 0x28, 0x28),
  background: Color.fromARGB(255, 0xfa, 0xfa, 0xfa),
  cursor: Color.fromARGB(255, 0x28, 0x28, 0x28),
  palette: [
    Color.fromARGB(255, 0x00, 0x00, 0x00),
    Color.fromARGB(255, 0xcc, 0x00, 0x00),
    Color.fromARGB(255, 0x00, 0x80, 0x00),
    Color.fromARGB(255, 0x80, 0x80, 0x00),
    Color.fromARGB(255, 0x00, 0x00, 0xcc),
    Color.fromARGB(255, 0xcc, 0x00, 0xcc),
    Color.fromARGB(255, 0x00, 0x80, 0x80),
    Color.fromARGB(255, 0xc0, 0xc0, 0xc0),
    Color.fromARGB(255, 0x80, 0x80, 0x80),
    Color.fromARGB(255, 0xff, 0x00, 0x00),
    Color.fromARGB(255, 0x00, 0xff, 0x00),
    Color.fromARGB(255, 0xff, 0xff, 0x00),
    Color.fromARGB(255, 0x00, 0x00, 0xff),
    Color.fromARGB(255, 0xff, 0x00, 0xff),
    Color.fromARGB(255, 0x00, 0xff, 0xff),
    Color.fromARGB(255, 0xff, 0xff, 0xff),
  ],
);

/// Dracula theme.
const draculaTheme = TerminalTheme(
  name: 'dracula',
  foreground: Color.fromARGB(255, 0xf8, 0xf8, 0xf2),
  background: Color.fromARGB(255, 0x28, 0x2a, 0x36),
  cursor: Color.fromARGB(255, 0xff, 0xff, 0xff),
  palette: [
    Color.fromARGB(255, 0x00, 0x00, 0x00),
    Color.fromARGB(255, 0xff, 0x55, 0x55),
    Color.fromARGB(255, 0x50, 0xfa, 0x7b),
    Color.fromARGB(255, 0xf1, 0xfa, 0x8c),
    Color.fromARGB(255, 0x6a, 0xbf, 0xff),
    Color.fromARGB(255, 0xff, 0x79, 0xc6),
    Color.fromARGB(255, 0x8b, 0xe9, 0xfd),
    Color.fromARGB(255, 0xb0, 0xb0, 0xb0),
    Color.fromARGB(255, 0x28, 0x2a, 0x36),
    Color.fromARGB(255, 0xff, 0x6e, 0x67),
    Color.fromARGB(255, 0x5a, 0xff, 0x7c),
    Color.fromARGB(255, 0xf4, 0xf9, 0x9f),
    Color.fromARGB(255, 0xca, 0xa9, 0xfa),
    Color.fromARGB(255, 0xff, 0x92, 0xd0),
    Color.fromARGB(255, 0x9a, 0xed, 0xfe),
    Color.fromARGB(255, 0xff, 0xff, 0xff),
  ],
);

/// Solarized Dark theme.
const solarizedDarkTheme = TerminalTheme(
  name: 'solarized-dark',
  foreground: Color.fromARGB(255, 0x83, 0x94, 0x96),
  background: Color.fromARGB(255, 0x00, 0x2b, 0x36),
  cursor: Color.fromARGB(255, 0xee, 0xe8, 0xd5),
  palette: [
    Color.fromARGB(255, 0x07, 0x36, 0x42),
    Color.fromARGB(255, 0xdc, 0x32, 0x2f),
    Color.fromARGB(255, 0x85, 0x99, 0x00),
    Color.fromARGB(255, 0xb5, 0x89, 0x00),
    Color.fromARGB(255, 0x26, 0x8b, 0xd2),
    Color.fromARGB(255, 0xd3, 0x36, 0x82),
    Color.fromARGB(255, 0x2a, 0xa1, 0x98),
    Color.fromARGB(255, 0xee, 0xe8, 0xd5),
    Color.fromARGB(255, 0x00, 0x2b, 0x36),
    Color.fromARGB(255, 0xcb, 0x4b, 0x16),
    Color.fromARGB(255, 0x58, 0x6e, 0x75),
    Color.fromARGB(255, 0x82, 0x86, 0x00),
    Color.fromARGB(255, 0x83, 0x94, 0x96),
    Color.fromARGB(255, 0x6c, 0x71, 0xc4),
    Color.fromARGB(255, 0x93, 0xa1, 0xa1),
    Color.fromARGB(255, 0xfd, 0xf6, 0xe3),
  ],
);

/// Solarized Light theme.
const solarizedLightTheme = TerminalTheme(
  name: 'solarized-light',
  foreground: Color.fromARGB(255, 0x65, 0x7b, 0x83),
  background: Color.fromARGB(255, 0xfd, 0xf6, 0xe3),
  cursor: Color.fromARGB(255, 0x07, 0x36, 0x42),
  palette: [
    Color.fromARGB(255, 0x07, 0x36, 0x42),
    Color.fromARGB(255, 0xdc, 0x32, 0x2f),
    Color.fromARGB(255, 0x85, 0x99, 0x00),
    Color.fromARGB(255, 0xb5, 0x89, 0x00),
    Color.fromARGB(255, 0x26, 0x8b, 0xd2),
    Color.fromARGB(255, 0xd3, 0x36, 0x82),
    Color.fromARGB(255, 0x2a, 0xa1, 0x98),
    Color.fromARGB(255, 0xee, 0xe8, 0xd5),
    Color.fromARGB(255, 0x00, 0x2b, 0x36),
    Color.fromARGB(255, 0xcb, 0x4b, 0x16),
    Color.fromARGB(255, 0x58, 0x6e, 0x75),
    Color.fromARGB(255, 0x82, 0x86, 0x00),
    Color.fromARGB(255, 0x83, 0x94, 0x96),
    Color.fromARGB(255, 0x6c, 0x71, 0xc4),
    Color.fromARGB(255, 0x93, 0xa1, 0xa1),
    Color.fromARGB(255, 0xfd, 0xf6, 0xe3),
  ],
);

/// Gruvbox theme.
const gruvboxTheme = TerminalTheme(
  name: 'gruvbox',
  foreground: Color.fromARGB(255, 0xeb, 0xdb, 0xb2),
  background: Color.fromARGB(255, 0x28, 0x28, 0x28),
  cursor: Color.fromARGB(255, 0xeb, 0xdb, 0xb2),
  palette: [
    Color.fromARGB(255, 0x28, 0x28, 0x28),
    Color.fromARGB(255, 0xcc, 0x24, 0x1d),
    Color.fromARGB(255, 0x98, 0x97, 0x1a),
    Color.fromARGB(255, 0xd7, 0x99, 0x21),
    Color.fromARGB(255, 0x45, 0x85, 0x88),
    Color.fromARGB(255, 0xb1, 0x62, 0x86),
    Color.fromARGB(255, 0x68, 0x9d, 0x6a),
    Color.fromARGB(255, 0xa8, 0x99, 0x84),
    Color.fromARGB(255, 0x92, 0x83, 0x74),
    Color.fromARGB(255, 0xfb, 0x49, 0x34),
    Color.fromARGB(255, 0xb8, 0xbb, 0x26),
    Color.fromARGB(255, 0xfa, 0xbd, 0x2f),
    Color.fromARGB(255, 0x83, 0xa5, 0x98),
    Color.fromARGB(255, 0xd3, 0x86, 0x9b),
    Color.fromARGB(255, 0x8e, 0xc0, 0x7c),
    Color.fromARGB(255, 0xeb, 0xdb, 0xb2),
  ],
);

/// Nord theme — Arctic, north-bluish color palette.
const nordTheme = TerminalTheme(
  name: 'nord',
  foreground: Color.fromARGB(255, 0xd8, 0xde, 0xe9),
  background: Color.fromARGB(255, 0x2e, 0x34, 0x40),
  cursor: Color.fromARGB(255, 0xd8, 0xde, 0xe9),
  palette: [
    Color.fromARGB(255, 0x3b, 0x42, 0x52), // 0  black
    Color.fromARGB(255, 0xbf, 0x61, 0x6a), // 1  red
    Color.fromARGB(255, 0xa3, 0xbe, 0x8c), // 2  green
    Color.fromARGB(255, 0xeb, 0xcb, 0x8b), // 3  yellow
    Color.fromARGB(255, 0x81, 0xa1, 0xc1), // 4  blue
    Color.fromARGB(255, 0xb4, 0x8e, 0xad), // 5  magenta
    Color.fromARGB(255, 0x88, 0xc0, 0xd0), // 6  cyan
    Color.fromARGB(255, 0xe5, 0xe9, 0xf0), // 7  white
    Color.fromARGB(255, 0x4c, 0x56, 0x6a), // 8  bright black
    Color.fromARGB(255, 0xbf, 0x61, 0x6a), // 9  bright red
    Color.fromARGB(255, 0xa3, 0xbe, 0x8c), // 10 bright green
    Color.fromARGB(255, 0xeb, 0xcb, 0x8b), // 11 bright yellow
    Color.fromARGB(255, 0x81, 0xa1, 0xc1), // 12 bright blue
    Color.fromARGB(255, 0xb4, 0x8e, 0xad), // 13 bright magenta
    Color.fromARGB(255, 0x8f, 0xbc, 0xbb), // 14 bright cyan
    Color.fromARGB(255, 0xe5, 0xe9, 0xf0), // 15 bright white
  ],
);

/// Tokyo Night theme.
const tokyoNightTheme = TerminalTheme(
  name: 'tokyo-night',
  foreground: Color.fromARGB(255, 0xa9, 0xb1, 0xd6),
  background: Color.fromARGB(255, 0x1a, 0x1b, 0x26),
  cursor: Color.fromARGB(255, 0xc0, 0xca, 0xf5),
  palette: [
    Color.fromARGB(255, 0x15, 0x16, 0x1e),
    Color.fromARGB(255, 0xf7, 0x76, 0x8e),
    Color.fromARGB(255, 0x9e, 0xce, 0x6a),
    Color.fromARGB(255, 0xe0, 0xaf, 0x68),
    Color.fromARGB(255, 0x7a, 0xa2, 0xf7),
    Color.fromARGB(255, 0xbb, 0x9a, 0xf7),
    Color.fromARGB(255, 0x7d, 0xc1, 0xd7),
    Color.fromARGB(255, 0xa9, 0xb1, 0xd6),
    Color.fromARGB(255, 0x41, 0x47, 0x57),
    Color.fromARGB(255, 0xf7, 0x76, 0x8e),
    Color.fromARGB(255, 0x9e, 0xce, 0x6a),
    Color.fromARGB(255, 0xe0, 0xaf, 0x68),
    Color.fromARGB(255, 0x7a, 0xa2, 0xf7),
    Color.fromARGB(255, 0xbb, 0x9a, 0xf7),
    Color.fromARGB(255, 0x7d, 0xc1, 0xd7),
    Color.fromARGB(255, 0xc0, 0xca, 0xf5),
  ],
);

/// Catppuccin Mocha theme.
const catppuccinMochaTheme = TerminalTheme(
  name: 'catppuccin-mocha',
  foreground: Color.fromARGB(255, 0xcd, 0xd6, 0xf4),
  background: Color.fromARGB(255, 0x1e, 0x1e, 0x2e),
  cursor: Color.fromARGB(255, 0xf5, 0xe0, 0xdc),
  palette: [
    Color.fromARGB(255, 0x45, 0x47, 0x59),
    Color.fromARGB(255, 0xf3, 0x8b, 0xa8),
    Color.fromARGB(255, 0xa6, 0xe3, 0xa1),
    Color.fromARGB(255, 0xf9, 0xe2, 0xaf),
    Color.fromARGB(255, 0x89, 0xb4, 0xfa),
    Color.fromARGB(255, 0xfa, 0xe3, 0xb0),
    Color.fromARGB(255, 0x94, 0xe2, 0xd5),
    Color.fromARGB(255, 0xba, 0xc2, 0xde),
    Color.fromARGB(255, 0x58, 0x5b, 0x70),
    Color.fromARGB(255, 0xf3, 0x8b, 0xa8),
    Color.fromARGB(255, 0xa6, 0xe3, 0xa1),
    Color.fromARGB(255, 0xf9, 0xe2, 0xaf),
    Color.fromARGB(255, 0x89, 0xb4, 0xfa),
    Color.fromARGB(255, 0xf5, 0xc2, 0xe7),
    Color.fromARGB(255, 0x94, 0xe2, 0xd5),
    Color.fromARGB(255, 0xc6, 0xd0, 0xf5),
  ],
);

/// All 9 built-in themes, keyed by name.
final Map<String, TerminalTheme> builtinThemes = {
  'dark': darkTheme,
  'light': lightTheme,
  'dracula': draculaTheme,
  'solarized-dark': solarizedDarkTheme,
  'solarized-light': solarizedLightTheme,
  'gruvbox': gruvboxTheme,
  'nord': nordTheme,
  'tokyo-night': tokyoNightTheme,
  'catppuccin-mocha': catppuccinMochaTheme,
};

/// Ordered list of all built-in theme names.
const List<String> builtinThemeNames = [
  'dark',
  'light',
  'dracula',
  'solarized-dark',
  'solarized-light',
  'gruvbox',
  'nord',
  'tokyo-night',
  'catppuccin-mocha',
];

/// Look up a theme by name (case-insensitive).
TerminalTheme themeByName(String name) {
  final key = name.toLowerCase();
  return builtinThemes[key] ?? darkTheme;
}
