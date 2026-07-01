//! Tab session — bundles App + PtySession + event channel for one terminal tab.
//!
//! Each [`TabSession`] owns one or more [`PaneSession`]s (terminal + PTY +
//! event channel) managed by a [`SplitTree`] for tmux-style split panes.
//!
//! The [`DesktopApp`](crate::window::DesktopApp) holds a `Vec<TabSession>`
//! and an `active` index to support multi-tab terminal sessions.

use ggterm_core::pty::PtySession;

use crate::app::{App, spawn_pty_reader};
use crate::event::EventSender;
use crate::shell_integration::ShellIntegrationConfig;
use crate::splits::{PaneId, SplitTree};

/// A single terminal pane — owns App (Terminal + Parser + Grid), PTY, event channel.
struct PaneSession {
    /// Terminal application state (Parser + Terminal + Grid).
    app: App,
    /// PTY session (owned shell process).
    pty: Option<PtySession>,
    /// Event sender for the PTY reader thread.
    event_tx: EventSender,
    /// P21-D: True when grid needs re-prepare before next draw.
    /// Defaults to `true` so the first frame does a full prepare.
    needs_reprepare: bool,
    /// Shell program path, saved for restart.
    shell: String,
}

impl PaneSession {
    /// Create a new pane session with an optional working directory.
    fn new_with_cwd(
        cols: u16,
        rows: u16,
        shell: &str,
        cwd: Option<&std::path::Path>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let shell_integration = ShellIntegrationConfig::prepare(shell);
        let (program, spawn_args) = shell_integration.spawn_args();
        let env_vars = shell_integration.env_vars();

        let pty =
            PtySession::open_with_cwd(cols, rows, Some(&program), &spawn_args, &env_vars, cwd)?;

        let (mut app, event_tx) = App::new(cols as usize, rows as usize);
        app.start();

        let reader = pty.try_clone_reader()?;
        spawn_pty_reader(reader, event_tx.clone());

        Ok(Self {
            app,
            pty: Some(pty),
            event_tx,
            needs_reprepare: true,
            shell: shell.to_string(),
        })
    }

    /// Create a test-only pane session with no real PTY.
    #[cfg(test)]
    fn new_test(cols: usize, rows: usize) -> Self {
        let (mut app, event_tx) = App::new(cols, rows);
        app.start();
        Self {
            app,
            pty: None,
            event_tx,
            needs_reprepare: true,
            shell: String::new(),
        }
    }

    /// Restart the shell process: drop old PTY, create new one with same
    /// shell + cwd, reset terminal grid.
    fn restart_shell(&mut self) {
        // Save cwd before dropping old pty.
        let cwd = self.app.terminal().cwd().map(|p| p.to_path_buf());
        let cols = self.app.grid().width() as u16;
        let rows = self.app.grid().height() as u16;

        // Drop old PTY (kills child process).
        self.pty = None;

        // Reset terminal grid.
        crate::terminal_actions::clear_screen_and_scrollback(self.app.grid_mut());

        // Spawn new shell.
        let shell_integration = ShellIntegrationConfig::prepare(&self.shell);
        let (program, spawn_args) = shell_integration.spawn_args();
        let env_vars = shell_integration.env_vars();

        match PtySession::open_with_cwd(
            cols,
            rows,
            Some(&program),
            &spawn_args,
            &env_vars,
            cwd.as_deref(),
        ) {
            Ok(new_pty) => {
                if let Ok(reader) = new_pty.try_clone_reader() {
                    spawn_pty_reader(reader, self.event_tx.clone());
                }
                self.pty = Some(new_pty);
                self.needs_reprepare = true;
                log::info!("Restarted shell: {}", program);
            }
            Err(e) => {
                log::error!("Failed to restart shell: {}", e);
            }
        }
    }
}

/// A single terminal tab session.
///
/// Owns one or more [`PaneSession`]s managed by a [`SplitTree`] for split panes.
/// All existing accessor methods (app, app_mut, pump, etc.) operate on the
/// **active pane** — the pane currently focused in the split tree.
pub struct TabSession {
    /// All panes in this tab. PaneId indexes directly into this Vec.
    /// Dead panes (removed from split tree) remain in the Vec for stable indexing
    /// but are not rendered or pumped.
    panes: Vec<Option<PaneSession>>,
    /// Split-pane layout tree.
    split_tree: SplitTree,
    /// Tab title (from OSC 0/2 or default).
    title: String,
}

