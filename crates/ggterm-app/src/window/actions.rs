//! DesktopApp action methods — tab, split, clipboard, theme, session.

use super::*;

impl DesktopApp {
    /// Handle the active pane's shell exit.
    ///
    /// Returns `true` if the app should exit (last pane in last tab).
    /// Returns `false` if a pane or tab was closed and the app should continue.
    ///
    /// After removing a pane, the remaining pane is resized to fill the full
    /// content area, and a redraw is requested.
    pub(super) fn handle_pane_exit(&mut self) -> bool {
        let pane_count = self.sessions[self.active].pane_count();
        if pane_count > 1 {
            // Multi-pane: remove the dead pane, keep the tab alive.
            log::info!(
                "Pane shell exited, closing pane (tab had {} panes)",
                pane_count
            );
            self.sessions[self.active].remove_active_pane();

            // Resize remaining pane(s) to fill the full content area.
            // The grid was sized for the split; now it needs full-window dimensions.
            if let Some(ref renderer) = self.renderer {
                let bounds = self.content_area_bounds();
                let cell_w = renderer.cell_width();
                let cell_h = renderer.cell_height();
                let session = &mut self.sessions[self.active];
                let tree = session.split_tree().clone();
                let areas = tree.areas(bounds);
                session.resize_panes_to_areas(&areas, cell_w, cell_h);
            }

            // Request a redraw to show the updated layout.
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
            false
        } else if self.sessions.len() > 1 {
            // Single-pane tab but multiple tabs: close this tab.
            log::info!("Tab shell exited, closing tab");
            self.close_tab();
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
            false
        } else {
            // Last pane in last tab: app should exit.
            true
        }
    }

    pub(super) fn handle_settings_left(&mut self) {
        match self.settings.selected {
            crate::settings_ui::SettingsField::Theme => {
                // Cycle theme backward
                let opts = crate::settings_ui::THEME_OPTIONS;
                let idx = opts
                    .iter()
                    .position(|&t| t == self.settings.theme)
                    .unwrap_or(0);
                let prev = if idx == 0 { opts.len() - 1 } else { idx - 1 };
                self.settings.theme = opts[prev].to_string();
                self.settings.dirty = true;
            }
            crate::settings_ui::SettingsField::FontSize => self.settings.font_size_down(),
            crate::settings_ui::SettingsField::Scrollback => self.settings.scrollback_down(),
            crate::settings_ui::SettingsField::AiEnabled => self.settings.toggle_ai(),
            _ => {}
        }
    }

    /// Handle Right arrow in settings (increase/cycle forward).
    pub(super) fn handle_settings_right(&mut self) {
        match self.settings.selected {
            crate::settings_ui::SettingsField::Theme => self.settings.cycle_theme(),
            crate::settings_ui::SettingsField::FontSize => self.settings.font_size_up(),
            crate::settings_ui::SettingsField::Scrollback => self.settings.scrollback_up(),
            crate::settings_ui::SettingsField::AiEnabled => self.settings.toggle_ai(),
            _ => {}
        }
    }

    // ── Tab management (P10-A) ──

    /// Open a new tab: create a TabSession with a fresh PTY.
    pub(super) fn open_tab(&mut self) {
        let cols = self.config.cols;
        let rows = self.config.rows;
        match TabSession::new(cols, rows, self.shell()) {
            Ok(session) => {
                self.sessions.push(session);
                self.active = self.sessions.len() - 1;
                log::info!("Opened tab {}", self.active + 1);
            }
            Err(e) => {
                log::error!("Failed to open tab: {e}");
            }
        }
    }

    /// Close the active tab (keep at least 1).
    pub(super) fn close_tab(&mut self) {
        if self.sessions.len() <= 1 {
            return;
        }
        self.sessions.remove(self.active);
        if self.active >= self.sessions.len() {
            self.active = self.sessions.len() - 1;
        }
        log::info!("Closed tab, active={}", self.active + 1);
    }

    /// Switch to a specific tab by index (0-based).
    pub(super) fn switch_tab(&mut self, index: usize) {
        if index < self.sessions.len() {
            self.active = index;
        }
    }

    /// Switch to the next tab (wraps).
    pub(super) fn next_tab(&mut self) {
        self.active = (self.active + 1) % self.sessions.len();
    }

