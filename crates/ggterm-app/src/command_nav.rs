//! Command block navigation: scroll-to-command, exit status query, command history.
//!
//! Uses the [`CommandBlock`] API from `ggterm-core` to provide:
//! - Cursor-based navigation between command blocks (next/previous/jump)
//! - Scroll target computation (scroll viewport to a specific command)
//! - Command history extraction (text of all completed commands)
//! - Exit status summary (success/fail counts, last N results)
//!
//! Phase 8-D additions:
//! - [`CommandNavState`] — overlay visibility + jump logic for keyboard navigation
//! - [`CommandNavOverlay`] — renders a status bar (command text + exit code colors)

use ggterm_core::{CommandBlock, Terminal};

/// Navigator that tracks which command block the user has selected.
///
/// The navigator is a lightweight overlay on top of [`Terminal::command_blocks()`].
/// It does not own terminal state — call methods with a `&Terminal` reference.
#[derive(Debug, Clone)]
pub struct CommandNavigator {
    /// Index into the command blocks vector. `None` = no selection.
    selected: Option<usize>,
}

impl Default for CommandNavigator {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandNavigator {
    /// Create a new navigator with no selection.
    pub fn new() -> Self {
        Self { selected: None }
    }

    // ------------------------------------------------------------------
    // Selection
    // ------------------------------------------------------------------

    /// Return the index of the currently selected command block, if any.
    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    /// Explicitly set the selected index.
    /// Clamps to the valid range; `None` clears the selection.
    pub fn set_selected(&mut self, index: Option<usize>, terminal: &Terminal) {
        match index {
            None => self.selected = None,
            Some(i) => {
                let count = terminal.command_blocks().len();
                if count == 0 {
                    self.selected = None;
                } else {
                    self.selected = Some(i.min(count - 1));
                }
            }
        }
    }

    /// Move selection to the next command block (towards more recent commands).
    /// Returns the new selection, or `None` if already at the end or no commands.
    pub fn next_command(&mut self, terminal: &Terminal) -> Option<usize> {
        let blocks = terminal.command_blocks();
        if blocks.is_empty() {
            return None;
        }
        let max = blocks.len() - 1;
        let new_idx = match self.selected {
            None => max, // start from the most recent
            Some(i) => {
                if i >= max {
                    return self.selected;
                }
                i + 1
            }
        };
        self.selected = Some(new_idx);
        self.selected
    }

    /// Move selection to the previous command block (towards older commands).
    /// Returns the new selection, or `None` if already at the first or no commands.
    pub fn prev_command(&mut self, terminal: &Terminal) -> Option<usize> {
        let blocks = terminal.command_blocks();
        if blocks.is_empty() {
            return None;
        }
        let new_idx = match self.selected {
            None => 0, // start from the oldest
            Some(0) => return self.selected,
            Some(i) => i - 1,
        };
        self.selected = Some(new_idx);
        self.selected
    }

    /// Select the first (oldest) command block.
    /// Returns the index if any commands exist.
    pub fn first_command(&mut self, terminal: &Terminal) -> Option<usize> {
        if terminal.command_blocks().is_empty() {
            self.selected = None;
        } else {
            self.selected = Some(0);
        }
        self.selected
    }

    /// Select the last (most recent) command block.
    /// Returns the index if any commands exist.
    pub fn last_command(&mut self, terminal: &Terminal) -> Option<usize> {
        let blocks = terminal.command_blocks();
        if blocks.is_empty() {
            self.selected = None;
        } else {
            self.selected = Some(blocks.len() - 1);
        }
        self.selected
    }

    /// Clear the current selection.
    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    /// Return the currently selected command block, if any.
    pub fn current_block(&self, terminal: &Terminal) -> Option<CommandBlock> {
        self.selected
            .and_then(|i| terminal.command_blocks().get(i).cloned())
    }

    // ------------------------------------------------------------------
    // Scroll targets
    // ------------------------------------------------------------------

    /// Compute the scroll offset (in rows) that would bring the currently
    /// selected command block's prompt into view at the top of the viewport.
    ///
    /// Returns `None` if no block is selected.
    pub fn scroll_target(&self, terminal: &Terminal) -> Option<usize> {
        self.current_block(terminal).map(|b| b.prompt_row)
    }

    /// Compute the scroll offset for a specific block index.
    /// Returns `None` if the index is out of bounds.
    pub fn scroll_to_index(&self, terminal: &Terminal, index: usize) -> Option<usize> {
        terminal.command_blocks().get(index).map(|b| b.prompt_row)
    }

    // ------------------------------------------------------------------
    // Querying
    // ------------------------------------------------------------------

    /// Return the number of command blocks.
    pub fn block_count(&self, terminal: &Terminal) -> usize {
        terminal.command_blocks().len()
    }

    /// Return true if any command has failed (non-zero exit code).
    pub fn has_failures(&self, terminal: &Terminal) -> bool {
        terminal.command_blocks().iter().any(|b| b.is_failure())
    }

    /// Return the exit code of the selected command block.
    pub fn selected_exit_code(&self, terminal: &Terminal) -> Option<i32> {
        self.current_block(terminal).and_then(|b| b.exit_code)
    }

    /// Return whether the selected command succeeded.
    pub fn selected_succeeded(&self, terminal: &Terminal) -> Option<bool> {
        self.current_block(terminal).map(|b| b.is_success())
    }

    // ------------------------------------------------------------------
    // Command history
    // ------------------------------------------------------------------

