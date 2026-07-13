//! DesktopApp action methods — tab, split, clipboard, theme, session.

use super::*;

impl DesktopApp {
    /// Base64-encode data (used for OSC 52 clipboard query response).
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
            self.show_toast("Pane closed — shell exited");
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
            self.show_toast("Tab closed — shell exited");
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
    /// Close all tabs and open a fresh single tab.
    pub(super) fn new_session(&mut self) {
        let cols = self.config.cols;
        let rows = self.config.rows;
        let shell = self.shell().to_string();

        // Send reset sequences to all existing PTYs.
        for session in &mut self.sessions {
            session.write_to_all_panes(
                b"\x1b[?2004l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1015l\x1b[?25h\x1bc",
            );
        }

        // Drop all sessions.
        self.sessions.clear();
        self.active = 0;
        self.last_active_tab = None;

        // Create a fresh session.
        match TabSession::new(cols, rows, &shell) {
            Ok(session) => {
                self.sessions.push(session);
                self.active = 0;
                self.selection.clear();
                self.selection_auto_scroll = 0;
                self.show_toast("New session started");
                log::info!("New session: all tabs closed, fresh tab opened");
            }
            Err(e) => {
                log::error!("Failed to create new session: {e}");
            }
        }
        if let Some(ref window) = self.window {
            window.request_redraw();
        }
    }

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
            // Last tab: close the window instead of just returning.
            // This matches macOS behavior (Cmd+W on last tab closes window)
            // and is more intuitive on all platforms.
            self.should_quit = true;
            return;
        }
        // Don't close pinned tabs.
        if self.sessions[self.active].is_pinned() {
            self.show_toast("Tab is pinned — unpin first to close");
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
    /// Open a new ggterm window by re-launching the binary.
    pub(super) fn open_new_window(&self) {
        let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("ggterm"));
        match std::process::Command::new(&exe).spawn() {
            Ok(_) => log::info!("Launched new ggterm window"),
            Err(e) => log::error!("Failed to open new window: {}", e),
        }
    }

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
        // Reset sequences to restore terminal to a clean state on exit.
        // These disable extended modes that DECSTR (soft reset) doesn't reset.
        //
        // \x1b[?2004l  — bracketed paste off
        // \x1b[?1004l  — focus event reporting off
        // \x1b[?1000l  — mouse tracking off
        // \x1b[?1002l  — mouse button event off
        // \x1b[?1003l  — mouse any event off
        // \x1b[?1006l  — SGR mouse off
        // \x1b[?1015l  — URXVT mouse off
        // \x1b[?1016l  — SGR-pixel mouse off
        // \x1b[?7727l  — alternate scroll off
        // \x1b[?2026l  — synchronized output off
        // \x1b[?1l     — cursor keys normal
        // \x1b[?25h    — show cursor
        // \x1b[?12l    — cursor blink off
        // \x1b>        — keypad numeric
        // \x1b[!p      — soft reset (DECSTR)
        let reset = b"\x1b[?2004l\x1b[?1004l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1015l\x1b[?1016l\x1b[?7727l\x1b[?2026l\x1b[?1l\x1b[?25h\x1b[?12l\x1b>\x1b[!p";
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

    /// Swap the active pane's position with the next pane.
    /// The split geometry stays the same, but the terminal content
    /// moves to a different region.
    pub(super) fn swap_active_pane(&mut self) {
        let count = self.active_session().pane_count();
        if count < 2 {
            self.show_toast("Need 2+ panes to swap");
            return;
        }
        self.active_session_mut()
            .split_tree_mut()
            .swap_active_with_next();
        self.show_toast("Panes swapped");
        if let Some(ref window) = self.window {
            window.request_redraw();
        }
    }

    /// Toggle pin on the active tab. Pinned tabs cannot be closed.
    pub(super) fn toggle_pin_tab(&mut self) {
        let pinned = self.active_session_mut().toggle_pin();
        if pinned {
            self.show_toast("Tab pinned");
        } else {
            self.show_toast("Tab unpinned");
        }
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
        let line: String = row_data.text();
        if let Some((_, _, url)) = crate::mouse::detect_url_at_position(&line, col) {
            crate::mouse::open_url(&url);
            self.show_toast(format!("Opened: {}", &url[..url.len().min(60)]));
        } else {
            self.show_toast("No URL at cursor");
        }
    }

    /// Search the currently selected text on the web using the default browser.
    /// If no text is selected, shows a toast message.
    pub(super) fn search_web_for_selection(&mut self) {
        if !self.selection.is_active() {
            self.show_toast("Select text to search".to_string());
            return;
        }

        // Extract selected text via the clipboard pipeline.
        self.copy_selection_to_clipboard();
        let text = crate::clipboard::read_clipboard().unwrap_or_default();
        let query = text.trim();
        if query.is_empty() {
            self.show_toast("Selection is empty".to_string());
            return;
        }

        // URL-encode the query and open in browser.
        let encoded: String = query
            .chars()
            .map(|c| match c {
                ' ' => "+".into(),
                c if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~' => {
                    c.to_string()
                }
                c => format!("%{:02X}", c as u32),
            })
            .collect();
        let url = self
            .config_mgr
            .as_ref()
            .map(|m| m.config().terminal.search_engine.replace("%s", &encoded))
            .unwrap_or_else(|| format!("https://www.google.com/search?q={}", encoded));
        crate::mouse::open_url(&url);
        self.show_toast(format!("Searching: {}", &query[..query.len().min(40)]));
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

    /// Count words in the current text selection.
    /// A "word" is a maximal run of non-whitespace characters.
    pub(super) fn count_selection_words(&self) -> usize {
        let Some(((sx, sy), (ex, ey))) = self.selection.normalized() else {
            return 0;
        };
        let grid = self.active_session().app().grid();
        let mut text = String::new();
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
                {
                    text.push(cell.ch);
                }
            }
            text.push('\n');
        }
        text.split_whitespace().count()
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

        if let Some(block) = last_block
            && let Some(end_row) = block.end_row
        {
            // Output region: from output_row (or command_row+1) to end_row.
            let start_row = block
                .output_row
                .unwrap_or_else(|| block.command_row.unwrap_or(block.prompt_row) + 1);

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

    /// Copy the text of the last entered command (not its output).
    pub(super) fn copy_last_command(&mut self) {
        let (scrollback, width) = {
            let grid = self.active_session().app().grid();
            (grid.scrollback_len(), grid.width())
        };

        let blocks = self.active_session().app().terminal().command_blocks();

        // Find the last block with a command_row.
        let last_block = blocks.iter().rev().find(|b| b.command_row.is_some());

        if let Some(block) = last_block
            && let Some(cmd_row) = block.command_row
        {
            let display_row = cmd_row.saturating_sub(scrollback);
            if display_row < self.active_session().app().grid().height() {
                self.selection.start(0, display_row as u16);
                self.selection.extend(width as u16, display_row as u16);
                self.selection.finish();
                self.selection_auto_scroll = 0;
                self.copy_selection_to_clipboard();
            }
        }
    }

    /// Re-execute the last entered command.
    pub(super) fn rerun_last_command(&mut self) {
        // First, extract command text via selection → clipboard.
        self.copy_last_command();
        let text = crate::clipboard::read_clipboard().unwrap_or_default();

        if text.trim().is_empty() {
            self.show_toast("No previous command to rerun".to_string());
            return;
        }

        // Clear selection and send the command + Enter.
        self.selection.clear();
        let mut bytes = text.into_bytes();
        bytes.push(b'\n');
        self.write_to_pty(&bytes);
        self.show_toast("Re-running last command".to_string());
    }

    /// Re-execute the last command in a new tab.
    pub(super) fn rerun_in_new_tab(&mut self) {
        self.copy_last_command();
        let text = crate::clipboard::read_clipboard().unwrap_or_default();

        if text.trim().is_empty() {
            self.show_toast("No previous command to rerun".to_string());
            return;
        }

        // Prepare bytes before open_tab (avoids borrow conflict).
        let mut bytes = text.into_bytes();
        bytes.push(b'\n');
        self.selection.clear();
        self.open_tab();
        self.write_to_pty(&bytes);
        self.show_toast("Re-running in new tab".to_string());
    }

    /// Re-execute the last command in a new split pane.
    pub(super) fn rerun_in_split(&mut self) {
        self.copy_last_command();
        let text = crate::clipboard::read_clipboard().unwrap_or_default();

        if text.trim().is_empty() {
            self.show_toast("No previous command to rerun".to_string());
            return;
        }

        let mut bytes = text.into_bytes();
        bytes.push(b'\n');
        self.selection.clear();
        self.split_pane_horizontal();
        self.write_to_pty(&bytes);
        self.show_toast("Re-running in split".to_string());
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
            self.show_toast("Nothing selected to copy");
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

    /// Copy the current selection as a fenced Markdown code block.
    /// Wraps the text in triple backticks for easy pasting into
    /// GitHub issues, docs, or chat.
    pub(super) fn copy_selection_as_markdown(&mut self) {
        // Reuse the existing selection-to-text logic by copying to clipboard,
        // then re-reading and wrapping.
        self.copy_selection_to_clipboard();

        // Read back the copied text.
        if let Some(text) = crate::clipboard::read_clipboard() {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                // Determine language hint from content.
                let lang = detect_language_hint(trimmed);
                let markdown = if lang.is_empty() {
                    format!("```\n{}\n```", trimmed)
                } else {
                    format!("```{}\n{}\n```", lang, trimmed)
                };
                crate::clipboard::set_clipboard_bytes(markdown.as_bytes());
                self.show_toast(format!("Copied as Markdown ({} chars)", markdown.len()));
            }
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

        // Safety: warn on very large pastes (>100KB) to prevent terminal
        // slowdowns from processing massive clipboard content.
        const MAX_PASTE_WARN: usize = 100_000;
        if text.len() > MAX_PASTE_WARN {
            let kb = text.len() / 1024;
            self.show_toast(format!("Pasting {kb}KB (large paste may be slow)"));
        }

        let bracketed = self.active_session().app().terminal().bracketed_paste();

        // Safety: if the program doesn't support bracketed paste and the
        // clipboard contains newlines, strip trailing newlines so an extra
        // Enter isn't sent at the end (which would auto-execute the last
        // line before the user has a chance to review).
        let text = if !bracketed && text.contains('\n') {
            let stripped = text.trim_end_matches(['\n', '\r']);
            let line_count = stripped.lines().count();
            if line_count > 1 {
                self.show_toast(format!("Pasted {} lines (bracketed paste off)", line_count));
            }
            stripped.to_string()
        } else {
            text
        };

        let bytes = crate::clipboard::bracket_paste(&text, bracketed);
        log::debug!("Paste: {} bytes (bracketed={})", bytes.len(), bracketed);
        self.write_to_pty(&bytes);
    }

    /// Paste clipboard content and press Enter to immediately execute.
    /// Strips trailing newlines so only one Enter is sent.
    pub(super) fn paste_and_run(&mut self) {
        let Some(text) = crate::clipboard::read_clipboard() else {
            self.show_toast("Clipboard empty".to_string());
            return;
        };
        let text = text.trim_end_matches(['\n', '\r']);
        if text.is_empty() {
            self.show_toast("Clipboard empty".to_string());
            return;
        }
        let bracketed = self.active_session().app().terminal().bracketed_paste();
        let bytes = crate::clipboard::bracket_paste(text, bracketed);
        self.write_to_pty(&bytes);
        self.write_to_pty(b"\n");
        self.show_toast("Pasted and executed".to_string());
    }
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

    /// Save the currently selected text to a file in the home directory.
    pub(super) fn save_selection_to_file(&mut self) {
        if !self.selection.is_active() {
            self.show_toast("Select text to save".to_string());
            return;
        }

        // Copy to clipboard, then read back to get the text.
        self.copy_selection_to_clipboard();
        let text = crate::clipboard::read_clipboard().unwrap_or_default();

        if text.trim().is_empty() {
            self.show_toast("Selection is empty".to_string());
            return;
        }

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let filename = format!("ggterm-selection-{ts}.txt");

        let path = std::env::var("HOME")
            .map(|h| std::path::PathBuf::from(h).join(&filename))
            .unwrap_or_else(|_| std::path::PathBuf::from(&filename));

        match std::fs::write(&path, &text) {
            Ok(_) => {
                let lines = text.lines().count();
                self.show_toast(format!("Saved {lines} lines to ~/{filename}"));
            }
            Err(e) => {
                self.show_toast(format!("Save failed: {e}"));
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
        let mut active_bell = false;
        let mut any_tab_bell = false;
        for (i, session) in self.sessions.iter_mut().enumerate() {
            if session.app_mut().terminal_mut().take_bell() {
                any_tab_bell = true;
                if i == active {
                    active_bell = true;
                } else {
                    session.mark_unread();
                    session.mark_bell();
                }
            }
        }
        if active_bell {
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
        // Request dock/taskbar attention when window is unfocused and any tab has a bell.
        // Also show a desktop notification so the user knows to check the terminal.
        if any_tab_bell && !self.window_focused {
            if let Some(ref window) = self.window {
                window.request_user_attention(Some(winit::window::UserAttentionType::Critical));
            }
            // Show a system notification for the bell.
            let tab_title = self
                .sessions
                .get(self.active)
                .and_then(|s| {
                    let title = s.title().to_string();
                    if title.is_empty() { None } else { Some(title) }
                })
                .unwrap_or_else(|| "GGTerm".to_string());
            self.show_desktop_notification("Terminal Bell", &format!("Bell in: {}", tab_title));
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
            self.show_desktop_notification(&title, &body);
        }
    }

    /// Show a cross-platform desktop notification.
    /// macOS: `osascript display notification`
    /// Linux: `notify-send`
    /// Windows: PowerShell toast notification
    fn show_desktop_notification(&self, title: &str, body: &str) {
        log::info!("Desktop notification: {} — {}", title, body);
        #[cfg(target_os = "macos")]
        {
            let et = title.replace('\\', "\\\\").replace('"', "\\\"");
            let eb = body.replace('\\', "\\\\").replace('"', "\\\"");
            let script = format!("display notification \"{}\" with title \"{}\"", eb, et);
            std::process::Command::new("osascript")
                .args(["-e", &script])
                .spawn()
                .ok();
        }
        #[cfg(target_os = "linux")]
        {
            let et = title.replace('\'', "\\'");
            let eb = body.replace('\'', "\\'");
            std::process::Command::new("notify-send")
                .args([&et, &eb])
                .spawn()
                .ok();
        }
        #[cfg(target_os = "windows")]
        {
            let et = title.replace('\'', "\\'");
            let eb = body.replace('\'', "\\'");
            let ps = format!(
                "[reflection.assembly]::LoadWithPartialName('System.Windows.Forms'); \
                 $n = New-Object System.Windows.Forms.NotifyIcon; \
                 $n.Icon = [System.Drawing.SystemIcons]::Information; \
                 $n.Visible = $true; \
                 $n.ShowBalloonTip(5000, '{}', '{}', [System.Windows.Forms.ToolTipIcon]::Info)",
                et, eb
            );
            std::process::Command::new("powershell")
                .args(["-Command", &ps])
                .spawn()
                .ok();
        }
        // On unsupported platforms, the notification is only logged.
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            log::warn!("Desktop notifications not supported on this platform");
        }
    }

    /// Poll for command completion: when a long-running command finishes while
    /// the window is unfocused, show a desktop notification.
    ///
    /// Uses OSC 133 shell integration marks to detect command boundaries.
    /// Only fires if: (1) window is unfocused, (2) command ran >= 10 seconds,
    /// (3) we haven't already notified about this particular command.
    pub(super) fn poll_command_complete(&mut self) {
        // Check config: is notification on completion enabled?
        let (notify_enabled, min_secs) = if let Some(ref mgr) = self.config_mgr {
            let t = &mgr.config().terminal;
            (t.notify_on_complete, t.min_notify_duration_secs)
        } else {
            (true, 10)
        };
        if !notify_enabled {
            return;
        }

        let duration = self
            .active_session()
            .app()
            .terminal()
            .last_command_duration();

        match duration {
            Some(d) => {
                // Skip if we already notified about this exact duration.
                if self.last_notified_cmd_duration == Some(d) {
                    return;
                }
                // Only notify for commands that ran >= threshold seconds.
                if d.as_secs() < min_secs {
                    self.last_notified_cmd_duration = Some(d);
                    return;
                }
                // Only notify when the window is not focused.
                if self.window_focused {
                    return;
                }

                // Mark as notified before sending to avoid double-fire.
                self.last_notified_cmd_duration = Some(d);

                let secs = d.as_secs();
                let term = self.active_session().app().terminal();
                let exit_code = term.last_exit_code();
                let succeeded = term.last_command_succeeded();

                // Try to get the command text from the last command block.
                let cmd_text = term
                    .command_blocks()
                    .last()
                    .and_then(|block| {
                        let row = block.command_row.unwrap_or(block.prompt_row);
                        let scrollback = term.grid().scrollback_len();
                        if row >= scrollback {
                            Some(term.extract_row_text(row - scrollback))
                        } else {
                            None
                        }
                    })
                    .filter(|s| !s.is_empty());

                let title = if succeeded {
                    "Command finished".to_string()
                } else {
                    format!("Command failed (exit {})", exit_code.unwrap_or(-1))
                };

                let body = if let Some(ref cmd) = cmd_text {
                    // Truncate long commands.
                    let short = if cmd.len() > 60 {
                        format!("{}…", &cmd[..57])
                    } else {
                        cmd.clone()
                    };
                    format!("{} ({}s)", short, secs)
                } else {
                    format!("Completed in {}s", secs)
                };

                self.show_desktop_notification(&title, &body);

                // Request dock/taskbar attention.
                if let Some(ref window) = self.window {
                    window.request_user_attention(Some(winit::window::UserAttentionType::Critical));
                }
            }
            None => {
                // No completed command — reset tracker so next completion fires.
                self.last_notified_cmd_duration = None;
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
                        .map(|row| row.text().trim().to_string())
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

    /// Copy the current selection as styled HTML, preserving colors and attributes.
    pub(super) fn copy_selection_as_html(&mut self) {
        let Some(((sx, sy), (ex, ey))) = self.selection.normalized() else {
            self.show_toast("No text selected");
            return;
        };

        let grid = self.active_session().app().grid();
        let theme = self.active_session().app().theme();

        let html = selection_to_html(grid, sx, sy, ex, ey, theme);

        if html.is_empty() {
            self.show_toast("No text selected");
        } else {
            crate::clipboard::set_clipboard_bytes(html.as_bytes());
            self.show_toast("Copied as HTML");
        }
    }

    /// Run the current selection as a shell command.
    /// Copies selection to clipboard, then writes it + newline to PTY.
    pub(super) fn run_selection_as_command(&mut self) {
        self.copy_selection_to_clipboard();

        if let Some(text) = crate::clipboard::read_clipboard() {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                self.show_toast("No text selected");
                return;
            }

            // Write the command + Enter to PTY.
            let mut input = trimmed.to_string();
            input.push('\n');
            self.write_to_pty(input.as_bytes());
            self.show_toast(format!("Running: {}", truncate_for_toast(trimmed)));
        } else {
            self.show_toast("No text selected");
        }
    }

    /// Open the current selection in the user's $EDITOR.
    /// Writes selection to a temp file, opens editor, then reads back.
    pub(super) fn edit_selection_in_editor(&mut self) {
        // First, copy the selection to clipboard to extract text.
        self.copy_selection_to_clipboard();

        let text = crate::clipboard::read_clipboard().unwrap_or_default();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            self.show_toast("No text selected");
            return;
        }

        // Write to temp file.
        let tmp = std::env::temp_dir().join("ggterm-selection.txt");
        if let Err(e) = std::fs::write(&tmp, trimmed) {
            self.show_toast(format!("Failed: {e}"));
            return;
        }

        // Get editor from $EDITOR or fall back to `vi`.
        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
        let editor_path = editor.split_whitespace().next().unwrap_or("vi");
        let editor_args: Vec<String> = editor
            .split_whitespace()
            .skip(1)
            .map(String::from)
            .collect();

        let mut cmd = std::process::Command::new(editor_path);
        cmd.args(&editor_args).arg(&tmp);

        match cmd.status() {
            Ok(status) if status.success() => {
                // Read back the edited content and write to PTY.
                if let Ok(edited) = std::fs::read_to_string(&tmp)
                    && !edited.is_empty()
                {
                    self.write_to_pty(edited.as_bytes());
                    self.show_toast("Pasted edited selection");
                }
                let _ = std::fs::remove_file(&tmp);
            }
            Ok(_) => {
                self.show_toast("Editor exited with error");
            }
            Err(e) => {
                self.show_toast(format!("Failed to open editor: {e}"));
            }
        }
    }

    /// Search the current selection on the web (opens default browser).
    pub(super) fn search_selection_on_web(&mut self) {
        // First, copy the selection to clipboard to extract the text.
        self.copy_selection_to_clipboard();

        if let Some(text) = crate::clipboard::read_clipboard() {
            let query = text.trim();
            if query.is_empty() {
                self.show_toast("No text selected");
                return;
            }

            // URL-encode the query.
            let encoded = url_encode(query);
            let url = format!("https://www.google.com/search?q={}", encoded);

            // Open in default browser.
            let (cmd, args) = if cfg!(target_os = "macos") {
                ("open", vec![url])
            } else if cfg!(target_os = "windows") {
                ("cmd", vec!["/C".to_string(), "start".to_string(), url])
            } else {
                ("xdg-open", vec![url])
            };
            match std::process::Command::new(cmd).args(&args).spawn() {
                Ok(_) => self.show_toast(format!("Searching: {}", query)),
                Err(e) => self.show_toast(format!("Failed: {e}")),
            }
        } else {
            self.show_toast("No text selected");
        }
    }

    /// Open the current working directory in the system file manager.
    /// macOS: Finder, Linux: xdg-open, Windows: explorer.
    pub(super) fn open_cwd_in_file_manager(&mut self) {
        if let Some(cwd) = self.active_session().cwd() {
            let path = cwd.to_string_lossy().to_string();
            let (cmd, args) = if cfg!(target_os = "macos") {
                ("open", vec![path])
            } else if cfg!(target_os = "windows") {
                ("explorer", vec![path])
            } else {
                ("xdg-open", vec![path])
            };
            match std::process::Command::new(cmd).args(&args).spawn() {
                Ok(_) => self.show_toast("Opened file manager"),
                Err(e) => self.show_toast(format!("Failed: {e}")),
            }
        } else {
            self.show_toast("No working directory available");
        }
    }

    /// Clear the screen of all tabs (sends ESC[H ESC[2J to each tab's active pane).
    /// Force reload the config file on the next tick.
    pub(super) fn reload_configuration(&mut self) {
        self.force_config_reload = true;
        self.show_toast("Reloading configuration...");
        if let Some(ref window) = self.window {
            window.request_redraw();
        }
    }

    /// Send RIS (full reset, ESC c) to all panes in all tabs.
    pub(super) fn reset_all_tabs(&mut self) {
        let ris = b"\x1bc"; // RIS — Reset to Initial State
        let tab_count = self.sessions.len();
        for i in 0..tab_count {
            self.sessions[i].write_to_all_panes(ris);
        }
        self.show_toast(format!("Reset {} tabs", tab_count));
        if let Some(ref window) = self.window {
            window.request_redraw();
        }
    }

    /// Send Ctrl+C (0x03) to all panes in all tabs.
    pub(super) fn send_ctrl_c_all_panes(&mut self) {
        let ctrl_c = [0x03u8];
        let tab_count = self.sessions.len();
        let mut total_panes = 0usize;

        for i in 0..tab_count {
            total_panes += self.sessions[i].pane_count();
            self.sessions[i].write_to_all_panes(&ctrl_c);
        }
        self.show_toast(format!(
            "Sent Ctrl+C to {} panes in {} tabs",
            total_panes, tab_count
        ));
    }

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
            "window.new" => {
                self.open_new_window();
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
            "tab.toggle_pin" => {
                self.toggle_pin_tab();
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
            "terminal.search_web" => {
                self.search_web_for_selection();
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
            "terminal.clear_and_reset" => {
                // Clear scrollback + screen, then full RIS reset.
                self.active_session_mut().app_mut().terminal_mut().grid_mut().clear_scrollback();
                self.active_session_mut().app_mut().terminal_mut().ris();
                self.show_toast("Terminal cleared and reset".to_string());
            }
            "terminal.pipe_selection" => {
                if !self.selection.is_active() {
                    self.show_toast("Select text first".to_string());
                } else {
                    // Prompt user for a shell command via a simple input state.
                    // The command will be run with selection as stdin, output to clipboard.
                    self.pipe_command_input.clear();
                    self.pipe_command_active = true;
                    self.show_toast("Enter shell command, then press Enter".to_string());
                }
            }
            "terminal.copy_cwd_path" => {
                let cwd = self.active_session().cwd().map(std::path::PathBuf::from);
                match cwd {
                    Some(path) => {
                        crate::clipboard::set_clipboard_bytes(path.to_string_lossy().as_bytes());
                        self.show_toast(format!("Copied: {}", path.display()));
                    }
                    None => {
                        self.show_toast("No working directory known".to_string());
                    }
                }
            }
            "terminal.save_scrollback" => {
                self.save_scrollback_to_file();
            }
            "terminal.save_selection" => {
                self.save_selection_to_file();
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
            "terminal.copy_command_with_output" => {
                let text = self
                    .active_session()
                    .app()
                    .terminal()
                    .last_command_with_output_text();
                match text {
                    Some(t) if !t.is_empty() => {
                        crate::clipboard::set_clipboard_bytes(t.as_bytes());
                        let lines = t.lines().count();
                        self.show_toast(format!("Copied command + {lines} lines"));
                    }
                    _ => {
                        self.show_toast("No command output available".to_string());
                    }
                }
            }
            "terminal.copy_last_command" => {
                self.copy_last_command();
                self.show_toast("Copied last command".to_string());
            }
            "terminal.rerun" => {
                self.rerun_last_command();
            }
            "terminal.rerun_new_tab" => {
                self.rerun_in_new_tab();
            }
            "terminal.rerun_split" => {
                self.rerun_in_split();
            }
            "terminal.copy_visible" => {
                let text = self.active_session().app().grid().export_visible_text();
                crate::clipboard::set_clipboard_bytes(text.as_bytes());
                self.show_toast(format!("Copied {} chars (visible screen)", text.len()));
            }
            "terminal.copy_markdown" => {
                self.copy_selection_as_markdown();
            }
            "terminal.open_in_finder" => {
                self.open_cwd_in_file_manager();
            }
            "terminal.open_shell_config" => {
                self.open_shell_config();
            }
            "terminal.open_config_folder" => {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                let config_dir = std::path::PathBuf::from(&home).join(".ggterm");
                // Create if doesn't exist
                if !config_dir.exists() {
                    let _ = std::fs::create_dir_all(&config_dir);
                }
                #[cfg(target_os = "macos")]
                {
                    let _ = std::process::Command::new("open").arg(&config_dir).spawn();
                }
                #[cfg(all(unix, not(target_os = "macos")))]
                {
                    let _ = std::process::Command::new("xdg-open")
                        .arg(&config_dir)
                        .spawn();
                }
                #[cfg(target_os = "windows")]
                {
                    let _ = std::process::Command::new("explorer")
                        .arg(&config_dir)
                        .spawn();
                }
                self.show_toast("Opened ~/.ggterm".to_string());
            }
            "terminal.open_cwd_in_file_manager" => {
                let cwd = self.active_session().cwd().map(std::path::PathBuf::from);
                if let Some(ref path) = cwd {
                    #[cfg(target_os = "macos")]
                    {
                        let _ = std::process::Command::new("open").arg(path).spawn();
                    }
                    #[cfg(all(unix, not(target_os = "macos")))]
                    {
                        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
                    }
                    #[cfg(target_os = "windows")]
                    {
                        let _ = std::process::Command::new("explorer").arg(path).spawn();
                    }
                    self.show_toast(format!("Opened {}", path.display()));
                } else {
                    self.show_toast("No working directory known".to_string());
                }
            }
            "terminal.send_ctrl_c_all" => {
                self.send_ctrl_c_all_panes();
            }
            "terminal.reset_all" => {
                self.reset_all_tabs();
            }
            "terminal.info" => {
                let session = self.active_session();
                let grid = session.app().grid();
                let term = session.app().terminal();
                let cwd = session
                    .cwd()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                let remote = term
                    .remote_host()
                    .map(|h| format!(" | SSH:{h}"))
                    .unwrap_or_default();
                let info = format!(
                    "{}x{} | scrollback: {} | cwd: {}{}",
                    grid.width(),
                    grid.height(),
                    grid.scrollback_len(),
                    cwd,
                    remote
                );
                self.show_toast(info);
            }
            "terminal.jump_first" => {
                let blocks = self.active_session().app().terminal().command_blocks();
                if let Some(first) = blocks.first() {
                    self.active_session_mut()
                        .app_mut()
                        .grid_mut()
                        .scroll_to_grid_row(first.prompt_row);
                    self.show_toast("Jumped to first command".to_string());
                } else {
                    self.show_toast("No commands in history".to_string());
                }
            }
            "terminal.jump_last" => {
                let blocks = self.active_session().app().terminal().command_blocks();
                if let Some(last) = blocks.last() {
                    self.active_session_mut()
                        .app_mut()
                        .grid_mut()
                        .scroll_to_grid_row(last.prompt_row);
                    self.show_toast("Jumped to latest command".to_string());
                } else {
                    self.show_toast("No commands in history".to_string());
                }
            }
            "terminal.export_commands" => {
                let blocks = self.active_session().app().terminal().command_blocks();
                let grid = self.active_session().app().grid();
                let scrollback = grid.scrollback_len();
                let mut lines = Vec::new();
                for block in &blocks {
                    if let Some(cmd_row) = block.command_row {
                        let display_row = cmd_row.saturating_sub(scrollback);
                        if let Some(row) = grid.display_row(display_row) {
                            lines.push(row.text());
                        }
                    }
                }
                if lines.is_empty() {
                    self.show_toast("No commands to export".to_string());
                } else {
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let filename = format!("ggterm-commands-{ts}.txt");
                    let path = std::env::var("HOME")
                        .map(|h| std::path::PathBuf::from(h).join(&filename))
                        .unwrap_or_else(|_| std::path::PathBuf::from(&filename));
                    let content = lines.join("\n");
                    match std::fs::write(&path, &content) {
                        Ok(_) => self.show_toast(format!(
                            "Exported {} commands to ~/{filename}",
                            lines.len()
                        )),
                        Err(e) => self.show_toast(format!("Export failed: {e}")),
                    }
                }
            }
            "terminal.interrupt_all" => {
                let count = self.sessions.len();
                for session in &mut self.sessions {
                    session.write_to_all_panes(&[0x03]); // Ctrl+C
                }
                self.show_toast(format!("Sent Ctrl+C to all panes ({count} tabs)"));
            }
            "terminal.paste_and_run" => {
                self.paste_and_run();
            }
            "terminal.open_terminal_in_cwd" => {
                let cwd = self.active_session().cwd().map(std::path::PathBuf::from);
                if let Some(ref path) = cwd {
                    #[cfg(target_os = "macos")]
                    {
                        let _ = std::process::Command::new("open")
                            .arg("-a")
                            .arg("Terminal")
                            .arg(path)
                            .spawn();
                    }
                    #[cfg(all(unix, not(target_os = "macos")))]
                    {
                        let term = std::env::var("TERMINAL")
                            .unwrap_or_else(|_| "x-terminal-emulator".to_string());
                        let _ = std::process::Command::new(&term)
                            .arg("--working-directory")
                            .arg(path)
                            .spawn();
                    }
                    #[cfg(target_os = "windows")]
                    {
                        let _ = std::process::Command::new("cmd")
                            .arg("/C")
                            .arg("start")
                            .arg("cmd")
                            .arg("/K")
                            .arg(format!("cd /d {}", path.display()))
                            .spawn();
                    }
                    self.show_toast(format!("Opened terminal at {}", path.display()));
                } else {
                    self.show_toast("No working directory known".to_string());
                }
            }
            "config.reload" => {
                self.reload_configuration();
            }
            "config.open" => {
                self.open_config_file();
            }
            "terminal.scroll_mode" => {
                self.scroll_mode = !self.scroll_mode;
                if self.scroll_mode {
                    self.show_toast("Scroll mode: j/k scroll, G/g jump, q/Esc exit");
                } else {
                    self.show_toast("Exited scroll mode");
                }
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            "split.swap" => {
                self.swap_active_pane();
            }
            "view.toggle_cursor_line" => {
                if let Some(mgr) = &mut self.config_mgr {
                    let new_val = !mgr.config().appearance.cursor_line_highlight;
                    mgr.config_mut().appearance.cursor_line_highlight = new_val;
                    if new_val {
                        self.show_toast("Cursor line highlight: ON");
                    } else {
                        self.show_toast("Cursor line highlight: OFF");
                    }
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                }
            }
            "terminal.new_session" => {
                self.new_session();
            }
            "terminal.search_selection" => {
                self.search_selection_on_web();
            }
            "terminal.edit_selection" => {
                self.edit_selection_in_editor();
            }
            "terminal.run_selection" => {
                self.run_selection_as_command();
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
            crate::tab_bar::TabMenuAction::CloseOtherTabs => {
                if let Some(idx) = self.tab_context_menu.tab_index
                    && self.sessions.len() > 1
                {
                    // Swap the kept tab to position 0, then truncate.
                    self.sessions.swap(0, idx);
                    self.sessions.truncate(1);
                    self.active = 0;
                    self.show_toast("Closed other tabs");
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                }
            }
            crate::tab_bar::TabMenuAction::CloseTabsRight => {
                if let Some(idx) = self.tab_context_menu.tab_index
                    && idx + 1 < self.sessions.len()
                {
                    let count = self.sessions.len() - idx - 1;
                    self.sessions.truncate(idx + 1);
                    if self.active > idx {
                        self.active = idx;
                    }
                    self.show_toast(format!("Closed {count} tab(s) to the right"));
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
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

    /// Open the shell's rc file (.zshrc, .bashrc, .config/fish/config.fish)
    /// in the system's default editor.
    pub(super) fn open_shell_config(&mut self) {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
        let Some(home) = home else {
            self.show_toast("Cannot find home directory");
            return;
        };
        let home = std::path::PathBuf::from(home);

        // Determine which shell rc file to open based on the $SHELL env var.
        let shell = std::env::var("SHELL").unwrap_or_default();
        let rc_path = if shell.contains("zsh") {
            home.join(".zshrc")
        } else if shell.contains("fish") {
            home.join(".config/fish/config.fish")
        } else {
            home.join(".bashrc")
        };

        if !rc_path.exists() {
            self.show_toast(format!("{} not found — create it first", rc_path.display()));
            return;
        }

        // Prefer $EDITOR (Unix convention: vim, nano, code, etc).
        // Fall back to system default file opener.
        let editor = std::env::var("EDITOR").ok();
        let result = if let Some(ed) = editor.filter(|e| !e.is_empty()) {
            // Parse editor command (e.g. "code --wait" → ["code", "--wait"]).
            let mut parts = ed.split_whitespace();
            let cmd = parts.next().unwrap_or(&ed);
            let args: Vec<&str> = parts.collect();
            let mut command = std::process::Command::new(cmd);
            command.args(&args).arg(&rc_path);
            command.spawn()
        } else {
            #[cfg(target_os = "macos")]
            {
                std::process::Command::new("open").arg(&rc_path).spawn()
            }
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                std::process::Command::new("xdg-open").arg(&rc_path).spawn()
            }
            #[cfg(target_os = "windows")]
            {
                std::process::Command::new("cmd")
                    .args(["/C", "start", ""])
                    .arg(&rc_path)
                    .spawn()
            }
        };

        match result {
            Ok(_) => self.show_toast(format!("Opened {}", rc_path.display())),
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

            crate::p2p_share::log_to_file(&format!(
                "is_in_share_button: cursor=({mx},{my}), share_btn=[{share_x:.0}-{:.0}, {share_top:.0}-{share_bot:.0}]",
                share_x + share_w
            ));

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
            self.show_toast("P2P: Device connected — sharing terminal");
            // Auto-hide the QR overlay — terminal is now usable.
            self.p2p_share.visible = false;

            // DO NOT send resize — it was injected control frames into the
            // data stream and causing garbled output. The mobile terminal
            // has its own dimensions.

            // Send accumulated PTY output (the raw bytes from the shell).
            let screen_data = self.active_session_mut().app_mut().take_pty_tee();
            crate::p2p_share::log_to_file(&format!(
                "poll_p2p: sending {} bytes of accumulated PTY data",
                screen_data.len()
            ));
            self.p2p_share.tee_output(&screen_data);

            // Send Ctrl+L to the shell. This clears the screen and redraws
            // the prompt WITHOUT executing any pending user input (unlike \n).
            // The redraw output gets tee'd to the mobile device.
            self.active_session_mut().write_to_pty(b"\x0c");

            // Force window redraw so about_to_wait keeps cycling.
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
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


/// Truncate a string for toast display (max 40 chars).
fn truncate_for_toast(s: &str) -> String {
    if s.len() > 40 {
        format!("{}...", &s[..37])
    } else {
        s.to_string()
    }
}

/// URL-encode a string for query parameters (spaces become +).
fn url_encode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for &b in input.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            result.push(b as char);
        } else if b == b' ' {
            result.push('+');
        } else {
            result.push_str(&format!("%{b:02X}"));
        }
    }
    result
}

/// Resolve a Color to an RGB hex string (e.g., "#ff0000").
fn color_to_hex(color: &ggterm_core::Color, theme: &ggterm_render::RenderTheme) -> String {
    match color {
        ggterm_core::Color::Default => "#c0c0c0".to_string(),
        ggterm_core::Color::Indexed(i) => {
            if (*i as usize) < 16 {
                match theme.palette[*i as usize] {
                    ggterm_core::Color::Rgb(r, g, b) => format!("#{:02x}{:02x}{:02x}", r, g, b),
                    _ => "#c0c0c0".to_string(),
                }
            } else {
                "#c0c0c0".to_string()
            }
        }
        ggterm_core::Color::Rgb(r, g, b) => format!("#{:02x}{:02x}{:02x}", r, g, b),
    }
}

/// Escape HTML special characters.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Convert a selection region to styled HTML with inline CSS.
fn selection_to_html(
    grid: &ggterm_core::Grid,
    sx: u16,
    sy: u16,
    ex: u16,
    ey: u16,
    theme: &ggterm_render::RenderTheme,
) -> String {
    let mut html = String::from(
        "<pre style=\"font-family:monospace;font-size:14px;background:#1a1a2e;color:#c0c0c0;padding:12px;border-radius:6px;overflow-x:auto\">",
    );

    let width = grid.width() as u16;

    for row in sy..=ey {
        let col_start = if row == sy { sx } else { 0 };
        let col_end = if row == ey { ex } else { width - 1 };

        let mut span_buf = String::new();
        let mut cur_fg: Option<String> = None;
        let mut cur_bold = false;
        let mut cur_italic = false;

        for col in col_start..=col_end {
            let cell = match grid.display_cell(col as usize, row as usize) {
                Some(c) => c,
                None => continue,
            };
            if cell.is_wide_spacer() {
                continue;
            }

            // Determine effective fg color (bright override for bold).
            let cell_fg = if cell.flags.contains(ggterm_core::CellFlags::BOLD)
                && let ggterm_core::Color::Indexed(i) = cell.fg
                && i < 8
            {
                ggterm_core::Color::Indexed(i + 8)
            } else {
                cell.fg
            };
            let fg_hex = if matches!(cell_fg, ggterm_core::Color::Default) {
                None
            } else {
                Some(color_to_hex(&cell_fg, theme))
            };
            let bold = cell.flags.contains(ggterm_core::CellFlags::BOLD);
            let italic = cell.flags.contains(ggterm_core::CellFlags::ITALIC);

            // Flush if style changed.
            if fg_hex != cur_fg || bold != cur_bold || italic != cur_italic {
                flush_span(&mut span_buf, &cur_fg, cur_bold, cur_italic, &mut html);
                cur_fg = fg_hex;
                cur_bold = bold;
                cur_italic = italic;
            }

            span_buf.push(cell.ch);
            for &c in &cell.combining {
                span_buf.push(c);
            }
        }
        flush_span(&mut span_buf, &cur_fg, cur_bold, cur_italic, &mut html);
        html.push('\n');
    }

    html.push_str("</pre>");
    html
}

/// Flush accumulated text as an HTML span with inline style.
fn flush_span(buf: &mut String, fg: &Option<String>, bold: bool, italic: bool, html: &mut String) {
    if buf.is_empty() {
        return;
    }
    let mut style = String::new();
    if let Some(c) = fg {
        style.push_str(&format!("color:{}", c));
    }
    if bold {
        if !style.is_empty() {
            style.push(';');
        }
        style.push_str("font-weight:bold");
    }
    if italic {
        if !style.is_empty() {
            style.push(';');
        }
        style.push_str("font-style:italic");
    }
    let escaped = html_escape(buf);
    if style.is_empty() {
        html.push_str(&escaped);
    } else {
        html.push_str(&format!("<span style=\"{}\">{}</span>", style, escaped));
    }
    buf.clear();
}

/// Detect a programming language from terminal output content.
/// Returns a language string for Markdown code fencing, or empty string.
fn detect_language_hint(text: &str) -> &'static str {
    let first_line = text.lines().next().unwrap_or("");
    // Shell prompts.
    if first_line.starts_with('$')
        || first_line.starts_with(">")
        || first_line.contains("bash")
        || first_line.contains("zsh")
    {
        return "bash";
    }
    // Rust compiler output.
    if text.contains("cargo")
        || text.contains("rustc")
        || first_line.contains("-->")
        || text.contains("error[E")
    {
        return "rust";
    }
    // Python traceback.
    if text.contains("Traceback") || text.contains("python") || text.contains("File \"") {
        return "python";
    }
    // Git output.
    if text.contains("git ")
        || first_line.starts_with("commit ")
        || first_line.starts_with("Author:")
    {
        return "diff";
    }
    // JSON.
    if first_line.trim_start().starts_with('{') || first_line.trim_start().starts_with('[') {
        return "json";
    }
    // SQL.
    if text.to_uppercase().contains("SELECT ")
        || text.to_uppercase().contains("INSERT INTO")
        || text.to_uppercase().contains("CREATE TABLE")
    {
        return "sql";
    }
    ""
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_language_rust() {
        assert_eq!(
            detect_language_hint("error[E0308]: mismatched types"),
            "rust"
        );
        assert_eq!(
            detect_language_hint("  --> src/main.rs:10:5\ncargo build"),
            "rust"
        );
    }

    #[test]
    fn test_detect_language_python() {
        assert_eq!(
            detect_language_hint("Traceback (most recent call last)"),
            "python"
        );
    }

    #[test]
    fn test_detect_language_bash() {
        assert_eq!(detect_language_hint("$ ls -la"), "bash");
    }

    #[test]
    fn test_detect_language_json() {
        assert_eq!(detect_language_hint(r#"{"key": "value"}"#), "json");
    }

    #[test]
    fn test_detect_language_none() {
        assert_eq!(detect_language_hint("hello world"), "");
    }

    #[test]
    fn test_url_encode_basic() {
        assert_eq!(url_encode("hello"), "hello");
        assert_eq!(url_encode("hello world"), "hello+world");
    }

    #[test]
    fn test_url_encode_special_chars() {
        assert_eq!(url_encode("a&b=c"), "a%26b%3Dc");
        assert_eq!(url_encode("100%"), "100%25");
    }

    #[test]
    fn test_url_encode_safe_chars() {
        assert_eq!(url_encode("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("hello"), "hello");
        assert_eq!(html_escape("a<b>c"), "a&lt;b&gt;c");
        assert_eq!(html_escape("a&b"), "a&amp;b");
        assert_eq!(html_escape("a&<b>"), "a&amp;&lt;b&gt;");
    }

    #[test]
    fn test_truncate_for_toast_short() {
        assert_eq!(truncate_for_toast("hello"), "hello");
    }

    #[test]
    fn test_truncate_for_toast_long() {
        let long = "a".repeat(50);
        let result = truncate_for_toast(&long);
        assert!(result.ends_with("..."));
        assert_eq!(result.len(), 40);
    }

    #[test]
    fn test_color_to_hex_rgb() {
        let theme = ggterm_render::RenderTheme::by_name("dark").unwrap();
        assert_eq!(
            color_to_hex(&ggterm_core::Color::Rgb(255, 0, 0), &theme),
            "#ff0000"
        );
        assert_eq!(
            color_to_hex(&ggterm_core::Color::Rgb(0, 128, 255), &theme),
            "#0080ff"
        );
    }

    #[test]
    fn test_color_to_hex_indexed() {
        let theme = ggterm_render::RenderTheme::by_name("dark").unwrap();
        // Color index 1 = red in most palettes
        let result = color_to_hex(&ggterm_core::Color::Indexed(1), &theme);
        assert!(result.starts_with('#'));
        assert_eq!(result.len(), 7);
    }

    #[test]
    fn test_color_to_hex_default() {
        let theme = ggterm_render::RenderTheme::by_name("dark").unwrap();
        assert_eq!(
            color_to_hex(&ggterm_core::Color::Default, &theme),
            "#c0c0c0"
        );
    }

    #[test]
    fn test_flush_span_plain() {
        let mut html = String::new();
        let mut buf = "hello".to_string();
        flush_span(&mut buf, &None, false, false, &mut html);
        assert_eq!(html, "hello");
    }

    #[test]
    fn test_flush_span_bold() {
        let mut html = String::new();
        let mut buf = "bold text".to_string();
        flush_span(&mut buf, &Some("#ff0000".into()), true, false, &mut html);
        assert!(html.contains("color:#ff0000"));
        assert!(html.contains("font-weight:bold"));
        assert!(html.contains("bold text"));
    }

    #[test]
    fn test_flush_span_empty() {
        let mut html = String::new();
        let mut buf = String::new();
        flush_span(&mut buf, &Some("#fff".into()), true, true, &mut html);
        assert!(html.is_empty());
    }
}
