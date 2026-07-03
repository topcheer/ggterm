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

            // Immediately persist the updated layout so it survives crashes.
            self.save_session_on_exit();

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
            crate::settings_ui::SettingsField::Theme => self.settings.cycle_theme_prev(),
            crate::settings_ui::SettingsField::FontSize => self.settings.font_size_down(),
            crate::settings_ui::SettingsField::CursorStyle => {
                self.settings.cycle_cursor_style_prev()
            }
            crate::settings_ui::SettingsField::Scrollback => self.settings.scrollback_down(),
            crate::settings_ui::SettingsField::AiEnabled => self.settings.toggle_ai(),
            crate::settings_ui::SettingsField::RestoreSession => {
                self.settings.toggle_restore_session()
            }
            _ => {}
        }
        self.apply_settings_live();
    }

    /// Handle Right arrow in settings (increase/cycle forward).
    pub(super) fn handle_settings_right(&mut self) {
        match self.settings.selected {
            crate::settings_ui::SettingsField::Theme => self.settings.cycle_theme(),
            crate::settings_ui::SettingsField::FontSize => self.settings.font_size_up(),
            crate::settings_ui::SettingsField::CursorStyle => self.settings.cycle_cursor_style(),
            crate::settings_ui::SettingsField::Scrollback => self.settings.scrollback_up(),
            crate::settings_ui::SettingsField::AiEnabled => self.settings.toggle_ai(),
            crate::settings_ui::SettingsField::RestoreSession => {
                self.settings.toggle_restore_session()
            }
            _ => {}
        }
        self.apply_settings_live();
    }

    /// Load current config values into the settings state before opening.
    pub(super) fn load_settings_from_config(&mut self) {
        if let Some(ref mgr) = self.config_mgr {
            let cfg = mgr.config();
            self.settings
                .load_from_config(&crate::settings_ui::SettingsSnapshot {
                    theme: cfg.appearance.theme.clone(),
                    font_size: cfg.appearance.font_size,
                    font_family: cfg.appearance.font_family.clone(),
                    cursor_style: cfg.appearance.cursor_style.clone(),
                    scrollback_lines: cfg.terminal.scrollback_lines,
                    shell: cfg.terminal.shell.clone(),
                    restore_session: cfg.terminal.restore_session,
                    ai_enabled: cfg.ai.enabled,
                    ai_endpoint: cfg.ai.api_endpoint.clone(),
                    ai_model: cfg.ai.model.clone(),
                });
        }
    }

    /// Apply appearance changes immediately for live visual feedback.
    fn apply_settings_live(&mut self) {
        // Theme
        let theme = self.settings.theme.clone();
        self.active_session_mut()
            .app_mut()
            .theme_manager()
            .set_by_name(&theme);
        self.apply_theme_to_renderer();
        self.last_applied_theme = theme;

        // Font size
        let font_size = self.settings.font_size as f32;
        if let Some(ref mut renderer) = self.renderer {
            renderer.set_font_size(font_size);
        }
        self.last_applied_font_size = font_size;
    }

    /// Apply all settings to ConfigManager and save to disk.
    /// Called when settings overlay is closed.
    pub(super) fn apply_settings_on_close(&mut self) {
        if !self.settings.dirty {
            return;
        }

        if let Some(ref mut mgr) = self.config_mgr {
            let cfg = mgr.config_mut();
            cfg.appearance.theme = self.settings.theme.clone();
            cfg.appearance.font_size = self.settings.font_size;
            cfg.appearance.font_family = self.settings.font_family.clone();
            cfg.appearance.cursor_style = self.settings.cursor_style.clone();
            cfg.terminal.scrollback_lines = self.settings.scrollback_lines;
            cfg.terminal.shell = self.settings.shell.clone();
            cfg.terminal.restore_session = self.settings.restore_session;
            cfg.ai.enabled = self.settings.ai_enabled;
            cfg.ai.api_endpoint = self.settings.ai_endpoint.clone();
            cfg.ai.model = self.settings.ai_model.clone();

            // Save to disk.
            if let Err(e) = mgr.save() {
                log::error!("Failed to save config: {e}");
                self.settings.set_error(format!("Save failed: {e}"));
            } else {
                self.show_toast("Settings saved");
            }
        }

        // Apply scrollback to all sessions.
        let scrollback = self.settings.scrollback_lines;
        for session in &mut self.sessions {
            let pane_ids: Vec<usize> = session.pane_ids();
            for pane_id in pane_ids {
                if let Some(app) = session.pane_app_mut(pane_id) {
                    app.terminal_mut().grid_mut().set_scrollback(scrollback);
                }
            }
        }

        self.settings.dirty = false;
    }

    // ── Tab management (P10-A) ──

    /// Open a new tab: create a TabSession with a fresh PTY.
    pub(super) fn open_tab(&mut self) {
        let cols = self.config.cols;
        let rows = self.config.rows;
        // P31: Inherit cwd from active tab (OSC 7 tracking).
        let cwd = self.active_session().cwd().map(|p| p.to_path_buf());
        match TabSession::new_with_cwd(cols, rows, self.shell(), cwd.as_deref()) {
            Ok(session) => {
                self.sessions.push(session);
                self.active = self.sessions.len() - 1;
                self.selection.clear();
                log::info!("Opened tab {}", self.active + 1);
            }
            Err(e) => {
                log::error!("Failed to open tab: {e}");
            }
        }
    }

    /// Execute an action from the "+" dropdown menu.
    pub(super) fn execute_new_tab_menu_action(&mut self, index: usize) {
        use crate::new_tab_menu::NewTabMenuAction;
        match NewTabMenuAction::all().get(index).copied() {
            Some(NewTabMenuAction::NewTab) => self.open_tab(),
            Some(NewTabMenuAction::SplitHorizontal) => self.split_pane_horizontal(),
            Some(NewTabMenuAction::SplitVertical) => self.split_pane_vertical(),
            None => {}
        }
        if let Some(ref window) = self.window {
            window.request_redraw();
        }
    }

    /// Close the active tab (keep at least 1).
    pub(super) fn close_tab(&mut self) {
        if self.sessions.len() <= 1 {
            return;
        }
        // Save the cwd of the active pane for "reopen closed tab".
        self.last_closed_cwd = self.sessions[self.active].cwd().map(|p| p.to_path_buf());
        self.sessions.remove(self.active);
        if self.active >= self.sessions.len() {
            self.active = self.sessions.len() - 1;
        }
        log::info!("Closed tab, active={}", self.active + 1);
        self.save_session_on_exit();
    }

    /// Reopen the last closed tab in its original working directory.
    pub(super) fn reopen_closed_tab(&mut self) {
        if let Some(cwd) = self.last_closed_cwd.take() {
            match TabSession::new_with_cwd(
                self.config.cols,
                self.config.rows,
                self.shell(),
                Some(&cwd),
            ) {
                Ok(session) => {
                    self.sessions.push(session);
                    self.active = self.sessions.len() - 1;
                    self.selection.clear();
                    log::info!("Reopened closed tab in {:?}", cwd);
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                }
                Err(e) => {
                    log::error!("Failed to reopen tab: {e}");
                    self.show_toast("Failed to reopen tab");
                }
            }
        } else {
            self.show_toast("No recently closed tabs");
        }
    }

    /// Switch to a specific tab by index (0-based).
    pub(super) fn switch_tab(&mut self, index: usize) {
        if index < self.sessions.len() && index != self.active {
            self.selection.clear();
            self.selection_auto_scroll = 0;
            self.active = index;
            self.sessions[self.active].clear_unread();
        }
    }

    /// Switch to the next tab (wraps).
    pub(super) fn next_tab(&mut self) {
        self.selection.clear();
        self.selection_auto_scroll = 0;
        self.active = (self.active + 1) % self.sessions.len();
        self.sessions[self.active].clear_unread();
    }

    /// Switch to the previous tab (wraps).
    pub(super) fn prev_tab(&mut self) {
        self.selection.clear();
        self.selection_auto_scroll = 0;
        self.active = if self.active == 0 {
            self.sessions.len() - 1
        } else {
            self.active - 1
        };
        self.sessions[self.active].clear_unread();
    }

    // ── P23-E: Tab reordering ──

    /// Move a tab from one position to another (drag reordering).
    pub(super) fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.sessions.len() || to >= self.sessions.len() || from == to {
            return;
        }
        let session = self.sessions.remove(from);
        self.sessions.insert(to, session);
        self.active = to;
        log::info!("P23-E: moved tab {} → {}", from + 1, to + 1);
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
        // P31: Capture current window geometry for persistence.
        let (win_x, win_y, win_w, win_h) = if let Some(ref window) = self.window {
            let outer = window.outer_position();
            let inner = window.inner_size();
            let scale = window.scale_factor();
            // Convert physical → logical for persistence.
            let logical_w = (inner.width as f64 / scale) as u32;
            let logical_h = (inner.height as f64 / scale) as u32;
            (
                outer.as_ref().ok().map(|p| p.x),
                outer.as_ref().ok().map(|p| p.y),
                Some(logical_w),
                Some(logical_h),
            )
        } else {
            (None, None, None, None)
        };

        crate::session::SessionData {
            version: 1,
            tabs,
            active_tab: self.active,
            window_x: win_x,
            window_y: win_y,
            window_width: win_w,
            window_height: win_h,
        }
    }

    /// Save session to disk on exit.
    pub(super) fn save_session_on_exit(&mut self) {
        let restore = self
            .config_mgr
            .as_ref()
            .map(|m| m.config().terminal.restore_session)
            .unwrap_or(false);
        if !restore {
            let _ = crate::session::clear_session();
            return;
        }
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

        // Auto-scroll to bottom on user input (standard terminal UX).
        {
            let grid = self
                .active_session_mut()
                .app_mut()
                .terminal_mut()
                .grid_mut();
            if grid.is_scrolled() {
                grid.reset_viewport();
                self.smooth_scroll.reset();
            }
        }

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
        // P31: Inherit cwd from active pane.
        let cwd = self.active_session().cwd().map(|p| p.to_path_buf());
        match self.active_session_mut().split_horizontal_with_cwd(
            cols,
            rows,
            &shell,
            cwd.as_deref(),
        ) {
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
        // P31: Inherit cwd from active pane.
        let cwd = self.active_session().cwd().map(|p| p.to_path_buf());
        match self
            .active_session_mut()
            .split_vertical_with_cwd(cols, rows, &shell, cwd.as_deref())
        {
            Ok(id) => log::info!("Vertical split → new pane {id}"),
            Err(e) => log::error!("Failed to split vertical: {e}"),
        }
    }

    /// Toggle pane zoom mode (tmux-style zoom).
    ///
    /// When zoomed, only the active pane is rendered at full window size.
    /// Toggling again restores the previous split layout.
    pub(super) fn toggle_pane_zoom(&mut self) {
        let has_splits = !self.active_session().split_tree().is_single();
        if !has_splits && !self.pane_zoomed {
            self.show_toast("No splits to zoom");
            return;
        }
        self.pane_zoomed = !self.pane_zoomed;
        if self.pane_zoomed {
            self.show_toast("Pane zoomed (Ctrl+Shift+Z to restore)");
        } else {
            self.show_toast("Pane zoom restored");
        }
        log::info!("Pane zoom: {}", self.pane_zoomed);
    }

    /// Open URL at the current cursor position (or hovered link if any).
    ///
    /// Checks OSC 8 hyperlinks first, then plain-text URL detection.
    pub(super) fn open_url_at_cursor(&mut self) {
        // If a link is currently hovered, use that.
        if let Some(ref link) = self.hovered_link {
            let url = link.0.clone();
            crate::mouse::open_url(&url);
            self.show_toast(format!("Opened: {}", &url[..url.len().min(60)]));
            return;
        }

        // Otherwise check cursor position.
        let (row, col) = self.active_session().app().cursor();

        let grid = self.active_session().app().grid();
        let row_data = match grid.display_row(row) {
            Some(r) => r,
            None => return,
        };

        // Check OSC 8 hyperlink on cursor cell.
        if col < row_data.cells.len()
            && let Some(ref link) = row_data.cells[col].hyperlink
        {
            crate::mouse::open_url(link);
            self.show_toast(format!("Opened: {}", &link[..link.len().min(60)]));
            return;
        }

        // Plain-text URL detection.
        let line: String = row_data.cells.iter().map(|c| c.ch).collect();
        if let Some((_, _, url)) = crate::mouse::detect_url_at_position(&line, col) {
            crate::mouse::open_url(&url);
            self.show_toast(format!("Opened: {}", &url[..url.len().min(60)]));
        } else {
            self.show_toast("No URL at cursor");
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
    pub(super) fn copy_selection_to_clipboard(&mut self) {
        let Some(((sx, sy), (ex, ey))) = self.selection.normalized() else {
            return;
        };

        // Helper: get cell text including combining characters.
        // Uses display_cell to correctly handle scrollback scroll position.
        let cell_text = |x: u16, y: u16, grid: &ggterm_core::Grid| -> String {
            if let Some(cell) = grid.display_cell(x as usize, y as usize) {
                let mut s = String::new();
                s.push(cell.ch);
                for &c in &cell.combining {
                    s.push(c);
                }
                s
            } else {
                String::new()
            }
        };

        let grid = self.active_session().app().grid();
        let mut text = String::new();

        if sy == ey {
            // Single-line selection.
            for x in sx..=ex {
                text.push_str(&cell_text(x, sy, grid));
            }
        } else {
            // Multi-line selection.
            // First line: from sx to end of row.
            let width = grid.width();
            for x in sx..width as u16 {
                text.push_str(&cell_text(x, sy, grid));
            }
            text.push('\n');
            // Middle lines: full rows.
            for y in (sy + 1)..ey {
                for x in 0..width as u16 {
                    text.push_str(&cell_text(x, y, grid));
                }
                text.push('\n');
            }
            // Last line: from start of row to ex.
            for x in 0..=ex {
                text.push_str(&cell_text(x, ey, grid));
            }
        }

        // Trim trailing whitespace per line and remove trailing empty lines.
        let mut lines: Vec<&str> = text.lines().map(|l| l.trim_end()).collect();
        // Remove trailing empty lines.
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        // Remove leading empty lines.
        while lines.first().is_some_and(|l| l.is_empty()) {
            lines.remove(0);
        }
        let text = lines.join("\n");

        if !text.is_empty() {
            log::debug!("Clipboard copy: {} chars", text.len());
            crate::clipboard::set_clipboard_bytes(text.as_bytes());
            // P30-C: Show toast feedback.
            self.show_toast(format!("Copied {} chars", text.len()));
        }
    }

    /// P30-C: Show a toast notification that fades after ~2 seconds.
    pub(super) fn show_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), 120)); // ~2s at 60fps
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
        let active = self.active;
        let mut any_bell = false;
        for (i, session) in self.sessions.iter_mut().enumerate() {
            if session.app_mut().terminal_mut().take_bell() {
                if i == active {
                    // Active tab: visual bell + sound.
                    any_bell = true;
                } else {
                    // Non-active tab: mark as unread (blue dot).
                    session.mark_unread();
                }
            }
        }
        if any_bell {
            self.visual_bell_frames = VISUAL_BELL_DURATION_FRAMES;
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
        self.show_toast(format!("Theme: {name}"));
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

    // ── P30-A: Scrollbar scroll-to-position ────────────────────────

    // ── P31: Profile cycling ───────────────────────────────────────

    /// Cycle to the next config profile (if profiles are defined).
    pub(super) fn cycle_profile(&mut self) {
        let Some(mgr) = self.config_mgr.as_mut() else {
            self.show_toast("No config loaded");
            return;
        };
        let current = self.status_bar.active_profile().unwrap_or("").to_string();
        let next = mgr.config_mut().cycle_profile(&current);
        match next {
            Some(next) => {
                log::info!("Switching profile: {} → {}", current, next);
                if let Err(e) = mgr.config_mut().apply_profile(&next) {
                    log::error!("Failed to apply profile '{next}': {e}");
                    return;
                }
                self.status_bar.set_profile(&next);
                self.show_toast(format!("Profile: {}", next));
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            None => {
                self.show_toast("No profiles configured");
            }
        }
    }

    /// Export current config to clipboard as TOML.
    pub(super) fn export_config(&mut self) {
        let Some(mgr) = self.config_mgr.as_ref() else {
            self.show_toast("No config loaded");
            return;
        };
        match mgr.config().export_to_toml() {
            Ok(toml) => {
                crate::clipboard::set_clipboard_bytes(toml.as_bytes());
                self.show_toast(format!("Config exported ({} bytes)", toml.len()));
            }
            Err(e) => {
                log::error!("Config export failed: {e}");
                self.show_toast("Export failed");
            }
        }
    }

    /// Import configuration from clipboard contents (Ctrl+Shift+Alt+I).
    pub(super) fn import_config(&mut self) {
        let Some(clipboard) = crate::clipboard::read_clipboard() else {
            self.show_toast("Clipboard is empty");
            return;
        };
        match crate::config::Config::import_from_toml(&clipboard) {
            Ok(new_config) => {
                let theme_name = new_config.appearance.theme.clone();
                let font_size = new_config.appearance.font_size;
                let mut mgr = crate::config::ConfigManager::new();
                *mgr.config_mut() = new_config;
                self.config_mgr = Some(mgr);
                // Save to disk.
                if let Some(ref mut mgr) = self.config_mgr {
                    let _ = mgr.save();
                }
                // Apply theme via theme manager.
                self.active_session_mut()
                    .app_mut()
                    .theme_manager()
                    .set_by_name(&theme_name);
                self.apply_theme_to_renderer();
                // Apply font size.
                if let Some(ref mut renderer) = self.renderer {
                    renderer.set_font_size(font_size as f32);
                }
                self.last_applied_theme = theme_name;
                self.last_applied_font_size = font_size as f32;
                self.show_toast(format!("Config imported ({} bytes)", clipboard.len()));
            }
            Err(e) => {
                log::error!("Config import failed: {e}");
                self.show_toast("Import failed: invalid TOML");
            }
        }
    }

    /// Reset configuration to defaults (Ctrl+Shift+Alt+R).
    pub(super) fn reset_config(&mut self) {
        let default_config = crate::config::Config::reset_to_defaults();
        let theme_name = default_config.appearance.theme.clone();
        let font_size = default_config.appearance.font_size;
        let mut mgr = crate::config::ConfigManager::new();
        *mgr.config_mut() = default_config;
        self.config_mgr = Some(mgr);
        // Save to disk.
        if let Some(ref mut mgr) = self.config_mgr {
            let _ = mgr.save();
        }
        // Apply theme.
        self.active_session_mut()
            .app_mut()
            .theme_manager()
            .set_by_name(&theme_name);
        self.apply_theme_to_renderer();
        // Apply font size.
        if let Some(ref mut renderer) = self.renderer {
            renderer.set_font_size(font_size as f32);
        }
        self.last_applied_theme = theme_name;
        self.last_applied_font_size = font_size as f32;
        self.show_toast("Config reset to defaults");
    }

    /// Reset window layout to a single pane (Ctrl+Shift+Alt+N).
    /// Clears session persistence so next startup is clean.
    pub(super) fn reset_layout(&mut self) {
        // Clear session file so next launch starts fresh.
        let _ = crate::session::clear_session();

        // Close all tabs except the first, remove all splits from it.
        while self.sessions.len() > 1 {
            self.sessions.pop();
        }
        self.active = 0;

        // Collapse split tree to single pane.
        if self.sessions[0].pane_count() > 1 {
            let shell = self.shell().to_string();
            match crate::tab_session::TabSession::new_with_cwd(
                self.config.cols,
                self.config.rows,
                &shell,
                None,
            ) {
                Ok(new_session) => {
                    self.sessions[0] = new_session;
                }
                Err(e) => {
                    log::error!("Failed to create new session: {e}");
                }
            }
        }

        if let Some(ref window) = self.window {
            window.request_redraw();
        }
        self.show_toast("Layout reset to single pane");
    }

    // ── P30-A: Scrollbar scroll-to-position ────────────────────────

    /// Scroll the active session's grid so the scrollbar thumb aligns with
    /// the given pixel Y position.
    pub(super) fn scroll_to_scrollbar_pos(&mut self, py: f32) {
        let bounds = self.content_area_bounds();
        let track_y = bounds.y as f32;
        let track_h = bounds.height as f32;

        // Clamp py within track bounds.
        let clamped = py.clamp(track_y, track_y + track_h);

        // Normalized position: 0.0 = top (oldest), 1.0 = bottom (newest).
        let norm = if track_h > 0.0 {
            (clamped - track_y) / track_h
        } else {
            1.0
        };

        // Get scroll state.
        let (scrollback_len, _height, current_offset) = {
            let grid = self.active_session().app().grid();
            (grid.scrollback_len(), grid.height(), grid.display_offset())
        };
        if scrollback_len == 0 {
            return;
        }

        // Target display_offset: 0 = bottom (newest), scrollback_len = top (oldest).
        let target_offset = ((1.0 - norm) * scrollback_len as f32).round() as usize;
        let target_offset = target_offset.min(scrollback_len);

        // Apply delta.
        let delta = target_offset as i64 - current_offset as i64;
        let grid = self
            .active_session_mut()
            .app_mut()
            .terminal_mut()
            .grid_mut();
        if delta > 0 {
            grid.scroll_up_viewport(delta as usize);
        } else if delta < 0 {
            grid.scroll_down_viewport((-delta) as usize);
        }
    }

    // ── P28: Command palette action dispatch ──────────────────────

    /// Execute an action by its command palette ID.
    pub(super) fn execute_command_palette_action(&mut self, id: &str) {
        match id {
            "perf.toggle" => {
                self.perf_monitor.toggle();
            }
            "sound.toggle" => {
                self.sound_player.toggle();
            }
            "shell.switch" => {
                self.shell_switcher.toggle();
            }
            "workspace.next" => {
                self.workspaces.cycle_next();
                self.animations.tab_switch();
            }
            "workspace.prev" => {
                self.workspaces.cycle_prev();
            }
            "workspace.add" => {
                let name = format!("ws-{}", self.workspaces.len());
                self.workspaces.add_workspace(&name);
                self.workspaces.set_active(&name);
            }
            "cursor.trail" => {
                self.cursor_particles
                    .set_effect(crate::perf_monitor::CursorEffect::Trail);
            }
            "cursor.glow" => {
                self.cursor_particles
                    .set_effect(crate::perf_monitor::CursorEffect::Glow);
            }
            "cursor.none" => {
                self.cursor_particles
                    .set_effect(crate::perf_monitor::CursorEffect::None);
            }
            "tab.new" => {
                self.open_tab();
            }
            "tab.close" => {
                self.close_tab();
            }
            "tab.next" => {
                self.next_tab();
            }
            "tab.move_left" => {
                if self.active > 0 {
                    self.move_tab(self.active, self.active - 1);
                }
            }
            "tab.move_right" => {
                if self.active < self.sessions.len() - 1 {
                    self.move_tab(self.active, self.active + 1);
                }
            }
            "split.zoom" => {
                self.toggle_pane_zoom();
            }
            "terminal.open_url" => {
                self.open_url_at_cursor();
            }
            "terminal.copy" => {
                self.copy_selection_to_clipboard();
            }
            "terminal.clear" => {
                // Clear visible screen by sending Ctrl+L equivalent.
                self.write_to_pty(b"\x1b[H\x1b[2J");
            }
            "terminal.reset" => {
                self.active_session_mut().app_mut().terminal_mut().ris();
            }
            _ => {
                log::debug!("Unhandled command palette action: {}", id);
            }
        }
    }

    /// Execute a tab context menu action.
    pub(super) fn execute_tab_menu_action(&mut self, action: crate::tab_bar::TabMenuAction) {
        match action {
            crate::tab_bar::TabMenuAction::NewTab => {
                self.open_tab();
            }
            crate::tab_bar::TabMenuAction::CloseTab => {
                if let Some(idx) = self.tab_context_menu.tab_index
                    && idx < self.sessions.len()
                {
                    self.switch_tab(idx);
                    self.close_tab();
                }
            }
            crate::tab_bar::TabMenuAction::DuplicateTab => {
                // Open a new tab (same shell as current).
                self.open_tab();
            }
            crate::tab_bar::TabMenuAction::RenameTab => {
                if let Some(idx) = self.tab_context_menu.tab_index
                    && idx < self.sessions.len()
                {
                    self.renaming_tab = Some(idx);
                    self.rename_text = self.sessions[idx].title().to_owned();
                }
            }
            crate::tab_bar::TabMenuAction::NextTab => {
                self.next_tab();
            }
            crate::tab_bar::TabMenuAction::PrevTab => {
                self.prev_tab();
            }
        }
    }

    /// Open the config file in the system's default editor (Ctrl+Shift+,).
    /// Creates a default config if one doesn't exist.
    pub(super) fn open_config_file(&mut self) {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
        let Some(home) = home else {
            self.show_toast("Cannot find home directory");
            return;
        };
        let dir = std::path::PathBuf::from(home).join(".ggterm");
        let path = dir.join("config.toml");

        // Create config directory + default config if missing.
        if !path.exists() {
            let _ = std::fs::create_dir_all(&dir);
            let default_toml = self
                .config_mgr
                .as_ref()
                .and_then(|m| m.config().export_to_toml().ok())
                .unwrap_or_default();
            if std::fs::write(&path, &default_toml).is_err() {
                self.show_toast("Failed to create config file");
                return;
            }
        }

        // Open with system default editor.
        #[cfg(target_os = "macos")]
        let result = std::process::Command::new("open").arg(&path).spawn();
        #[cfg(target_os = "linux")]
        let result = std::process::Command::new("xdg-open").arg(&path).spawn();
        #[cfg(target_os = "windows")]
        let result = std::process::Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(&path)
            .spawn();

        match result {
            Ok(_) => self.show_toast(format!("Opened {}", path.display())),
            Err(_) => self.show_toast("Failed to open editor"),
        }
    }
}