    /// Extract command text from all completed commands.
    ///
    /// Each entry is the text of the command_row (the row where the user
    /// typed the command, between CommandStart and OutputStart).
    /// Running commands (no CommandEnd) are excluded.
    pub fn command_history(&self, terminal: &Terminal) -> Vec<String> {
        terminal
            .command_blocks()
            .into_iter()
            .filter(|b| b.is_complete())
            .filter_map(|b| {
                b.command_row
                    .map(|row| terminal.extract_absolute_row_text(row))
            })
            .collect()
    }

    /// Extract command text including the currently running command.
    pub fn command_history_all(&self, terminal: &Terminal) -> Vec<String> {
        terminal
            .command_blocks()
            .into_iter()
            .filter_map(|b| {
                b.command_row
                    .map(|row| terminal.extract_absolute_row_text(row))
            })
            .collect()
    }

    /// Search command history for commands containing the given substring.
    /// Case-insensitive. Returns matching command texts in order.
    pub fn search_history(&self, terminal: &Terminal, query: &str) -> Vec<String> {
        let lower = query.to_lowercase();
        self.command_history(terminal)
            .into_iter()
            .filter(|text| text.to_lowercase().contains(&lower))
            .collect()
    }

    /// Find the index of the Nth command matching the query.
    pub fn find_command(&self, terminal: &Terminal, query: &str) -> Option<usize> {
        let lower = query.to_lowercase();
        terminal.command_blocks().iter().position(|b| {
            if let Some(row) = b.command_row {
                terminal
                    .extract_absolute_row_text(row)
                    .to_lowercase()
                    .contains(&lower)
            } else {
                false
            }
        })
    }

    // ------------------------------------------------------------------
    // Exit status summary
    // ------------------------------------------------------------------

    /// Summarize the exit status of all completed commands.
    pub fn exit_status_summary(&self, terminal: &Terminal) -> ExitStatusSummary {
        let blocks = terminal.command_blocks();
        let total = blocks.iter().filter(|b| b.is_complete()).count();
        let succeeded = blocks.iter().filter(|b| b.is_success()).count();
        let failed = blocks.iter().filter(|b| b.is_failure()).count();
        let running = blocks.iter().filter(|b| b.is_running()).count();

        ExitStatusSummary {
            total,
            succeeded,
            failed,
            running,
        }
    }

    /// Return the last N exit codes (most recent last).
    pub fn recent_exit_codes(&self, terminal: &Terminal, n: usize) -> Vec<i32> {
        terminal
            .command_blocks()
            .iter()
            .rev()
            .filter_map(|b| b.exit_code)
            .take(n)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }
}

/// Summary of command execution status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExitStatusSummary {
    /// Total number of completed commands.
    pub total: usize,
    /// Number of commands that succeeded (exit code 0).
    pub succeeded: usize,
    /// Number of commands that failed (non-zero exit code).
    pub failed: usize,
    /// Number of commands currently running.
    pub running: usize,
}

impl ExitStatusSummary {
    /// Return the success rate as a fraction (0.0 to 1.0).
    /// Returns 0.0 if no commands have completed.
    pub fn success_rate(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        self.succeeded as f64 / self.total as f64
    }

    /// Return true if all commands succeeded.
    pub fn all_succeeded(&self) -> bool {
        self.total > 0 && self.failed == 0
    }

    /// Return true if any command failed.
    pub fn any_failed(&self) -> bool {
        self.failed > 0
    }
}

impl std::fmt::Display for ExitStatusSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} total ({} ok, {} fail, {} running)",
            self.total, self.succeeded, self.failed, self.running
        )
    }
}

// ══════════════════════════════════════════════════════════════════
// Phase 8-D: Command Navigation Overlay
// ══════════════════════════════════════════════════════════════════

/// State for the command navigation overlay.
///
/// Wraps a [`CommandNavigator`] with visibility tracking and provides
/// jump logic for keyboard-driven command block navigation (Ctrl+Shift+Up/Down).
#[derive(Debug, Clone)]
pub struct CommandNavState {
    navigator: CommandNavigator,
    visible: bool,
}

impl Default for CommandNavState {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandNavState {
    /// Create new state with overlay hidden and no selection.
    pub fn new() -> Self {
        Self {
            navigator: CommandNavigator::new(),
            visible: false,
        }
    }

    // ── Visibility ──

    /// Whether the overlay is currently visible.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Show the overlay.
    pub fn show(&mut self) {
        self.visible = true;
    }

    /// Hide the overlay and clear selection.
    pub fn hide(&mut self) {
        self.visible = false;
        self.navigator.clear_selection();
    }

    /// Toggle overlay visibility.
    pub fn toggle(&mut self) {
        if self.visible {
            self.hide();
        } else {
            self.show();
        }
    }

    // ── Navigator access ──

    /// Access the underlying navigator.
    pub fn navigator(&self) -> &CommandNavigator {
        &self.navigator
    }

    /// Mutable access to the navigator.
    pub fn navigator_mut(&mut self) -> &mut CommandNavigator {
        &mut self.navigator
    }

    // ── Jump logic ──

    /// Jump to the next (more recent) command block.
    /// Auto-shows the overlay. Returns the selected index.
    pub fn jump_next(&mut self, terminal: &Terminal) -> Option<usize> {
        let result = self.navigator.next_command(terminal);
        self.visible = true;
        result
    }

    /// Jump to the previous (older) command block.
    pub fn jump_prev(&mut self, terminal: &Terminal) -> Option<usize> {
        let result = self.navigator.prev_command(terminal);
        self.visible = true;
        result
    }

