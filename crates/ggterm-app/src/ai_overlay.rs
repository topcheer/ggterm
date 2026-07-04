//! AI overlay state management for the desktop terminal.
//!
//! Manages the visibility, content, and lifecycle of the AI assistant
//! overlay panel. The overlay appears at the bottom of the terminal and
//! shows AI responses (explain, suggest, error help, NL2command).
//!
//! Lifecycle: `Hidden → Thinking → Result → Hidden`

#![cfg(feature = "ai")]

/// Which AI action triggered the current overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AIAction {
    Explain,
    Suggest,
    ErrorHelp,
    NL2Command,
}

impl AIAction {
    /// Human-readable label for the overlay header.
    pub fn label(&self) -> &'static str {
        match self {
            AIAction::Explain => "Explain",
            AIAction::Suggest => "Suggest",
            AIAction::ErrorHelp => "Error Help",
            AIAction::NL2Command => "NL→Cmd",
        }
    }
}

impl From<ggterm_ai::Action> for AIAction {
    fn from(a: ggterm_ai::Action) -> Self {
        match a {
            ggterm_ai::Action::Explain => AIAction::Explain,
            ggterm_ai::Action::Suggest => AIAction::Suggest,
            ggterm_ai::Action::ErrorHelp => AIAction::ErrorHelp,
            ggterm_ai::Action::NL2Command => AIAction::NL2Command,
        }
    }
}

/// State of the AI overlay panel.
#[derive(Debug, Clone)]
pub struct AIOverlayState {
    /// Whether the overlay is currently visible.
    visible: bool,
    /// Whether the AI is currently thinking (request in flight).
    busy: bool,
    /// The action that triggered the current overlay.
    action: Option<AIAction>,
    /// The response text to display (None = no response yet).
    content: Option<String>,
    /// NL2Command input text (user types a natural language query).
    nl2cmd_input: String,
    /// Whether the user is currently typing in the NL2Command input.
    nl2cmd_typing: bool,
}

impl Default for AIOverlayState {
    fn default() -> Self {
        Self::new()
    }
}

impl AIOverlayState {
    /// Create a new hidden overlay.
    pub fn new() -> Self {
        Self {
            visible: false,
            busy: false,
            action: None,
            content: None,
            nl2cmd_input: String::new(),
            nl2cmd_typing: false,
        }
    }

    /// Whether the overlay is currently visible.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Whether the AI is currently thinking.
    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// The action that triggered this overlay.
    pub fn action(&self) -> Option<AIAction> {
        self.action
    }

    /// The response text (if any).
    pub fn content(&self) -> Option<&str> {
        self.content.as_deref()
    }

    /// Start a new AI request: show overlay, set busy, clear old content.
    pub fn start_request(&mut self, action: impl Into<AIAction>) {
        self.visible = true;
        self.busy = true;
        self.action = Some(action.into());
        self.content = None;
    }

    /// Set the response text and clear busy state.
    pub fn set_response(&mut self, text: impl Into<String>) {
        self.busy = false;
        self.content = Some(text.into());
    }

    /// Append streaming text delta. Used for progressive display.
    /// The overlay stays in busy state until `set_response` is called.
    pub fn append_streaming(&mut self, text: &str) {
        if self.content.is_none() {
            self.content = Some(String::new());
        }
        if let Some(ref mut content) = self.content {
            content.push_str(text);
        }
    }

    /// Set an error message and clear busy state.
    pub fn set_error(&mut self, error: impl std::fmt::Display) {
        self.busy = false;
        self.content = Some(format!("Error: {error}"));
    }

    /// Hide the overlay and reset state.
    pub fn hide(&mut self) {
        self.visible = false;
        self.busy = false;
        self.content = None;
        self.nl2cmd_input.clear();
        self.nl2cmd_typing = false;
    }

