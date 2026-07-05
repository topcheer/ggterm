//! Shell integration — auto-inject OSC 133 hooks into the spawned shell.
//!
//! When GGTerm spawns a shell, it can automatically inject the prompt
//! command markers (OSC 133) without requiring manual user setup.
//!
//! ## Approach
//!
//! 1. Set `GGTERM=1` env var so the shell knows it's running under GGTerm.
//! 2. For each supported shell, pass the integration script via the
//!    mechanism that shell supports:
//!    - **bash**: `--rcfile <script>` or `BASH_ENV` for non-interactive
//!    - **zsh**: `ZDOTDIR` pointing to a temp dir with `.zshrc`
//!    - **fish**: `-C "source <script>"` flag

use std::fs;
use std::path::PathBuf;

/// Bash shell integration script.
pub const BASH_INTEGRATION: &str = include_str!("../../../shell/bash.sh");

/// Zsh shell integration script.
pub const ZSH_INTEGRATION: &str = include_str!("../../../shell/zsh.zsh");

/// Fish shell integration script.
pub const FISH_INTEGRATION: &str = include_str!("../../../shell/fish.fish");

/// Shell type detected from the shell path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Bash,
    Zsh,
    Fish,
    /// Unknown shell — no auto-injection possible.
    Unknown,
}

impl ShellKind {
    /// Detect shell kind from a path (e.g., `/bin/zsh` → `Zsh`).
    pub fn from_path(path: &str) -> Self {
        let name = path.rsplit('/').next().unwrap_or(path);
        match name {
            "bash" => Self::Bash,
            "zsh" => Self::Zsh,
            "fish" => Self::Fish,
            _ => Self::Unknown,
        }
    }

    /// The integration script content for this shell kind.
    pub fn integration_script(self) -> Option<&'static str> {
        match self {
            Self::Bash => Some(BASH_INTEGRATION),
            Self::Zsh => Some(ZSH_INTEGRATION),
            Self::Fish => Some(FISH_INTEGRATION),
            Self::Unknown => None,
        }
    }
}

/// Configuration for shell integration injection.
#[derive(Debug, Clone)]
pub struct ShellIntegrationConfig {
    /// Shell kind (determines injection mechanism).
    pub kind: ShellKind,
    /// Original shell path (e.g., `/bin/zsh`).
    pub shell_path: String,
    /// Temp directory where integration scripts are written.
    pub temp_dir: PathBuf,
    /// Whether integration was successfully prepared.
    pub prepared: bool,
}

impl ShellIntegrationConfig {
    /// Prepare shell integration for the given shell path.
    ///
    /// Writes the integration script to a temp file and returns the
    /// configuration needed to inject it into the shell.
    ///
    /// Returns a config with `prepared = false` if the shell is unknown
    /// or writing fails (caller should fall back to plain shell).
    pub fn prepare(shell_path: &str) -> Self {
        let kind = ShellKind::from_path(shell_path);

        let Some(script) = kind.integration_script() else {
            return Self {
                kind: ShellKind::Unknown,
                shell_path: shell_path.to_string(),
                temp_dir: PathBuf::new(),
                prepared: false,
            };
        };

        // Use a stable temp dir under the system temp.
        let temp_dir = std::env::temp_dir().join("ggterm-shell-integration");
        if fs::create_dir_all(&temp_dir).is_err() {
            return Self {
                kind,
                shell_path: shell_path.to_string(),
                temp_dir,
                prepared: false,
            };
        }

        let script_path = temp_dir.join(script_filename(kind));
        if fs::write(&script_path, script).is_err() {
            return Self {
                kind,
                shell_path: shell_path.to_string(),
                temp_dir,
                prepared: false,
            };
        }

        Self {
            kind,
            shell_path: shell_path.to_string(),
            temp_dir,
            prepared: true,
        }
    }

    /// Get the modified shell arguments to inject the integration script.
    ///
    /// Returns `(program, args)` that should be used to spawn the shell.
    pub fn spawn_args(&self) -> (String, Vec<String>) {
        if !self.prepared {
            return (self.shell_path.clone(), vec![]);
        }

        let script_path = self.temp_dir.join(script_filename(self.kind));
        let script_str = script_path.to_string_lossy().into_owned();

        match self.kind {
            ShellKind::Bash => {
                // bash --rcfile <script> -i
                (
                    self.shell_path.clone(),
                    vec!["--rcfile".into(), script_str, "-i".into()],
                )
            }
            ShellKind::Zsh => {
                // Write a .zshrc in a ZDOTDIR that sources the integration.
                let zdotdir = self.temp_dir.join("zdotdir");
                let _ = fs::create_dir_all(&zdotdir);
                let zshrc = zdotdir.join(".zshrc");
                let _ = fs::write(
                    &zshrc,
                    format!(
                        "# Auto-generated by GGTerm\n\
                         [[ -f ~/.zshrc ]] && source ~/.zshrc\n\
                         source {script_str}\n"
                    ),
                );
                // Set ZDOTDIR via env var — spawn zsh normally, it will read .zshrc
                (self.shell_path.clone(), vec![])
            }
            ShellKind::Fish => {
                // fish -C "source <script>"
                (
                    self.shell_path.clone(),
                    vec!["-C".into(), format!("source {script_str}")],
                )
            }
            ShellKind::Unknown => (self.shell_path.clone(), vec![]),
        }
    }

