//! Default render theme — colors and cursor configuration.
//!
//! Extended in Phase 5 with named color schemes (`NamedTheme`) and a
//! `ThemeManager` for hot-swapping.

use ggterm_core::Color;

/// Cursor display style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorStyle {
    #[default]
    Block,
    Underline,
    Bar,
}

/// Theme controlling default colors and cursor appearance.
#[derive(Debug, Clone)]
pub struct RenderTheme {
    /// Default foreground color (when cell fg = Color::Default).
    pub default_fg: Color,
    /// Default background color (when cell bg = Color::Default).
    pub default_bg: Color,
    /// Cursor foreground (text color inside cursor).
    pub cursor_fg: Color,
    /// Cursor background.
    pub cursor_bg: Color,
    /// Cursor style.
    pub cursor_style: CursorStyle,
    /// 16-color ANSI palette for Indexed(0..15).
    pub palette: [Color; 16],
    /// Selection background color (for mouse-selected text).
    pub selection_bg: Color,
}

impl Default for RenderTheme {
    fn default() -> Self {
        Self::dark_default()
    }
}

impl RenderTheme {
    /// Resolve a `Color` value to an RGB triple.
    pub fn resolve(&self, color: &Color) -> (u8, u8, u8) {
        match color {
            Color::Default => self.default_fg_rgb(),
            Color::Indexed(n) => self.resolve_indexed(*n),
            Color::Rgb(r, g, b) => (*r, *g, *b),
        }
    }

    /// Resolve foreground color, falling back to default_fg.
    pub fn resolve_fg(&self, color: &Color) -> (u8, u8, u8) {
        match color {
            Color::Default => self.default_fg_rgb(),
            Color::Indexed(n) => self.resolve_indexed(*n),
            Color::Rgb(r, g, b) => (*r, *g, *b),
        }
    }

    /// Resolve background color, falling back to default_bg.
    pub fn resolve_bg(&self, color: &Color) -> (u8, u8, u8) {
        match color {
            Color::Default => self.default_bg_rgb(),
            Color::Indexed(n) => self.resolve_indexed(*n),
            Color::Rgb(r, g, b) => (*r, *g, *b),
        }
    }

    fn default_fg_rgb(&self) -> (u8, u8, u8) {
        match self.default_fg {
            Color::Rgb(r, g, b) => (r, g, b),
            Color::Indexed(n) => self.resolve_indexed(n),
            Color::Default => (0xe0, 0xe0, 0xe0),
        }
    }

    fn default_bg_rgb(&self) -> (u8, u8, u8) {
        match self.default_bg {
            Color::Rgb(r, g, b) => (r, g, b),
            Color::Indexed(n) => self.resolve_indexed(n),
            Color::Default => (0x1a, 0x1a, 0x1a),
        }
    }

    fn resolve_indexed(&self, n: u8) -> (u8, u8, u8) {
        match n {
            0..=15 => {
                if let Color::Rgb(r, g, b) = self.palette[n as usize] {
                    (r, g, b)
                } else {
                    (0xff, 0xff, 0xff)
                }
            }
            16..=231 => {
                let idx = (n - 16) as usize;
                let r = idx / 36;
                let g = (idx % 36) / 6;
                let b = idx % 6;
                let component = |v: usize| -> u8 { if v == 0 { 0 } else { 55 + v as u8 * 40 } };
                (component(r), component(g), component(b))
            }
            232..=255 => {
                let v = 8 + (n - 232) * 10;
                (v, v, v)
            }
        }
    }

    // ── Built-in Themes ──────────────────────────────────────────

    /// Dark default theme (matching the original Phase 1 defaults).
    pub fn dark_default() -> Self {
        Self {
            default_fg: Color::Rgb(0xe0, 0xe0, 0xe0),
            default_bg: Color::Rgb(0x1a, 0x1a, 0x1a),
            cursor_fg: Color::Rgb(0x1a, 0x1a, 0x1a),
            cursor_bg: Color::Rgb(0xe0, 0xe0, 0xe0),
            cursor_style: CursorStyle::Block,
            palette: Color::default_palette(),
            selection_bg: Color::Rgb(0x44, 0x44, 0x66),
        }
    }