    /// Jump to the first (oldest) command block.
    pub fn jump_first(&mut self, terminal: &Terminal) -> Option<usize> {
        let result = self.navigator.first_command(terminal);
        self.visible = true;
        result
    }

    /// Jump to the last (most recent) command block.
    pub fn jump_last(&mut self, terminal: &Terminal) -> Option<usize> {
        let result = self.navigator.last_command(terminal);
        self.visible = true;
        result
    }

    /// Reset: hide overlay, clear selection.
    pub fn reset(&mut self) {
        self.visible = false;
        self.navigator.clear_selection();
    }

    // ── Derived info ──

    /// Get the scroll target row for the selected block.
    pub fn scroll_target(&self, terminal: &Terminal) -> Option<usize> {
        self.navigator.scroll_target(terminal)
    }

    /// Get the currently selected command block.
    pub fn current_block(&self, terminal: &Terminal) -> Option<CommandBlock> {
        self.navigator.current_block(terminal)
    }

    /// Get the exit status summary.
    pub fn exit_summary(&self, terminal: &Terminal) -> ExitStatusSummary {
        self.navigator.exit_status_summary(terminal)
    }
}

/// Renders a command navigation status bar overlay.
///
/// Colors:
/// - Green: success (exit 0)
/// - Red: failure (exit non-zero)
/// - Yellow: running
/// - Blue: no selection
pub struct CommandNavOverlay {
    max_cmd_width: usize,
}

impl Default for CommandNavOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandNavOverlay {
    /// Create a new overlay with default settings.
    pub fn new() -> Self {
        Self { max_cmd_width: 60 }
    }

    /// Set the maximum command text width.
    pub fn set_max_cmd_width(&mut self, width: usize) {
        self.max_cmd_width = width;
    }

    /// Get the maximum command text width.
    pub fn max_cmd_width(&self) -> usize {
        self.max_cmd_width
    }

    /// Truncate text to `max_cmd_width` chars, adding "..." if truncated.
    fn truncate(&self, text: &str) -> String {
        let chars: Vec<char> = text.chars().collect();
        if chars.len() <= self.max_cmd_width {
            text.to_string()
        } else {
            let kept: String = chars[..self.max_cmd_width.saturating_sub(3)]
                .iter()
                .collect();
            format!("{kept}...")
        }
    }

    /// Determine ANSI foreground color based on selected block's exit status.
    fn status_color(&self, state: &CommandNavState, terminal: &Terminal) -> &'static str {
        let Some(idx) = state.navigator().selected_index() else {
            return "\x1b[34m"; // blue (no selection)
        };
        let blocks = terminal.command_blocks();
        let Some(block) = blocks.get(idx) else {
            return "\x1b[34m";
        };
        if block.is_success() {
            "\x1b[32m" // green
        } else if block.is_failure() {
            "\x1b[31m" // red
        } else {
            "\x1b[33m" // yellow (running)
        }
    }

    /// Render the overlay as plain text (no ANSI codes).
    ///
    /// Returns empty string if the overlay is not visible.
    pub fn render_text(&self, state: &CommandNavState, terminal: &Terminal) -> String {
        if !state.is_visible() {
            return String::new();
        }

        let blocks = terminal.command_blocks();
        let total = blocks.len();

        if total == 0 {
            return "No commands".to_string();
        }

        let summary = state.navigator().exit_status_summary(terminal);

        if let Some(idx) = state.navigator().selected_index() {
            if let Some(block) = blocks.get(idx) {
                let cmd_text = block
                    .command_row
                    .map(|row| self.truncate(&terminal.extract_absolute_row_text(row)))
                    .unwrap_or_else(|| "(prompt)".to_string());

                let exit_part = if block.is_success() {
                    format!("exit: {}", block.exit_code.unwrap_or(0))
                } else if block.is_failure() {
                    format!("exit: {}", block.exit_code.unwrap_or(-1))
                } else {
                    "running".to_string()
                };

                format!("[{}/{}] {} ({})", idx + 1, total, cmd_text, exit_part)
            } else {
                format!("[?/{total}]")
            }
        } else {
            format!(
                "{} cmd ({} ok, {} fail)",
                summary.total, summary.succeeded, summary.failed
            )
        }
    }

    /// Render the overlay as ANSI-colored text.
    pub fn render_colored(&self, state: &CommandNavState, terminal: &Terminal) -> String {
        if !state.is_visible() {
            return String::new();
        }

        let text = self.render_text(state, terminal);
        if text.is_empty() {
            return text;
        }

        let color = self.status_color(state, terminal);
        format!("{color}{text}\x1b[0m")
    }

