//! Modern semantic UI color palette for overlay elements.
//!
//! Provides [`UiPalette`] — a structured set of semantic colors used by the
//! tab bar, pane borders, status bar, and dialog overlays.  The default
//! palette is based on **Tokyo Night** (#1a1b26 base), giving a richer dark
//! tone than pure black.
//!
//! This module is intentionally separate from `crate::theme` (terminal ANSI
//! colors) and `ggterm_render::theme::RenderTheme` (GPU text colors).
//! It deals purely with chrome / overlay UI.

// ── UiPalette ──────────────────────────────────────────────────────────

/// Semantic color values as linear RGB floats (0.0–1.0) for direct use
/// in wgpu vertex buffers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiPalette {
    // ── Surfaces ──
    /// Window-level background behind everything.
    pub app_background: [f32; 4],
    /// Tab bar / status bar surface (semi-transparent over app bg).
    pub surface: [f32; 4],
    /// Hovered surface (slightly lighter).
    pub surface_hover: [f32; 4],
    /// Dialog / popover background.
    pub dialog_bg: [f32; 4],

    // ── Borders ──
    /// Default separator border.
    pub border: [f32; 4],
    /// Inactive pane border (dim).
    pub border_inactive: [f32; 4],
    /// Active pane border (bright accent).
    pub border_active: [f32; 4],

    // ── Accent ──
    /// Primary accent color (buttons, highlights).
    pub accent: [f32; 4],
    /// Glow color for active tab / pane (blended behind active border).
    pub accent_glow: [f32; 4],

    // ── Text ──
    /// Primary text on surfaces.
    pub text_primary: [f32; 4],
    /// Secondary / muted text.
    pub text_secondary: [f32; 4],
    /// Disabled / placeholder text.
    pub text_muted: [f32; 4],

    // ── Tab-specific ──
    /// Active tab background.
    pub tab_bg_active: [f32; 4],
    /// Inactive tab background.
    pub tab_bg_inactive: [f32; 4],
    /// Close button normal color.
    pub tab_close: [f32; 4],
    /// Close button hover color.
    pub tab_close_hover: [f32; 4],

    // ── Status colors ──
    pub success: [f32; 4],
    pub warning: [f32; 4],
    pub error: [f32; 4],
}

impl UiPalette {
    /// Tokyo Night inspired dark palette.
    ///
    /// Base: #1a1b26, surfaces derived with subtle blue-purple tones.
    pub fn tokyo_night() -> Self {
        Self {
            // Surfaces
            app_background: rgba(0x1a, 0x1b, 0x26, 255),
            surface: rgba(0x24, 0x25, 0x3a, 220),
            surface_hover: rgba(0x2a, 0x2c, 0x42, 230),
            dialog_bg: rgba(0x1f, 0x20, 0x35, 240),

            // Borders
            border: rgba(0x3b, 0x42, 0x52, 180),
            border_inactive: rgba(0x3b, 0x42, 0x52, 120),
            border_active: rgba(0x7a, 0xa2, 0xf7, 255),

            // Accent
            accent: rgba(0x7a, 0xa2, 0xf7, 255),
            accent_glow: rgba(0x7a, 0xa2, 0xf7, 40),

            // Text
            text_primary: rgba(0xc0, 0xca, 0xf5, 255),
            text_secondary: rgba(0xa9, 0xb1, 0xd6, 200),
            text_muted: rgba(0x6b, 0x70, 0x90, 180),

            // Tab-specific
            tab_bg_active: rgba(0x2a, 0x2c, 0x42, 255),
            tab_bg_inactive: rgba(0x1a, 0x1b, 0x26, 0),
            tab_close: rgba(0xa9, 0xb1, 0xd6, 150),
            tab_close_hover: rgba(0xf7, 0x76, 0x8e, 255),

            // Status
            success: rgba(0x9e, 0xce, 0x6a, 255),
            warning: rgba(0xe0, 0xaf, 0x68, 255),
            error: rgba(0xf7, 0x76, 0x8e, 255),
        }
    }

