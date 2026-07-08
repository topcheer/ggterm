//! P28-H: Quick Shell switcher — status bar dropdown to switch shells.
//!
//! Detects available shells on the system and provides a dropdown menu
//! in the status bar to quickly spawn a new tab/pane with a different shell.

/// Shell identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellInfo {
    /// Display name (e.g., "zsh", "bash").
    pub name: String,
    /// Full path to the shell binary (e.g., "/bin/zsh").
    pub path: String,
    /// Whether this is the current default shell.
    pub is_default: bool,
    /// Shell version (if detected).
    pub version: Option<String>,
}

impl ShellInfo {
    /// Create new shell info.
    pub fn new(name: &str, path: &str) -> Self {
        Self {
            name: name.to_string(),
            path: path.to_string(),
            is_default: false,
            version: None,
        }
    }
}

/// Common shell paths to check.
const SHELL_PATHS: &[(&str, &str)] = &[
    ("zsh", "/bin/zsh"),
    ("bash", "/bin/bash"),
    ("fish", "/usr/local/bin/fish"),
    ("fish", "/opt/homebrew/bin/fish"),
    ("nu", "/usr/local/bin/nu"),
    ("nu", "/opt/homebrew/bin/nu"),
    ("sh", "/bin/sh"),
    ("dash", "/bin/dash"),
    ("ksh", "/bin/ksh"),
    ("tcsh", "/bin/tcsh"),
    ("pwsh", "/usr/local/bin/pwsh"),
    ("pwsh", "/opt/homebrew/bin/pwsh"),
    ("elvish", "/usr/local/bin/elvish"),
    ("elvish", "/opt/homebrew/bin/elvish"),
];

/// State for the shell switcher dropdown.
#[derive(Debug, Default)]
pub struct ShellSwitcherState {
    /// Whether the dropdown is open.
    pub open: bool,
    /// Detected shells on this system.
    shells: Vec<ShellInfo>,
    /// Currently selected shell (for dropdown navigation).
    pub selected: usize,
    /// Current default shell path.
    current_shell: String,
}

impl ShellSwitcherState {
    /// Create new shell switcher and detect available shells.
    pub fn new() -> Self {
        let current = get_default_shell();
        let mut state = Self {
            open: false,
            shells: Vec::new(),
            selected: 0,
            current_shell: current,
        };
        state.detect_shells();
        state
    }

    /// Toggle dropdown visibility.
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Close the dropdown.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Detect available shells on the system.
    pub fn detect_shells(&mut self) {
        self.shells.clear();
        let mut seen_names = std::collections::HashSet::new();

        for (name, path) in SHELL_PATHS {
            // Avoid duplicate shell names (e.g., fish at two paths)
            if seen_names.contains(*name) {
                continue;
            }
            if std::path::Path::new(path).exists() {
                let mut info = ShellInfo::new(name, path);
                info.is_default = *path == self.current_shell
                    || self.current_shell.ends_with(&format!("/{}", name));
                info.version = detect_shell_version(path);
                self.shells.push(info);
                seen_names.insert(*name);
            }
        }

        // Always ensure current shell is present
        if !self.shells.iter().any(|s| s.is_default) {
            let name = std::path::Path::new(&self.current_shell)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| "shell".to_string());
            let mut info = ShellInfo::new(&name, &self.current_shell);
            info.is_default = true;
            info.version = detect_shell_version(&self.current_shell);
            self.shells.insert(0, info);
        }
    }

    /// Get the list of detected shells.
    pub fn shells(&self) -> &[ShellInfo] {
        &self.shells
    }

    /// Number of detected shells.
    pub fn len(&self) -> usize {
        self.shells.len()
    }

    /// Whether no shells were detected (should never happen).
    pub fn is_empty(&self) -> bool {
        self.shells.is_empty()
    }

    /// Move selection up.
    pub fn select_up(&mut self) {
        if self.shells.is_empty() {
            return;
        }
        self.selected = if self.selected == 0 {
            self.shells.len() - 1
        } else {
            self.selected - 1
        };
    }

    /// Move selection down.
    pub fn select_down(&mut self) {
        if self.shells.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.shells.len();
    }

    /// Get the currently selected shell.
    pub fn selected_shell(&self) -> Option<&ShellInfo> {
        self.shells.get(self.selected)
    }

    /// Get the default shell.
    pub fn default_shell(&self) -> Option<&ShellInfo> {
        self.shells.iter().find(|s| s.is_default)
    }

    /// Set the current shell path.
    pub fn set_current_shell(&mut self, path: &str) {
        self.current_shell = path.to_string();
        self.detect_shells();
    }

    /// Get the current shell path.
    pub fn current_shell(&self) -> &str {
        &self.current_shell
    }

    /// Get the display name for the status bar.
    pub fn status_bar_label(&self) -> String {
        let shell_name = std::path::Path::new(&self.current_shell)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| self.current_shell.clone());
        format!("Shell: {}", shell_name)
    }
}

