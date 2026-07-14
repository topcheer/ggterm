//! Status bar: cursor position, tab count, and active mode indicators.
//!
//! Renders a single-line summary string suitable for display in the window
//! title or a dedicated status line.
//!
//! Example output: `"Row:6 Col:11 | Tab 1/3 | bell | search"`

// ── StatusBar ───────────────────────────────────────────────────────────

/// Format a duration for display in the status bar.
/// Shows sub-second timing for fast commands, human-readable for long ones.
pub fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 0.001 {
        format!("{:.0}μs", d.as_micros())
    } else if secs < 1.0 {
        format!("{:.0}ms", d.as_millis())
    } else if secs < 60.0 {
        format!("{:.1}s", secs)
    } else {
        let m = d.as_secs() / 60;
        let s = d.as_secs() % 60;
        format!("{}m{}s", m, s)
    }
}

/// Aggregated terminal status for display in the window title or status line.
///
/// Updated every redraw from the active session's terminal state and the
/// DesktopApp's mode flags (bell, search, AI overlay).
pub struct StatusBar {
    /// Cursor row (0-based terminal row).
    pub cursor_row: usize,
    /// Cursor column (0-based terminal column).
    pub cursor_col: usize,
    /// Total number of open tabs.
    pub tab_count: usize,
    /// Number of panes in the active tab (>1 means splits exist).
    pub pane_count: usize,
    /// Index of the active tab (0-based).
    pub active_tab: usize,
    /// Whether the terminal bell was recently triggered.
    pub bell_active: bool,
    /// Whether the scrollback search bar is open.
    pub search_active: bool,
    /// Whether the AI overlay is visible.
    pub ai_active: bool,
    /// Last command exit code (None = no command completed yet).
    pub exit_code: Option<i32>,
    /// Configuration validation error message (P21-G).
    ///
    /// When set, a `!ERROR!` indicator is prepended to the status bar format.
    /// The renderer can use `has_config_error()` / `config_error_text()` to
    /// draw a red indicator.
    pub config_error: Option<String>,
    /// Currently active profile name (P22-C).
    ///
    /// Empty string = no profile active (base config).
    pub profile_name: String,
    /// P25-D: Broadcast input mode label (empty = none).
    pub broadcast_mode: String,
    /// P25-E: Whether session recording is active.
    pub recording: bool,
    /// P28-D: Active workspace name (empty = default).
    pub workspace_name: String,
    /// P28-G: Whether sound is enabled.
    pub sound_enabled: bool,
    /// P28-H: Active shell name (e.g., "zsh", "bash").
    pub shell_name: String,
    /// Current working directory (OSC 7). Empty = unknown.
    pub cwd: String,
    /// Remote SSH host (OSC 1337 RemoteHost=). Empty = local.
    pub remote_host: String,
    /// Whether pane zoom mode is active.
    pub pane_zoomed: bool,
    /// Current font size for zoom indicator (shown when non-default).
    pub font_size: f32,
    /// True when cursor line highlight is enabled.
    pub cursor_line: bool,
    /// True when scrollback browse mode is active (vim-style navigation).
    pub scroll_mode: bool,
    /// Task progress (0.0–1.0) from OSC 9;4. None = no active progress.
    pub progress: Option<f32>,
    /// Whether P2P terminal sharing is active.
    pub p2p_active: bool,
    /// Duration of the last completed command (e.g., "1.2s").
    /// Empty = no command has completed or shell integration inactive.
    pub command_duration: String,
    /// True when a command is currently executing (between OSC 133;B and 133;D).
    pub command_running: bool,
    /// Live elapsed time of the currently running command (e.g., "3.2s").
    /// Empty when no command is running. Updated every frame from the event loop.
    pub command_timer: String,
    /// Spinner frame counter (incremented externally for animation).
    pub spinner_frame: u32,
    /// Character count of current text selection (0 = no selection).
    pub selection_count: usize,
    /// Number of words in the current selection (0 when no selection).
    pub selection_words: usize,
    /// True when terminal input is locked (read-only mode).
    pub locked: bool,
    /// Session uptime as a formatted string (e.g., "5m", "1h23m").
    pub uptime: String,
    /// Git branch name (empty = not in a git repo).
    pub git_branch: String,
    /// Active theme name (e.g., "dark", "tokyo-night").
    pub theme_name: String,
    /// Terminal dimensions as "COLS×ROWS" (e.g., "120×40").
    pub dimensions: String,
    /// Exit code of the last completed command (None = no command completed or shell integration inactive).
    /// Displayed in status bar as a red segment when non-zero.
    pub last_exit_code: Option<i32>,
    /// Whether to show a system clock at the end of the status bar.
    /// Default: false (enabled at runtime in the event loop).
    pub show_clock: bool,
    /// Number of output lines from the last completed command.
    /// Shown as "~5L" in the status bar (L = lines).
    pub last_output_lines: Option<usize>,
    /// Hovered URL or hyperlink (OSC 8). Shown in status bar for link preview.
    pub hovered_link: Option<String>,
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusBar {
    /// Create a new status bar with default (empty) state.
    pub fn new() -> Self {
        Self {
            cursor_row: 0,
            cursor_col: 0,
            tab_count: 1,
            pane_count: 1,
            active_tab: 0,
            bell_active: false,
            search_active: false,
            ai_active: false,
            exit_code: None,
            config_error: None,
            profile_name: String::new(),
            broadcast_mode: String::new(),
            recording: false,
            workspace_name: String::new(),
            sound_enabled: false,
            shell_name: String::new(),
            cwd: String::new(),
            remote_host: String::new(),
            pane_zoomed: false,
            font_size: 14.0,
            cursor_line: false,
            scroll_mode: false,
            progress: None,
            p2p_active: false,
            command_duration: String::new(),
            command_running: false,
            command_timer: String::new(),
            spinner_frame: 0,
            selection_count: 0,
            selection_words: 0,
            locked: false,
            uptime: String::new(),
            git_branch: String::new(),
            theme_name: String::new(),
            dimensions: String::new(),
            last_exit_code: None,
            hovered_link: None,
            show_clock: false,
            last_output_lines: None,
        }
    }

    /// Update the cursor position.
    pub fn update_cursor(&mut self, row: usize, col: usize) {
        self.cursor_row = row;
        self.cursor_col = col;
    }

    /// Update tab information.
    pub fn update_tabs(&mut self, count: usize, active: usize) {
        self.tab_count = count;
        self.active_tab = active;
    }

    /// Set the number of panes in the active tab.
    pub fn update_pane_count(&mut self, count: usize) {
        self.pane_count = count;
    }

    /// Set the bell indicator.
    pub fn set_bell(&mut self, active: bool) {
        self.bell_active = active;
    }

    /// Set the search indicator.
    pub fn set_search(&mut self, active: bool) {
        self.search_active = active;
    }

    /// Set the AI overlay indicator.
    pub fn set_ai(&mut self, active: bool) {
        self.ai_active = active;
    }

    /// Set the last command exit code (P17-E).
    pub fn set_exit_code(&mut self, code: Option<i32>) {
        self.exit_code = code;
    }

    /// Set a configuration validation error message (P21-G).
    ///
    /// Pass `None` or an empty string to clear.
    pub fn set_config_error(&mut self, msg: Option<String>) {
        match msg {
            Some(m) if !m.is_empty() => self.config_error = Some(m),
            _ => self.config_error = None,
        }
    }

    /// Clear the configuration error indicator (P21-G).
    pub fn clear_config_error(&mut self) {
        self.config_error = None;
    }

    /// Returns `true` if a config error is currently displayed (P21-G).
    pub fn has_config_error(&self) -> bool {
        self.config_error.is_some()
    }

    /// Returns the config error message for renderer use (P21-G).
    pub fn config_error_text(&self) -> Option<&str> {
        self.config_error.as_deref()
    }

    /// Set the active profile name (P22-C).
    ///
    /// Pass an empty string to indicate the base config (no profile active).
    pub fn set_profile(&mut self, name: impl Into<String>) {
        self.profile_name = name.into();
    }

    /// Returns the active profile name, or `None` if no profile is active (P22-C).
    pub fn active_profile(&self) -> Option<&str> {
        if self.profile_name.is_empty() {
            None
        } else {
            Some(&self.profile_name)
        }
    }

    /// Format the status bar as a single-line string.
    ///
    /// Example: `"!ERROR! | Row:6 Col:11 | Tab 1/3 | exit:0 | bell | search | ai"`
    ///
    /// When a config error is set, `!ERROR!` is prepended so the renderer
    /// can highlight it in red.
    pub fn format(&self) -> String {
        let mut parts: Vec<String> = Vec::with_capacity(8);

        // Config error indicator — shown first for visibility (P21-G).
        if self.config_error.is_some() {
            parts.push("!ERROR!".to_string());
        }

        // Cursor position (always shown).
        parts.push(format!(
            "Row:{} Col:{}",
            self.cursor_row + 1,
            self.cursor_col + 1
        ));

        // Tab info (only show "Tab x/y" when more than 1 tab).
        if self.tab_count > 1 {
            parts.push(format!("Tab {}/{}", self.active_tab + 1, self.tab_count));
        }
        if self.pane_count > 1 {
            parts.push(format!("{} panes", self.pane_count));
        }

        // Command exit code (P17-E).
        if let Some(code) = self.exit_code {
            if code == 0 {
                parts.push("exit:0".to_string());
            } else {
                parts.push(format!("exit:{}", code));
            }
        }

        // Last command output line count.
        if let Some(lines) = self.last_output_lines {
            parts.push(format!("{}L", lines));
        }

        // Command execution duration.
        if !self.command_duration.is_empty() {
            parts.push(format!("⏱{}", self.command_duration));
        }

        // Running command spinner.
        if self.command_running {
            let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let frame = frames[(self.spinner_frame as usize) % frames.len()];
            // Show live timer if available.
            if !self.command_timer.is_empty() {
                parts.push(format!("{frame} {}", self.command_timer));
            } else {
                parts.push(frame.to_string());
            }
        } else if !self.command_timer.is_empty() {
            // Idle timer: show time since last output (reuses command_timer field).
            parts.push(format!("idle {}", self.command_timer));
        }

        // Selection character count.
        if self.selection_count > 0 {
            if self.selection_words > 0 {
                parts.push(format!(
                    "SEL:{}c/{}w",
                    self.selection_count, self.selection_words
                ));
            } else {
                parts.push(format!("SEL:{}c", self.selection_count));
            }
        }

        // Terminal lock indicator.
        if self.locked {
            parts.push("LOCK".to_string());
        }

        // Session uptime.
        if !self.uptime.is_empty() {
            parts.push(self.uptime.clone());
        }

        // Git branch.
        if !self.git_branch.is_empty() {
            parts.push(format!(" {}", self.git_branch));
        }

        // Active theme name.
        if !self.theme_name.is_empty() {
            parts.push(format!("theme:{}", self.theme_name));
        }
        if !self.dimensions.is_empty() {
            parts.push(self.dimensions.clone());
        }

        // Mode indicators.
        if self.bell_active {
            parts.push("bell".to_string());
        }
        if self.search_active {
            parts.push("search".to_string());
        }
        if self.ai_active {
            parts.push("ai".to_string());
        }

        // Active profile name (P22-C).
        if !self.profile_name.is_empty() {
            parts.push(format!("@{}", self.profile_name));
        }

        // P25-D: Broadcast mode indicator.
        if !self.broadcast_mode.is_empty() {
            parts.push(format!("BCAST:{}", self.broadcast_mode));
        }

        // P25-E: Recording indicator.
        if self.recording {
            parts.push("REC".to_string());
        }

        // Pane zoom indicator.
        if self.pane_zoomed {
            parts.push("ZOOM".to_string());
        }
        // Font size indicator — show only when non-default.
        if (self.font_size - 14.0).abs() > 0.1 {
            parts.push(format!("{}px", self.font_size as u32));
        }
        if self.cursor_line {
            parts.push("CL".to_string());
        }

        // Scrollback browse mode indicator.
        if self.scroll_mode {
            parts.push("SCROLL".to_string());
        }

        // Progress indicator (OSC 9;4).
        if let Some(pct) = self.progress {
            parts.push(format!("{:.0}%", pct * 100.0));
        }

        // System clock.
        if self.show_clock {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let secs = now.as_secs() % 86400;
            let h = (secs / 3600 + local_offset_hours()) % 24;
            let m = (secs / 60) % 60;
            parts.push(format!("{:02}:{:02}", h, m));
        }

        parts.join(" | ")
    }

    /// Format the status bar as colored segments for modern overlay rendering.
    ///
    /// Returns a list of `(text, color)` pairs where color is `(r, g, b)` in `[0, 255]`.
    /// Each segment is separated by a dim separator `|`.
    ///
    /// The renderer can use this to draw each segment at the correct position
    /// with appropriate coloring (e.g. red for errors, green for exit 0).
    pub fn format_segments(&self) -> Vec<(String, (u8, u8, u8))> {
        let text_color: (u8, u8, u8) = (180, 180, 190);
        let dim_color: (u8, u8, u8) = (90, 90, 100);
        let accent_color: (u8, u8, u8) = (120, 180, 255);
        let warn_color: (u8, u8, u8) = (230, 180, 80);
        let err_color: (u8, u8, u8) = (230, 80, 80);
        let ok_color: (u8, u8, u8) = (100, 200, 120);

        let mut segs: Vec<(String, (u8, u8, u8))> = Vec::new();

        macro_rules! seg {
            ($text:expr, $color:expr) => {{
                if !segs.is_empty() {
                    segs.push((String::from(" | "), dim_color));
                }
                segs.push(($text, $color));
            }};
        }

        // Config error indicator (red).
        if self.config_error.is_some() {
            seg!("!ERROR!".to_string(), err_color);
        }

        // Cursor position (1-indexed — terminal convention).
        seg!(
            format!("{}:{}", self.cursor_row + 1, self.cursor_col + 1),
            accent_color
        );

        // Tab info.
        if self.tab_count > 1 {
            seg!(
                format!("Tab {}/{}", self.active_tab + 1, self.tab_count),
                text_color
            );
        }
        // Pane count (shown only when splits exist).
        if self.pane_count > 1 {
            seg!(format!("{} panes", self.pane_count), accent_color);
        }

        // Exit code.
        if let Some(code) = self.exit_code {
            let color = if code == 0 { ok_color } else { err_color };
            seg!(format!("exit:{}", code), color);
        }

        // Last command output line count (shown as "~5L" for 5 lines).
        if let Some(lines) = self.last_output_lines {
            seg!(format!("{}L", lines), dim_color);
        }

        // Command execution duration.
        if !self.command_duration.is_empty() {
            seg!(self.command_duration.clone(), dim_color);
        }

        // Running command spinner with live timer.
        if self.command_running {
            let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let frame = frames[(self.spinner_frame as usize) % frames.len()];
            let label = if !self.command_timer.is_empty() {
                format!("{frame} {}", self.command_timer)
            } else {
                frame.to_string()
            };
            seg!(label, accent_color);
        } else if !self.command_timer.is_empty() {
            seg!(format!("idle {}", self.command_timer), dim_color);
        }

        // Selection character count.
        if self.selection_count > 0 {
            let label = if self.selection_words > 0 {
                format!("SEL:{}c/{}w", self.selection_count, self.selection_words)
            } else {
                format!("SEL:{}c", self.selection_count)
            };
            seg!(label, warn_color);
        }

        // Terminal lock indicator.
        if self.locked {
            seg!("LOCK".to_string(), err_color);
        }

        // Session uptime.
        if !self.uptime.is_empty() {
            seg!(self.uptime.clone(), dim_color);
        }

        // Git branch (shown in accent green).
        if !self.git_branch.is_empty() {
            seg!(format!(" {}", self.git_branch), (120u8, 200, 120));
        }

        // Active theme name (dim color — informational only).
        if !self.theme_name.is_empty() {
            seg!(format!("theme:{}", self.theme_name), dim_color);
        }
        if !self.dimensions.is_empty() {
            seg!(self.dimensions.clone(), dim_color);
        }

        // Mode indicators.
        if self.bell_active {
            seg!("BELL".to_string(), warn_color);
        }
        if self.search_active {
            seg!("SEARCH".to_string(), accent_color);
        }
        if self.ai_active {
            seg!("AI".to_string(), accent_color);
        }

        // Profile.
        if !self.profile_name.is_empty() {
            seg!(format!("@{}", self.profile_name), text_color);
        }

        // Broadcast mode.
        if !self.broadcast_mode.is_empty() {
            seg!(format!("BCAST:{}", self.broadcast_mode), warn_color);
        }

        // Recording.
        if self.recording {
            seg!("REC".to_string(), warn_color);
        }

        // Pane zoom.
        if self.pane_zoomed {
            seg!("ZOOM".to_string(), accent_color);
        }
        // Font size indicator — show only when zoomed (non-default).
        if (self.font_size - 14.0).abs() > 0.1 {
            seg!(format!("{}px", self.font_size as u32), dim_color);
        }
        if self.cursor_line {
            seg!("CL".to_string(), dim_color);
        }

        // Scrollback browse mode.
        if self.scroll_mode {
            seg!("SCROLL".to_string(), accent_color);
        }

        // P28-D: Workspace.
        if !self.workspace_name.is_empty() && self.workspace_name != "default" {
            seg!(format!("WS:{}", self.workspace_name), accent_color);
        }

        // P28-G: Sound indicator.
        if self.sound_enabled {
            seg!("SND".to_string(), ok_color);
        }

        // P28-H: Shell name.
        if !self.shell_name.is_empty() {
            seg!(self.shell_name.clone(), text_color);
        }

        // Remote SSH host (from OSC 1337 RemoteHost=).
        if !self.remote_host.is_empty() {
            seg!(format!("SSH:{}", self.remote_host), accent_color);
        }

        // CWD (from OSC 7). Show basename only for compactness.
        if !self.cwd.is_empty() {
            let display = std::path::Path::new(&self.cwd)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&self.cwd);
            let home = dirs_or_home();
            let display = if let Some(ref home) = home {
                display.replace(home.as_str(), "~")
            } else {
                display.to_string()
            };
            seg!(display, dim_color);
        }

        // Progress indicator (OSC 9;4).
        if let Some(pct) = self.progress {
            seg!(format!("{:.0}%", pct * 100.0), ok_color);
        }

        // Last command exit code — show only when non-zero (failure).
        if let Some(code) = self.last_exit_code
            && code != 0
        {
            seg!(format!("exit:{}", code), err_color);
        }

        // Hovered URL/hyperlink preview.
        if let Some(ref link) = self.hovered_link {
            let display = if link.len() > 60 {
                format!("{}...", &link[..57])
            } else {
                link.clone()
            };
            seg!(display, accent_color);
        }

        // P2P sharing indicator.
        if self.p2p_active {
            seg!("SHARE".to_string(), accent_color);
        }

        // System clock — always shown at the end (like tmux status-right).
        if self.show_clock {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let secs = now.as_secs() % 86400; // seconds since midnight UTC
            let h = (secs / 3600 + local_offset_hours()) % 24;
            let m = (secs / 60) % 60;
            seg!(format!("{:02}:{:02}", h, m), dim_color);
        }

        segs
    }
}