    /// Render a full-width status bar padded to `width`.
    pub fn render_bar(&self, state: &CommandNavState, terminal: &Terminal, width: usize) -> String {
        if !state.is_visible() {
            return String::new();
        }

        let text = self.render_text(state, terminal);
        if text.is_empty() {
            return String::new();
        }

        let color = self.status_color(state, terminal);
        let display_len = text.chars().count() + 2;
        let padding = width.saturating_sub(display_len);

        format!("{} {}{}\x1b[0m", color, text, " ".repeat(padding))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ggterm_core::Terminal;

    /// Feed raw bytes into the terminal as if received from the PTY.
    fn feed(term: &mut Terminal, data: &[u8]) {
        let mut p = ggterm_core::Parser::new();
        p.feed(data, term);
    }

    /// Helper: emit a complete command cycle at the given row.
    fn emit_cycle(term: &mut Terminal, row: usize, exit_code: Option<i32>) {
        // Prompt start
        feed(term, b"\x1b]133;A\x07");
        // Command start
        feed(term, b"\x1b]133;B\x07");
        // Move cursor to simulate output
        for _ in 0..=row {
            feed(term, b"\n");
        }
        // Output start
        feed(term, b"\x1b]133;C\x07");
        // Command end with exit code
        match exit_code {
            Some(code) => {
                let seq = format!("\x1b]133;D;{}\x07", code);
                feed(term, seq.as_bytes());
            }
            None => {
                feed(term, b"\x1b]133;D\x07");
            }
        }
    }

    // ================================================================
    // CommandNavigator basics
    // ================================================================

    #[test]
    fn t_nav_new_empty() {
        let term = Terminal::new(80, 24);
        let nav = CommandNavigator::new();
        assert_eq!(nav.selected_index(), None);
        assert_eq!(nav.block_count(&term), 0);
        assert!(nav.command_history(&term).is_empty());
    }

    #[test]
    fn t_nav_default_is_empty() {
        let term = Terminal::new(80, 24);
        let nav = CommandNavigator::default();
        assert_eq!(nav.selected_index(), None);
        assert_eq!(nav.block_count(&term), 0);
    }

    #[test]
    fn t_nav_next_command_empty() {
        let term = Terminal::new(80, 24);
        let mut nav = CommandNavigator::new();
        assert_eq!(nav.next_command(&term), None);
    }

    #[test]
    fn t_nav_prev_command_empty() {
        let term = Terminal::new(80, 24);
        let mut nav = CommandNavigator::new();
        assert_eq!(nav.prev_command(&term), None);
    }

    #[test]
    fn t_nav_first_command_empty() {
        let term = Terminal::new(80, 24);
        let mut nav = CommandNavigator::new();
        assert_eq!(nav.first_command(&term), None);
    }

    #[test]
    fn t_nav_last_command_empty() {
        let term = Terminal::new(80, 24);
        let mut nav = CommandNavigator::new();
        assert_eq!(nav.last_command(&term), None);
    }

    // ================================================================
    // Single command navigation
    // ================================================================

    #[test]
    fn t_nav_next_command_single() {
        let mut term = Terminal::new(80, 24);
        emit_cycle(&mut term, 0, Some(0));

        let mut nav = CommandNavigator::new();
        let idx = nav.next_command(&term);
        assert_eq!(idx, Some(0));
        assert_eq!(nav.selected_index(), Some(0));
    }

    #[test]
    fn t_nav_prev_after_next_single() {
        let mut term = Terminal::new(80, 24);
        emit_cycle(&mut term, 0, Some(0));

        let mut nav = CommandNavigator::new();
        nav.next_command(&term);
        // prev should stay at 0 (only 1 command)
        let idx = nav.prev_command(&term);
        assert_eq!(idx, Some(0));
    }

    #[test]
    fn t_nav_clear_selection() {
        let mut term = Terminal::new(80, 24);
        emit_cycle(&mut term, 0, Some(0));

        let mut nav = CommandNavigator::new();
        nav.next_command(&term);
        assert!(nav.selected_index().is_some());
        nav.clear_selection();
        assert_eq!(nav.selected_index(), None);
    }

    // ================================================================
    // Multiple commands navigation
    // ================================================================

    #[test]
    fn t_nav_multiple_commands_forward() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 2, Some(0));
        emit_cycle(&mut term, 4, Some(127));

        let mut nav = CommandNavigator::new();
        assert_eq!(nav.block_count(&term), 3);