impl TabSession {
    /// Create a new tab session with a single pane: spawn a PTY, wire up
    /// the reader thread, and create a new App.
    pub fn new(cols: u16, rows: u16, shell: &str) -> Result<Self, Box<dyn std::error::Error>> {
        Self::new_with_cwd(cols, rows, shell, None)
    }

    /// Create a new tab session with an optional working directory.
    /// When `cwd` is provided (e.g., from OSC 7), the shell starts in that directory.
    pub fn new_with_cwd(
        cols: u16,
        rows: u16,
        shell: &str,
        cwd: Option<&std::path::Path>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let pane = PaneSession::new_with_cwd(cols, rows, shell, cwd)?;
        let title = shell.rsplit('/').next().unwrap_or(shell).to_string();
        Ok(Self {
            panes: vec![Some(pane)],
            split_tree: SplitTree::new(0),
            title,
        })
    }

    /// Create a test-only TabSession with no real PTY.
    #[cfg(test)]
    pub fn new_test(cols: usize, rows: usize) -> Self {
        let pane = PaneSession::new_test(cols, rows);
        Self {
            panes: vec![Some(pane)],
            split_tree: SplitTree::new(0),
            title: "test".to_string(),
        }
    }

    // ── Active pane accessors (compatible with pre-split API) ──

    /// Get the active pane's App (Terminal + Parser + Grid).
    pub fn app(&self) -> &App {
        &self.active_pane().app
    }

    /// Get the active pane's App mutably.
    pub fn app_mut(&mut self) -> &mut App {
        &mut self.active_pane_mut().app
    }

    /// Get the active pane's PTY session.
    pub fn pty(&self) -> Option<&PtySession> {
        self.active_pane().pty.as_ref()
    }

    /// Get the active pane's PTY session mutably.
    pub fn pty_mut(&mut self) -> Option<&mut PtySession> {
        self.active_pane_mut().pty.as_mut()
    }

    /// Get the active pane's event sender.
    pub fn event_tx(&self) -> &EventSender {
        &self.active_pane().event_tx
    }

    /// Get the tab title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// P29-B: Scroll all panes' viewports simultaneously (sync scroll).
    pub fn scroll_all_panes_viewport(&mut self, lines: i32) {
        for pane in self.panes.iter_mut().flatten() {
            let grid = pane.app.terminal_mut().grid_mut();
            if lines > 0 {
                grid.scroll_up_viewport(lines as usize);
            } else {
                grid.scroll_down_viewport((-lines) as usize);
            }
        }
    }