/// Estimate local timezone offset in hours (heuristic — not exact but
/// close enough for a status bar clock). Uses `date +%z` on Unix.
fn local_offset_hours() -> u64 {
    use std::sync::OnceLock;
    static CACHED_OFFSET: OnceLock<u64> = OnceLock::new();

    *CACHED_OFFSET.get_or_init(|| {
        #[cfg(unix)]
        {
            if let Ok(out) = std::process::Command::new("date").arg("+%z").output()
                && out.status.success()
            {
                let s = String::from_utf8_lossy(&out.stdout);
                if s.len() >= 5 {
                    let sign = if s.starts_with('-') { -1i64 } else { 1i64 };
                    if let Ok(h) = s[1..3].parse::<i64>() {
                        return ((sign * h) as u64 + 24) % 24;
                    }
                }
            }
            0
        }
        #[cfg(not(unix))]
        {
            0
        }
    })
}

/// Helper to get home directory (avoids adding a dependency to this function's scope).
fn dirs_or_home() -> Option<String> {
    std::env::var("HOME").ok()
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_default_format() {
        let sb = StatusBar::new();
        let formatted = sb.format();
        // Default: cursor (0,0), single tab (not shown), no flags.
        assert_eq!(formatted, "Row:1 Col:1");
    }

    #[test]
    fn t_update_cursor() {
        let mut sb = StatusBar::new();
        sb.update_cursor(5, 10);
        assert_eq!(sb.format(), "Row:6 Col:11");
    }

    #[test]
    fn t_bell_search_ai_flags() {
        let mut sb = StatusBar::new();
        sb.update_cursor(3, 7);
        sb.set_bell(true);
        sb.set_search(true);
        sb.set_ai(true);
        assert_eq!(sb.format(), "Row:4 Col:8 | bell | search | ai");
    }

    #[test]
    fn t_multi_tab_display() {
        let mut sb = StatusBar::new();
        sb.update_cursor(0, 0);
        sb.update_tabs(3, 1); // 3 tabs, active = index 1 (Tab 2/3)
        assert_eq!(sb.format(), "Row:1 Col:1 | Tab 2/3");
    }

    #[test]
    fn t_all_flags_cleared() {
        let mut sb = StatusBar::new();
        sb.update_cursor(10, 20);
        sb.update_tabs(2, 0);
        sb.set_bell(true);
        sb.set_search(true);
        sb.set_ai(true);

        // Now clear everything.
        sb.set_bell(false);
        sb.set_search(false);
        sb.set_ai(false);

        // Should show cursor + tabs only.
        assert_eq!(sb.format(), "Row:11 Col:21 | Tab 1/2");
    }

    #[test]
    fn t_single_tab_not_shown() {
        let mut sb = StatusBar::new();
        sb.update_cursor(0, 0);
        sb.update_tabs(1, 0);
        // With a single tab, "Tab 1/1" should NOT appear.
        assert_eq!(sb.format(), "Row:1 Col:1");
    }

    #[test]
    fn t_bell_only() {
        let mut sb = StatusBar::new();
        sb.set_bell(true);
        assert_eq!(sb.format(), "Row:1 Col:1 | bell");
    }

    #[test]
    fn t_search_only() {
        let mut sb = StatusBar::new();
        sb.set_search(true);
        assert_eq!(sb.format(), "Row:1 Col:1 | search");
    }

    // ── P17-E: Exit code tests ──────────────────────────────────────

    #[test]
    fn t_exit_code_zero_shows_ok() {
        let mut sb = StatusBar::new();
        sb.set_exit_code(Some(0));
        assert_eq!(sb.format(), "Row:1 Col:1 | exit:0");
    }

    #[test]
    fn t_exit_code_nonzero_shows_code() {
        let mut sb = StatusBar::new();
        sb.set_exit_code(Some(127));
        assert_eq!(sb.format(), "Row:1 Col:1 | exit:127");
    }

    #[test]
    fn t_exit_code_none_not_shown() {
        let mut sb = StatusBar::new();
        sb.set_exit_code(None);
        assert_eq!(sb.format(), "Row:1 Col:1");
    }

    // ── P21-G: Config error indicator tests ──────────────────────────

    #[test]
    fn t_config_error_default_none() {
        let sb = StatusBar::new();
        assert!(!sb.has_config_error());
        assert!(sb.config_error_text().is_none());
    }

    #[test]
    fn t_config_error_shown_in_format() {
        let mut sb = StatusBar::new();
        sb.set_config_error(Some("font_size out of range".to_string()));
        assert!(sb.has_config_error());
        assert_eq!(sb.config_error_text(), Some("font_size out of range"));
        assert_eq!(sb.format(), "!ERROR! | Row:1 Col:1");
    }

    #[test]
    fn t_config_error_with_other_flags() {
        let mut sb = StatusBar::new();
        sb.update_cursor(5, 10);
        sb.update_tabs(2, 0);
        sb.set_bell(true);
        sb.set_config_error(Some("bad theme".to_string()));
        assert_eq!(sb.format(), "!ERROR! | Row:6 Col:11 | Tab 1/2 | bell");
    }

    #[test]
    fn t_config_error_cleared() {
        let mut sb = StatusBar::new();
        sb.set_config_error(Some("oops".to_string()));
        assert!(sb.has_config_error());

        sb.clear_config_error();
        assert!(!sb.has_config_error());
        assert_eq!(sb.format(), "Row:1 Col:1");
    }

    #[test]
    fn t_config_error_set_none_clears() {
        let mut sb = StatusBar::new();
        sb.set_config_error(Some("oops".to_string()));
        sb.set_config_error(None);
        assert!(!sb.has_config_error());
    }

    #[test]
    fn t_config_error_empty_string_clears() {
        let mut sb = StatusBar::new();
        sb.set_config_error(Some("oops".to_string()));
        sb.set_config_error(Some(String::new()));
        assert!(!sb.has_config_error());
    }

    #[test]
    fn t_config_error_precedes_cursor() {
        let mut sb = StatusBar::new();
        sb.update_cursor(3, 7);
        sb.set_config_error(Some("e".to_string()));
        let formatted = sb.format();
        // ERROR must appear before cursor position.
        let err_pos = formatted.find("!ERROR!").unwrap();
        let cursor_pos = formatted.find("Row:4").unwrap();
        assert!(err_pos < cursor_pos);
    }

    // ── P22-C: Profile display tests ──────────────────────────────────

    #[test]
    fn t_profile_default_empty() {
        let sb = StatusBar::new();
        assert!(sb.active_profile().is_none());
        assert!(sb.profile_name.is_empty());
    }

    #[test]
    fn t_profile_set_and_read() {
        let mut sb = StatusBar::new();
        sb.set_profile("presentation");
        assert_eq!(sb.active_profile(), Some("presentation"));
    }

    #[test]
    fn t_profile_set_empty_clears() {
        let mut sb = StatusBar::new();
        sb.set_profile("compact");
        sb.set_profile("");
        assert!(sb.active_profile().is_none());
    }

    #[test]
    fn t_profile_shown_in_format() {
        let mut sb = StatusBar::new();
        sb.update_cursor(5, 10);
        sb.set_profile("presentation");
        assert_eq!(sb.format(), "Row:6 Col:11 | @presentation");
    }

    #[test]
    fn t_profile_with_other_flags() {
        let mut sb = StatusBar::new();
        sb.update_cursor(0, 0);
        sb.set_bell(true);
        sb.set_profile("compact");
        assert_eq!(sb.format(), "Row:1 Col:1 | bell | @compact");
    }

    #[test]
    fn t_profile_not_shown_when_empty() {
        let mut sb = StatusBar::new();
        sb.set_profile("");
        assert_eq!(sb.format(), "Row:1 Col:1");
    }

    #[test]
    fn t_profile_appears_after_ai() {
        let mut sb = StatusBar::new();
        sb.set_ai(true);
        sb.set_profile("test");
        let formatted = sb.format();
        let ai_pos = formatted.find("ai").unwrap();
        let profile_pos = formatted.find("@test").unwrap();
        assert!(ai_pos < profile_pos);
    }

    // ── format_segments tests ────────────────────────────────

    #[test]
    fn t_segments_default() {
        let sb = StatusBar::new();
        let segs = sb.format_segments();
        // Default: just cursor position "1:1" with accent color.
        assert!(!segs.is_empty());
        assert_eq!(segs[0].0, "1:1");
    }

    #[test]
    fn t_segments_cursor_position() {
        let mut sb = StatusBar::new();
        sb.update_cursor(5, 10);
        let segs = sb.format_segments();
        // 1-indexed display: internal 5,10 → displayed as 6:11
        assert_eq!(segs[0].0, "6:11");
    }

    #[test]
    fn t_segments_error_segment() {
        let mut sb = StatusBar::new();
        sb.set_config_error(Some("bad config".to_string()));
        let segs = sb.format_segments();
        // First segment should be "!ERROR!".
        assert_eq!(segs[0].0, "!ERROR!");
    }

    #[test]
    fn t_segments_exit_code_colors() {
        let mut sb = StatusBar::new();
        sb.set_exit_code(Some(0));
        let segs = sb.format_segments();
        let exit_seg = segs.iter().find(|(t, _)| t.starts_with("exit:"));
        assert!(exit_seg.is_some());
        // Exit 0 should be green-ish (high green channel).
        let (_, color) = exit_seg.unwrap();
        assert!(color.1 > color.0 && color.1 > color.2); // g > r && g > b

        sb.set_exit_code(Some(1));
        let segs = sb.format_segments();
        let exit_seg = segs.iter().find(|(t, _)| t.starts_with("exit:"));
        let (_, color) = exit_seg.unwrap();
        assert!(color.0 > color.1); // r > g for error
    }

    #[test]
    fn t_segments_mode_indicators() {
        let mut sb = StatusBar::new();
        sb.set_bell(true);
        sb.set_search(true);
        sb.set_ai(true);
        let segs = sb.format_segments();
        let texts: Vec<&str> = segs.iter().map(|(t, _)| t.as_str()).collect();
        assert!(texts.contains(&"BELL"));
        assert!(texts.contains(&"SEARCH"));
        assert!(texts.contains(&"AI"));
    }

    #[test]
    fn t_segments_profile() {
        let mut sb = StatusBar::new();
        sb.set_profile("dark");
        let segs = sb.format_segments();
        let texts: Vec<&str> = segs.iter().map(|(t, _)| t.as_str()).collect();
        assert!(texts.contains(&"@dark"));
    }

    #[test]
    fn t_segments_recording() {
        let mut sb = StatusBar::new();
        sb.recording = true;
        let segs = sb.format_segments();
        let texts: Vec<&str> = segs.iter().map(|(t, _)| t.as_str()).collect();
        assert!(texts.contains(&"REC"));
    }

    #[test]
    fn t_segments_separators_dim_color() {
        let mut sb = StatusBar::new();
        sb.update_cursor(1, 1);
        sb.set_search(true);
        let segs = sb.format_segments();
        // Should contain " | " separators with dim color.
        let seps: Vec<_> = segs.iter().filter(|(t, _)| t == " | ").collect();
        assert!(!seps.is_empty());
        // Separator color should be dim (all channels low).
        let (_, color) = seps[0];
        assert!(color.0 < 120 && color.1 < 120 && color.2 < 120);
    }

    #[test]
    fn t_segments_remote_host() {
        let mut sb = StatusBar::new();
        sb.remote_host = "root@server.com".into();
        let segs = sb.format_segments();
        let texts: Vec<&str> = segs.iter().map(|(t, _)| t.as_str()).collect();
        assert!(
            texts.contains(&"SSH:root@server.com"),
            "should contain SSH host indicator, got: {texts:?}"
        );
    }

    #[test]
    fn t_format_duration_microseconds() {
        let d = std::time::Duration::from_micros(500);
        assert_eq!(format_duration(d), "500μs");
    }

    #[test]
    fn t_format_duration_milliseconds() {
        let d = std::time::Duration::from_millis(250);
        assert_eq!(format_duration(d), "250ms");
    }

    #[test]
    fn t_format_duration_seconds() {
        let d = std::time::Duration::from_millis(1500);
        assert_eq!(format_duration(d), "1.5s");
    }

    #[test]
    fn t_format_duration_minutes() {
        let d = std::time::Duration::from_secs(125);
        assert_eq!(format_duration(d), "2m5s");
    }

    #[test]
    fn t_status_bar_command_duration_shown() {
        let mut sb = StatusBar::new();
        sb.command_duration = "1.2s".into();
        let formatted = sb.format();
        assert!(
            formatted.contains("1.2s"),
            "should show duration: {formatted}"
        );
    }

    #[test]
    fn t_status_bar_command_duration_empty_omitted() {
        let mut sb = StatusBar::new();
        sb.exit_code = Some(0);
        let formatted = sb.format();
        assert!(
            !formatted.contains("⏱"),
            "should not show clock when empty: {formatted}"
        );
    }

    #[test]
    fn t_status_bar_running_spinner_shown() {
        let mut sb = StatusBar::new();
        sb.command_running = true;
        sb.spinner_frame = 0;
        let formatted = sb.format();
        assert!(
            formatted.contains("⠋"),
            "should show spinner frame 0: {formatted}"
        );
        sb.spinner_frame = 3;
        let formatted = sb.format();
        assert!(
            formatted.contains("⠸"),
            "should show spinner frame 3: {formatted}"
        );
    }

    #[test]
    fn t_status_bar_running_spinner_omitted_when_idle() {
        let sb = StatusBar::new();
        let formatted = sb.format();
        assert!(
            !formatted.contains("⠋") && !formatted.contains("⠙"),
            "should not show spinner when idle: {formatted}"
        );
    }

    #[test]
    fn t_status_bar_running_timer_shown() {
        let mut sb = StatusBar::new();
        sb.command_running = true;
        sb.command_timer = "2.5s".into();
        sb.spinner_frame = 0;
        let formatted = sb.format();
        assert!(
            formatted.contains("2.5s"),
            "should show live timer when running: {formatted}"
        );
    }

    #[test]
    fn t_status_bar_running_timer_empty_shows_spinner_only() {
        let mut sb = StatusBar::new();
        sb.command_running = true;
        sb.command_timer.clear();
        sb.spinner_frame = 0;
        let formatted = sb.format();
        assert!(
            formatted.contains("⠋"),
            "should still show spinner when timer empty: {formatted}"
        );
    }

    #[test]
    fn t_status_bar_selection_count_shown() {
        let mut sb = StatusBar::new();
        sb.selection_count = 42;
        let formatted = sb.format();
        assert!(
            formatted.contains("SEL:42c"),
            "should show selection count: {formatted}"
        );
    }

    #[test]
    fn t_status_bar_selection_count_zero_omitted() {
        let sb = StatusBar::new();
        let formatted = sb.format();
        assert!(
            !formatted.contains("SEL:"),
            "should not show SEL when 0: {formatted}"
        );
    }

    #[test]
    fn t_status_bar_uptime_shown() {
        let mut sb = StatusBar::new();
        sb.uptime = "5m".into();
        let formatted = sb.format();
        assert!(formatted.contains("5m"), "should show uptime: {formatted}");
    }

    #[test]
    fn t_status_bar_selection_words_shown() {
        let mut sb = StatusBar::new();
        sb.selection_count = 42;
        sb.selection_words = 7;
        let formatted = sb.format();
        assert!(
            formatted.contains("SEL:42c/7w"),
            "should show chars and words: {formatted}"
        );
    }

    #[test]
    fn t_status_bar_selection_words_zero_omitted() {
        let mut sb = StatusBar::new();
        sb.selection_count = 5;
        sb.selection_words = 0;
        let formatted = sb.format();
        assert!(
            formatted.contains("SEL:5c") && !formatted.contains("/"),
            "should show chars only when no words: {formatted}"
        );
    }

    #[test]
    fn t_status_bar_uptime_empty_omitted() {
        let sb = StatusBar::new();
        let formatted = sb.format();
        assert!(
            !formatted.contains("0m"),
            "should not show uptime when empty: {formatted}"
        );
    }

    #[test]
    fn t_status_bar_lock_shown() {
        let mut sb = StatusBar::new();
        sb.locked = true;
        let formatted = sb.format();
        assert!(
            formatted.contains("LOCK"),
            "should show LOCK indicator: {formatted}"
        );
    }

    #[test]
    fn t_status_bar_git_branch_shown() {
        let mut sb = StatusBar::new();
        sb.git_branch = "main".into();
        let formatted = sb.format();
        assert!(
            formatted.contains("main"),
            "should show git branch: {formatted}"
        );
    }

    #[test]
    fn t_status_bar_exit_code_success() {
        let mut sb = StatusBar::new();
        sb.exit_code = Some(0);
        let formatted = sb.format();
        assert!(
            formatted.contains("exit:0"),
            "should show exit:0 for success: {formatted}"
        );
    }

    #[test]
    fn t_status_bar_exit_code_failure() {
        let mut sb = StatusBar::new();
        sb.exit_code = Some(127);
        let formatted = sb.format();
        assert!(
            formatted.contains("exit:127"),
            "should show exit:127 for failure: {formatted}"
        );
    }

    #[test]
    fn t_status_bar_exit_code_none_omitted() {
        let sb = StatusBar::new();
        let formatted = sb.format();
        assert!(
            !formatted.contains("exit:"),
            "should not show exit when no command completed: {formatted}"
        );
    }

    #[test]
    fn t_status_bar_idle_displayed() {
        let mut sb = StatusBar::new();
        sb.command_running = false;
        sb.command_timer = "30s".into();
        let formatted = sb.format();
        assert!(
            formatted.contains("idle 30s"),
            "should show idle timer: {formatted}"
        );
    }

    #[test]
    fn t_status_bar_idle_not_shown_when_running() {
        let mut sb = StatusBar::new();
        sb.command_running = true;
        sb.command_timer = "30s".into();
        let formatted = sb.format();
        assert!(
            !formatted.contains("idle"),
            "should not show idle when command is running: {formatted}"
        );
    }

    #[test]
    fn t_status_bar_idle_not_shown_when_empty() {
        let mut sb = StatusBar::new();
        sb.command_running = false;
        sb.command_timer = String::new();
        let formatted = sb.format();
        assert!(
            !formatted.contains("idle"),
            "should not show idle when timer is empty: {formatted}"
        );
    }
}
