//! About dialog content and state (P19-A).
//!
//! Displays application version, build info, and tech stack.
//! When `visible` is true, the overlay is rendered on top of the terminal.

/// Application metadata shown in the About dialog.
#[derive(Debug, Clone)]
pub struct AboutDialog {
    /// Whether the dialog is currently visible.
    pub visible: bool,
    /// Application name.
    pub app_name: &'static str,
    /// Semantic version string.
    pub version: &'static str,
    /// Short Git commit hash (compile-time).
    pub git_hash: &'static str,
    /// Build date (compile-time).
    pub build_date: &'static str,
    /// Tech stack description lines.
    pub tech_stack: &'static [&'static str],
    /// GitHub repository URL.
    pub homepage: &'static str,
    /// License.
    pub license: &'static str,
}

impl Default for AboutDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl AboutDialog {
    /// Create an about dialog with compiled-in metadata.
    pub fn new() -> Self {
        Self {
            visible: false,
            app_name: "GGTerm",
            version: env!("CARGO_PKG_VERSION"),
            git_hash: option_env!("GIT_HASH").unwrap_or("unknown"),
            build_date: option_env!("BUILD_DATE").unwrap_or("unknown"),
            tech_stack: &[
                "Rust 2024 Edition",
                "wgpu — WebGPU graphics",
                "glyphon — text shaping & layout",
                "winit — cross-platform windowing",
                "portable-pty — PTY integration",
            ],
            homepage: "https://github.com/topcheer/ggterm",
            license: "MIT OR Apache-2.0",
        }
    }

    /// Toggle dialog visibility.
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    /// Show the dialog.
    pub fn show(&mut self) {
        self.visible = true;
    }

    /// Hide the dialog.
    pub fn hide(&mut self) {
        self.visible = false;
    }

    /// Format the dialog as a multi-line string (for overlay or logging).
    pub fn format_text(&self) -> String {
        let mut lines = String::new();
        lines.push_str(&format!("{}\n", self.app_name));
        lines.push_str(&format!("Version {}\n", self.version));
        if self.git_hash != "unknown" {
            lines.push_str(&format!("Git: {}\n", self.git_hash));
        }
        if self.build_date != "unknown" {
            lines.push_str(&format!("Built: {}\n", self.build_date));
        }
        lines.push_str("\nTech Stack:\n");
        for tech in self.tech_stack {
            lines.push_str(&format!("  • {tech}\n"));
        }
        lines.push_str(&format!("\n{}\n", self.homepage));
        lines.push_str(&format!("Licensed under {}\n", self.license));
        lines
    }

    /// Lines of text to render in the overlay.
    pub fn overlay_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(self.app_name.to_string());
        lines.push(format!("Version {}", self.version));
        if self.git_hash != "unknown" {
            lines.push(format!("Git: {}", self.git_hash));
        }
        if self.build_date != "unknown" {
            lines.push(format!("Built: {}", self.build_date));
        }
        lines.push(String::new());
        lines.push("Tech Stack:".to_string());
        for tech in self.tech_stack {
            lines.push(format!("  {tech}"));
        }
        lines.push(String::new());
        lines.push(self.homepage.to_string());
        lines
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_about_default_hidden() {
        let dlg = AboutDialog::new();
        assert!(!dlg.visible);
    }

    #[test]
    fn test_about_toggle() {
        let mut dlg = AboutDialog::new();
        assert!(!dlg.visible);
        dlg.toggle();
        assert!(dlg.visible);
        dlg.toggle();
        assert!(!dlg.visible);
    }

    #[test]
    fn test_about_show_hide() {
        let mut dlg = AboutDialog::new();
        dlg.show();
        assert!(dlg.visible);
        dlg.hide();
        assert!(!dlg.visible);
    }

    #[test]
    fn test_about_app_name() {
        let dlg = AboutDialog::new();
        assert_eq!(dlg.app_name, "GGTerm");
    }

    #[test]
    fn test_about_version_not_empty() {
        let dlg = AboutDialog::new();
        assert!(!dlg.version.is_empty());
    }

