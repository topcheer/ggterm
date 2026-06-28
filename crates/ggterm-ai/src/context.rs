//! AI context builder — extracts structured context from the terminal state.
//!
//! The [`AIContext`] is a snapshot of relevant terminal information that gets
//! injected into LLM prompts. It includes the last command, its exit code,
//! its output (truncated to a budget), and recent command history.

use std::fmt::Write as _;

use ggterm_core::Terminal;

/// Maximum characters of command output to include in the context.
const DEFAULT_OUTPUT_BUDGET: usize = 2000;

/// Maximum number of recent commands to include in history.
const DEFAULT_HISTORY_LIMIT: usize = 10;

/// A snapshot of terminal state for AI prompts.
///
/// Built from a [`Terminal`] reference. The context is designed to be
/// token-budget-aware: output is truncated and history is capped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AIContext {
    /// The most recent command text (the command the user typed).
    pub last_command: Option<String>,
    /// Exit code of the most recent completed command (`None` if still running).
    pub last_exit_code: Option<i32>,
    /// Output of the most recent command (truncated to `output_budget` chars).
    pub last_output: Option<String>,
    /// Up to `history_limit` recent completed commands (oldest first).
    pub recent_commands: Vec<String>,
    /// Working directory (if detectable from the prompt or environment).
    pub cwd: Option<String>,
    /// Shell name (e.g. "bash", "zsh").
    pub shell: Option<String>,
}

impl AIContext {
    /// Build context from the terminal's command blocks.
    ///
    /// Extracts the last completed (or running) command, its output, and
    /// recent command history. The navigator is not required — we read
    /// `command_blocks()` directly from the terminal.
    pub fn from_terminal(terminal: &Terminal) -> Self {
        Self::from_terminal_with_budget(terminal, DEFAULT_OUTPUT_BUDGET, DEFAULT_HISTORY_LIMIT)
    }

    /// Build context with custom output budget and history limit.
    pub fn from_terminal_with_budget(
        terminal: &Terminal,
        output_budget: usize,
        history_limit: usize,
    ) -> Self {
        let blocks = terminal.command_blocks();
        let mut ctx = AIContext::default();

        if blocks.is_empty() {
            return ctx;
        }

        // Find the last block that has a command_row.
        let last = blocks.last().expect("blocks is non-empty");
        ctx.last_command = last.command_row.map(|row| terminal.extract_row_text(row));
        ctx.last_exit_code = last.exit_code;

        // Extract output: rows between output_row and end_row (or current cursor).
        if let Some(output_row) = last.output_row {
            let end_row = last.end_row.unwrap_or_else(|| {
                // If the command hasn't ended, use cursor Y as the end.
                let (_, cy) = terminal.cursor();
                cy
            });
            ctx.last_output =
                Some(extract_output(terminal, output_row, end_row, output_budget));
        }

        // Build recent command history (completed commands only, oldest first).
        let completed: Vec<_> = blocks.iter().filter(|b| b.is_complete()).collect();
        let start = completed.len().saturating_sub(history_limit);
        for b in &completed[start..] {
            if let Some(row) = b.command_row {
                ctx.recent_commands.push(terminal.extract_row_text(row));
            }
        }

        ctx
    }

    /// Set the working directory.
    #[must_use]
    pub fn with_cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// Set the shell name.
    #[must_use]
    pub fn with_shell(mut self, shell: impl Into<String>) -> Self {
        self.shell = Some(shell.into());
        self
    }

    /// Format the context as a human-readable string suitable for an LLM system prompt.
    ///
    /// ```text
    /// Terminal Context:
    /// - Shell: zsh
    /// - Working directory: /home/user/project
    /// - Last command: git status
    /// - Exit code: 0
    ///
    /// Recent commands:
    /// 1. cd project
    /// 2. npm install
    /// 3. git status
    ///
    /// Last command output:
    /// On branch main
    /// nothing to commit, working tree clean
    /// ```
    pub fn to_prompt_string(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "Terminal Context:");

        if let Some(ref shell) = self.shell {
            let _ = writeln!(out, "- Shell: {shell}");
        }
        if let Some(ref cwd) = self.cwd {
            let _ = writeln!(out, "- Working directory: {cwd}");
        }

        match &self.last_command {
            Some(cmd) => {
                let _ = writeln!(out, "- Last command: {cmd}");
            }
            None => {
                let _ = writeln!(out, "- Last command: (none)");
            }
        }

        match self.last_exit_code {
            Some(0) => {
                let _ = writeln!(out, "- Exit code: 0 (success)");
            }
            Some(code) => {
                let _ = writeln!(out, "- Exit code: {code} (failed)");
            }
            None => {
                let _ = writeln!(out, "- Exit code: (still running)");
            }
        }

        if !self.recent_commands.is_empty() {
            let _ = writeln!(out, "\nRecent commands:");
            for (i, cmd) in self.recent_commands.iter().enumerate() {
                let _ = writeln!(out, "{}. {cmd}", i + 1);
            }
        }