    /// Switch to the previous tab (wraps).
    pub(super) fn prev_tab(&mut self) {
        self.active = if self.active == 0 {
            self.sessions.len() - 1
        } else {
            self.active - 1
        };
    }

    // ── P23-E: Tab reordering ──

    /// Move the active tab to a new position.
    #[allow(dead_code)]
    pub(super) fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.sessions.len() || to >= self.sessions.len() || from == to {
            return;
        }
        let session = self.sessions.remove(from);
        self.sessions.insert(to, session);
        self.active = to;
        log::info!("P23-E: moved tab {} → {}", from + 1, to + 1);
    }

    /// Start dragging a tab (called on mouse press in tab bar area).
    #[allow(dead_code)]
    pub(super) fn start_tab_drag(&mut self, tab_index: usize) {
        self.drag_tab = Some(tab_index);
        log::debug!("P23-E: started dragging tab {}", tab_index + 1);
    }

    /// Get the tab index at a given x pixel position (for click/drag in tab bar).
    #[allow(dead_code)]
    pub(super) fn tab_index_at_x(&self, x: f64, screen_width: f32) -> Option<usize> {
        if self.sessions.is_empty() {
            return None;
        }
        let tab_w = screen_width as f64 / self.sessions.len() as f64;
        if tab_w <= 0.0 {
            return None;
        }
        let idx = (x / tab_w) as usize;
        if idx < self.sessions.len() {
            Some(idx)
        } else {
            None
        }
    }

    // ── P22-A: Session save/restore ──

    /// Capture the current session layout into serializable form.
    pub(super) fn capture_session(&self) -> crate::session::SessionData {
        let tabs: Vec<crate::session::TabData> = self
            .sessions
            .iter()
            .map(|session| {
                let pane_ids = session.pane_ids();
                let panes: Vec<crate::session::PaneData> = pane_ids
                    .iter()
                    .map(|_id| crate::session::PaneData {
                        shell: self.shell().to_string(),
                        cwd: String::new(),
                    })
                    .collect();
                crate::session::TabData {
                    title: session.title().to_string(),
                    active_pane: session.split_tree().active(),
                    panes,
                    splits: crate::session::capture_split_tree(session.split_tree()),
                }
            })
            .collect();
        crate::session::SessionData {
            version: 1,
            tabs,
            active_tab: self.active,
        }
    }

    /// Save session to disk on exit.
    pub(super) fn save_session_on_exit(&mut self) {
        let data = self.capture_session();
        if let Err(e) = crate::session::save_session(&data) {
            log::warn!("Failed to save session: {e}");
        } else {
            log::info!("Session saved ({} tab(s))", data.tabs.len());
        }
    }

    /// Restore tabs/panes/splits from a saved session plan.
    ///
    /// Replaces all existing sessions with the restored ones.
    pub(super) fn restore_from_plan(&mut self, plan: &crate::session::SessionPlan) {
        if plan.tabs.is_empty() {
            return;
        }

        let cols = self.config.cols;
        let rows = self.config.rows;
        let default_shell = self.shell().to_string();
        let mut new_sessions: Vec<TabSession> = Vec::with_capacity(plan.tabs.len());

        for tab_spec in &plan.tabs {
            let effective_shell = if tab_spec.panes.is_empty() {
                &default_shell
            } else {
                &tab_spec.panes[0].shell
            };

            match TabSession::new(cols, rows, effective_shell) {
                Ok(mut session) => {
                    let pane_count = tab_spec.panes.len();
                    for i in 1..pane_count {
                        let pane_shell = if tab_spec.panes[i].shell.is_empty() {
                            &default_shell
                        } else {
                            &tab_spec.panes[i].shell
                        };
                        if i % 2 == 1 {
                            let _ = session.split_horizontal(cols, rows, pane_shell);
                        } else {
                            let _ = session.split_vertical(cols, rows, pane_shell);
                        }
                    }

                    // Restore exact split tree structure.
                    if pane_count > 1 {
                        let restored_tree = crate::session::restore_split_tree(
                            &tab_spec.splits,
                            tab_spec.active_pane,
                        );
                        *session.split_tree_mut() = restored_tree;
                    }

                    if !tab_spec.title.is_empty() {
                        session.set_title(tab_spec.title.clone());
                    }

                    new_sessions.push(session);
                }
                Err(e) => {
                    log::error!("Failed to restore tab '{}': {e}", tab_spec.title);
                }
            }
        }

        if !new_sessions.is_empty() {
            self.sessions = new_sessions;
            self.active = plan.active_tab.min(self.sessions.len() - 1);
            log::info!(
                "Restored {} tab(s), active={}",
                self.sessions.len(),
                self.active
            );
        }
    }

    // ── P22-E: Drag & drop file support ──

    /// Handle a file dropped onto the terminal window.
    ///
    /// Converts the file path to a quoted string and writes it to the
    /// active pane's PTY.
    pub(super) fn handle_dropped_file(&mut self, path: std::path::PathBuf) {
        let path_str = path.to_string_lossy();
        let quoted = quote_shell_path(&path_str);
        let bytes = format!("{quoted}\n").into_bytes();
        log::info!("Dropped file: {} → writing to PTY", path_str);
        self.active_session_mut().write_to_pty(&bytes);
    }

    /// Write encoded keyboard bytes to the active PTY.
    ///
    /// When broadcast mode is active (P25-D), bytes are also written to all
    /// panes in the current tab (AllPanes) or all tabs (AllTabs).
    pub(super) fn write_to_pty(&mut self, bytes: &[u8]) {
        use crate::broadcast_input::BroadcastMode;

        match self.broadcast.mode {
            BroadcastMode::None => {
                self.active_session_mut().write_to_pty(bytes);
            }
            BroadcastMode::AllPanes => {
                // P25-D: Write to all panes in the active tab.
                self.active_session_mut().write_to_all_panes(bytes);
            }
            BroadcastMode::AllTabs => {
                // P25-D: Write to all tabs' active panes.
                for session in self.sessions.iter_mut() {
                    session.write_to_pty(bytes);
                }
            }
        }

        // P25-E: Feed bytes to recorder if active.
        if let Some(ref mut recorder) = self.recorder {
            let _ = recorder.feed(bytes);
        }
    }

    // ── P19-B: Split pane management ──

    /// Split the active pane horizontally (left | right).
    ///
    /// Creates a new PTY + App for the new pane.
    pub(super) fn split_pane_horizontal(&mut self) {
        let cols = self.config.cols;
        let rows = self.config.rows;
        let shell = self.shell().to_string();
        match self
            .active_session_mut()
            .split_horizontal(cols, rows, &shell)
        {
            Ok(id) => log::info!("Horizontal split → new pane {id}"),
            Err(e) => log::error!("Failed to split horizontal: {e}"),
        }
    }

    /// Split the active pane vertically (top / bottom).
    ///
    /// Creates a new PTY + App for the new pane.
    pub(super) fn split_pane_vertical(&mut self) {
        let cols = self.config.cols;
        let rows = self.config.rows;
        let shell = self.shell().to_string();
        match self.active_session_mut().split_vertical(cols, rows, &shell) {
            Ok(id) => log::info!("Vertical split → new pane {id}"),
            Err(e) => log::error!("Failed to split vertical: {e}"),
        }
    }

    #[cfg(feature = "ai")]
    pub(super) fn trigger_ai_request(&mut self, action: ggterm_ai::Action) {
        // Show overlay immediately.
        self.ai_overlay.start_request(action);

        // Build context from terminal.
        let ctx = ggterm_ai::AIContext::from_terminal(self.active_session().app().terminal());
        let req = crate::ai_bridge::AIRequest::new(action, ctx);

        if let Some(ref mut bridge) = self.ai_bridge {
            if !bridge.request(req) {
                self.ai_overlay.set_error("AI is busy, please wait...");
            }
        } else {
            self.ai_overlay
                .set_error("AI not configured (set ai.api_endpoint in config)");
        }
    }

    /// Poll the AIBridge for a completed result and update the overlay.
    #[cfg(feature = "ai")]
    pub(super) fn poll_ai_bridge(&mut self) {
        if let Some(ref mut bridge) = self.ai_bridge
            && let Some(response) = bridge.poll_result()
        {
            match response.result {
                Ok(text) => self.ai_overlay.set_response(text),
                Err(e) => self.ai_overlay.set_error(e),
            }
        }
    }

    // ── Mouse handling (P9-D, pending integration) ────────────────

    /// Copy the current text selection to the clipboard.
    ///
    /// Extracts text from the grid between selection start and end.
    pub(super) fn copy_selection_to_clipboard(&self) {
        let Some(((sx, sy), (ex, ey))) = self.selection.normalized() else {
            return;
        };

        let grid = self.active_session().app().grid();
        let mut text = String::new();

        if sy == ey {
            // Single-line selection.
            for x in sx..=ex {
                if let Some(cell) = grid.cell(x as usize, sy as usize) {
                    text.push(cell.ch);
                }
            }
        } else {
            // Multi-line selection.
            // First line: from sx to end of row.
            let width = grid.width();
            for x in sx..width as u16 {
                if let Some(cell) = grid.cell(x as usize, sy as usize) {
                    text.push(cell.ch);
                }
            }
            text.push('\n');
            // Middle lines: full rows.
            for y in (sy + 1)..ey {
                for x in 0..width as u16 {
                    if let Some(cell) = grid.cell(x as usize, y as usize) {
                        text.push(cell.ch);
                    }
                }
                text.push('\n');
            }
            // Last line: from start of row to ex.
            for x in 0..=ex {
                if let Some(cell) = grid.cell(x as usize, ey as usize) {
                    text.push(cell.ch);
                }
            }
        }

        // Trim trailing whitespace per line.
        let text = text
            .lines()
            .map(|l| l.trim_end())
            .collect::<Vec<_>>()
            .join("\n");

        if !text.is_empty() {
            log::debug!("Clipboard copy: {} chars", text.len());
            crate::clipboard::set_clipboard_bytes(text.as_bytes());
        }
    }

    /// Paste text from the system clipboard into the PTY.
    ///
    /// Reads from the clipboard via `pbpaste` (macOS) or platform equivalent.
    /// If bracketed paste mode is active, wraps the text in escape markers.
    pub(super) fn paste_from_clipboard(&mut self) {
        let Some(text) = crate::clipboard::read_clipboard() else {
            log::debug!("Paste: clipboard empty or unavailable");
            return;
        };
        if text.is_empty() {
            return;
        }

        let bracketed = self.active_session().app().terminal().bracketed_paste();
        let bytes = crate::clipboard::bracket_paste(&text, bracketed);
        log::debug!("Paste: {} bytes (bracketed={})", bytes.len(), bracketed);
        self.write_to_pty(&bytes);
    }

    /// Poll for pending OSC 52 clipboard set operations.
    ///
    /// Called from `about_to_wait` to apply any OSC 52 clipboard changes
    /// that programs have requested.
    pub(super) fn poll_osc52_clipboard(&mut self) {
        if let Some(data) = self
            .active_session_mut()
            .app_mut()
            .terminal_mut()
            .take_pending_clipboard_set()
        {
            log::debug!("OSC 52 clipboard set: {} bytes", data.len());
            crate::clipboard::set_clipboard_bytes(&data);
        }
    }

    /// Poll for bell events from the terminal and trigger visual + audio bell (P11-E, P28-G).
    pub(super) fn poll_bell(&mut self) {
        if self
            .active_session_mut()
            .app_mut()
            .terminal_mut()
            .take_bell()
        {
            self.visual_bell_frames = VISUAL_BELL_DURATION_FRAMES;
            // P28-G: Play bell sound if enabled and rate limiter allows.
            if self.bell_limiter.check() {
                self.sound_player.play(crate::sound::SoundType::Bell);
            }
            log::debug!("Bell triggered");
        }
    }

    /// Poll for desktop notifications from OSC 9/777 (P24-E).
    ///
    /// On macOS, uses `osascript` to display a notification.
    /// On other platforms, logs the notification.
    pub(super) fn poll_notification(&mut self) {
        if let Some((title, body)) = self
            .active_session_mut()
            .app_mut()
            .terminal_mut()
            .take_pending_notification()
        {
            log::info!("Desktop notification: {} — {}", title, body);
            // macOS: use osascript for notifications
            #[cfg(target_os = "macos")]
            {
                let escaped_title = title.replace('"', "\\\"");
                let escaped_body = body.replace('"', "\\\"");
                let script = format!(
                    "display notification \"{}\" with title \"{}\"",
                    escaped_body, escaped_title
                );
                std::process::Command::new("osascript")
                    .args(["-e", &script])
                    .spawn()
                    .ok();
            }
        }
    }

    // ── P28-C: Command history sync ──────────────────────────────

    /// Sync OSC 133 command blocks into the command history sidebar.
    /// Called from about_to_wait to keep the sidebar up to date.
    pub(super) fn poll_command_history(&mut self) {
        // Only sync if the sidebar is visible to avoid overhead.
        if !self.cmd_history.visible {
            return;
        }

        // Extract all data from immutable self first, then mutate cmd_history.
        let blocks = self.active_session().app().terminal().command_blocks();

        // Quick check: if the block count hasn't changed, skip.
        let current_len = self.cmd_history.len();
        if blocks.len() == current_len {
            return;
        }

        // Build entries from blocks (borrowing grid immutably).
        let grid = self.active_session().app().grid();
        let scrollback = grid.scrollback_len();
        let grid_h = grid.height();
        let entries: Vec<(String, usize, Option<i32>)> = blocks
            .iter()
            .map(|block| {
                let cmd_row = block.command_row.unwrap_or(block.prompt_row);
                let cmd_text = if cmd_row < scrollback + grid_h {
                    let display_row = cmd_row.saturating_sub(scrollback);
                    grid.display_row(display_row)
                        .map(|row| {
                            let s: String = row.cells.iter().map(|c| c.ch).collect::<String>();
                            s.trim().to_string()
                        })
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                let text = if cmd_text.is_empty() {
                    "(unknown)".to_string()
                } else {
                    cmd_text
                };
                (text, block.prompt_row, block.exit_code)
            })
            .collect();

        // Now mutate cmd_history (no immutable borrows outstanding).
        self.cmd_history.clear();
        for (text, row, exit_code) in entries {
            self.cmd_history.add(text, row);
            if let Some(code) = exit_code {
                self.cmd_history.complete_last(code);
            }
        }
    }

    // ── Font zoom (P11-A) ─────────────────────────────────────────

    /// Apply the current font zoom level to the renderer (P11-A).
    ///
    /// Calls `set_font_size()` on the GlyphonRenderer, which recomputes
    /// cell metrics. The actual cell dimension change triggers a resize
    /// on the next `about_to_wait` cycle.
    pub(super) fn apply_font_size(&mut self) {
        let size = self.font_zoom.current_size();
        if let Some(ref mut renderer) = self.renderer {
            renderer.set_font_size(size);
            log::info!("Font size: {size:.1}px");
        }
    }

    // ── Window controls (P11-C) ───────────────────────────────────

    /// Apply the active theme from the App's ThemeManager to the GPU renderer (P11-D).
    pub(super) fn apply_theme_to_renderer(&mut self) {
        // Clone the theme first to avoid borrow conflict between
        // active_session() and renderer.
        let theme = self
            .active_session_mut()
            .app_mut()
            .theme_manager()
            .current()
            .clone();
        if let Some(ref mut renderer) = self.renderer {
            renderer.set_theme(theme);
            log::debug!("Theme applied to renderer");
        }
    }

    /// Cycle through available themes (P11-D).
    pub(super) fn cycle_theme(&mut self) {
        let name = {
            let mgr = self.active_session_mut().app_mut().theme_manager();
            mgr.cycle_next();
            mgr.current_name().to_owned()
        };
        self.apply_theme_to_renderer();
        log::info!("Theme: {name}");
    }

    /// Toggle fullscreen mode.
    pub(super) fn toggle_fullscreen(&mut self) {
        if let Some(ref window) = self.window {
            self.fullscreen = !self.fullscreen;
            if self.fullscreen {
                window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
            } else {
                window.set_fullscreen(None);
            }
            log::info!("Fullscreen: {}", self.fullscreen);
        }
    }

    /// Toggle window maximized state.
    pub(super) fn toggle_maximized(&mut self) {
        if let Some(ref window) = self.window {
            self.maximized = !self.maximized;
            window.set_maximized(self.maximized);
            log::info!("Maximized: {}", self.maximized);
        }
    }
}
