//! Build and version information for GGTerm.
//!
//! Compile-time constants for the About dialog and diagnostics.

/// GGTerm version string.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Package name.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// Package description.
pub const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");

/// License.
pub const LICENSE: &str = env!("CARGO_PKG_LICENSE");

/// Repository URL.
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// Build info — technology stack.
pub const TECH_STACK: &str = "Rust + wgpu + glyphon + winit";

/// Git commit hash (set via build script or env).
pub fn git_hash() -> &'static str {
    option_env!("GIT_HASH").unwrap_or("unknown")
}

/// Build date (set via build script or env).
pub fn build_date() -> &'static str {
    option_env!("BUILD_DATE").unwrap_or("unknown")
}

/// Full version string: "v0.1.0 (abc1234, 2026-06-30)".
pub fn full_version() -> String {
    format!("v{} ({}, {})", VERSION, git_hash(), build_date())
}

/// About dialog text.
pub fn about_text() -> String {
    format!(
        "{name} {version}\n\n\
         {description}\n\n\
         Built with: {tech}\n\
         License: {license}\n\
         Repository: {repo}\n\
         Build: {git} ({date})",
        name = NAME,
        version = full_version(),
        description = DESCRIPTION,
        tech = TECH_STACK,
        license = LICENSE,
        repo = REPOSITORY,
        git = git_hash(),
        date = build_date(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_not_empty() {
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn test_name_not_empty() {
        assert!(!NAME.is_empty());
        assert!(NAME.contains("ggterm"));
    }

    #[test]
    fn test_full_version_format() {
        let v = full_version();
        assert!(v.starts_with("v"));
        assert!(v.contains("("));
        assert!(v.contains(")"));
    }

    #[test]
    fn test_about_text_contains_key_info() {
        let text = about_text();
        assert!(text.contains("ggterm"));
        assert!(text.contains("Rust"));
        assert!(text.contains("License"));
        assert!(text.contains("Repository"));
    }
}