    #[test]
    fn test_about_tech_stack() {
        let dlg = AboutDialog::new();
        assert!(!dlg.tech_stack.is_empty());
        assert!(dlg.tech_stack.iter().any(|t| t.contains("Rust")));
        assert!(dlg.tech_stack.iter().any(|t| t.contains("wgpu")));
        assert!(dlg.tech_stack.iter().any(|t| t.contains("glyphon")));
        assert!(dlg.tech_stack.iter().any(|t| t.contains("winit")));
    }

    #[test]
    fn test_about_homepage() {
        let dlg = AboutDialog::new();
        assert!(dlg.homepage.starts_with("https://"));
        assert!(dlg.homepage.contains("ggterm"));
    }

    #[test]
    fn test_about_license() {
        let dlg = AboutDialog::new();
        assert!(dlg.license.contains("MIT") || dlg.license.contains("Apache"));
    }

    #[test]
    fn test_format_text_contents() {
        let dlg = AboutDialog::new();
        let text = dlg.format_text();
        assert!(text.contains("GGTerm"));
        assert!(text.contains("Version"));
        assert!(text.contains("Tech Stack"));
        assert!(text.contains("https://"));
        assert!(text.contains("MIT"));
    }

    #[test]
    fn test_overlay_lines() {
        let dlg = AboutDialog::new();
        let lines = dlg.overlay_lines();
        assert!(!lines.is_empty());
        assert_eq!(lines[0], "GGTerm");
        assert!(lines[1].starts_with("Version"));
        // Should contain a tech stack line
        assert!(lines.iter().any(|l| l.contains("wgpu")));
        // Should contain homepage
        assert!(lines.iter().any(|l| l.starts_with("https://")));
    }

    // ── P19-H: Integration edge cases ─────────────────────────

    #[test]
    fn test_format_text_multi_line_structure() {
        let dlg = AboutDialog::new();
        let text = dlg.format_text();
        let lines: Vec<&str> = text.lines().collect();
        // Must have enough lines: app name, version, blank, tech header,
        // 5 tech items, blank, homepage.
        assert!(
            lines.len() >= 10,
            "expected >=10 lines, got {}",
            lines.len()
        );
        // First line is app name.
        assert_eq!(lines[0], "GGTerm");
        // Contains blank separator before tech stack.
        assert!(lines.iter().any(|l| l.is_empty()));
    }

    #[test]
    fn test_overlay_lines_has_tech_items() {
        let dlg = AboutDialog::new();
        let lines = dlg.overlay_lines();
        // Each tech_stack entry should appear as a line.
        for tech in dlg.tech_stack {
            assert!(
                lines.iter().any(|l| l.contains(tech)),
                "missing tech: {tech}"
            );
        }
    }

    #[test]
    fn test_default_equals_new() {
        let d1 = AboutDialog::default();
        let d2 = AboutDialog::new();
        assert_eq!(d1.visible, d2.visible);
        assert_eq!(d1.app_name, d2.app_name);
        assert_eq!(d1.version, d2.version);
        assert_eq!(d1.tech_stack.len(), d2.tech_stack.len());
    }

    #[test]
    fn test_show_after_hide() {
        let mut dlg = AboutDialog::new();
        dlg.show();
        assert!(dlg.visible);
        dlg.hide();
        assert!(!dlg.visible);
        dlg.show(); // can show again after hiding
        assert!(dlg.visible);
    }

    #[test]
    fn test_overlay_lines_ends_with_homepage() {
        let dlg = AboutDialog::new();
        let lines = dlg.overlay_lines();
        let last = lines.last().unwrap();
        assert!(last.starts_with("https://"));
    }

    #[test]
    fn test_git_hash_and_build_date_fields() {
        let dlg = AboutDialog::new();
        // Fields exist and are either "unknown" or have a value.
        assert!(!dlg.git_hash.is_empty());
        assert!(!dlg.build_date.is_empty());
    }
}