    /// Light default theme for bright environments.
    pub fn light_default() -> Self {
        Self {
            default_fg: Color::Rgb(0x1a, 0x1a, 0x1a),
            default_bg: Color::Rgb(0xf5, 0xf5, 0xf5),
            cursor_fg: Color::Rgb(0xf5, 0xf5, 0xf5),
            cursor_bg: Color::Rgb(0x1a, 0x1a, 0x1a),
            cursor_style: CursorStyle::Block,
            palette: [
                Color::Rgb(0x00, 0x00, 0x00), // 0  black
                Color::Rgb(0xcc, 0x00, 0x00), // 1  red
                Color::Rgb(0x4e, 0x9a, 0x06), // 2  green
                Color::Rgb(0xc4, 0xa0, 0x00), // 3  yellow
                Color::Rgb(0x34, 0x65, 0xa4), // 4  blue
                Color::Rgb(0x75, 0x50, 0x7b), // 5  magenta
                Color::Rgb(0x06, 0x98, 0x9a), // 6  cyan
                Color::Rgb(0xd3, 0xd7, 0xcf), // 7  white
                Color::Rgb(0x55, 0x57, 0x53), // 8  bright black
                Color::Rgb(0xef, 0x29, 0x29), // 9  bright red
                Color::Rgb(0x8a, 0xe2, 0x34), // 10 bright green
                Color::Rgb(0xfc, 0xe9, 0x4f), // 11 bright yellow
                Color::Rgb(0x72, 0x9f, 0xcf), // 12 bright blue
                Color::Rgb(0xad, 0x7f, 0xa8), // 13 bright magenta
                Color::Rgb(0x34, 0xe2, 0xe2), // 14 bright cyan
                Color::Rgb(0xee, 0xee, 0xec), // 15 bright white
            ],
            selection_bg: Color::Rgb(0xaa, 0xcc, 0xff),
        }
    }

    /// Dracula-inspired dark theme with vibrant accent colors.
    pub fn dracula() -> Self {
        Self {
            default_fg: Color::Rgb(0xf8, 0xf8, 0xf2),
            default_bg: Color::Rgb(0x28, 0x2a, 0x36),
            cursor_fg: Color::Rgb(0x28, 0x2a, 0x36),
            cursor_bg: Color::Rgb(0xf8, 0xf8, 0xf2),
            cursor_style: CursorStyle::Bar,
            palette: [
                Color::Rgb(0x00, 0x00, 0x00), // 0  black
                Color::Rgb(0xff, 0x55, 0x55), // 1  red
                Color::Rgb(0x50, 0xfa, 0x7b), // 2  green
                Color::Rgb(0xf1, 0xfa, 0x8c), // 3  yellow
                Color::Rgb(0xca, 0xa9, 0xfa), // 4  blue
                Color::Rgb(0xff, 0x79, 0xc6), // 5  magenta
                Color::Rgb(0x8b, 0xe9, 0xfd), // 6  cyan
                Color::Rgb(0xbf, 0xbf, 0xbf), // 7  white
                Color::Rgb(0x4d, 0x4d, 0x4d), // 8  bright black
                Color::Rgb(0xff, 0x6e, 0x67), // 9  bright red
                Color::Rgb(0x5a, 0xff, 0x7a), // 10 bright green
                Color::Rgb(0xf4, 0xf9, 0x9f), // 11 bright yellow
                Color::Rgb(0xca, 0xa9, 0xfa), // 12 bright blue
                Color::Rgb(0xff, 0x92, 0xd0), // 13 bright magenta
                Color::Rgb(0x9a, 0xff, 0xed), // 14 bright cyan
                Color::Rgb(0xe6, 0xe6, 0xe6), // 15 bright white
            ],
            selection_bg: Color::Rgb(0x6a, 0x4a, 0x8a),
        }
    }

    /// Look up a built-in theme by name (case-insensitive).
    ///
    /// Returns `Some(theme)` for known names, `None` otherwise.
    /// Supported names: "dark", "light", "dracula".
    pub fn by_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "dark" | "dark-default" | "default" => Some(Self::dark_default()),
            "light" | "light-default" => Some(Self::light_default()),
            "dracula" => Some(Self::dracula()),
            _ => None,
        }
    }

    /// Return all available built-in theme names.
    pub fn builtin_names() -> &'static [&'static str] {
        &["dark", "light", "dracula"]
    }
}