    /// Environment variables to set for the spawned shell.
    pub fn env_vars(&self) -> Vec<(String, String)> {
        let mut vars = vec![
            ("GGTERM".to_string(), "1".to_string()),
            (
                "GGTERM_VERSION".to_string(),
                env!("CARGO_PKG_VERSION").to_string(),
            ),
            // Set TERM so terminfo-based programs (vim, htop, ncurses)
            // work correctly. Use xterm-256color for broad compatibility.
            ("TERM".to_string(), "xterm-256color".to_string()),
            // Advertise true color support.
            ("COLORTERM".to_string(), "truecolor".to_string()),
        ];

        if self.prepared && self.kind == ShellKind::Zsh {
            let zdotdir = self.temp_dir.join("zdotdir");
            vars.push((
                "ZDOTDIR".to_string(),
                zdotdir.to_string_lossy().into_owned(),
            ));
        }

        vars
    }
}

/// Get the script filename for a shell kind.
fn script_filename(kind: ShellKind) -> &'static str {
    match kind {
        ShellKind::Bash => "ggterm_bash.sh",
        ShellKind::Zsh => "ggterm_zsh.zsh",
        ShellKind::Fish => "ggterm_fish.fish",
        ShellKind::Unknown => "ggterm_unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_detect_bash() {
        assert_eq!(ShellKind::from_path("/bin/bash"), ShellKind::Bash);
        assert_eq!(ShellKind::from_path("/usr/local/bin/bash"), ShellKind::Bash);
        assert_eq!(ShellKind::from_path("bash"), ShellKind::Bash);
    }

    #[test]
    fn t_detect_zsh() {
        assert_eq!(ShellKind::from_path("/bin/zsh"), ShellKind::Zsh);
        assert_eq!(
            ShellKind::from_path("/opt/homebrew/bin/zsh"),
            ShellKind::Zsh
        );
    }

    #[test]
    fn t_detect_fish() {
        assert_eq!(ShellKind::from_path("/usr/bin/fish"), ShellKind::Fish);
    }

    #[test]
    fn t_detect_unknown() {
        assert_eq!(ShellKind::from_path("/bin/sh"), ShellKind::Unknown);
        assert_eq!(ShellKind::from_path("/bin/dash"), ShellKind::Unknown);
        assert_eq!(ShellKind::from_path("pwsh"), ShellKind::Unknown);
    }

    #[test]
    fn t_integration_scripts_exist() {
        assert!(!BASH_INTEGRATION.is_empty());
        assert!(BASH_INTEGRATION.contains("133"));
        assert!(!ZSH_INTEGRATION.is_empty());
        assert!(ZSH_INTEGRATION.contains("133"));
        assert!(!FISH_INTEGRATION.is_empty());
        assert!(FISH_INTEGRATION.contains("133"));
    }

    #[test]
    fn t_prepare_bash() {
        let config = ShellIntegrationConfig::prepare("/bin/bash");
        assert!(config.prepared);
        assert_eq!(config.kind, ShellKind::Bash);
        let (program, args) = config.spawn_args();
        assert_eq!(program, "/bin/bash");
        assert!(args.iter().any(|a| a == "--rcfile"));
    }

    #[test]
    fn t_prepare_zsh() {
        let config = ShellIntegrationConfig::prepare("/bin/zsh");
        assert!(config.prepared);
        assert_eq!(config.kind, ShellKind::Zsh);
        let env_vars = config.env_vars();
        assert!(env_vars.iter().any(|(k, _)| k == "ZDOTDIR"));
    }

    #[test]
    fn t_prepare_fish() {
        let config = ShellIntegrationConfig::prepare("/usr/bin/fish");
        assert!(config.prepared);
        assert_eq!(config.kind, ShellKind::Fish);
        let (_, args) = config.spawn_args();
        assert!(args.iter().any(|a| a == "-C"));
    }

    #[test]
    fn t_prepare_unknown_not_prepared() {
        let config = ShellIntegrationConfig::prepare("/bin/sh");
        assert!(!config.prepared);
        let (program, args) = config.spawn_args();
        assert_eq!(program, "/bin/sh");
        assert!(args.is_empty());
    }

    #[test]
    fn t_env_vars_always_set_ggterm() {
        let config = ShellIntegrationConfig::prepare("/bin/sh");
        let vars = config.env_vars();
        assert!(vars.iter().any(|(k, v)| k == "GGTERM" && v == "1"));
        assert!(vars.iter().any(|(k, _)| k == "GGTERM_VERSION"));
    }

    #[test]
    fn t_script_filename() {
        assert_eq!(script_filename(ShellKind::Bash), "ggterm_bash.sh");
        assert_eq!(script_filename(ShellKind::Zsh), "ggterm_zsh.zsh");
        assert_eq!(script_filename(ShellKind::Fish), "ggterm_fish.fish");
    }
}
