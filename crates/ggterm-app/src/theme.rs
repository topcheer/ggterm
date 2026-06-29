//! App-level theme management.
//!
//! Bridges the render-level `ThemeManager` with the application event loop.
//! When a theme is swapped, listeners are notified so they can re-render.

use ggterm_render::{RenderTheme, ThemeManager};

/// Callback type invoked when the theme changes.
pub type ThemeChangeCallback = Box<dyn Fn(&RenderTheme) + Send>;

/// App-level theme manager with notification support.
pub struct AppTheme {
    inner: ThemeManager,
    on_change: Option<ThemeChangeCallback>,
}

impl AppTheme {
    /// Create with the dark default theme.
    pub fn new() -> Self {
        Self {
            inner: ThemeManager::with_default(),
            on_change: None,
        }
    }

    /// Create with a specific starting theme.
    pub fn with_theme(theme: RenderTheme, name: impl Into<String>) -> Self {
        Self {
            inner: ThemeManager::new(theme, name),
            on_change: None,
        }
    }

    /// Register a callback fired when the theme changes.
    pub fn on_change(&mut self, f: ThemeChangeCallback) {
        self.on_change = Some(f);
    }

    /// Swap theme by name. Returns `true` on success.
    /// Fires the change callback if the theme was found.
    pub fn set_by_name(&mut self, name: &str) -> bool {
        if self.inner.set_by_name(name) {
            if let Some(ref f) = self.on_change {
                f(self.inner.current());
            }
            true
        } else {
            false
        }
    }

    /// Set a custom theme directly.
    pub fn set_theme(&mut self, theme: RenderTheme, name: impl Into<String>) {
        self.inner.set_theme(theme, name);
        if let Some(ref f) = self.on_change {
            f(self.inner.current());
        }
    }

    /// Get current theme reference.
    pub fn current(&self) -> &RenderTheme {
        self.inner.current()
    }

    /// Get current theme name.
    pub fn current_name(&self) -> &str {
        self.inner.current_name()
    }

    /// Cycle to the next built-in theme (wraps around).
    pub fn cycle_next(&mut self) {
        let names = ThemeManager::available_themes();
        let current = self.inner.current_name();
        let idx = names.iter().position(|&n| n == current).unwrap_or(0);
        let next_idx = (idx + 1) % names.len();
        self.set_by_name(names[next_idx]);
    }

    /// List available theme names.
    pub fn available() -> &'static [&'static str] {
        ThemeManager::available_themes()
    }
}

impl Default for AppTheme {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn t_app_theme_default() {
        let at = AppTheme::new();
        assert_eq!(at.current_name(), "dark");
    }

    #[test]
    fn t_app_theme_set_by_name() {
        let mut at = AppTheme::new();
        assert!(at.set_by_name("light"));
        assert_eq!(at.current_name(), "light");
    }

    #[test]
    fn t_app_theme_set_unknown() {
        let mut at = AppTheme::new();
        assert!(!at.set_by_name("nonexistent"));
        assert_eq!(at.current_name(), "dark");
    }

    #[test]
    fn t_app_theme_set_custom() {
        let mut at = AppTheme::new();
        at.set_theme(RenderTheme::dracula(), "my-theme");
        assert_eq!(at.current_name(), "my-theme");
    }

    #[test]
    fn t_app_theme_on_change_fired() {
        let call_count = Arc::new(Mutex::new(0));
        let cc = Arc::clone(&call_count);
        let mut at = AppTheme::new();
        at.on_change(Box::new(move |_| {
            *cc.lock().unwrap() += 1;
        }));
        at.set_by_name("light");
        assert_eq!(*call_count.lock().unwrap(), 1);
    }

    #[test]
    fn t_app_theme_on_change_not_fired_on_fail() {
        let call_count = Arc::new(Mutex::new(0));
        let cc = Arc::clone(&call_count);
        let mut at = AppTheme::new();
        at.on_change(Box::new(move |_| {
            *cc.lock().unwrap() += 1;
        }));
        at.set_by_name("nonexistent");
        assert_eq!(*call_count.lock().unwrap(), 0);
    }

    #[test]
    fn t_app_theme_on_change_fires_on_custom() {
        let call_count = Arc::new(Mutex::new(0));
        let cc = Arc::clone(&call_count);
        let mut at = AppTheme::new();
        at.on_change(Box::new(move |_| {
            *cc.lock().unwrap() += 1;
        }));
        at.set_theme(RenderTheme::dracula(), "custom");
        assert_eq!(*call_count.lock().unwrap(), 1);
    }

    #[test]
    fn t_app_theme_cycle_next() {
        let mut at = AppTheme::new();
        assert_eq!(at.current_name(), "dark");
        // Cycle through all 6 themes.
        let names = ThemeManager::available_themes();
        for &expected in &names[1..] {
            at.cycle_next();
            assert_eq!(at.current_name(), expected);
        }
        // Wrap back to dark.
        at.cycle_next();
        assert_eq!(at.current_name(), "dark");
    }

    #[test]
    fn t_app_theme_available() {
        let names = AppTheme::available();
        assert!(names.contains(&"dark"));
        assert!(names.contains(&"light"));
        assert!(names.contains(&"dracula"));
    }

    #[test]
    fn t_app_theme_current_ref() {
        let at = AppTheme::new();
        let theme = at.current();
        assert!(theme.default_fg != theme.default_bg);
    }

    #[test]
    fn t_app_theme_with_theme_constructor() {
        let at = AppTheme::with_theme(RenderTheme::dracula(), "startup-dracula");
        assert_eq!(at.current_name(), "startup-dracula");
        let theme = at.current();
        assert_eq!(theme.default_bg, RenderTheme::dracula().default_bg);
    }
}