        if let Some(ref output) = self.last_output {
            if !output.is_empty() {
                let _ = writeln!(out, "\nLast command output:");
                let _ = writeln!(out, "{output}");
            }
        }

        out
    }

    /// Return true if there's no meaningful context (no commands have been run).
    pub fn is_empty(&self) -> bool {
        self.last_command.is_none()
            && self.last_exit_code.is_none()
            && self.last_output.is_none()
            && self.recent_commands.is_empty()
    }
}

/// Extract output text from a range of rows, respecting the character budget.
///
/// Starts at `output_row` and reads forward until `end_row` (exclusive) or
/// the budget is exhausted. Trailing whitespace is trimmed per line.
fn extract_output(terminal: &Terminal, output_row: usize, end_row: usize, budget: usize) -> String {
    let mut lines = Vec::new();
    let mut total = 0usize;

    let mut row = output_row;
    while row < end_row {
        let text = terminal.extract_row_text(row);
        let line_len = text.len() + 1; // +1 for newline
        if total + line_len > budget {
            // Truncate: add what fits, then an ellipsis marker.
            let remaining = budget.saturating_sub(total);
            if remaining > 3 {
                lines.push(format!("{}...", &text[..remaining.saturating_sub(3).min(text.len())]));
            } else {
                lines.push("...".to_string());
            }
            break;
        }
        total += line_len;
        lines.push(text);
        row += 1;
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ggterm_core::Terminal;

    /// Feed raw bytes into the terminal (simulates PTY output).
    fn feed(term: &mut Terminal, data: &[u8]) {
        let mut parser = ggterm_core::Parser::new();
        parser.feed(data, term);
    }

    /// Emit a full command cycle with OSC 133 marks.
    fn emit_cycle(term: &mut Terminal, output_lines: &[&str], exit_code: Option<i32>) {
        // Prompt start
        feed(term, b"\x1b]133;A\x07");
        // Command start
        feed(term, b"\x1b]133;B\x07");
        // Move to next line
        feed(term, b"\n");
        // Output start
        feed(term, b"\x1b]133;C\x07");
        // Emit output lines
        for line in output_lines {
            feed(term, line.as_bytes());
            feed(term, b"\r\n");
        }
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

    #[test]
    fn t_context_empty_terminal() {
        let term = Terminal::new(80, 24);
        let ctx = AIContext::from_terminal(&term);
        assert!(ctx.is_empty());
        assert!(ctx.last_command.is_none());
        assert!(ctx.last_exit_code.is_none());
        assert!(ctx.last_output.is_none());
        assert!(ctx.recent_commands.is_empty());
    }

    #[test]
    fn t_context_single_command() {
        let mut term = Terminal::new(80, 24);
        emit_cycle(&mut term, &["hello world"], Some(0));

        let ctx = AIContext::from_terminal(&term);
        // last_command might be empty since we didn't write actual command text
        // but exit_code should be captured
        assert_eq!(ctx.last_exit_code, Some(0));
        assert!(!ctx.is_empty());
    }

    #[test]
    fn t_context_failed_command() {
        let mut term = Terminal::new(80, 24);
        emit_cycle(&mut term, &["error: file not found"], Some(1));

        let ctx = AIContext::from_terminal(&term);
        assert_eq!(ctx.last_exit_code, Some(1));
    }

    #[test]
    fn t_context_running_command_no_exit() {
        let mut term = Terminal::new(80, 24);
        // Emit just A + B + C (no D = still running)
        feed(&mut term, b"\x1b]133;A\x07");
        feed(&mut term, b"\x1b]133;B\x07");
        feed(&mut term, b"\n");
        feed(&mut term, b"\x1b]133;C\x07");
        feed(&mut term, b"some output");

        let ctx = AIContext::from_terminal(&term);
        // Running command: no exit code
        assert_eq!(ctx.last_exit_code, None);
    }

    #[test]
    fn t_context_multiple_commands_history() {
        let mut term = Terminal::new(80, 30);
        emit_cycle(&mut term, &["output1"], Some(0));
        emit_cycle(&mut term, &["output2"], Some(0));
        emit_cycle(&mut term, &["output3"], Some(0));

        let ctx = AIContext::from_terminal(&term);
        assert_eq!(ctx.recent_commands.len(), 3); // All 3 completed
    }

    #[test]
    fn t_context_history_limit() {
        let mut term = Terminal::new(80, 50);
        for i in 0..15 {
            emit_cycle(&mut term, &[&format!("out{}", i)], Some(0));
        }

        let ctx = AIContext::from_terminal_with_budget(&term, 2000, 5);
        assert_eq!(ctx.recent_commands.len(), 5); // Capped at 5
    }

    #[test]
    fn t_context_output_extraction() {
        let mut term = Terminal::new(80, 24);
        emit_cycle(&mut term, &["line1", "line2", "line3"], Some(0));

        let ctx = AIContext::from_terminal(&term);
        assert!(ctx.last_output.is_some());
        let output = ctx.last_output.unwrap();
        assert!(output.contains("line1") || output.contains("line2") || output.contains("line3"));
    }

    #[test]
    fn t_context_output_budget_truncation() {
        let mut term = Terminal::new(200, 50);
        // Generate lots of output
        let lines: Vec<String> = (0..30).map(|i| format!("This is line {} with some content", i)).collect();
        let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        emit_cycle(&mut term, &line_refs, Some(0));

        let ctx = AIContext::from_terminal_with_budget(&term, 100, 10);
        assert!(ctx.last_output.is_some());
        let output = ctx.last_output.unwrap();
        // Should be truncated to around 100 chars
        assert!(output.len() <= 105); // Small overflow for "..."
        assert!(output.ends_with("..."));
    }

    #[test]
    fn t_context_prompt_string_format() {
        let ctx = AIContext {
            last_command: Some("git status".to_string()),
            last_exit_code: Some(0),
            last_output: Some("On branch main".to_string()),
            recent_commands: vec!["cd project".to_string(), "git status".to_string()],
            cwd: Some("/home/user/project".to_string()),
            shell: Some("zsh".to_string()),
        };

        let prompt = ctx.to_prompt_string();
        assert!(prompt.contains("Shell: zsh"));
        assert!(prompt.contains("Working directory: /home/user/project"));
        assert!(prompt.contains("Last command: git status"));
        assert!(prompt.contains("Exit code: 0 (success)"));
        assert!(prompt.contains("Recent commands:"));
        assert!(prompt.contains("1. cd project"));
        assert!(prompt.contains("2. git status"));
        assert!(prompt.contains("On branch main"));
    }

    #[test]
    fn t_context_prompt_string_failed() {
        let ctx = AIContext {
            last_command: Some("make build".to_string()),
            last_exit_code: Some(2),
            ..Default::default()
        };

        let prompt = ctx.to_prompt_string();
        assert!(prompt.contains("Exit code: 2 (failed)"));
    }

    #[test]
    fn t_context_prompt_string_running() {
        let ctx = AIContext {
            last_command: Some("npm install".to_string()),
            last_exit_code: None,
            ..Default::default()
        };

        let prompt = ctx.to_prompt_string();
        assert!(prompt.contains("Exit code: (still running)"));
    }

    #[test]
    fn t_context_prompt_string_empty() {
        let ctx = AIContext::default();
        let prompt = ctx.to_prompt_string();
        assert!(prompt.contains("Last command: (none)"));
    }

    #[test]
    fn t_context_prompt_string_no_output() {
        let ctx = AIContext {
            last_command: Some("echo hi".to_string()),
            last_exit_code: Some(0),
            last_output: None,
            ..Default::default()
        };

        let prompt = ctx.to_prompt_string();
        // Should not contain "Last command output" section
        assert!(!prompt.contains("Last command output:"));
    }

    #[test]
    fn t_context_prompt_string_empty_output() {
        let ctx = AIContext {
            last_command: Some("cd /tmp".to_string()),
            last_exit_code: Some(0),
            last_output: Some(String::new()),
            ..Default::default()
        };

        let prompt = ctx.to_prompt_string();
        // Empty output should not produce the section either
        assert!(!prompt.contains("Last command output:"));
    }

    #[test]
    fn t_context_with_cwd_builder() {
        let ctx = AIContext::default().with_cwd("/usr/local/bin");
        assert_eq!(ctx.cwd.as_deref(), Some("/usr/local/bin"));
    }

    #[test]
    fn t_context_with_shell_builder() {
        let ctx = AIContext::default().with_shell("bash");
        assert_eq!(ctx.shell.as_deref(), Some("bash"));
    }

    #[test]
    fn t_context_is_empty_default() {
        assert!(AIContext::default().is_empty());
    }

    #[test]
    fn t_context_is_not_empty_with_command() {
        let ctx = AIContext {
            last_command: Some("ls".to_string()),
            ..Default::default()
        };
        assert!(!ctx.is_empty());
    }

    #[test]
    fn t_context_no_completed_commands_excludes_running() {
        let mut term = Terminal::new(80, 24);
        // Only running command, no completed ones
        feed(&mut term, b"\x1b]133;A\x07");
        feed(&mut term, b"\x1b]133;B\x07");

        let ctx = AIContext::from_terminal(&term);
        // recent_commands only includes completed commands
        assert!(ctx.recent_commands.is_empty());
    }

    #[test]
    fn t_context_default_budget() {
        assert_eq!(DEFAULT_OUTPUT_BUDGET, 2000);
    }

    #[test]
    fn t_context_default_history() {
        assert_eq!(DEFAULT_HISTORY_LIMIT, 10);
    }
}
