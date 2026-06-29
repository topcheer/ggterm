//! Hook system — terminal event interception for plugins.
//!
//! Hooks are the mechanism through which plugins observe and influence
//! the terminal. Each hook represents a specific point in the terminal
//! event lifecycle.

/// Types of hooks a plugin can register for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookType {
    /// User input before it's sent to the PTY. Can be transformed or denied.
    OnInput,
    /// Terminal output after it's parsed. Read-only observation.
    OnOutput,
    /// Command started (OSC 133;C mark received).
    OnCommandStart,
    /// Command finished (OSC 133;D mark received). Includes exit code.
    OnCommandEnd,
    /// Terminal resized.
    OnResize,
    /// Theme changed.
    OnThemeChange,
}

impl HookType {
    /// All available hook types.
    pub fn all() -> &'static [HookType] {
        &[
            HookType::OnInput,
            HookType::OnOutput,
            HookType::OnCommandStart,
            HookType::OnCommandEnd,
            HookType::OnResize,
            HookType::OnThemeChange,
        ]
    }

    /// Human-readable name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::OnInput => "on_input",
            Self::OnOutput => "on_output",
            Self::OnCommandStart => "on_command_start",
            Self::OnCommandEnd => "on_command_end",
            Self::OnResize => "on_resize",
            Self::OnThemeChange => "on_theme_change",
        }
    }
}

impl std::fmt::Display for HookType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// A concrete hook event dispatched to plugins.
#[derive(Debug, Clone)]
pub enum Hook {
    /// User input is about to be sent to the PTY.
    OnInput(String),
    /// Terminal output was just rendered.
    OnOutput(String),
    /// A command is starting.
    OnCommandStart(String),
    /// A command has finished.
    OnCommandEnd { command: String, exit_code: i32 },
    /// Terminal was resized.
    OnResize { cols: usize, rows: usize },
    /// Theme was changed.
    OnThemeChange { from: String, to: String },
}

impl Hook {
    /// Get the hook type for this event.
    pub fn hook_type(&self) -> HookType {
        match self {
            Self::OnInput(_) => HookType::OnInput,
            Self::OnOutput(_) => HookType::OnOutput,
            Self::OnCommandStart(_) => HookType::OnCommandStart,
            Self::OnCommandEnd { .. } => HookType::OnCommandEnd,
            Self::OnResize { .. } => HookType::OnResize,
            Self::OnThemeChange { .. } => HookType::OnThemeChange,
        }
    }

    /// Get text content if this hook carries text.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::OnInput(s) | Self::OnOutput(s) | Self::OnCommandStart(s) => Some(s),
            Self::OnCommandEnd { command, .. } => Some(command),
            Self::OnResize { .. } | Self::OnThemeChange { .. } => None,
        }
    }

    /// Create an OnInput hook.
    pub fn input(text: impl Into<String>) -> Self {
        Self::OnInput(text.into())
    }

    /// Create an OnOutput hook.
    pub fn output(text: impl Into<String>) -> Self {
        Self::OnOutput(text.into())
    }

    /// Create an OnCommandStart hook.
    pub fn command_start(cmd: impl Into<String>) -> Self {
        Self::OnCommandStart(cmd.into())
    }

    /// Create an OnCommandEnd hook.
    pub fn command_end(cmd: impl Into<String>, exit_code: i32) -> Self {
        Self::OnCommandEnd {
            command: cmd.into(),
            exit_code,
        }
    }

    /// Create an OnResize hook.
    pub fn resize(cols: usize, rows: usize) -> Self {
        Self::OnResize { cols, rows }
    }

    /// Create an OnThemeChange hook.
    pub fn theme_change(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self::OnThemeChange {
            from: from.into(),
            to: to.into(),
        }
    }
}

