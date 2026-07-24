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
    /// Whether the terminal bell was recently triggered.
    pub bell_active: bool,
    /// Whether the scrollback search bar is open.
    pub search_active: bool,
    /// Whether the AI overlay is visible.
    pub ai_active: bool,
    // (exit_code field removed — consolidated into last_exit_code to fix
    // stale exit code display during command execution.)
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
    /// Current working directory (OSC 7). Empty = unknown.
    pub cwd: String,
    /// Remote SSH host (OSC 1337 RemoteHost=). Empty = local.
    pub remote_host: String,
    /// Whether pane zoom mode is active.
    pub pane_zoomed: bool,
    /// Current font size for zoom indicator (shown when non-default).
    pub font_size: f32,
    /// True when scrollback browse mode is active (vim-style navigation).
    pub scroll_mode: bool,
    /// Task progress (0.0–1.0) from OSC 9;4. None = no active progress.
    pub progress: Option<f32>,
    /// Whether P2P terminal sharing is active.
    pub p2p_active: bool,
    /// True when a command is currently executing (between OSC 133;B and 133;D).
    pub command_running: bool,
    /// Live elapsed time of the currently running command (e.g., "3.2s").
    /// Empty when no command is running. Updated every frame from the event loop.
    pub command_timer: String,
    /// Spinner frame counter (incremented externally for animation).
    pub spinner_frame: u32,
    /// Character count of current text selection (0 = no selection).
    pub selection_count: usize,
    /// Line count of current text selection (0 = no selection).
    pub selection_lines: usize,
    /// True when terminal input is locked (read-only mode).
    pub locked: bool,
    /// Git branch name (empty = not in a git repo).
    pub git_branch: String,
    /// Exit code of the last completed command (None = no command completed or shell integration inactive).
    /// Displayed in status bar as a red segment when non-zero.
    pub last_exit_code: Option<i32>,
    /// Whether to show a system clock at the end of the status bar.
    /// Default: false (enabled at runtime in the event loop).
    pub show_clock: bool,
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
            bell_active: false,
            search_active: false,
            ai_active: false,
            config_error: None,
            profile_name: String::new(),
            broadcast_mode: String::new(),
            cwd: String::new(),
            remote_host: String::new(),
            pane_zoomed: false,
            font_size: 14.0,
            scroll_mode: false,
            progress: None,
            p2p_active: false,
            command_running: false,
            command_timer: String::new(),
            spinner_frame: 0,
            selection_count: 0,
            selection_lines: 0,
            locked: false,
            git_branch: String::new(),
            last_exit_code: None,
            hovered_link: None,
            show_clock: false,
        }
    }

    /// Update the cursor position.
    pub fn update_cursor(&mut self, row: usize, col: usize) {
        self.cursor_row = row;
        self.cursor_col = col;
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
        // format_segments already includes " | " separator segments.
        let segs = self.format_segments();
        segs.into_iter().map(|(text, _)| text).collect::<String>()
    }

    /// Format the status bar as colored segments for modern overlay rendering.
    ///
    /// Returns a list of `(text, color)` pairs where color is `(r, g, b)` in `[0, 255]`.
    /// Each segment is separated by a dim separator `|`.
    ///
    /// The renderer can use this to draw each segment at the correct position
    /// with appropriate coloring (e.g. red for errors, green for exit 0).
    pub fn format_segments(&self) -> Vec<(String, (u8, u8, u8))> {
        let mut segs = Vec::with_capacity(24);
        self.format_segments_into(&mut segs);
        segs
    }

    /// Write formatted segments into the provided Vec (reuses allocation).
    /// The Vec is cleared first. Use this in render hot paths to avoid
    /// per-frame Vec allocation.
    pub fn format_segments_into(&self, segs: &mut Vec<(String, (u8, u8, u8))>) {
        let dim_color: (u8, u8, u8) = (90, 90, 100);
        let accent_color: (u8, u8, u8) = (120, 180, 255);
        let warn_color: (u8, u8, u8) = (230, 180, 80);
        let err_color: (u8, u8, u8) = (230, 80, 80);
        let ok_color: (u8, u8, u8) = (100, 200, 120);

        segs.clear();

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

        // ── Core status (always relevant) ──

        // Cursor position.
        seg!(
            format!("{}:{}", self.cursor_row + 1, self.cursor_col + 1),
            accent_color
        );

        // CWD (from OSC 7). Show basename only for compactness.
        if !self.cwd.is_empty() {
            let display = std::path::Path::new(&self.cwd)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&self.cwd);
            seg!(display.to_string(), dim_color);
        }

        // Git branch.
        if !self.git_branch.is_empty() {
            seg!(format!(" {}", self.git_branch), (120u8, 200, 120));
        }

        // ── Activity indicators (only when active) ──

        // Running command spinner + timer.
        if self.command_running {
            let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let frame = frames[(self.spinner_frame as usize) % frames.len()];
            seg!(
                if !self.command_timer.is_empty() {
                    format!("{frame} {}", self.command_timer)
                } else {
                    frame.to_string()
                },
                accent_color
            );
        }

        // Selection count — multi-line shows line count, single-line shows char count.
        if self.selection_count > 0 {
            if self.selection_lines > 1 {
                seg!(
                    format!("SEL:{}L {}c", self.selection_lines, self.selection_count),
                    warn_color
                );
            } else {
                seg!(format!("SEL:{}", self.selection_count), warn_color);
            }
        }

        // ── Alerts (only when triggered) ──

        if self.locked {
            seg!("LOCK".to_string(), err_color);
        }
        if self.bell_active {
            seg!("BELL".to_string(), warn_color);
        }
        if self.search_active {
            seg!("SEARCH".to_string(), accent_color);
        }
        if self.ai_active {
            seg!("AI".to_string(), accent_color);
        }
        if self.scroll_mode {
            seg!("SCROLL".to_string(), accent_color);
        }

        // Exit code — only show on failure.
        if let Some(code) = self.last_exit_code
            && code != 0
        {
            seg!(format!("exit:{}", code), err_color);
        }

        // ── Context (only when relevant) ──

        // Pane zoom.
        if self.pane_zoomed {
            seg!("ZOOM".to_string(), accent_color);
        }
        // Broadcast mode.
        if !self.broadcast_mode.is_empty() {
            seg!("BCAST".to_string(), warn_color);
        }
        // Remote SSH host.
        if !self.remote_host.is_empty() {
            seg!(format!("SSH:{}", self.remote_host), accent_color);
        }
        // P2P sharing.
        if self.p2p_active {
            seg!("SHARE".to_string(), accent_color);
        }
        // Hovered URL preview.
        if let Some(ref link) = self.hovered_link {
            let display = if link.chars().count() > 50 {
                let truncated: String = link.chars().take(47).collect();
                format!("{truncated}...")
            } else {
                link.clone()
            };
            seg!(display, accent_color);
        }
        // Font zoom indicator.
        if (self.font_size - 14.0).abs() > 0.1 {
            seg!(format!("{}px", self.font_size as u32), dim_color);
        }
        // Progress.
        if let Some(pct) = self.progress {
            seg!(format!("{:.0}%", pct * 100.0), ok_color);
        }

        // System clock.
        if self.show_clock {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let secs = now.as_secs() % 86400; // seconds since midnight UTC
            let h = (secs / 3600 + local_offset_hours()) % 24;
            let m = (secs / 60) % 60;
            seg!(format!("{:02}:{:02}", h, m), dim_color);
        }
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

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_default_format() {
        let sb = StatusBar::new();
        let formatted = sb.format();
        // Default: cursor 1:1, no flags.
        assert_eq!(formatted, "1:1");
    }

    #[test]
    fn t_update_cursor() {
        let mut sb = StatusBar::new();
        sb.update_cursor(5, 10);
        assert_eq!(sb.format(), "6:11");
    }

    #[test]
    fn t_bell_search_ai_flags() {
        let mut sb = StatusBar::new();
        sb.update_cursor(3, 7);
        sb.set_bell(true);
        sb.set_search(true);
        sb.set_ai(true);
        assert_eq!(sb.format(), "4:8 | BELL | SEARCH | AI");
    }

    #[test]
    fn t_multi_tab_display() {
        let mut sb = StatusBar::new();
        sb.update_cursor(0, 0);
        // Tabs no longer shown in simplified format.
        assert_eq!(sb.format(), "1:1");
    }

    #[test]
    fn t_all_flags_cleared() {
        let mut sb = StatusBar::new();
        sb.update_cursor(10, 20);
        sb.set_bell(true);
        sb.set_search(true);
        sb.set_ai(true);

        sb.set_bell(false);
        sb.set_search(false);
        sb.set_ai(false);

        assert_eq!(sb.format(), "11:21");
    }

    #[test]
    fn t_single_tab_not_shown() {
        let mut sb = StatusBar::new();
        sb.update_cursor(0, 0);
        assert_eq!(sb.format(), "1:1");
    }

    #[test]
    fn t_bell_only() {
        let mut sb = StatusBar::new();
        sb.set_bell(true);
        assert_eq!(sb.format(), "1:1 | BELL");
    }

    #[test]
    fn t_search_only() {
        let mut sb = StatusBar::new();
        sb.set_search(true);
        assert_eq!(sb.format(), "1:1 | SEARCH");
    }

    // ── P17-E: Exit code tests ──────────────────────────────────────

    #[test]
    fn t_exit_code_zero_shows_ok() {
        let mut sb = StatusBar::new();
        sb.last_exit_code = Some(0);
        // Exit 0 is hidden in simplified format.
        assert_eq!(sb.format(), "1:1");
    }

    #[test]
    fn t_exit_code_nonzero_shows_code() {
        let mut sb = StatusBar::new();
        sb.last_exit_code = Some(127);
        assert_eq!(sb.format(), "1:1 | exit:127");
    }

    #[test]
    fn t_exit_code_none_not_shown() {
        let mut sb = StatusBar::new();
        sb.last_exit_code = None;
        assert_eq!(sb.format(), "1:1");
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
        assert_eq!(sb.format(), "!ERROR! | 1:1");
    }

    #[test]
    fn t_config_error_with_other_flags() {
        let mut sb = StatusBar::new();
        sb.update_cursor(5, 10);
        sb.set_bell(true);
        sb.set_config_error(Some("bad theme".to_string()));
        assert_eq!(sb.format(), "!ERROR! | 6:11 | BELL");
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
        // Exit 0 is now hidden (simplified — only errors shown).
        sb.last_exit_code = Some(0);
        let segs = sb.format_segments();
        let exit_seg = segs.iter().find(|(t, _)| t.starts_with("exit:"));
        assert!(exit_seg.is_none());

        // Exit 1 should be red-ish (high red channel).
        sb.last_exit_code = Some(1);
        let segs = sb.format_segments();
        let exit_seg = segs.iter().find(|(t, _)| t.starts_with("exit:"));
        assert!(exit_seg.is_some());
        let (_, color) = exit_seg.unwrap();
        assert!(color.0 > color.1 && color.0 > color.2); // r > g && r > b
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
    fn t_status_bar_command_duration_empty_omitted() {
        let mut sb = StatusBar::new();
        sb.last_exit_code = Some(0);
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
    fn t_status_bar_selection_count_zero_omitted() {
        let sb = StatusBar::new();
        let formatted = sb.format();
        assert!(
            !formatted.contains("SEL:"),
            "should not show SEL when 0: {formatted}"
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
    fn t_status_bar_exit_code_failure() {
        let mut sb = StatusBar::new();
        sb.last_exit_code = Some(127);
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