    /// Start NL2Command input mode.
    pub fn start_nl2cmd_input(&mut self) {
        self.visible = true;
        self.busy = false;
        self.content = None;
        self.action = Some(AIAction::NL2Command);
        self.nl2cmd_input.clear();
        self.nl2cmd_typing = true;
    }

    /// Append a character to the NL2Command input.
    pub fn nl2cmd_append(&mut self, ch: char) {
        if self.nl2cmd_typing {
            self.nl2cmd_input.push(ch);
        }
    }

    /// Remove the last character from NL2Command input.
    pub fn nl2cmd_backspace(&mut self) {
        if self.nl2cmd_typing {
            self.nl2cmd_input.pop();
        }
    }

    /// Submit the NL2Command input. Returns the natural language query.
    pub fn nl2cmd_submit(&mut self) -> Option<String> {
        if !self.nl2cmd_typing || self.nl2cmd_input.trim().is_empty() {
            return None;
        }
        self.nl2cmd_typing = false;
        let query = self.nl2cmd_input.trim().to_string();
        self.busy = true;
        Some(query)
    }

    /// Check if user is currently typing in NL2Command input.
    pub fn is_nl2cmd_typing(&self) -> bool {
        self.nl2cmd_typing
    }

    /// Get the NL2Command input text.
    pub fn nl2cmd_input(&self) -> &str {
        &self.nl2cmd_input
    }

    /// Toggle overlay visibility.
    pub fn toggle(&mut self) {
        if self.visible {
            self.hide();
        } else {
            self.visible = true;
        }
    }

    /// Render the overlay as plain text for the status bar.
    ///
    /// Returns empty string if not visible.
    pub fn render_text(&self) -> String {
        if !self.visible {
            return String::new();
        }

        let action_label = self.action.map(|a| a.label()).unwrap_or("AI");

        if self.busy {
            return format!(" AI [{}]: Thinking... (Esc to cancel) ", action_label);
        }

        match &self.content {
            Some(text) => {
                // Truncate to a reasonable width for a status bar.
                let truncated = if text.chars().count() > 200 {
                    format!("{}...", text.chars().take(197).collect::<String>())
                } else {
                    text.clone()
                };
                format!(" AI [{}]: {} (Esc to close) ", action_label, truncated)
            }
            None => format!(" AI [{}]: (no response) ", action_label),
        }
    }