/// Result of a plugin hook handler.
///
/// Controls how the terminal responds after the plugin processes a hook.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum HookResult {
    /// Allow the action to proceed normally (default).
    #[default]
    Allow,
    /// Deny the action — block it from proceeding.
    Deny,
    /// Replace the text content with a modified version.
    Transform(String),
    /// Allow but attach a metadata annotation for other plugins.
    Annotate(String, String),
}

impl HookResult {
    /// Create an Allow result.
    pub fn allow() -> Self {
        Self::Allow
    }

    /// Create a Deny result.
    pub fn deny() -> Self {
        Self::Deny
    }

    /// Create a Transform result.
    pub fn transform(text: impl Into<String>) -> Self {
        Self::Transform(text.into())
    }

    /// Create an Annotate result.
    pub fn annotate(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self::Annotate(key.into(), value.into())
    }

    /// Whether this result allows the action.
    pub fn is_allow(&self) -> bool {
        matches!(self, Self::Allow | Self::Transform(_) | Self::Annotate(..))
    }

    /// Whether this result denies the action.
    pub fn is_deny(&self) -> bool {
        matches!(self, Self::Deny)
    }

    /// Whether this result transforms text.
    pub fn is_transform(&self) -> bool {
        matches!(self, Self::Transform(_))
    }

    /// Get the transformed text if any.
    pub fn transformed_text(&self) -> Option<&str> {
        match self {
            Self::Transform(t) => Some(t),
            _ => None,
        }
    }
}

/// Aggregates results from multiple plugins for a single hook dispatch.
///
/// Priority: `Deny` > `Transform` > `Annotate` > `Allow`.
/// If any plugin returns `Deny`, the final result is `Deny`.
/// If multiple plugins return `Transform`, the last one wins.
#[derive(Debug, Clone, Default)]
pub struct HookResultAggregator {
    denied: bool,
    final_text: Option<String>,
    annotations: Vec<(String, String)>,
}

impl HookResultAggregator {
    /// Create a new empty aggregator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a plugin's result.
    pub fn add(&mut self, result: HookResult) {
        match result {
            HookResult::Allow => {}
            HookResult::Deny => self.denied = true,
            HookResult::Transform(text) => self.final_text = Some(text),
            HookResult::Annotate(k, v) => self.annotations.push((k, v)),
        }
    }

    /// Compute the final aggregated result.
    pub fn finalize(self) -> HookResult {
        if self.denied {
            return HookResult::Deny;
        }
        if let Some(text) = self.final_text {
            return HookResult::Transform(text);
        }
        HookResult::Allow
    }