/// Manages the active theme and provides hot-swap functionality.
pub struct ThemeManager {
    current: RenderTheme,
    current_name: String,
}

impl ThemeManager {
    /// Create a new ThemeManager with the given theme.
    pub fn new(theme: RenderTheme, name: impl Into<String>) -> Self {
        Self {
            current: theme,
            current_name: name.into(),
        }
    }

    /// Create a ThemeManager starting with the dark default theme.
    pub fn with_default() -> Self {
        Self::new(RenderTheme::dark_default(), "dark")
    }

    /// Get the current theme.
    pub fn current(&self) -> &RenderTheme {
        &self.current
    }

    /// Get the current theme name.
    pub fn current_name(&self) -> &str {
        &self.current_name
    }

    /// Swap to a new theme. Returns `true` if the theme was found and applied.
    pub fn set_by_name(&mut self, name: &str) -> bool {
        if let Some(theme) = RenderTheme::by_name(name) {
            self.current = theme;
            self.current_name = name.to_lowercase();
            true
        } else {
            false
        }
    }

    /// Set a custom theme directly.
    pub fn set_theme(&mut self, theme: RenderTheme, name: impl Into<String>) {
        self.current = theme;
        self.current_name = name.into();
    }

    /// List all available theme names.
    pub fn available_themes() -> &'static [&'static str] {
        RenderTheme::builtin_names()
    }
}