    /// Render with ANSI color codes.
    pub fn render_colored(&self) -> String {
        if !self.visible {
            return String::new();
        }

        let action_label = self.action.map(|a| a.label()).unwrap_or("AI");

        if self.busy {
            // Yellow for "thinking"
            return format!("\x1b[33m AI [{action_label}]: Thinking... \x1b[0m");
        }

        match &self.content {
            Some(text) => {
                let truncated = if text.chars().count() > 200 {
                    format!("{}...", text.chars().take(197).collect::<String>())
                } else {
                    text.clone()
                };
                // Cyan for response
                format!("\x1b[36m AI [{action_label}]: {truncated} \x1b[0m")
            }
            None => format!("\x1b[34m AI [{action_label}]: (no response) \x1b[0m"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Construction ──

    #[test]
    fn test_new_overlay_hidden() {
        let state = AIOverlayState::new();
        assert!(!state.is_visible());
        assert!(!state.is_busy());
        assert!(state.action().is_none());
        assert!(state.content().is_none());
    }

    #[test]
    fn test_default_is_hidden() {
        let state = AIOverlayState::default();
        assert!(!state.is_visible());
    }

    // ── Action label ──

    #[test]
    fn test_action_labels() {
        assert_eq!(AIAction::Explain.label(), "Explain");
        assert_eq!(AIAction::Suggest.label(), "Suggest");
        assert_eq!(AIAction::ErrorHelp.label(), "Error Help");
        assert_eq!(AIAction::NL2Command.label(), "NL→Cmd");
    }

    #[test]
    fn test_action_from_ai_action() {
        assert_eq!(
            AIAction::from(ggterm_ai::Action::Explain),
            AIAction::Explain
        );
        assert_eq!(
            AIAction::from(ggterm_ai::Action::Suggest),
            AIAction::Suggest
        );
        assert_eq!(
            AIAction::from(ggterm_ai::Action::ErrorHelp),
            AIAction::ErrorHelp
        );
        assert_eq!(
            AIAction::from(ggterm_ai::Action::NL2Command),
            AIAction::NL2Command
        );
    }

    // ── start_request lifecycle ──

    #[test]
    fn test_start_request_shows_overlay() {
        let mut state = AIOverlayState::new();
        state.start_request(AIAction::Explain);
        assert!(state.is_visible());
        assert!(state.is_busy());
        assert_eq!(state.action(), Some(AIAction::Explain));
        assert!(state.content().is_none());
    }

    #[test]
    fn test_start_request_clears_previous() {
        let mut state = AIOverlayState::new();
        state.start_request(AIAction::Explain);
        state.set_response("old response");
        assert_eq!(state.content(), Some("old response"));

        // New request clears old content
        state.start_request(AIAction::Suggest);
        assert!(state.is_busy());
        assert!(state.content().is_none());
        assert_eq!(state.action(), Some(AIAction::Suggest));
    }

    // ── set_response ──

    #[test]
    fn test_set_response_clears_busy() {
        let mut state = AIOverlayState::new();
        state.start_request(AIAction::Explain);
        assert!(state.is_busy());

        state.set_response("This command lists files.");
        assert!(!state.is_busy());
        assert!(state.is_visible());
        assert_eq!(state.content(), Some("This command lists files."));
    }

    #[test]
    fn test_set_response_preserves_action() {
        let mut state = AIOverlayState::new();
        state.start_request(AIAction::Suggest);
        state.set_response("Try: git status");
        assert_eq!(state.action(), Some(AIAction::Suggest));
    }

    // ── set_error ──

    #[test]
    fn test_set_error_clears_busy() {
        let mut state = AIOverlayState::new();
        state.start_request(AIAction::Explain);
        state.set_error("No API key configured");
        assert!(!state.is_busy());
        assert!(state.content().unwrap().contains("Error: No API key"));
    }

    #[test]
    fn test_set_error_format() {
        let mut state = AIOverlayState::new();
        state.start_request(AIAction::ErrorHelp);
        state.set_error(ggterm_ai::AIError::NoApiKey);
        let content = state.content().unwrap();
        assert!(content.contains("Error:"));
        assert!(content.contains("no API key"));
    }

    // ── hide ──

    #[test]
    fn test_hide_resets_all() {
        let mut state = AIOverlayState::new();
        state.start_request(AIAction::Explain);
        state.set_response("some text");

        state.hide();
        assert!(!state.is_visible());
        assert!(!state.is_busy());
        assert!(state.action().is_none());
        assert!(state.content().is_none());
    }

    #[test]
    fn test_hide_when_already_hidden() {
        let mut state = AIOverlayState::new();
        state.hide(); // Should not panic
        assert!(!state.is_visible());
    }

    // ── toggle ──

    #[test]
    fn test_toggle_shows_hidden() {
        let mut state = AIOverlayState::new();
        assert!(!state.is_visible());
        state.toggle();
        assert!(state.is_visible());
    }

    #[test]
    fn test_toggle_hides_visible() {
        let mut state = AIOverlayState::new();
        state.start_request(AIAction::Explain);
        assert!(state.is_visible());
        state.toggle();
        assert!(!state.is_visible());
    }

    // ── render_text ──

    #[test]
    fn test_render_text_hidden() {
        let state = AIOverlayState::new();
        assert!(state.render_text().is_empty());
    }

    #[test]
    fn test_render_text_thinking() {
        let mut state = AIOverlayState::new();
        state.start_request(AIAction::Explain);
        let text = state.render_text();
        assert!(text.contains("AI"));
        assert!(text.contains("Explain"));
        assert!(text.contains("Thinking"));
    }

    #[test]
    fn test_render_text_with_response() {
        let mut state = AIOverlayState::new();
        state.start_request(AIAction::Suggest);
        state.set_response("Try: ls -la");
        let text = state.render_text();
        assert!(text.contains("AI"));
        assert!(text.contains("Suggest"));
        assert!(text.contains("Try: ls -la"));
        assert!(text.contains("Esc"));
    }

    #[test]
    fn test_render_text_truncates_long_response() {
        let mut state = AIOverlayState::new();
        state.start_request(AIAction::Explain);
        let long = "x".repeat(300);
        state.set_response(long);
        let text = state.render_text();
        // Should be truncated with "..."
        assert!(text.contains("..."));
        assert!(text.len() < 300);
    }

    #[test]
    fn test_render_text_no_response() {
        let mut state = AIOverlayState::new();
        state.visible = true;
        state.action = Some(AIAction::Explain);
        state.busy = false;
        let text = state.render_text();
        assert!(text.contains("no response"));
    }

    // ── render_colored ──

    #[test]
    fn test_render_colored_hidden() {
        let state = AIOverlayState::new();
        assert!(state.render_colored().is_empty());
    }

    #[test]
    fn test_render_colored_thinking() {
        let mut state = AIOverlayState::new();
        state.start_request(AIAction::Explain);
        let colored = state.render_colored();
        // Yellow ANSI code
        assert!(colored.contains("\x1b[33m"));
        assert!(colored.contains("Thinking"));
    }

    #[test]
    fn test_render_colored_response() {
        let mut state = AIOverlayState::new();
        state.start_request(AIAction::Suggest);
        state.set_response("git status");
        let colored = state.render_colored();
        // Cyan ANSI code
        assert!(colored.contains("\x1b[36m"));
        assert!(colored.contains("git status"));
    }

    // ── Full lifecycle ──

    #[test]
    fn test_full_lifecycle_explain() {
        let mut state = AIOverlayState::new();

        // 1. User triggers Ctrl+Shift+E
        state.start_request(AIAction::Explain);
        assert!(state.is_visible());
        assert!(state.is_busy());

        // 2. AI response arrives
        state.set_response("This lists all files.");
        assert!(!state.is_busy());
        assert!(state.is_visible());

        // 3. User presses Esc
        state.hide();
        assert!(!state.is_visible());
    }

    #[test]
    fn test_full_lifecycle_error() {
        let mut state = AIOverlayState::new();

        state.start_request(AIAction::ErrorHelp);
        assert!(state.is_busy());

        state.set_error("Connection refused");
        assert!(!state.is_busy());
        assert!(state.content().unwrap().contains("Connection refused"));

        state.hide();
        assert!(!state.is_visible());
    }

    #[test]
    fn test_repeated_requests() {
        let mut state = AIOverlayState::new();

        // First request
        state.start_request(AIAction::Explain);
        state.set_response("response 1");
        assert_eq!(state.content(), Some("response 1"));

        // Second request (different action)
        state.start_request(AIAction::Suggest);
        assert!(state.is_busy());
        assert!(state.content().is_none());
        state.set_response("response 2");
        assert_eq!(state.content(), Some("response 2"));
        assert_eq!(state.action(), Some(AIAction::Suggest));
    }

    #[test]
    fn test_busy_overlay_does_not_block_new_request() {
        // The overlay allows starting a new request while busy
        // (AIBridge itself will reject, but overlay state should allow it)
        let mut state = AIOverlayState::new();
        state.start_request(AIAction::Explain);
        assert!(state.is_busy());

        // Overlay allows re-triggering
        state.start_request(AIAction::Suggest);
        assert!(state.is_busy());
        assert_eq!(state.action(), Some(AIAction::Suggest));
    }
}