/// Get the default shell path.
fn get_default_shell() -> String {
    // GGTERM_EXEC: set by `ggterm -e <command>` — execute a command
    // instead of launching an interactive shell (like xterm -e).
    if let Ok(exec_cmd) = std::env::var("GGTERM_EXEC")
        && !exec_cmd.is_empty()
    {
        // Use the user's shell to run the command with -c.
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        return format!("{shell} -c '{exec_cmd}'");
    }
    // $SHELL environment variable
    if let Ok(shell) = std::env::var("SHELL") {
        return shell;
    }
    // Fallback
    ggterm_core::pty::default_shell()
}

/// Try to detect shell version (returns a short version string).
fn detect_shell_version(shell_path: &str) -> Option<String> {
    let name = std::path::Path::new(shell_path).file_name()?.to_str()?;

    let output = std::process::Command::new(shell_path)
        .arg("--version")
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout.lines().next()?;

    // Extract version number from common formats
    match name {
        "zsh" => first_line
            .split_whitespace()
            .nth(1)
            .map(|v| format!("v{}", v.split('(').next().unwrap_or(v))),
        "bash" => first_line
            .split_whitespace()
            .nth(3)
            .map(|v| format!("v{}", v)),
        "fish" => first_line
            .split(',')
            .next()
            .and_then(|s| s.split_whitespace().nth(2).map(|v| format!("v{}", v))),
        _ => Some(first_line.chars().take(20).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_shell_info_new() {
        let info = ShellInfo::new("zsh", "/bin/zsh");
        assert_eq!(info.name, "zsh");
        assert_eq!(info.path, "/bin/zsh");
        assert!(!info.is_default);
        assert!(info.version.is_none());
    }

    #[test]
    fn t_shell_switcher_default() {
        let state = ShellSwitcherState::new();
        assert!(!state.open);
        assert!(!state.is_empty()); // should detect at least the current shell
    }

    #[test]
    fn t_shell_switcher_toggle() {
        let mut state = ShellSwitcherState::new();
        assert!(!state.open);
        state.toggle();
        assert!(state.open);
        state.toggle();
        assert!(!state.open);
    }

    #[test]
    fn t_shell_switcher_close() {
        let mut state = ShellSwitcherState::new();
        state.open = true;
        state.close();
        assert!(!state.open);
    }

    #[test]
    fn t_shell_switcher_detects_shells() {
        let state = ShellSwitcherState::new();
        assert!(state.len() >= 1); // at least the default
    }

    #[test]
    fn t_shell_switcher_has_default() {
        let state = ShellSwitcherState::new();
        assert!(state.default_shell().is_some());
        assert!(state.default_shell().unwrap().is_default);
    }

    #[test]
    fn t_shell_switcher_navigation() {
        let mut state = ShellSwitcherState::new();
        let count = state.len();
        if count > 1 {
            state.select_down();
            assert_eq!(state.selected, 1);
            state.select_down();
            assert_eq!(state.selected, 2 % count);
            state.select_up();
            assert_eq!(state.selected, if count > 2 { 1 } else { 0 });
        }
    }

    #[test]
    fn t_shell_switcher_select_wraps_up() {
        let mut state = ShellSwitcherState::new();
        state.selected = 0;
        state.select_up();
        assert_eq!(state.selected, state.len() - 1);
    }

    #[test]
    fn t_shell_switcher_select_wraps_down() {
        let mut state = ShellSwitcherState::new();
        let count = state.len();
        state.selected = count - 1;
        state.select_down();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn t_shell_switcher_selected_shell() {
        let state = ShellSwitcherState::new();
        assert!(state.selected_shell().is_some());
    }

    #[test]
    fn t_shell_switcher_status_bar_label() {
        let state = ShellSwitcherState::new();
        let label = state.status_bar_label();
        assert!(label.starts_with("Shell:"));
    }

    #[test]
    fn t_shell_switcher_set_current() {
        let mut state = ShellSwitcherState::new();
        state.set_current_shell("/bin/bash");
        assert_eq!(state.current_shell(), "/bin/bash");
    }

    #[test]
    fn t_get_default_shell_not_empty() {
        let shell = get_default_shell();
        assert!(!shell.is_empty());
    }
}