    /// Nord-inspired palette (cool blue-greys).
    pub fn nord() -> Self {
        Self {
            app_background: rgba(0x2e, 0x34, 0x40, 255),
            surface: rgba(0x3b, 0x42, 0x52, 220),
            surface_hover: rgba(0x43, 0x4c, 0x5e, 230),
            dialog_bg: rgba(0x35, 0x3c, 0x4a, 240),
            border: rgba(0x4c, 0x56, 0x6a, 180),
            border_inactive: rgba(0x4c, 0x56, 0x6a, 120),
            border_active: rgba(0x88, 0xc0, 0xd0, 255),
            accent: rgba(0x88, 0xc0, 0xd0, 255),
            accent_glow: rgba(0x88, 0xc0, 0xd0, 40),
            text_primary: rgba(0xec, 0xef, 0xf4, 255),
            text_secondary: rgba(0xd8, 0xde, 0xe9, 200),
            text_muted: rgba(0x81, 0xa1, 0xc1, 180),
            tab_bg_active: rgba(0x43, 0x4c, 0x5e, 255),
            tab_bg_inactive: rgba(0x2e, 0x34, 0x40, 0),
            tab_close: rgba(0xd8, 0xde, 0xe9, 150),
            tab_close_hover: rgba(0xbf, 0x61, 0x6a, 255),
            success: rgba(0xa3, 0xbe, 0x8c, 255),
            warning: rgba(0xeb, 0xcb, 0x8b, 255),
            error: rgba(0xbf, 0x61, 0x6a, 255),
        }
    }

    /// Catppuccin Mocha inspired palette.
    pub fn catppuccin_mocha() -> Self {
        Self {
            app_background: rgba(0x1e, 0x1e, 0x2e, 255),
            surface: rgba(0x31, 0x31, 0x44, 220),
            surface_hover: rgba(0x3a, 0x3a, 0x52, 230),
            dialog_bg: rgba(0x28, 0x28, 0x3c, 240),
            border: rgba(0x45, 0x47, 0x59, 180),
            border_inactive: rgba(0x45, 0x47, 0x59, 120),
            border_active: rgba(0x89, 0xb4, 0xfa, 255),
            accent: rgba(0x89, 0xb4, 0xfa, 255),
            accent_glow: rgba(0x89, 0xb4, 0xfa, 40),
            text_primary: rgba(0xc6, 0xd0, 0xf5, 255),
            text_secondary: rgba(0xba, 0xc2, 0xde, 200),
            text_muted: rgba(0x7f, 0x84, 0x9c, 180),
            tab_bg_active: rgba(0x3a, 0x3a, 0x52, 255),
            tab_bg_inactive: rgba(0x1e, 0x1e, 0x2e, 0),
            tab_close: rgba(0xba, 0xc2, 0xde, 150),
            tab_close_hover: rgba(0xf3, 0x8b, 0xa8, 255),
            success: rgba(0xa6, 0xe3, 0xa1, 255),
            warning: rgba(0xf9, 0xe2, 0xaf, 255),
            error: rgba(0xf3, 0x8b, 0xa8, 255),
        }
    }

    /// Light mode palette (Solarized Light base).
    pub fn light() -> Self {
        Self {
            app_background: rgba(0xfd, 0xf6, 0xe3, 255),
            surface: rgba(0xee, 0xe8, 0xd5, 220),
            surface_hover: rgba(0xe6, 0xde, 0xc8, 230),
            dialog_bg: rgba(0xf5, 0xee, 0xd8, 240),
            border: rgba(0xcb, 0xd0, 0xc4, 180),
            border_inactive: rgba(0xcb, 0xd0, 0xc4, 120),
            border_active: rgba(0x26, 0x8b, 0xd2, 255),
            accent: rgba(0x26, 0x8b, 0xd2, 255),
            accent_glow: rgba(0x26, 0x8b, 0xd2, 40),
            text_primary: rgba(0x07, 0x36, 0x42, 255),
            text_secondary: rgba(0x58, 0x6e, 0x75, 220),
            text_muted: rgba(0x93, 0xa1, 0xa1, 180),
            tab_bg_active: rgba(0xe6, 0xde, 0xc8, 255),
            tab_bg_inactive: rgba(0xfd, 0xf6, 0xe3, 0),
            tab_close: rgba(0x58, 0x6e, 0x75, 150),
            tab_close_hover: rgba(0xdc, 0x32, 0x2f, 255),
            success: rgba(0x85, 0x99, 0x00, 255),
            warning: rgba(0xb5, 0x89, 0x00, 255),
            error: rgba(0xdc, 0x32, 0x2f, 255),
        }
    }