        // next should go to last (index 2)
        assert_eq!(nav.next_command(&term), Some(2));
        // prev should go to index 1
        assert_eq!(nav.prev_command(&term), Some(1));
        // prev again to index 0
        assert_eq!(nav.prev_command(&term), Some(0));
        // prev at 0 stays at 0
        assert_eq!(nav.prev_command(&term), Some(0));
    }

    #[test]
    fn t_nav_multiple_commands_backward() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 2, Some(1));

        let mut nav = CommandNavigator::new();
        // prev from None goes to first (0)
        assert_eq!(nav.prev_command(&term), Some(0));
        // prev at 0 stays
        assert_eq!(nav.prev_command(&term), Some(0));
    }

    #[test]
    fn t_nav_first_last_command() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 2, Some(0));
        emit_cycle(&mut term, 4, Some(0));

        let mut nav = CommandNavigator::new();
        assert_eq!(nav.first_command(&term), Some(0));
        assert_eq!(nav.last_command(&term), Some(2));
    }

    // ================================================================
    // Set selected
    // ================================================================

    #[test]
    fn t_nav_set_selected_valid() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 2, Some(0));

        let mut nav = CommandNavigator::new();
        nav.set_selected(Some(1), &term);
        assert_eq!(nav.selected_index(), Some(1));
    }

    #[test]
    fn t_nav_set_selected_clamp() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));

        let mut nav = CommandNavigator::new();
        nav.set_selected(Some(100), &term);
        assert_eq!(nav.selected_index(), Some(0));
    }

    #[test]
    fn t_nav_set_selected_none() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));

        let mut nav = CommandNavigator::new();
        nav.set_selected(Some(0), &term);
        nav.set_selected(None, &term);
        assert_eq!(nav.selected_index(), None);
    }

    #[test]
    fn t_nav_set_selected_empty_blocks() {
        let term = Terminal::new(80, 24);
        let mut nav = CommandNavigator::new();
        nav.set_selected(Some(0), &term);
        assert_eq!(nav.selected_index(), None);
    }

    // ================================================================
    // Current block & scroll target
    // ================================================================

    #[test]
    fn t_nav_current_block_none() {
        let mut term = Terminal::new(80, 24);
        emit_cycle(&mut term, 0, Some(0));

        let nav = CommandNavigator::new();
        assert!(nav.current_block(&term).is_none());
    }

    #[test]
    fn t_nav_current_block_selected() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));

        let mut nav = CommandNavigator::new();
        nav.next_command(&term);
        let block = nav.current_block(&term);
        assert!(block.is_some());
        assert_eq!(block.unwrap().exit_code, Some(0));
    }

    #[test]
    fn t_nav_scroll_target_none() {
        let mut term = Terminal::new(80, 24);
        emit_cycle(&mut term, 0, Some(0));

        let nav = CommandNavigator::new();
        assert!(nav.scroll_target(&term).is_none());
    }

    #[test]
    fn t_nav_scroll_target_selected() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 5, Some(0));

        let mut nav = CommandNavigator::new();
        nav.set_selected(Some(1), &term);
        let target = nav.scroll_target(&term);
        assert!(target.is_some());
        // target should be >= 1 (the prompt_row of block 1, after first cycle)
        assert!(target.unwrap() >= 1);
    }

    #[test]
    fn t_nav_scroll_to_index() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 3, Some(0));

        let nav = CommandNavigator::new();
        assert!(nav.scroll_to_index(&term, 0).is_some());
        assert!(nav.scroll_to_index(&term, 1).is_some());
        assert!(nav.scroll_to_index(&term, 99).is_none());
    }

    // ================================================================
    // Exit status queries
    // ================================================================

    #[test]
    fn t_nav_selected_exit_code() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(42));

        let mut nav = CommandNavigator::new();
        nav.next_command(&term);
        assert_eq!(nav.selected_exit_code(&term), Some(42));
    }

    #[test]
    fn t_nav_selected_succeeded() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));

        let mut nav = CommandNavigator::new();
        nav.next_command(&term);
        assert_eq!(nav.selected_succeeded(&term), Some(true));
    }

    #[test]
    fn t_nav_selected_failed() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(1));

        let mut nav = CommandNavigator::new();
        nav.next_command(&term);
        assert_eq!(nav.selected_succeeded(&term), Some(false));
    }

    #[test]
    fn t_nav_has_failures_true() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 2, Some(1));

        let nav = CommandNavigator::new();
        assert!(nav.has_failures(&term));
    }

    #[test]
    fn t_nav_has_failures_false() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 2, Some(0));

        let nav = CommandNavigator::new();
        assert!(!nav.has_failures(&term));
    }

    // ================================================================
    // Command history
    // ================================================================

    #[test]
    fn t_nav_command_history_empty() {
        let term = Terminal::new(80, 24);
        let nav = CommandNavigator::new();
        assert!(nav.command_history(&term).is_empty());
    }

    #[test]
    fn t_nav_command_history_complete_only() {
        let mut term = Terminal::new(80, 30);
        // Completed command
        emit_cycle(&mut term, 0, Some(0));
        // Running command (no D mark)
        feed(&mut term, b"\x1b]133;A\x07");

        let nav = CommandNavigator::new();
        let history = nav.command_history(&term);
        // Only 1 completed command
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn t_nav_command_history_all_includes_running() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        // Running command
        feed(&mut term, b"\x1b]133;A\x07");
        feed(&mut term, b"\x1b]133;B\x07");

        let nav = CommandNavigator::new();
        let history = nav.command_history_all(&term);
        assert!(history.len() >= 1);
    }

    #[test]
    fn t_nav_search_history_match() {
        let mut term = Terminal::new(80, 30);
        // Type "ls" as command
        feed(&mut term, b"\x1b]133;A\x07");
        feed(&mut term, b"$ ls");
        feed(&mut term, b"\x1b]133;B\x07");
        feed(&mut term, b"\x1b]133;C\x07");
        feed(&mut term, b"file.txt\n");
        feed(&mut term, b"\x1b]133;D;0\x07");

        // Type "pwd" as command
        feed(&mut term, b"\x1b]133;A\x07");
        feed(&mut term, b"$ pwd");
        feed(&mut term, b"\x1b]133;B\x07");
        feed(&mut term, b"\x1b]133;C\x07");
        feed(&mut term, b"/home\n");
        feed(&mut term, b"\x1b]133;D;0\x07");

        let nav = CommandNavigator::new();
        let results = nav.search_history(&term, "pwd");
        assert!(!results.is_empty(), "should find 'pwd' in history");
    }

    #[test]
    fn t_nav_search_history_case_insensitive() {
        let mut term = Terminal::new(80, 30);
        feed(&mut term, b"\x1b]133;A\x07");
        feed(&mut term, b"$ LS -la");
        feed(&mut term, b"\x1b]133;B\x07");
        feed(&mut term, b"\x1b]133;C\x07");
        feed(&mut term, b"file.txt\n");
        feed(&mut term, b"\x1b]133;D;0\x07");

        let nav = CommandNavigator::new();
        let results = nav.search_history(&term, "ls");
        assert!(
            !results.is_empty(),
            "case-insensitive search should find 'LS'"
        );
    }

    #[test]
    fn t_nav_search_history_no_match() {
        let mut term = Terminal::new(80, 30);
        feed(&mut term, b"\x1b]133;A\x07");
        feed(&mut term, b"$ ls");
        feed(&mut term, b"\x1b]133;B\x07");
        feed(&mut term, b"\x1b]133;C\x07");
        feed(&mut term, b"\x1b]133;D;0\x07");

        let nav = CommandNavigator::new();
        let results = nav.search_history(&term, "git");
        assert!(results.is_empty(), "should not find 'git'");
    }

    #[test]
    fn t_nav_find_command_found() {
        let mut term = Terminal::new(80, 30);
        feed(&mut term, b"\x1b]133;A\x07");
        feed(&mut term, b"$ ls");
        feed(&mut term, b"\x1b]133;B\x07");
        feed(&mut term, b"\x1b]133;C\x07");
        feed(&mut term, b"\x1b]133;D;0\x07");

        let nav = CommandNavigator::new();
        assert!(nav.find_command(&term, "ls").is_some());
    }

    #[test]
    fn t_nav_find_command_not_found() {
        let mut term = Terminal::new(80, 30);
        feed(&mut term, b"\x1b]133;A\x07");
        feed(&mut term, b"$ ls");
        feed(&mut term, b"\x1b]133;B\x07");
        feed(&mut term, b"\x1b]133;C\x07");
        feed(&mut term, b"\x1b]133;D;0\x07");

        let nav = CommandNavigator::new();
        assert!(nav.find_command(&term, "git").is_none());
    }

    // ================================================================
    // Exit status summary
    // ================================================================

    #[test]
    fn t_nav_exit_status_summary_empty() {
        let term = Terminal::new(80, 24);
        let nav = CommandNavigator::new();
        let summary = nav.exit_status_summary(&term);
        assert_eq!(summary.total, 0);
        assert_eq!(summary.succeeded, 0);
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.running, 0);
    }

    #[test]
    fn t_nav_exit_status_summary_mixed() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0)); // success
        emit_cycle(&mut term, 2, Some(0)); // success
        emit_cycle(&mut term, 4, Some(1)); // fail

        let nav = CommandNavigator::new();
        let summary = nav.exit_status_summary(&term);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.succeeded, 2);
        assert_eq!(summary.failed, 1);
    }

    #[test]
    fn t_nav_exit_status_summary_with_running() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0)); // completed
        feed(&mut term, b"\x1b]133;A\x07"); // prompt start
        feed(&mut term, b"\x1b]133;B\x07"); // command start (running, no D)

        let nav = CommandNavigator::new();
        let summary = nav.exit_status_summary(&term);
        assert_eq!(summary.total, 1);
        assert_eq!(summary.running, 1);
    }

    #[test]
    fn t_nav_exit_status_success_rate() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 2, Some(0));
        emit_cycle(&mut term, 4, Some(1));

        let nav = CommandNavigator::new();
        let summary = nav.exit_status_summary(&term);
        assert!((summary.success_rate() - 2.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn t_nav_exit_status_success_rate_empty() {
        let term = Terminal::new(80, 24);
        let nav = CommandNavigator::new();
        let summary = nav.exit_status_summary(&term);
        assert_eq!(summary.success_rate(), 0.0);
    }

    #[test]
    fn t_nav_exit_status_all_succeeded() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 2, Some(0));

        let nav = CommandNavigator::new();
        let summary = nav.exit_status_summary(&term);
        assert!(summary.all_succeeded());
        assert!(!summary.any_failed());
    }

    #[test]
    fn t_nav_exit_status_any_failed() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 2, Some(127));

        let nav = CommandNavigator::new();
        let summary = nav.exit_status_summary(&term);
        assert!(summary.any_failed());
        assert!(!summary.all_succeeded());
    }

    // ================================================================
    // Recent exit codes
    // ================================================================

    #[test]
    fn t_nav_recent_exit_codes_basic() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 2, Some(1));
        emit_cycle(&mut term, 4, Some(0));

        let nav = CommandNavigator::new();
        let codes = nav.recent_exit_codes(&term, 2);
        assert_eq!(codes.len(), 2);
        assert_eq!(codes, vec![1, 0]);
    }

    #[test]
    fn t_nav_recent_exit_codes_more_than_available() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));

        let nav = CommandNavigator::new();
        let codes = nav.recent_exit_codes(&term, 5);
        assert_eq!(codes.len(), 1);
    }

    #[test]
    fn t_nav_recent_exit_codes_empty() {
        let term = Terminal::new(80, 24);
        let nav = CommandNavigator::new();
        let codes = nav.recent_exit_codes(&term, 5);
        assert!(codes.is_empty());
    }

    // ================================================================
    // Display
    // ================================================================

    #[test]
    fn t_nav_exit_status_display() {
        let summary = ExitStatusSummary {
            total: 10,
            succeeded: 8,
            failed: 2,
            running: 1,
        };
        let s = format!("{}", summary);
        assert!(s.contains("10 total"));
        assert!(s.contains("8 ok"));
        assert!(s.contains("2 fail"));
    }

    // ================================================================
    // Edge cases
    // ================================================================

    #[test]
    fn t_nav_next_at_max_stays() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 2, Some(0));

        let mut nav = CommandNavigator::new();
        nav.next_command(&term); // go to last (1)
        assert_eq!(nav.next_command(&term), Some(1)); // stays at 1
    }

    #[test]
    fn t_nav_prev_at_zero_stays() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));

        let mut nav = CommandNavigator::new();
        nav.set_selected(Some(0), &term);
        assert_eq!(nav.prev_command(&term), Some(0)); // stays at 0
    }

    #[test]
    fn t_nav_block_count() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 2, Some(0));
        emit_cycle(&mut term, 4, Some(0));

        let nav = CommandNavigator::new();
        assert_eq!(nav.block_count(&term), 3);
    }

    #[test]
    fn t_nav_current_block_out_of_bounds() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));

        let mut nav = CommandNavigator::new();
        // Manually set to an invalid index (shouldn't happen via normal API)
        nav.set_selected(Some(0), &term);
        // After clear, current_block returns None
        nav.clear_selection();
        assert!(nav.current_block(&term).is_none());
    }

    // ================================================================
    // Phase 8-D: CommandNavState tests
    // ================================================================

    #[test]
    fn t_nav_state_default_hidden() {
        let state = CommandNavState::new();
        assert!(!state.is_visible());
    }

    #[test]
    fn t_nav_state_show_hide() {
        let mut state = CommandNavState::new();
        state.show();
        assert!(state.is_visible());
        state.hide();
        assert!(!state.is_visible());
    }

    #[test]
    fn t_nav_state_toggle() {
        let mut state = CommandNavState::new();
        assert!(!state.is_visible());
        state.toggle();
        assert!(state.is_visible());
        state.toggle();
        assert!(!state.is_visible());
    }

    #[test]
    fn t_nav_state_hide_clears_selection() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));

        let mut state = CommandNavState::new();
        state.jump_next(&term);
        assert!(state.navigator().selected_index().is_some());

        state.hide();
        assert!(state.navigator().selected_index().is_none());
    }

    #[test]
    fn t_nav_state_jump_next_shows_overlay() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));

        let mut state = CommandNavState::new();
        assert!(!state.is_visible());

        state.jump_next(&term);
        assert!(state.is_visible());
    }

    #[test]
    fn t_nav_state_jump_prev_shows_overlay() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));

        let mut state = CommandNavState::new();
        state.jump_prev(&term);
        assert!(state.is_visible());
    }

    #[test]
    fn t_nav_state_jump_next_empty() {
        let term = Terminal::new(80, 24);
        let mut state = CommandNavState::new();
        let result = state.jump_next(&term);
        assert_eq!(result, None);
    }

    #[test]
    fn t_nav_state_jump_sequence() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 2, Some(0));
        emit_cycle(&mut term, 4, Some(1));

        let mut state = CommandNavState::new();

        // First jump_next → most recent (index 2)
        assert_eq!(state.jump_next(&term), Some(2));
        // jump_prev → index 1
        assert_eq!(state.jump_prev(&term), Some(1));
        // jump_prev → index 0
        assert_eq!(state.jump_prev(&term), Some(0));
        // At beginning, stays at 0
        assert_eq!(state.jump_prev(&term), Some(0));
    }

    #[test]
    fn t_nav_state_jump_first_last() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 2, Some(0));
        emit_cycle(&mut term, 4, Some(0));

        let mut state = CommandNavState::new();
        assert_eq!(state.jump_first(&term), Some(0));
        assert_eq!(state.jump_last(&term), Some(2));
    }

    #[test]
    fn t_nav_state_scroll_target() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 5, Some(0));

        let mut state = CommandNavState::new();
        state.jump_first(&term);
        let target = state.scroll_target(&term);
        assert!(target.is_some());
    }

    #[test]
    fn t_nav_state_current_block() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));

        let mut state = CommandNavState::new();
        state.jump_next(&term);
        let block = state.current_block(&term);
        assert!(block.is_some());
    }

    #[test]
    fn t_nav_state_exit_summary() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 2, Some(1));

        let state = CommandNavState::new();
        let summary = state.exit_summary(&term);
        assert_eq!(summary.total, 2);
        assert_eq!(summary.succeeded, 1);
        assert_eq!(summary.failed, 1);
    }

    #[test]
    fn t_nav_state_reset() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));

        let mut state = CommandNavState::new();
        state.jump_next(&term);
        assert!(state.is_visible());

        state.reset();
        assert!(!state.is_visible());
        assert!(state.navigator().selected_index().is_none());
    }

    #[test]
    fn t_nav_state_navigator_access() {
        let mut state = CommandNavState::new();
        let _ = state.navigator();
        let _ = state.navigator_mut();
    }

    // ================================================================
    // Phase 8-D: CommandNavOverlay rendering tests
    // ================================================================

    #[test]
    fn t_overlay_render_text_hidden() {
        let term = Terminal::new(80, 24);
        let state = CommandNavState::new();
        let overlay = CommandNavOverlay::new();
        assert!(overlay.render_text(&state, &term).is_empty());
    }

    #[test]
    fn t_overlay_render_text_no_commands() {
        let term = Terminal::new(80, 24);
        let mut state = CommandNavState::new();
        state.show();

        let overlay = CommandNavOverlay::new();
        let text = overlay.render_text(&state, &term);
        assert!(text.contains("No commands"));
    }

    #[test]
    fn t_overlay_render_text_with_selection() {
        let mut term = Terminal::new(80, 30);
        feed(&mut term, b"\x1b]133;A\x07");
        feed(&mut term, b"$ ls -la");
        feed(&mut term, b"\x1b]133;B\x07");
        feed(&mut term, b"\x1b]133;C\x07");
        feed(&mut term, b"file.txt\n");
        feed(&mut term, b"\x1b]133;D;0\x07");

        let mut state = CommandNavState::new();
        state.jump_next(&term);

        let overlay = CommandNavOverlay::new();
        let text = overlay.render_text(&state, &term);
        assert!(text.contains("[1/1]"));
        assert!(text.contains("exit: 0"));
    }

    #[test]
    fn t_overlay_render_text_failed_command() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(127));

        let mut state = CommandNavState::new();
        state.jump_next(&term);

        let overlay = CommandNavOverlay::new();
        let text = overlay.render_text(&state, &term);
        assert!(text.contains("exit: 127"));
    }

    #[test]
    fn t_overlay_render_text_running_command() {
        let mut term = Terminal::new(80, 30);
        feed(&mut term, b"\x1b]133;A\x07");
        feed(&mut term, b"$ sleep 10");
        feed(&mut term, b"\x1b]133;B\x07");
        feed(&mut term, b"\x1b]133;C\x07");
        // No D mark — still running

        let mut state = CommandNavState::new();
        state.jump_next(&term);

        let overlay = CommandNavOverlay::new();
        let text = overlay.render_text(&state, &term);
        assert!(text.contains("running"));
    }

    #[test]
    fn t_overlay_render_text_no_selection_summary() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 2, Some(1));

        let mut state = CommandNavState::new();
        state.show();
        // No jump — no selection

        let overlay = CommandNavOverlay::new();
        let text = overlay.render_text(&state, &term);
        assert!(text.contains("2 cmd"));
    }

    #[test]
    fn t_overlay_render_colored_hidden() {
        let term = Terminal::new(80, 24);
        let state = CommandNavState::new();
        let overlay = CommandNavOverlay::new();
        assert!(overlay.render_colored(&state, &term).is_empty());
    }

    #[test]
    fn t_overlay_render_colored_success() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));

        let mut state = CommandNavState::new();
        state.jump_next(&term);

        let overlay = CommandNavOverlay::new();
        let colored = overlay.render_colored(&state, &term);
        assert!(colored.contains("\x1b[32m")); // green fg
    }

    #[test]
    fn t_overlay_render_colored_failure() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(1));

        let mut state = CommandNavState::new();
        state.jump_next(&term);

        let overlay = CommandNavOverlay::new();
        let colored = overlay.render_colored(&state, &term);
        assert!(colored.contains("\x1b[31m")); // red fg
    }

    #[test]
    fn t_overlay_render_colored_running() {
        let mut term = Terminal::new(80, 30);
        feed(&mut term, b"\x1b]133;A\x07");
        feed(&mut term, b"$ x");
        feed(&mut term, b"\x1b]133;B\x07");
        feed(&mut term, b"\x1b]133;C\x07");

        let mut state = CommandNavState::new();
        state.jump_next(&term);

        let overlay = CommandNavOverlay::new();
        let colored = overlay.render_colored(&state, &term);
        assert!(colored.contains("\x1b[33m")); // yellow fg
    }

    #[test]
    fn t_overlay_render_colored_no_selection() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));

        let mut state = CommandNavState::new();
        state.show();

        let overlay = CommandNavOverlay::new();
        let colored = overlay.render_colored(&state, &term);
        assert!(colored.contains("\x1b[34m")); // blue fg (no selection)
    }

    #[test]
    fn t_overlay_render_bar_width() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));

        let mut state = CommandNavState::new();
        state.jump_next(&term);

        let overlay = CommandNavOverlay::new();
        let bar = overlay.render_bar(&state, &term, 80);
        assert!(!bar.is_empty());
        assert!(bar.contains("\x1b[0m"));
    }

    #[test]
    fn t_overlay_render_bar_hidden() {
        let term = Terminal::new(80, 24);
        let state = CommandNavState::new();
        let overlay = CommandNavOverlay::new();
        assert!(overlay.render_bar(&state, &term, 80).is_empty());
    }

    #[test]
    fn t_overlay_truncate_long_command() {
        let mut term = Terminal::new(120, 30);
        feed(&mut term, b"\x1b]133;A\x07");
        feed(
            &mut term,
            b"$ git commit -am 'This is a very long commit message that should be truncated'",
        );
        feed(&mut term, b"\x1b]133;B\x07");
        feed(&mut term, b"\x1b]133;C\x07");
        feed(&mut term, b"\x1b]133;D;0\x07");

        let mut state = CommandNavState::new();
        state.jump_next(&term);

        let mut overlay = CommandNavOverlay::new();
        overlay.set_max_cmd_width(20);
        let text = overlay.render_text(&state, &term);
        assert!(text.contains("..."));
    }

    #[test]
    fn t_overlay_set_get_max_cmd_width() {
        let mut overlay = CommandNavOverlay::new();
        overlay.set_max_cmd_width(40);
        assert_eq!(overlay.max_cmd_width(), 40);
    }

    #[test]
    fn t_overlay_default() {
        let overlay = CommandNavOverlay::default();
        assert_eq!(overlay.max_cmd_width(), 60);
    }

    #[test]
    fn t_overlay_render_text_multiple_commands() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 2, Some(0));
        emit_cycle(&mut term, 4, Some(1));

        let mut state = CommandNavState::new();
        state.jump_first(&term);

        let overlay = CommandNavOverlay::new();
        let text = overlay.render_text(&state, &term);
        assert!(text.contains("[1/3]"));

        state.jump_last(&term);
        let text = overlay.render_text(&state, &term);
        assert!(text.contains("[3/3]"));
    }

    #[test]
    fn t_overlay_jump_next_then_prev_cycle() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, 0, Some(0));
        emit_cycle(&mut term, 2, Some(0));
        emit_cycle(&mut term, 4, Some(0));

        let mut state = CommandNavState::new();

        // Jump to last
        state.jump_next(&term);
        assert_eq!(state.navigator().selected_index(), Some(2));

        // Navigate backward
        state.jump_prev(&term);
        assert_eq!(state.navigator().selected_index(), Some(1));

        state.jump_prev(&term);
        assert_eq!(state.navigator().selected_index(), Some(0));

        // Can't go before first
        state.jump_prev(&term);
        assert_eq!(state.navigator().selected_index(), Some(0));
    }
}