    /// Set the tab title (e.g., from OSC 0/2).
    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = title.into();
    }

    /// Return the active pane's current working directory (P22-D).
    /// Set via OSC 7 by shell integration.
    pub fn cwd(&self) -> Option<&std::path::Path> {
        self.active_pane().app.terminal().cwd()
    }

    /// Return a specific pane's current working directory (P22-D).
    pub fn pane_cwd(&self, id: PaneId) -> Option<std::path::PathBuf> {
        self.panes
            .get(id)
            .and_then(|p| p.as_ref())
            .and_then(|p| p.app.terminal().cwd().map(|p| p.to_path_buf()))
    }

    /// Pump events for **all** panes from their PTY reader threads.
    pub fn pump(&mut self) {
        for pane in self.panes.iter_mut().flatten() {
            if pane.app.pump() {
                pane.needs_reprepare = true;
            }
        }
    }

    /// P21-D: Check if a pane needs grid re-prepare.
    pub fn pane_needs_prepare(&self, id: PaneId) -> bool {
        self.panes
            .get(id)
            .and_then(|p: &Option<PaneSession>| p.as_ref())
            .is_some_and(|p| p.needs_reprepare)
    }

    /// P21-D: Clear all panes' re-prepare flags. Call after render_frame().
    pub fn clear_prepare_flags(&mut self) {
        for pane in self.panes.iter_mut().flatten() {
            pane.needs_reprepare = false;
        }
    }

    /// Write bytes to the active pane's PTY.
    pub fn write_to_pty(&mut self, bytes: &[u8]) {
        if let Some(ref mut pty) = self.active_pane_mut().pty
            && let Err(e) = pty.write(bytes)
        {
            log::warn!("PTY write error: {e}");
        }
    }

    /// P25-D: Write bytes to **all** panes' PTYs (broadcast input mode).
    pub fn write_to_all_panes(&mut self, bytes: &[u8]) {
        for pane in self.panes.iter_mut().flatten() {
            if let Some(ref mut pty) = pane.pty
                && let Err(e) = pty.write(bytes)
            {
                log::warn!("PTY broadcast write error: {e}");
            }
        }
    }

    /// Check if the active pane's shell process is still alive.
    pub fn is_alive(&mut self) -> bool {
        self.active_pane_mut()
            .pty
            .as_mut()
            .is_some_and(|p| p.is_alive())
    }

    /// Check if the active pane's app is still running.
    pub fn is_running(&self) -> bool {
        self.active_pane().app.is_running()
    }

    /// Resize **all** panes' terminals and PTYs to the given dimensions.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        for pane in self.panes.iter_mut().flatten() {
            pane.app
                .handle_event(crate::event::AppEvent::Resize { cols, rows });
            if let Some(ref mut pty) = pane.pty
                && let Err(e) = pty.resize(cols, rows)
            {
                log::warn!("PTY resize failed: {e}");
            }
            // P21-D: Dimensions changed — all panes need re-prepare.
            pane.needs_reprepare = true;
        }
    }

    /// Resize each pane to match its pixel area in the split tree.
    ///
    /// Converts each pane's pixel `Rect` to cell dimensions using the given
    /// `cell_w`/`cell_h`, and resizes the pane's terminal grid + PTY if the
    /// dimensions changed. This ensures text wraps at the pane boundary, not
    /// at the full window width.
    pub fn resize_panes_to_areas(
        &mut self,
        areas: &[(PaneId, crate::splits::Rect)],
        cell_w: u32,
        cell_h: u32,
    ) {
        for (id, rect) in areas {
            let cols =
                ((rect.width as f32 / cell_w as f32) as u16).max(crate::desktop_config::MIN_COLS);
            let rows =
                ((rect.height as f32 / cell_h as f32) as u16).max(crate::desktop_config::MIN_ROWS);
            if let Some(pane) = self.panes.get_mut(*id).and_then(|p| p.as_mut()) {
                let cur_cols = pane.app.grid().width() as u16;
                let cur_rows = pane.app.grid().height() as u16;
                if cur_cols != cols || cur_rows != rows {
                    log::debug!(
                        "Pane {}: resize {}x{} → {}x{}",
                        id,
                        cur_cols,
                        cur_rows,
                        cols,
                        rows
                    );
                    pane.app
                        .handle_event(crate::event::AppEvent::Resize { cols, rows });
                    if let Some(ref mut pty) = pane.pty
                        && let Err(e) = pty.resize(cols, rows)
                    {
                        log::warn!("PTY resize failed: {e}");
                    }
                    pane.needs_reprepare = true;
                }
            }
        }
    }

    // ── Split-pane management (P19-B) ──

    /// Get the split-pane layout tree.
    pub fn split_tree(&self) -> &SplitTree {
        &self.split_tree
    }

    /// Get the split-pane layout tree mutably.
    pub fn split_tree_mut(&mut self) -> &mut SplitTree {
        &mut self.split_tree
    }

    /// Number of active panes in this tab.
    pub fn pane_count(&self) -> usize {
        self.split_tree.pane_count()
    }

    /// Collect all active pane IDs in visual order.
    pub fn pane_ids(&self) -> Vec<PaneId> {
        self.split_tree.pane_ids()
    }

    /// Get a specific pane's App by ID.
    pub fn pane_app(&self, id: PaneId) -> Option<&App> {
        self.panes.get(id).and_then(|p| p.as_ref()).map(|p| &p.app)
    }

    /// Get a specific pane's App mutably by ID.
    pub fn pane_app_mut(&mut self, id: PaneId) -> Option<&mut App> {
        self.panes
            .get_mut(id)
            .and_then(|p| p.as_mut())
            .map(|p| &mut p.app)
    }

    /// Split the active pane horizontally (left | right).
    ///
    /// The new pane becomes the active pane. Returns the new pane ID.
    pub fn split_horizontal(
        &mut self,
        cols: u16,
        rows: u16,
        shell: &str,
    ) -> Result<PaneId, Box<dyn std::error::Error>> {
        self.split_horizontal_with_cwd(cols, rows, shell, None)
    }

    /// Split horizontal with optional cwd inheritance.
    pub fn split_horizontal_with_cwd(
        &mut self,
        cols: u16,
        rows: u16,
        shell: &str,
        cwd: Option<&std::path::Path>,
    ) -> Result<PaneId, Box<dyn std::error::Error>> {
        let pane = PaneSession::new_with_cwd(cols, rows, shell, cwd)?;
        self.split_tree.split_horizontal(0.5);
        let new_id = self.split_tree.active();
        // PaneId matches Vec index — extend if needed.
        while self.panes.len() <= new_id {
            self.panes.push(None);
        }
        self.panes[new_id] = Some(pane);
        Ok(new_id)
    }

    /// Split the active pane vertically (top / bottom).
    ///
    /// The new pane becomes the active pane. Returns the new pane ID.
    pub fn split_vertical(
        &mut self,
        cols: u16,
        rows: u16,
        shell: &str,
    ) -> Result<PaneId, Box<dyn std::error::Error>> {
        self.split_vertical_with_cwd(cols, rows, shell, None)
    }

    /// Split vertical with optional cwd inheritance.
    pub fn split_vertical_with_cwd(
        &mut self,
        cols: u16,
        rows: u16,
        shell: &str,
        cwd: Option<&std::path::Path>,
    ) -> Result<PaneId, Box<dyn std::error::Error>> {
        let pane = PaneSession::new_with_cwd(cols, rows, shell, cwd)?;
        self.split_tree.split_vertical(0.5);
        let new_id = self.split_tree.active();
        while self.panes.len() <= new_id {
            self.panes.push(None);
        }
        self.panes[new_id] = Some(pane);
        Ok(new_id)
    }

    /// Remove the active pane from the split tree.
    ///
    /// The PTY is closed (dropped). Focus moves to the previous pane.
    /// Does nothing if this is the only pane (use `close_tab` instead).
    pub fn remove_active_pane(&mut self) {
        if self.split_tree.is_single() {
            return;
        }
        let active = self.split_tree.active();
        // Close PTY by dropping it.
        if let Some(slot) = self.panes.get_mut(active) {
            *slot = None;
        }
        self.split_tree.remove(active);
    }

    /// Cycle focus to the next pane.
    pub fn focus_next_pane(&mut self) {
        self.split_tree.focus_next();
    }

    /// Cycle focus to the previous pane.
    pub fn focus_prev_pane(&mut self) {
        self.split_tree.focus_prev();
    }

    /// Restart the shell process in the active pane.
    /// Drops the old PTY and spawns a fresh shell with the same config + cwd.
    pub fn restart_active_shell(&mut self) {
        self.active_pane_mut().restart_shell();
    }

    // ── Internal helpers ──

    fn active_pane(&self) -> &PaneSession {
        let id = self.split_tree.active();
        self.panes[id]
            .as_ref()
            .expect("active pane must exist in panes vec")
    }

    fn active_pane_mut(&mut self) -> &mut PaneSession {
        let id = self.split_tree.active();
        self.panes[id]
            .as_mut()
            .expect("active pane must exist in panes vec")
    }
}

