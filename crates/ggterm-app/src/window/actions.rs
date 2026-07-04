//! DesktopApp action methods — tab, split, clipboard, theme, session.

use super::*;

impl DesktopApp {
    /// Base64-encode bytes (for OSC 52 clipboard query response).
    fn base64_encode(data: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
        for chunk in data.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
            let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
            let triple = (b0 << 16) | (b1 << 8) | b2;
            result.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
            result.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
            if chunk.len() > 1 {
                result.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
            } else {
                result.push('=');
            }
            if chunk.len() > 2 {
                result.push(TABLE[(triple & 0x3F) as usize] as char);
            } else {
                result.push('=');
            }
        }
        result
    }

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
    #[allow(dead_code)]
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

    /// Open the independent settings window.
    /// If already open, focus it instead.
    pub fn open_settings_window(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.settings_window.is_some() {
            // Already open — just focus.
            if let Some(ref sw) = self.settings_window {
                sw.window.focus_window();
            }
            return;
        }

        // Build draft from current config.
        let draft = if let Some(ref mgr) = self.config_mgr {
            let cfg = mgr.config();
            crate::settings_window::SettingsDraft {
                theme: cfg.appearance.theme.clone(),
                font_size: cfg.appearance.font_size,
                font_family: cfg.appearance.font_family.clone(),
                cursor_style: cfg.appearance.cursor_style.clone(),
                scrollback_lines: cfg.terminal.scrollback_lines,
                shell: cfg.terminal.shell.clone(),
                restore_session: cfg.terminal.restore_session,
                ai_enabled: cfg.ai.enabled,
            }
        } else {
            crate::settings_window::SettingsDraft {
                theme: "dark".to_string(),
                font_size: 14,
                font_family: "monospace".to_string(),
                cursor_style: "block".to_string(),
                scrollback_lines: 10000,
                shell: String::new(),
                restore_session: false,
                ai_enabled: true,
            }
        };

        match crate::settings_window::SettingsWindowState::open(event_loop, draft) {
            Some(sw) => {
                self.settings_window = Some(sw);
                log::info!("Settings window opened");
            }
            None => {
                self.show_toast("Failed to open settings window");
            }
        }
    }

    /// Apply draft from settings window to config and terminal sessions.
    pub fn apply_settings_draft(&mut self, draft: &crate::settings_window::SettingsDraft) {
        let cursor_style_val = match draft.cursor_style.as_str() {
            "underline" => ggterm_core::CursorStyle::BlinkUnderline,
            "bar" => ggterm_core::CursorStyle::BlinkBar,
            _ => ggterm_core::CursorStyle::BlinkBlock,
        };

        // Apply theme + cursor to all sessions.
        for session in &mut self.sessions {
            for pane_id in session.pane_ids() {
                if let Some(app) = session.pane_app_mut(pane_id) {
                    app.theme_manager().set_by_name(&draft.theme);
                    app.terminal_mut().set_cursor_style(cursor_style_val);
                    app.terminal_mut()
                        .grid_mut()
                        .set_scrollback(draft.scrollback_lines);
                }
            }
        }

        // Apply font size.
        self.font_zoom.set_base_size(draft.font_size as f32);
        self.apply_font_size();

        // Apply theme to renderer.
        self.apply_theme_to_renderer();
        self.last_applied_theme = draft.theme.clone();

        // Save to config manager.
        if let Some(ref mut mgr) = self.config_mgr {
            let cfg = mgr.config_mut();
            cfg.appearance.theme = draft.theme.clone();
            cfg.appearance.font_size = draft.font_size;
            cfg.appearance.font_family = draft.font_family.clone();
            cfg.appearance.cursor_style = draft.cursor_style.clone();
            cfg.terminal.scrollback_lines = draft.scrollback_lines;
            cfg.terminal.shell = draft.shell.clone();
            cfg.terminal.restore_session = draft.restore_session;
            cfg.ai.enabled = draft.ai_enabled;
            let _ = mgr.save();
        }

        self.show_toast("Settings saved");
        log::info!("Settings applied from settings window");
    }

    /// Apply appearance changes immediately for live visual feedback.
    fn apply_settings_live(&mut self) {
        // Theme — apply to all sessions.
        let theme = self.settings.theme.clone();
        let cursor_style = match self.settings.cursor_style.as_str() {
            "underline" => ggterm_core::CursorStyle::BlinkUnderline,
            "bar" => ggterm_core::CursorStyle::BlinkBar,
            _ => ggterm_core::CursorStyle::BlinkBlock,
        };
        for session in &mut self.sessions {
            let pane_ids: Vec<usize> = session.pane_ids();
            for pane_id in pane_ids {
                if let Some(app) = session.pane_app_mut(pane_id) {
                    app.theme_manager().set_by_name(&theme);
                    app.terminal_mut().set_cursor_style(cursor_style);
                }
            }
        }
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

        // Apply scrollback + cursor style to all sessions.
        let scrollback = self.settings.scrollback_lines;
        let cursor_style = match self.settings.cursor_style.as_str() {
            "underline" => ggterm_core::CursorStyle::BlinkUnderline,
            "bar" => ggterm_core::CursorStyle::BlinkBar,
            _ => ggterm_core::CursorStyle::BlinkBlock,
        };
        for session in &mut self.sessions {
            let pane_ids: Vec<usize> = session.pane_ids();
            for pane_id in pane_ids {
                if let Some(app) = session.pane_app_mut(pane_id) {
                    app.terminal_mut().grid_mut().set_scrollback(scrollback);
                    app.terminal_mut().set_cursor_style(cursor_style);
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
            self.last_active_tab = Some(self.active);
            self.active = index;
            self.sessions[self.active].clear_unread();
        }
    }

    /// Toggle between the current tab and the last active tab.
    pub(super) fn toggle_last_tab(&mut self) {
        if let Some(last) = self.last_active_tab
            && last < self.sessions.len()
            && last != self.active
        {
            let current = self.active;
            self.active = last;
            self.last_active_tab = Some(current);
            self.selection.clear();
            self.selection_auto_scroll = 0;
            self.sessions[self.active].clear_unread();
        }
    }

    /// Duplicate the active tab: creates a new tab with the same shell and cwd.
    pub(super) fn duplicate_tab(&mut self) {
        let cwd = self.active_session().cwd().map(|p| p.to_path_buf());
        match TabSession::new_with_cwd(
            self.config.cols,
            self.config.rows,
            self.shell(),
            cwd.as_deref(),
        ) {
            Ok(session) => {
                self.sessions.push(session);
                self.active = self.sessions.len() - 1;
                self.selection.clear();
                log::info!("Duplicated tab {}", self.active);
                self.show_toast("Tab duplicated");
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            Err(e) => {
                log::error!("Failed to duplicate tab: {e}");
                self.show_toast("Failed to duplicate tab");
            }
        }
    }

    /// Close all tabs except the active one.
    pub(super) fn close_other_tabs(&mut self) {
        if self.sessions.len() <= 1 {
            return;
        }
        let active = self.active;
        self.sessions.swap_remove(active);
        self.sessions.truncate(1);
        self.active = 0;
        self.selection.clear();
        log::info!("Closed all other tabs");
        self.show_toast("Closed other tabs");
        self.save_session_on_exit();
        if let Some(ref window) = self.window {
            window.request_redraw();
        }
    }

    /// Switch to the next tab (wraps).
    pub(super) fn next_tab(&mut self) {
        self.selection.clear();
        self.selection_auto_scroll = 0;
        self.last_active_tab = Some(self.active);
        self.active = (self.active + 1) % self.sessions.len();
        self.sessions[self.active].clear_unread();
    }

    /// Switch to the previous tab (wraps).
    pub(super) fn prev_tab(&mut self) {
        self.selection.clear();
        self.selection_auto_scroll = 0;
        self.last_active_tab = Some(self.active);
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

    /// Send terminal mode reset sequences to all active PTYs before exit.
    ///
    /// This ensures the shell doesn't get stuck in modes like bracketed paste,
    /// mouse tracking, or application cursor keys after the terminal closes.
    /// Without this, the user's shell session may behave unexpectedly.
    pub(super) fn send_terminal_reset(&mut self) {
        // Reset sequences:
        // \x1b[?2004l  — bracketed paste off
        // \x1b[?1000l  — mouse tracking off
        // \x1b[?1002l  — mouse button event off
        // \x1b[?1003l  — mouse any event off
        // \x1b[?1006l  — SGR mouse off
        // \x1b[?1015l  — URXVT mouse off
        // \x1b[?1l     — cursor keys normal
        // \x1b[?25h    — show cursor
        // \x1b[?12l    — cursor blink off
        // \x1b>        — keypad numeric
        // \x1b[!p      — soft reset
        let reset = b"\x1b[?2004l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1015l\x1b[?1l\x1b[?25h\x1b[?12l\x1b>\x1b[!p";
        for session in &mut self.sessions {
            session.write_to_all_panes(reset);
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
    /// Toggle terminal input lock (read-only mode).
    pub(super) fn toggle_lock(&mut self) {
        self.locked = !self.locked;
        if self.locked {
            self.toast = Some(("Terminal locked".into(), 120));
        } else {
            self.toast = Some(("Terminal unlocked".into(), 120));
        }
    }

    pub(super) fn write_to_pty(&mut self, bytes: &[u8]) {
        // Terminal locked: block all input.
        if self.locked {
            return;
        }
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

        // Reset cursor blink cycle so the cursor is visible immediately
        // after the user types (standard terminal behavior).
        self.cursor_blink.reset();
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
        #[allow(unused_mut)]
        let mut req = crate::ai_bridge::AIRequest::new(action, ctx);

        // Read enable_tools from config.
        #[cfg(feature = "config-watch")]
        if let Some(ref mgr) = self.config_mgr {
            req.enable_tools = mgr.config().ai.enable_tools;
        }

        if let Some(ref mut bridge) = self.ai_bridge {
            if !bridge.request(req) {
                self.ai_overlay.set_error("AI is busy, please wait...");
            }
        } else {
            self.ai_overlay.set_error(
                "AI not configured.\n\
                 Set [ai] enabled=true and api_key in ~/.ggterm/config.toml,\n\
                 or set GGTERM_AI_API_KEY env var.",
            );
        }
    }

    /// Poll the AIBridge for streaming deltas and final result.
    #[cfg(feature = "ai")]
    pub(super) fn poll_ai_bridge(&mut self) {
        let Some(ref mut bridge) = self.ai_bridge else {
            return;
        };

        // Drain streaming deltas and append to overlay.
        let deltas = bridge.poll_deltas();
        for delta in deltas {
            self.ai_overlay.append_streaming(&delta);
        }

        // Check for final result.
        if let Some(response) = bridge.take_result() {
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
    /// Count characters in the current selection for status bar display.
    pub(super) fn count_selection_chars(&self) -> usize {
        let Some(((sx, sy), (ex, ey))) = self.selection.normalized() else {
            return 0;
        };
        let grid = self.active_session().app().grid();
        let mut count = 0usize;
        for row in sy..=ey {
            let (row_start, row_end) = if sy == ey {
                (sx as usize, ex as usize)
            } else if row == sy {
                (sx as usize, grid.width())
            } else if row == ey {
                (0, ex as usize)
            } else {
                (0, grid.width())
            };
            for col in row_start..row_end {
                if let Some(cell) = grid.display_cell(col, row as usize)
                    && !cell.is_wide_spacer()
                    && cell.ch != ' '
                {
                    count += 1 + cell.combining.len();
                }
            }
        }
        count
    }

    /// Select and copy the output of the last completed command.
    /// Uses OSC 133 marks to find the output region.
    pub(super) fn copy_last_command_output(&mut self) {
        let (scrollback, height, width) = {
            let grid = self.active_session().app().grid();
            (grid.scrollback_len(), grid.height(), grid.width())
        };

        let blocks = self.active_session().app().terminal().command_blocks();

        // Find the last completed command block.
        let last_block = blocks
            .iter()
            .rev()
            .find(|b| b.command_row.is_some() && b.end_row.is_some());

        if let Some(block) = last_block {
            // Output region: from output_row (or command_row+1) to end_row.
            let start_row = block
                .output_row
                .unwrap_or_else(|| block.command_row.unwrap_or(block.prompt_row) + 1);
            let end_row = block.end_row.unwrap();

            if end_row > start_row {
                // Convert absolute rows to display rows.
                let display_start = start_row.saturating_sub(scrollback);
                let display_end = end_row.saturating_sub(scrollback);

                if display_end > display_start && display_end <= height {
                    // Set selection covering the output rows.
                    self.selection.start(0, display_start as u16);
                    self.selection
                        .extend(width as u16, display_end.saturating_sub(1) as u16);
                    self.selection.finish();
                    self.selection_auto_scroll = 0;

                    // Now copy it.
                    self.copy_selection_to_clipboard();
                }
            }
        }
    }

    pub(super) fn copy_selection_to_clipboard(&mut self) {
        // Block (rectangular) selection: copy column-by-column.
        if self.selection.block_mode
            && let Some((x0, y0, x1, y1)) = self.selection.block_rect()
        {
            let grid = self.active_session().app().grid();
            let mut text = String::new();
            for row in y0..=y1 {
                for col in x0..=x1 {
                    if let Some(cell) = grid.display_cell(col as usize, row as usize) {
                        // Skip wide-character spacer cells.
                        if !cell.is_wide_spacer() {
                            text.push(cell.ch);
                            for &c in &cell.combining {
                                text.push(c);
                            }
                        }
                    }
                }
                text.push('\n');
            }
            // Trim trailing newline and empty lines.
            let mut lines: Vec<&str> = text.lines().map(|l| l.trim_end()).collect();
            while lines.last().is_some_and(|l| l.is_empty()) {
                lines.pop();
            }
            while lines.first().is_some_and(|l| l.is_empty()) {
                lines.remove(0);
            }
            let text = lines.join("\n");

            if !text.is_empty() {
                log::debug!("Block copy: {} chars", text.len());
                crate::clipboard::set_clipboard_bytes(text.as_bytes());
                self.show_toast(format!("Copied {} chars (block)", text.len()));
            }
            return;
        }

        let Some(((sx, sy), (ex, ey))) = self.selection.normalized() else {
            return;
        };

        // Helper: get cell text including combining characters.
        // Uses display_cell to correctly handle scrollback scroll position.
        let cell_text = |x: u16, y: u16, grid: &ggterm_core::Grid| -> String {
            if let Some(cell) = grid.display_cell(x as usize, y as usize) {
                // Skip wide-character spacer cells (continuation of a 2-cell-wide char).
                // The lead cell already contains the full character; the spacer is empty.
                if cell.is_wide_spacer() {
                    return String::new();
                }
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
    /// Safety: when bracketed paste is NOT active and the text contains
    /// newlines, we strip trailing newlines and show a warning toast to
    /// prevent accidental command execution.
    pub(super) fn paste_from_clipboard(&mut self) {
        let Some(text) = crate::clipboard::read_clipboard() else {
            log::debug!("Paste: clipboard empty or unavailable");
            return;
        };
        if text.is_empty() {
            return;
        }

        let bracketed = self.active_session().app().terminal().bracketed_paste();

        // Safety: if the program doesn't support bracketed paste and the
        // clipboard contains newlines, strip trailing newlines so the first
        // line doesn't get auto-executed as a command.
        let text = if !bracketed && text.contains('\n') {
            let stripped = text.trim_end_matches(['\n', '\r']);
            let line_count = text.lines().count();
            if line_count > 1 {
                self.show_toast(format!(
                    "Pasted first line ({} lines stripped)",
                    line_count.saturating_sub(1)
                ));
            }
            stripped.to_string()
        } else {
            text
        };

        let bytes = crate::clipboard::bracket_paste(&text, bracketed);
        log::debug!("Paste: {} bytes (bracketed={})", bytes.len(), bracketed);
        self.write_to_pty(&bytes);
    }

    /// Save the entire terminal scrollback + visible screen to a timestamped file.
    ///
    /// Writes to `~/ggterm-export-{unix_timestamp}.txt`.
    /// Shows a toast with the file path on success.
    pub(super) fn save_scrollback_to_file(&mut self) {
        let text = self.active_session().app().grid().export_text();

        // Generate timestamped filename using epoch seconds
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let filename = format!("ggterm-export-{ts}.txt");

        let path = std::env::var("HOME")
            .map(|h| std::path::PathBuf::from(h).join(&filename))
            .unwrap_or_else(|_| std::path::PathBuf::from(&filename));

        match std::fs::write(&path, &text) {
            Ok(_) => {
                let lines = text.lines().count();
                self.show_toast(format!("Saved {lines} lines to ~/{filename}"));
                log::info!("Scrollback saved to {}", path.display());
            }
            Err(e) => {
                self.show_toast(format!("Save failed: {e}"));
                log::error!("Scrollback save failed: {e}");
            }
        }
    }

    /// Export terminal output as a styled HTML document with ANSI colors preserved.
    ///
    /// The HTML file uses inline CSS to reproduce the terminal's colors,
    /// bold, italic, underline, and reverse video. Useful for sharing
    /// terminal output in documentation or bug reports.
    pub(super) fn export_html(&mut self) {
        let html = self.active_session().app().grid().export_html();

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let filename = format!("ggterm-export-{ts}.html");

        let path = std::env::var("HOME")
            .map(|h| std::path::PathBuf::from(h).join(&filename))
            .unwrap_or_else(|_| std::path::PathBuf::from(&filename));

        match std::fs::write(&path, &html) {
            Ok(_) => {
                self.show_toast(format!("Saved HTML to ~/{filename}"));
                log::info!("HTML export saved to {}", path.display());
            }
            Err(e) => {
                self.show_toast(format!("Export failed: {e}"));
                log::error!("HTML export failed: {e}");
            }
        }
    }

    /// Import SSH hosts from ~/.ssh/config and save to GGTerm's connection store.
    ///
    /// Reads the user's OpenSSH config file and imports host entries.
    /// Shows a toast with the number of imported hosts.
    pub(super) fn import_ssh_hosts(&mut self) {
        let mut store = crate::connection_manager::ConnectionStore::new();

        // Try loading existing store first.
        let store_path = std::env::var("HOME").ok().map(|h| {
            std::path::PathBuf::from(h)
                .join(".ggterm")
                .join("hosts.toml")
        });

        if let Some(ref path) = store_path
            && path.exists()
        {
            store = crate::connection_manager::ConnectionStore::load(path);
        }

        let added = store.import_ssh_config(true);

        if added > 0 {
            // Save updated store.
            if let Some(ref path) = store_path
                && let Err(e) = store.save(path)
            {
                log::error!("Failed to save hosts.toml: {e}");
            }
            self.show_toast(format!("Imported {added} SSH hosts from ~/.ssh/config"));
            log::info!("SSH config import: {added} hosts added");
        } else {
            self.show_toast("No new SSH hosts found in ~/.ssh/config");
            log::info!("SSH config import: no new hosts");
        }
    }

    /// Poll for pending OSC 52 clipboard set operations.
    ///
    /// Called from `about_to_wait` to apply any OSC 52 clipboard changes
    /// that programs have requested.
    pub(super) fn poll_osc52_clipboard(&mut self) {
        // Handle OSC 52 clipboard SET.
        if let Some(data) = self
            .active_session_mut()
            .app_mut()
            .terminal_mut()
            .take_pending_clipboard_set()
        {
            log::debug!("OSC 52 clipboard set: {} bytes", data.len());
            crate::clipboard::set_clipboard_bytes(&data);
        }

        // Handle OSC 52 clipboard QUERY: respond with current clipboard contents.
        if self
            .active_session_mut()
            .app_mut()
            .terminal_mut()
            .take_pending_clipboard_query()
            && let Some(text) = crate::clipboard::read_clipboard()
        {
            // Base64-encode the clipboard text and send the OSC 52 response.
            let b64 = Self::base64_encode(text.as_bytes());
            let resp = format!("\x1b]52;c;{b64}\x07");
            self.write_to_pty(resp.as_bytes());
            log::debug!(
                "OSC 52 clipboard query: responded with {} chars",
                text.len()
            );
        }
    }

    /// Poll for bell events from the terminal and trigger visual + audio bell (P11-E, P28-G).
    pub(super) fn poll_bell(&mut self) {
        // Check bell_mode from config: "none" disables all bell effects.
        let bell_mode = {
            #[cfg(feature = "config-watch")]
            {
                self.config_mgr
                    .as_ref()
                    .map(|m| m.config().terminal.bell_mode.as_str())
                    .unwrap_or("visual")
            }
            #[cfg(not(feature = "config-watch"))]
            {
                "visual"
            }
        };
        if bell_mode == "none" {
            // Still consume the bell flag to avoid buildup.
            for session in &mut self.sessions {
                let _ = session.app_mut().terminal_mut().take_bell();
            }
            return;
        }

        let active = self.active;
        let mut any_bell = false;
        for (i, session) in self.sessions.iter_mut().enumerate() {
            if session.app_mut().terminal_mut().take_bell() {
                if i == active {
                    any_bell = true;
                } else {
                    session.mark_unread();
                    session.mark_bell();
                }
            }
        }
        if any_bell {
            // Visual bell (flash) unless mode is "sound" only.
            if bell_mode == "visual" {
                self.visual_bell_frames = VISUAL_BELL_DURATION_FRAMES;
            }
            // Sound bell only when mode is "sound".
            if bell_mode == "sound" && self.bell_limiter.check() {
                self.sound_player.play(crate::sound::SoundType::Bell);
            }
            // Also play sound in visual mode if sound is enabled.
            if bell_mode == "visual" && self.bell_limiter.check() {
                self.sound_player.play(crate::sound::SoundType::Bell);
            }
            log::debug!("Bell triggered (mode: {})", bell_mode);
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
        // Apply to all sessions so every tab/pane gets the new theme.
        for session in &mut self.sessions {
            for pane_id in session.pane_ids() {
                if let Some(app) = session.pane_app_mut(pane_id) {
                    app.theme_manager().set_by_name(&name);
                }
            }
        }
        self.last_applied_theme = name.clone();
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

    /// Toggle always-on-top window level.
    pub(super) fn toggle_always_on_top(&mut self) {
        if let Some(ref window) = self.window {
            let level = if self.always_on_top {
                winit::window::WindowLevel::Normal
            } else {
                winit::window::WindowLevel::AlwaysOnTop
            };
            window.set_window_level(level);
            self.always_on_top = !self.always_on_top;
            if self.always_on_top {
                self.show_toast("Always on top: ON");
            } else {
                self.show_toast("Always on top: OFF");
            }
        }
    }

    // ── P30-A: Scrollbar scroll-to-position ────────────────────────

    // ── P31: Profile cycling ───────────────────────────────────────

    /// Cycle to the next config profile (if profiles are defined).
    pub(super) fn cycle_profile(&mut self) {
        let applied = if let Some(mgr) = self.config_mgr.as_mut() {
            let current = self.status_bar.active_profile().unwrap_or("").to_string();
            let next = mgr.config_mut().cycle_profile(&current);
            match next {
                Some(next) => {
                    log::info!("Switching profile: {} -> {}", current, next);
                    if let Err(e) = mgr.config_mut().apply_profile(&next) {
                        log::error!("Failed to apply profile '{next}': {e}");
                        None
                    } else {
                        Some(next)
                    }
                }
                None => {
                    self.show_toast("No profiles configured");
                    None
                }
            }
        } else {
            self.show_toast("No config loaded");
            None
        };

        // If a profile was applied, push its settings to all sessions.
        if let Some(ref profile_name) = applied {
            self.status_bar.set_profile(profile_name);
            self.show_toast(format!("Profile: {}", profile_name));

            // Read the updated config values.
            let (theme, font_size, scrollback) = if let Some(ref mgr) = self.config_mgr {
                let cfg = mgr.config();
                (
                    cfg.appearance.theme.clone(),
                    cfg.appearance.font_size as f32,
                    cfg.terminal.scrollback_lines,
                )
            } else {
                return;
            };

            // Apply theme + scrollback to all sessions.
            for session in &mut self.sessions {
                for pane_id in session.pane_ids() {
                    if let Some(app) = session.pane_app_mut(pane_id) {
                        app.theme_manager().set_by_name(&theme);
                        app.terminal_mut().grid_mut().set_scrollback(scrollback);
                    }
                }
            }
            self.last_applied_theme = theme;
            self.font_zoom.set_base_size(font_size);
            self.apply_font_size();
            self.apply_theme_to_renderer();

            if let Some(ref window) = self.window {
                window.request_redraw();
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

    /// Evenly distribute all split panes (reset ratios to 50/50).
    pub(super) fn balance_panes(&mut self) {
        let session = self.active_session_mut();
        if session.pane_count() > 1 {
            session.balance_splits();
            self.show_toast("Panes balanced");
        } else {
            self.show_toast("Nothing to balance");
        }
        if let Some(ref window) = self.window {
            window.request_redraw();
        }
    }

    /// Clear the screen of all tabs (sends ESC[H ESC[2J to each tab's active pane).
    pub(super) fn clear_all_tabs(&mut self) {
        let count = self.sessions.len();
        for i in 0..count {
            let prev = self.active;
            self.active = i;
            self.write_to_pty(b"\x1b[H\x1b[2J");
            self.active = prev;
        }
        self.show_toast(format!("Cleared {} tabs", count));
        if let Some(ref window) = self.window {
            window.request_redraw();
        }
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
            "tab.toggle_last" => {
                self.toggle_last_tab();
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
            "tab.duplicate" => {
                self.duplicate_tab();
            }
            "tab.close_others" => {
                self.close_other_tabs();
            }
            "tab.rename" => {
                self.renaming_tab = Some(self.active);
                self.rename_text = self.sessions[self.active].title().to_string();
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            "split.zoom" => {
                self.toggle_pane_zoom();
            }
            "split.balance" => {
                self.balance_panes();
            }
            "window.always_on_top" => {
                self.toggle_always_on_top();
            }
            "terminal.open_url" => {
                self.open_url_at_cursor();
            }
            "terminal.copy" => {
                self.copy_selection_to_clipboard();
            }
            "terminal.copy_cwd" => {
                if let Some(cwd) = self.active_session().cwd() {
                    crate::clipboard::set_clipboard_bytes(cwd.to_string_lossy().as_bytes());
                    self.show_toast("Copied path");
                } else {
                    self.show_toast("No path available");
                }
            }
            "terminal.clear" => {
                // Clear visible screen by sending Ctrl+L equivalent.
                self.write_to_pty(b"\x1b[H\x1b[2J");
            }
            "terminal.clear_all" => {
                self.clear_all_tabs();
            }
            "terminal.reset" => {
                self.active_session_mut().app_mut().terminal_mut().ris();
            }
            "terminal.save_scrollback" => {
                self.save_scrollback_to_file();
            }
            "terminal.export_html" => {
                self.export_html();
            }
            "terminal.import_ssh" => {
                self.import_ssh_hosts();
            }
            "terminal.copy_last_output" => {
                self.copy_last_command_output();
            }
            "terminal.toggle_lock" => {
                self.toggle_lock();
            }
            "settings.open" => {
                self.pending_open_settings = true;
            }
            "theme.cycle" => {
                self.active_session_mut()
                    .app_mut()
                    .theme_manager()
                    .cycle_next();
                self.apply_theme_to_renderer();
            }
            "font.zoom_in" => {
                self.font_zoom.zoom_in();
                self.apply_font_size();
            }
            "font.zoom_out" => {
                self.font_zoom.zoom_out();
                self.apply_font_size();
            }
            "font.zoom_reset" => {
                self.font_zoom.reset();
                self.apply_font_size();
            }
            "opacity.increase" => {
                self.adjust_opacity(0.05);
            }
            "opacity.decrease" => {
                self.adjust_opacity(-0.05);
            }
            "view.fullscreen" => {
                self.toggle_fullscreen();
            }
            "view.status_bar" => {
                self.status_bar_visible = !self.status_bar_visible;
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            "terminal.select_all" => {
                let grid = self.active_session().app().grid();
                let range = crate::terminal_actions::select_all_range(grid);
                self.selection
                    .start(range.start_col as u16, range.start_row as u16);
                self.selection
                    .extend(range.end_col as u16, range.end_row as u16);
                self.selection.finish();
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            "terminal.paste" => {
                self.paste_from_clipboard();
            }
            "tab.reopen_closed" => {
                self.reopen_closed_tab();
            }
            "split.close" => {
                self.active_session_mut().remove_active_pane();
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            "terminal.search" => {
                self.search.toggle();
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
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

    /// P2P: Check if mouse position is over the Share button in the status bar.
    #[cfg(feature = "p2p")]
    pub(super) fn is_in_share_button(&self) -> bool {
        let (mx, my) = self.cursor_pos;
        if let Some(ref renderer) = self.renderer {
            let cell_h = renderer.cell_height() as f32;
            let cell_w = renderer.cell_width() as f32;
            let screen_w = renderer.resolution_width() as f32;
            let screen_h = renderer.resolution_height() as f32;
            let bar_h = cell_h + 8.0;
            let bar_y = screen_h - bar_h;
            let pad_x = 12.0_f32;

            // Share button text and width must match render.rs exactly.
            let label = if self.p2p_share.visible {
                "Stop Share"
            } else {
                "Share"
            };
            let share_w = label.chars().count() as f32 * cell_w + 24.0;
            let share_x = screen_w - pad_x - share_w - 8.0;
            let share_top = bar_y + 3.0;
            let share_bot = bar_y + bar_h - 3.0;

            log::debug!(
                "share_button: mx={mx}, my={my}, x=[{share_x:.0}, {:.0}], y=[{share_top:.0}, {share_bot:.0}]",
                share_x + share_w
            );

            mx >= share_x as f64
                && mx <= (share_x + share_w) as f64
                && my >= share_top as f64
                && my <= share_bot as f64
        } else {
            false
        }
    }

    /// P2P: Toggle terminal sharing overlay.
    #[cfg(feature = "p2p")]
    pub(super) fn toggle_p2p_share(&mut self) {
        crate::p2p_share::log_to_file("toggle_p2p_share() called");
        self.p2p_share.toggle();
        if self.p2p_share.is_active() {
            match self.p2p_share.status {
                crate::p2p_share::P2pShareStatus::Error => {
                    self.show_toast("P2P: Failed to start sharing");
                }
                _ => {
                    self.show_toast("P2P: Sharing started — scan QR to connect");
                }
            }
        } else {
            self.show_toast("P2P: Sharing stopped");
        }
        if let Some(ref window) = self.window {
            window.request_redraw();
        }
    }

    /// P2P: Poll for incoming connections and tee PTY output.
    ///
    /// Called from `about_to_wait()` when P2P sharing is active.
    #[cfg(feature = "p2p")]
    pub(super) fn poll_p2p(&mut self) {
        // Check for new connections.
        let new_connection = self.p2p_share.poll_connection();
        if new_connection {
            self.show_toast("P2P: Device connected");
            // Forward current terminal dimensions.
            let (cols, rows) = {
                let session = self.active_session();
                let app = session.app();
                let grid = app.grid();
                (grid.width() as u16, grid.height() as u16)
            };
            self.p2p_share.resize(cols, rows);
        }

        // Read mobile input and forward to PTY.
        if self.p2p_share.status == crate::p2p_share::P2pShareStatus::Connected {
            let input = self.p2p_share.read_input();
            if !input.is_empty() {
                self.active_session_mut().write_to_pty(&input);
            }
        }
    }
}