    /// Get accumulated annotations.
    pub fn annotations(&self) -> &[(String, String)] {
        &self.annotations
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── HookType ──

    #[test]
    fn t_hook_type_all() {
        assert_eq!(HookType::all().len(), 6);
    }

    #[test]
    fn t_hook_type_name() {
        assert_eq!(HookType::OnInput.name(), "on_input");
        assert_eq!(HookType::OnOutput.name(), "on_output");
        assert_eq!(HookType::OnCommandStart.name(), "on_command_start");
        assert_eq!(HookType::OnCommandEnd.name(), "on_command_end");
        assert_eq!(HookType::OnResize.name(), "on_resize");
        assert_eq!(HookType::OnThemeChange.name(), "on_theme_change");
    }

    #[test]
    fn t_hook_type_display() {
        assert_eq!(format!("{}", HookType::OnInput), "on_input");
    }

    #[test]
    fn t_hook_type_eq_hash() {
        use std::collections::HashSet;
        let set: HashSet<HookType> = [HookType::OnInput, HookType::OnOutput]
            .into_iter()
            .collect();
        assert_eq!(set.len(), 2);
        assert!(set.contains(&HookType::OnInput));
    }

    // ── Hook ──

    #[test]
    fn t_hook_input() {
        let h = Hook::OnInput("ls".to_string());
        assert_eq!(h.hook_type(), HookType::OnInput);
        assert_eq!(h.text(), Some("ls"));
    }

    #[test]
    fn t_hook_output() {
        let h = Hook::OnOutput("hello".to_string());
        assert_eq!(h.hook_type(), HookType::OnOutput);
        assert_eq!(h.text(), Some("hello"));
    }

    #[test]
    fn t_hook_command_start() {
        let h = Hook::OnCommandStart("git push".to_string());
        assert_eq!(h.hook_type(), HookType::OnCommandStart);
        assert_eq!(h.text(), Some("git push"));
    }

    #[test]
    fn t_hook_command_end() {
        let h = Hook::OnCommandEnd {
            command: "ls".to_string(),
            exit_code: 0,
        };
        assert_eq!(h.hook_type(), HookType::OnCommandEnd);
        assert_eq!(h.text(), Some("ls"));
    }

    #[test]
    fn t_hook_resize() {
        let h = Hook::OnResize {
            cols: 120,
            rows: 40,
        };
        assert_eq!(h.hook_type(), HookType::OnResize);
        assert!(h.text().is_none());
    }

    #[test]
    fn t_hook_theme_change() {
        let h = Hook::OnThemeChange {
            from: "dark".to_string(),
            to: "light".to_string(),
        };
        assert_eq!(h.hook_type(), HookType::OnThemeChange);
        assert!(h.text().is_none());
    }

    // ── HookResult ──

    #[test]
    fn t_result_allow() {
        assert!(HookResult::allow().is_allow());
        assert!(!HookResult::Allow.is_deny());
    }

    #[test]
    fn t_result_deny() {
        assert!(HookResult::deny().is_deny());
        assert!(!HookResult::Deny.is_allow());
    }

    #[test]
    fn t_result_transform() {
        let r = HookResult::transform("modified");
        assert!(r.is_transform());
        assert!(r.is_allow());
        assert_eq!(r.transformed_text(), Some("modified"));
    }

    #[test]
    fn t_result_annotate() {
        let r = HookResult::annotate("k", "v");
        assert!(r.is_allow());
        assert!(!r.is_transform());
    }

    #[test]
    fn t_result_default() {
        assert_eq!(HookResult::default(), HookResult::Allow);
    }

    #[test]
    fn t_result_eq() {
        assert_eq!(HookResult::Allow, HookResult::Allow);
        assert_ne!(HookResult::Allow, HookResult::Deny);
        assert_eq!(HookResult::transform("a"), HookResult::transform("a"));
    }

    // ── Aggregator ──

    #[test]
    fn t_agg_empty() {
        assert_eq!(HookResultAggregator::new().finalize(), HookResult::Allow);
    }

    #[test]
    fn t_agg_all_allow() {
        let mut a = HookResultAggregator::new();
        a.add(HookResult::Allow);
        a.add(HookResult::Allow);
        assert_eq!(a.finalize(), HookResult::Allow);
    }

    #[test]
    fn t_agg_one_deny() {
        let mut a = HookResultAggregator::new();
        a.add(HookResult::Allow);
        a.add(HookResult::Deny);
        assert_eq!(a.finalize(), HookResult::Deny);
    }

    #[test]
    fn t_agg_last_transform_wins() {
        let mut a = HookResultAggregator::new();
        a.add(HookResult::transform("first"));
        a.add(HookResult::transform("second"));
        assert_eq!(a.finalize().transformed_text(), Some("second"));
    }

    #[test]
    fn t_agg_deny_over_transform() {
        let mut a = HookResultAggregator::new();
        a.add(HookResult::transform("text"));
        a.add(HookResult::Deny);
        assert_eq!(a.finalize(), HookResult::Deny);
    }

    #[test]
    fn t_agg_annotations() {
        let mut a = HookResultAggregator::new();
        a.add(HookResult::annotate("k1", "v1"));
        a.add(HookResult::annotate("k2", "v2"));
        assert_eq!(a.annotations().len(), 2);
    }
}