    /// Look up a palette by terminal theme name.
    pub fn for_theme(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "light" | "solarized-light" | "solarized_light" => Self::light(),
            "nord" => Self::nord(),
            "catppuccin-mocha" | "catppuccin_mocha" => Self::catppuccin_mocha(),
            // Default to Tokyo Night for all dark themes.
            _ => Self::tokyo_night(),
        }
    }
}

impl Default for UiPalette {
    fn default() -> Self {
        Self::tokyo_night()
    }
}

// ── Helpers ────────────────────────────────────────────────────────────

/// Convert 8-bit RGB + alpha to linear float array [r, g, b, a] (0.0–1.0).
fn rgba(r: u8, g: u8, b: u8, a: u8) -> [f32; 4] {
    [
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ]
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_tokyo_night_base_color() {
        let p = UiPalette::tokyo_night();
        // #1a1b26 → (26, 27, 38)
        assert!((p.app_background[0] - 26.0 / 255.0).abs() < 0.01);
        assert!((p.app_background[1] - 27.0 / 255.0).abs() < 0.01);
        assert!((p.app_background[2] - 38.0 / 255.0).abs() < 0.01);
        assert!((p.app_background[3] - 1.0).abs() < 0.01);
    }

    #[test]
    fn t_for_theme_dark_defaults_to_tokyo() {
        let p = UiPalette::for_theme("dracula");
        let tn = UiPalette::tokyo_night();
        assert_eq!(p.app_background, tn.app_background);
    }

    #[test]
    fn t_for_theme_light() {
        let p = UiPalette::for_theme("light");
        assert!((p.app_background[0] - 253.0 / 255.0).abs() < 0.01);
    }

    #[test]
    fn t_for_theme_nord() {
        let p = UiPalette::for_theme("nord");
        let n = UiPalette::nord();
        assert_eq!(p.accent, n.accent);
    }

    #[test]
    fn t_for_theme_catppuccin() {
        let p = UiPalette::for_theme("catppuccin-mocha");
        let c = UiPalette::catppuccin_mocha();
        assert_eq!(p.accent, c.accent);
    }

    #[test]
    fn t_rgba_conversion() {
        let c = rgba(255, 0, 128, 200);
        assert!((c[0] - 1.0).abs() < 0.01);
        assert!((c[1] - 0.0).abs() < 0.01);
        assert!((c[2] - 128.0 / 255.0).abs() < 0.01);
        assert!((c[3] - 200.0 / 255.0).abs() < 0.01);
    }

    #[test]
    fn t_accent_alpha_fully_opaque() {
        let p = UiPalette::tokyo_night();
        assert!((p.accent[3] - 1.0).abs() < 0.01);
    }

    #[test]
    fn tab_bg_inactive_transparent() {
        let p = UiPalette::tokyo_night();
        assert!(p.tab_bg_inactive[3] < 0.01); // alpha = 0
    }

    #[test]
    fn t_surface_semi_transparent() {
        let p = UiPalette::tokyo_night();
        // Surface alpha should be between 0 and 1 (semi-transparent).
        assert!(p.surface[3] > 0.0 && p.surface[3] < 1.0);
    }

    #[test]
    fn t_glow_low_alpha() {
        let p = UiPalette::tokyo_night();
        // Glow alpha should be very low (~15%).
        assert!(p.accent_glow[3] < 0.2);
    }

    #[test]
    fn t_default_is_tokyo_night() {
        let p = UiPalette::default();
        let tn = UiPalette::tokyo_night();
        assert_eq!(p, tn);
    }

    #[test]
    fn t_close_hover_is_redish() {
        let p = UiPalette::tokyo_night();
        // Close hover should have high red, lower green/blue.
        assert!(p.tab_close_hover[0] > 0.8);
        assert!(p.tab_close_hover[1] < 0.6);
    }
}
