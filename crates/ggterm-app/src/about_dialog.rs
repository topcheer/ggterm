//! About dialog state for GGTerm.
//!
//! Provides version info, tech stack details, and links.
//! Used by the menu bar Help > About action.

/// Version information shown in the About dialog.
#[derive(Debug, Clone)]
pub struct AboutInfo {
    pub version: &'static str,
    pub commit: &'static str,
    pub build_date: &'static str,
    pub rust_version: &'static str,
    pub features: Vec<&'static str>,
}

impl Default for AboutInfo {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            commit: env!("CARGO_PKG_VERSION"),
            build_date: "dev",
            rust_version: "1.85+",
            features: vec![
                "Multi-tab sessions",
                "AI assistant integration",
                "GPU-accelerated rendering (wgpu)",
                "Scrollback search",
                "Clipboard (OSC 52)",
                "6 built-in themes",
                "Configurable keybindings",
                "Font zoom & customization",
                "Hyperlink support",
                "Combining character support",
            ],
        }
    }
}

impl AboutInfo {
    /// Format the About dialog as a multi-line string for rendering.
    pub fn format(&self) -> String {
        let mut lines = Vec::new();
        lines.push("GGTerm".to_string());
        lines.push(format!("Version {} ({})", self.version, self.commit));
        lines.push(format!("Built: {}", self.build_date));
        lines.push(String::new());
        lines.push("GPU-accelerated AI-native terminal emulator".to_string());
        lines.push(String::new());
        lines.push("Built with:".to_string());
        lines.push(format!("  Rust {}", self.rust_version));
        lines.push("  wgpu + glyphon".to_string());
        lines.push("  winit 0.30".to_string());
        lines.push(String::new());
        lines.push("Features:".to_string());
        for f in &self.features {
            lines.push(format!("  • {}", f));
        }
        lines.push(String::new());
        lines.push("https://github.com/topcheer/ggterm".to_string());
        lines.push("MIT OR Apache-2.0".to_string());
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_about_info_default() {
        let info = AboutInfo::default();
        assert!(!info.version.is_empty());
        assert!(!info.features.is_empty());
    }

    #[test]
    fn test_about_format_contains_version() {
        let info = AboutInfo::default();
        let text = info.format();
        assert!(text.contains("GGTerm"));
        assert!(text.contains("Version"));
        assert!(text.contains("Rust"));
    }

    #[test]
    fn test_about_format_contains_features() {
        let info = AboutInfo::default();
        let text = info.format();
        assert!(text.contains("Features:"));
        assert!(text.contains("Multi-tab"));
        assert!(text.contains("AI assistant"));
    }

    #[test]
    fn test_about_format_contains_url() {
        let info = AboutInfo::default();
        let text = info.format();
        assert!(text.contains("github.com/topcheer/ggterm"));
    }

    #[test]
    fn test_about_format_contains_license() {
        let info = AboutInfo::default();
        let text = info.format();
        assert!(text.contains("MIT"));
        assert!(text.contains("Apache"));
    }
}