impl Default for ThemeManager {
    fn default() -> Self {
        Self::with_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_default_is_dark() {
        let theme = RenderTheme::default();
        let (r, g, b) = theme.default_bg_rgb();
        assert!(
            r < 128 && g < 128 && b < 128,
            "default theme should have dark bg"
        );
    }

    #[test]
    fn t_dark_default_colors() {
        let theme = RenderTheme::dark_default();
        let (r, g, b) = theme.resolve_fg(&Color::Default);
        assert!(r > 128, "dark theme fg should be light");
        let (r, g, b) = theme.resolve_bg(&Color::Default);
        assert!(r < 128, "dark theme bg should be dark");
    }

    #[test]
    fn t_light_default_colors() {
        let theme = RenderTheme::light_default();
        let (r, _, _) = theme.resolve_fg(&Color::Default);
        assert!(r < 128, "light theme fg should be dark");
        let (r, _, _) = theme.resolve_bg(&Color::Default);
        assert!(r > 128, "light theme bg should be light");
    }

    #[test]
    fn t_dracula_colors() {
        let theme = RenderTheme::dracula();
        // Dracula background is #282a36
        let (r, g, b) = theme.resolve_bg(&Color::Default);
        assert_eq!((r, g, b), (0x28, 0x2a, 0x36));
        // Dracula foreground is #f8f8f2
        let (r, g, b) = theme.resolve_fg(&Color::Default);
        assert_eq!((r, g, b), (0xf8, 0xf8, 0xf2));
    }

    #[test]
    fn t_dracula_cursor_style_is_bar() {
        let theme = RenderTheme::dracula();
        assert_eq!(theme.cursor_style, CursorStyle::Bar);
    }

    #[test]
    fn t_by_name_dark() {
        let theme = RenderTheme::by_name("dark").unwrap();
        assert_eq!(theme.default_bg, Color::Rgb(0x1a, 0x1a, 0x1a));
    }

    #[test]
    fn t_by_name_light() {
        let theme = RenderTheme::by_name("light").unwrap();
        assert_eq!(theme.default_bg, Color::Rgb(0xf5, 0xf5, 0xf5));
    }

    #[test]
    fn t_by_name_dracula() {
        let theme = RenderTheme::by_name("dracula").unwrap();
        assert_eq!(theme.default_bg, Color::Rgb(0x28, 0x2a, 0x36));
    }

    #[test]
    fn t_by_name_case_insensitive() {
        assert!(RenderTheme::by_name("DARK").is_some());
        assert!(RenderTheme::by_name("Light").is_some());
        assert!(RenderTheme::by_name("DRACULA").is_some());
    }

    #[test]
    fn t_by_name_alias() {
        assert!(RenderTheme::by_name("default").is_some());
        assert!(RenderTheme::by_name("dark-default").is_some());
        assert!(RenderTheme::by_name("light-default").is_some());
    }

    #[test]
    fn t_by_name_unknown() {
        assert!(RenderTheme::by_name("nonexistent").is_none());
    }

    #[test]
    fn t_builtin_names() {
        let names = RenderTheme::builtin_names();
        assert!(names.contains(&"dark"));
        assert!(names.contains(&"light"));
        assert!(names.contains(&"dracula"));
    }

    #[test]
    fn t_resolve_rgb_passthrough() {
        let theme = RenderTheme::dark_default();
        let c = Color::Rgb(0xff, 0x00, 0xff);
        let (r, g, b) = theme.resolve(&c);
        assert_eq!((r, g, b), (0xff, 0x00, 0xff));
    }

    #[test]
    fn t_resolve_indexed_0() {
        let theme = RenderTheme::dark_default();
        let c = Color::Indexed(0);
        let (_, g, b) = theme.resolve(&c);
        // Index 0 = black
        assert_eq!((g, b), (0, 0));
    }

    #[test]
    fn t_resolve_indexed_cube() {
        let theme = RenderTheme::dark_default();
        let c = Color::Indexed(16); // start of color cube
        let (r, g, b) = theme.resolve(&c);
        assert_eq!((r, g, b), (0, 0, 0));
    }

    #[test]
    fn t_resolve_indexed_grayscale() {
        let theme = RenderTheme::dark_default();
        let c = Color::Indexed(232); // start of grayscale
        let (r, g, b) = theme.resolve(&c);
        assert_eq!(r, 8);
        assert_eq!(g, 8);
        assert_eq!(b, 8);
    }

    #[test]
    fn t_selection_bg_dark() {
        let theme = RenderTheme::dark_default();
        assert!(matches!(theme.selection_bg, Color::Rgb(0x44, 0x44, 0x66)));
    }

    #[test]
    fn t_selection_bg_light() {
        let theme = RenderTheme::light_default();
        assert!(matches!(theme.selection_bg, Color::Rgb(0xaa, 0xcc, 0xff)));
    }

    // --- ThemeManager tests ---

    #[test]
    fn t_manager_default() {
        let mgr = ThemeManager::default();
        assert_eq!(mgr.current_name(), "dark");
    }

    #[test]
    fn t_manager_with_default() {
        let mgr = ThemeManager::with_default();
        assert_eq!(mgr.current_name(), "dark");
    }

    #[test]
    fn t_manager_current() {
        let mgr = ThemeManager::with_default();
        let theme = mgr.current();
        assert_eq!(theme.default_bg, Color::Rgb(0x1a, 0x1a, 0x1a));
    }

    #[test]
    fn t_manager_set_by_name_success() {
        let mut mgr = ThemeManager::with_default();
        assert!(mgr.set_by_name("light"));
        assert_eq!(mgr.current_name(), "light");
        assert_eq!(mgr.current().default_bg, Color::Rgb(0xf5, 0xf5, 0xf5));
    }

    #[test]
    fn t_manager_set_by_name_case_insensitive() {
        let mut mgr = ThemeManager::with_default();
        assert!(mgr.set_by_name("DRACULA"));
        assert_eq!(mgr.current_name(), "dracula");
    }

    #[test]
    fn t_manager_set_by_name_unknown() {
        let mut mgr = ThemeManager::with_default();
        assert!(!mgr.set_by_name("nonexistent"));
        // Theme should not change
        assert_eq!(mgr.current_name(), "dark");
    }

    #[test]
    fn t_manager_set_custom_theme() {
        let mut mgr = ThemeManager::with_default();
        let custom = RenderTheme::light_default();
        mgr.set_theme(custom, "custom");
        assert_eq!(mgr.current_name(), "custom");
    }

    #[test]
    fn t_manager_available_themes() {
        let names = ThemeManager::available_themes();
        assert!(names.contains(&"dark"));
        assert!(names.contains(&"light"));
        assert!(names.contains(&"dracula"));
    }

    #[test]
    fn t_manager_swap_multiple() {
        let mut mgr = ThemeManager::with_default();
        mgr.set_by_name("light");
        assert_eq!(mgr.current_name(), "light");
        mgr.set_by_name("dracula");
        assert_eq!(mgr.current_name(), "dracula");
        mgr.set_by_name("dark");
        assert_eq!(mgr.current_name(), "dark");
    }
}