/// Render a tab bar string from session titles, active index, and dirty flags.
///
/// Format: `1:zsh | 2:vim* | 3:logs!`
/// - `*` marks the active tab
/// - `!` marks tabs with unread output (dirty)
pub fn format_tab_bar(titles: &[String], active: usize, dirty: &[bool]) -> String {
    if titles.is_empty() {
        return String::new();
    }
    titles
        .iter()
        .enumerate()
        .map(|(i, title)| {
            let truncated = if title.chars().count() > 10 {
                format!("{}…", title.chars().take(9).collect::<String>())
            } else {
                title.clone()
            };
            let marker = if i == active {
                "*"
            } else if dirty.get(i).copied().unwrap_or(false) {
                "!"
            } else {
                ""
            };
            format!("{}:{}{}", i + 1, truncated, marker)
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Tab bar rendering tests ──

    #[test]
    fn test_format_tab_bar_single() {
        let titles = vec!["zsh".to_string()];
        let dirty = vec![false];
        assert_eq!(format_tab_bar(&titles, 0, &dirty), "1:zsh*");
    }

    #[test]
    fn test_format_tab_bar_multi() {
        let titles = vec!["zsh".to_string(), "vim".to_string(), "logs".to_string()];
        let dirty = vec![false, false, false];
        assert_eq!(
            format_tab_bar(&titles, 1, &dirty),
            "1:zsh | 2:vim* | 3:logs"
        );
    }

    #[test]
    fn test_format_tab_bar_empty() {
        assert_eq!(format_tab_bar(&[], 0, &[]), "");
    }

    #[test]
    fn test_format_tab_bar_last_active() {
        let titles = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let dirty = vec![false, false, false];
        assert_eq!(format_tab_bar(&titles, 2, &dirty), "1:a | 2:b | 3:c*");
    }

    #[test]
    fn test_format_tab_bar_dirty_marker() {
        let titles = vec!["zsh".to_string(), "vim".to_string()];
        let dirty = vec![false, true];
        assert_eq!(format_tab_bar(&titles, 0, &dirty), "1:zsh* | 2:vim!");
    }

    #[test]
    fn test_format_tab_bar_long_title_truncated() {
        let titles = vec!["very_long_process_name".to_string()];
        let dirty = vec![false];
        let bar = format_tab_bar(&titles, 0, &dirty);
        assert!(bar.contains("1:very_long…*"));
    }

    // ── Tab session lifecycle tests ──

    #[test]
    fn test_tab_session_new_test() {
        let session = TabSession::new_test(80, 24);
        assert!(session.is_running());
        assert!(session.pty().is_none());
        assert_eq!(session.title(), "test");
    }

    #[test]
    fn test_tab_session_set_title() {
        let mut session = TabSession::new_test(80, 24);
        session.set_title("vim");
        assert_eq!(session.title(), "vim");
    }

    #[test]
    fn test_tab_session_pump() {
        let mut session = TabSession::new_test(80, 24);
        session.pump();
    }

    #[test]
    fn test_tab_session_is_alive_no_pty() {
        let mut session = TabSession::new_test(80, 24);
        assert!(!session.is_alive());
    }

    #[test]
    fn test_tab_session_write_no_pty() {
        let mut session = TabSession::new_test(80, 24);
        session.write_to_pty(b"hello");
    }

    // ── Split pane tests ──

    #[test]
    fn test_split_creates_second_pane() {
        let mut session = TabSession::new_test(80, 24);
        assert_eq!(session.pane_count(), 1);

        // Create a test pane manually (no PTY needed).
        session.panes.push(Some(PaneSession::new_test(40, 24)));
        session.split_tree.split_horizontal(0.5);

        assert_eq!(session.pane_count(), 2);
        assert_eq!(session.pane_ids(), vec![0, 1]);
    }

    #[test]
    fn test_split_focus_switches_to_new_pane() {
        let mut session = TabSession::new_test(80, 24);
        session.panes.push(Some(PaneSession::new_test(40, 24)));
        session.split_tree.split_horizontal(0.5);
        assert_eq!(session.split_tree.active(), 1);
    }

    #[test]
    fn test_focus_next_cycles() {
        let mut session = TabSession::new_test(80, 24);
        session.panes.push(Some(PaneSession::new_test(40, 24)));
        session.split_tree.split_horizontal(0.5);
        // panes [0, 1], active = 1
        session.focus_next_pane();
        assert_eq!(session.split_tree.active(), 0); // wraps to first
        session.focus_next_pane();
        assert_eq!(session.split_tree.active(), 1);
    }

    #[test]
    fn test_focus_prev_cycles() {
        let mut session = TabSession::new_test(80, 24);
        session.panes.push(Some(PaneSession::new_test(40, 24)));
        session.split_tree.split_horizontal(0.5);
        // active = 1
        session.focus_prev_pane();
        assert_eq!(session.split_tree.active(), 0);
    }

    #[test]
    fn test_remove_pane_decreases_count() {
        let mut session = TabSession::new_test(80, 24);
        session.panes.push(Some(PaneSession::new_test(40, 24)));
        session.split_tree.split_horizontal(0.5);
        assert_eq!(session.pane_count(), 2);

        session.remove_active_pane();
        assert_eq!(session.pane_count(), 1);
    }

    #[test]
    fn test_remove_single_pane_is_noop() {
        let mut session = TabSession::new_test(80, 24);
        session.remove_active_pane();
        assert_eq!(session.pane_count(), 1);
    }

    #[test]
    fn test_pane_app_by_id() {
        let mut session = TabSession::new_test(80, 24);
        session.panes.push(Some(PaneSession::new_test(40, 24)));

        assert!(session.pane_app(0).is_some());
        assert!(session.pane_app(1).is_some());
        assert!(session.pane_app(99).is_none());
    }

    #[test]
    fn test_active_pane_app_returns_correct_pane() {
        let mut session = TabSession::new_test(80, 24);
        session.panes.push(Some(PaneSession::new_test(40, 24)));
        session.split_tree.split_horizontal(0.5);
        // active = 1 (new pane)
        let app1 = session.app();
        assert_eq!(app1.grid().width(), 40);

        session.focus_prev_pane();
        let app0 = session.app();
        assert_eq!(app0.grid().width(), 80);
    }

    #[test]
    fn test_resize_all_panes() {
        let mut session = TabSession::new_test(80, 24);
        session.panes.push(Some(PaneSession::new_test(40, 24)));
        session.split_tree.split_horizontal(0.5);

        session.resize(100, 30);
        // Both panes should be resized
        assert_eq!(session.pane_app(0).unwrap().grid().width(), 100);
        assert_eq!(session.pane_app(1).unwrap().grid().width(), 100);
    }

    // ── Tab navigation state tests ──

    #[test]
    fn test_tab_nav_next_wraps() {
        let count = 3;
        let mut active = 0usize;
        active = (active + 1) % count;
        assert_eq!(active, 1);
        active = (active + 1) % count;
        assert_eq!(active, 2);
        active = (active + 1) % count;
        assert_eq!(active, 0);
    }

    #[test]
    fn test_tab_nav_prev_wraps() {
        let count = 3;
        let mut active = 0usize;
        active = if active == 0 { count - 1 } else { active - 1 };
        assert_eq!(active, 2);
        active = if active == 0 { count - 1 } else { active - 1 };
        assert_eq!(active, 1);
    }

    #[test]
    fn test_tab_switch_by_index() {
        let count = 5;
        for key in 1..=count {
            let index = (key - 1).min(count - 1);
            assert_eq!(index, key - 1);
        }
        let key = 9;
        let index = (key - 1).min(count - 1);
        assert_eq!(index, 4);
    }

    #[test]
    fn test_tab_open_increases_count() {
        let mut count = 1usize;
        for _ in 0..3 {
            count += 1;
        }
        assert_eq!(count, 4);
    }

    #[test]
    fn test_tab_close_keeps_at_least_one() {
        let mut count = 1usize;
        if count > 1 {
            count -= 1;
        }
        assert_eq!(count, 1, "cannot close the last tab");
    }

    #[test]
    fn test_tab_close_middle_adjusts_active() {
        let mut tabs = vec!["A", "B", "C"];
        let mut active = 1usize;
        let close_idx = 1;

        tabs.remove(close_idx);
        if active >= tabs.len() {
            active = tabs.len() - 1;
        } else if close_idx < active {
            active -= 1;
        }
        assert_eq!(tabs, vec!["A", "C"]);
        assert_eq!(active, 1);
    }

    #[test]
    fn test_tab_close_first_keeps_active_zero() {
        let mut tabs = vec!["A", "B", "C"];
        let mut active = 0usize;
        let close_idx = 0;

        tabs.remove(close_idx);
        if active >= tabs.len() {
            active = tabs.len() - 1;
        } else if close_idx < active {
            active -= 1;
        }
        assert_eq!(tabs, vec!["B", "C"]);
        assert_eq!(active, 0);
    }

    #[test]
    fn test_tab_close_last_decrements_active() {
        let mut tabs = vec!["A", "B", "C"];
        let mut active = 2usize;
        let close_idx = 2;

        tabs.remove(close_idx);
        if active >= tabs.len() {
            active = tabs.len() - 1;
        } else if close_idx < active {
            active -= 1;
        }
        assert_eq!(tabs, vec!["A", "B"]);
        assert_eq!(active, 1);
    }

    #[test]
    fn test_tab_bar_format_with_dirty_and_active() {
        let titles = vec![
            "bash".to_string(),
            "ssh user@host".to_string(),
            "logs".to_string(),
        ];
        let dirty = vec![true, false, true];
        let bar = format_tab_bar(&titles, 1, &dirty);
        assert_eq!(bar, "1:bash! | 2:ssh user@…* | 3:logs!");
    }

    #[test]
    fn test_tab_bar_active_overrides_dirty() {
        let titles = vec!["a".to_string(), "b".to_string()];
        let dirty = vec![true, false];
        let bar = format_tab_bar(&titles, 0, &dirty);
        assert_eq!(bar, "1:a* | 2:b");
    }

    // ── P21-D: needs_prepare tests ────────────────────────────

    #[test]
    fn test_pane_needs_prepare_default_true() {
        // PaneSession::new_test sets needs_reprepare = true (first frame must prepare).
        let session = TabSession::new_test(80, 24);
        assert!(
            session.pane_needs_prepare(0),
            "default needs_prepare should be true for first frame"
        );
    }

    #[test]
    fn test_clear_prepare_flags() {
        let mut session = TabSession::new_test(80, 24);
        assert!(session.pane_needs_prepare(0));

        session.clear_prepare_flags();
        assert!(
            !session.pane_needs_prepare(0),
            "needs_prepare should be false after clear_prepare_flags()"
        );
    }

    #[test]
    fn test_resize_sets_needs_prepare() {
        let mut session = TabSession::new_test(80, 24);
        // Clear first (default is true)
        session.clear_prepare_flags();
        assert!(!session.pane_needs_prepare(0));

        // Resize should mark all panes as needing re-prepare.
        session.resize(120, 40);
        assert!(
            session.pane_needs_prepare(0),
            "needs_prepare should be true after resize"
        );
    }

    #[test]
    fn test_pump_does_not_mark_clean_pane() {
        let mut session = TabSession::new_test(80, 24);
        // Clear the default flag.
        session.clear_prepare_flags();

        // Pump with no PTY data should NOT set needs_prepare.
        session.pump();
        assert!(
            !session.pane_needs_prepare(0),
            "pump with no data should not set needs_prepare"
        );
    }

    // ── P22-D: OSC 7 cwd tests ────────────────────────────────

    #[test]
    fn test_cwd_default_none() {
        let session = TabSession::new_test(80, 24);
        assert!(session.cwd().is_none());
    }

    #[test]
    fn test_cwd_after_osc7() {
        let mut session = TabSession::new_test(80, 24);
        // Feed OSC 7 via the terminal event channel
        use crate::event::AppEvent;
        let osc7 = b"\x1b]7;file://localhost/home/user/projects\x1b\\".to_vec();
        session.app_mut().handle_event(AppEvent::PtyBytes(osc7));
        session.pump();
        assert_eq!(
            session.cwd(),
            Some(std::path::Path::new("/home/user/projects"))
        );
    }

    #[test]
    fn test_pane_cwd_independent() {
        let session = TabSession::new_test(80, 24);
        // Only one pane initially — cwd should be queryable per-pane
        assert!(session.pane_cwd(0).is_none());
    }
}
