//! Input event handlers — keyboard, mouse, cursor, resize.

use super::*;

impl DesktopApp {
    /// Handle window resize: store pending dimensions for debounced apply.
    ///
    /// During a drag-resize, winit fires many `Resized` events. We store the
    /// latest dimensions and defer the actual Terminal/PTY resize until the
    /// user stops dragging (100ms debounce). See `apply_pending_resize()`.
    pub(super) fn handle_resize(&mut self, width: u32, height: u32) {
        self.pending_resize = Some((width.max(1), height.max(1)));
        self.last_resize_time = Some(std::time::Instant::now());
    }

    /// Apply a pending resize if the debounce interval (100ms) has elapsed.
    ///
    /// Called from `about_to_wait()`. Returns `true` if a resize was applied.
    pub(super) fn apply_pending_resize(&mut self) -> bool {
        let Some((width, height)) = self.pending_resize else {
            return false;
        };
        let Some(last) = self.last_resize_time else {
            return false;
        };

        // Check if enough time has passed since the last resize event.
        if std::time::Instant::now().duration_since(last) < std::time::Duration::from_millis(100) {
            return false; // Not enough time elapsed — wait.
        }

        // Clear pending state.
        self.pending_resize = None;
        self.last_resize_time = None;

        // Resize GPU surface to match physical window.
        if let (Some(gpu), Some(surface)) = (&mut self.gpu, &self.surface) {
            gpu.resize(surface, width.max(1), height.max(1));
        }

        // Recreate renderer with the ACTUAL (clamped) surface dimensions.
        // gpu.resize() clamps to max_texture_dimension_2d, so we must use
        // gpu.config dimensions — not the raw width/height — to avoid
        // renderer resolution > surface extent (causes scissor rect panic).
        if let Some(gpu) = &self.gpu {
            self.renderer =
                Some(gpu.create_renderer(gpu.config.width, gpu.config.height, self.scale_factor));
        }

        // Get actual cols/rows from renderer.
        let new_cols = self
            .renderer
            .as_ref()
            .map(|r| r.cols() as u16)
            .unwrap_or(80)
            .max(10);
        let new_rows = self
            .renderer
            .as_ref()
            .map(|r| r.rows() as u16)
            .unwrap_or(24)
            .max(3);

        log::debug!(
            "Resize: {}x{}px → {}x{} cells",
            width,
            height,
            new_cols,
            new_rows
        );

        // Resize all sessions (not just active) so background tabs are ready
        // when the user switches to them.
        for session in self.sessions.iter_mut() {
            session.resize(new_cols, new_rows);
        }

        // Show a brief size indicator toast.
        self.show_toast(format!("{}x{}", new_cols, new_rows));

        true
    }

    /// Handle a winit key event using the existing keymap module.
    pub(super) fn handle_keyboard_input(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }

        // P30-B: Tab rename mode — intercept all keyboard input.
        if self.renaming_tab.is_some() {
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::Escape) => {
                    self.renaming_tab = None;
                    self.rename_text.clear();
                    return;
                }
                PhysicalKey::Code(KeyCode::Enter) => {
                    if let Some(idx) = self.renaming_tab.take() {
                        let title = std::mem::take(&mut self.rename_text);
                        if !title.is_empty() && idx < self.sessions.len() {
                            self.sessions[idx].set_title(title);
                        }
                    }
                    return;
                }
                PhysicalKey::Code(KeyCode::Backspace) => {
                    self.rename_text.pop();
                    return;
                }
                _ => {}
            }
            if let Some(c) = event.text.as_ref().and_then(|t| t.chars().next())
                && !c.is_control()
            {
                self.rename_text.push(c);
                return;
            }
            return;
        }

        // P27-C: Close context menu on Escape.
        if self.context_menu.visible
            && let PhysicalKey::Code(KeyCode::Escape) = &event.physical_key
        {
            self.context_menu.hide();
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
            return;
        }

        // Close "+" dropdown on Escape.
        if self.new_tab_menu.visible
            && let PhysicalKey::Code(KeyCode::Escape) = &event.physical_key
        {
            self.new_tab_menu.hide();
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
            return;
        }

        // ── P14-D: Config-driven keybinding dispatch ──
        // All configurable actions are resolved through check_keybinding().
        // The resolved_keybindings map is populated from ConfigManager at
        // startup and falls back to default_keybindings() when no config exists.
        if let PhysicalKey::Code(code) = &event.physical_key {
            let key_name = keycode_to_name(code);

            // Ctrl+T → new tab (also Cmd+T on macOS)
            if self.check_keybinding(
                "new_tab",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) || (cfg!(target_os = "macos") && self.mods.super_key && key_name == "t")
            {
                self.open_tab();
                return;
            }
            // Ctrl+Shift+T → reopen last closed tab (also Cmd+Shift+T on macOS)
            if (self.mods.ctrl && self.mods.shift && key_name == "t")
                || (cfg!(target_os = "macos")
                    && self.mods.super_key
                    && self.mods.shift
                    && key_name == "t")
            {
                self.reopen_closed_tab();
                return;
            }
            // Ctrl+Shift+Alt+D → duplicate active tab
            if self.mods.ctrl && self.mods.shift && self.mods.alt && key_name == "d" {
                self.duplicate_tab();
                return;
            }
            // Ctrl+Shift+Alt+W → close all other tabs
            if self.mods.ctrl && self.mods.shift && self.mods.alt && key_name == "w" {
                self.close_other_tabs();
                return;
            }
            // Ctrl+W → close tab (also Cmd+W on macOS)
            if self.check_keybinding(
                "close_tab",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) || (cfg!(target_os = "macos") && self.mods.super_key && key_name == "w")
            {
                if self.active_session().pane_count() > 1 {
                    // Multiple panes: close the active pane instead of the tab.
                    self.active_session_mut().remove_active_pane();
                } else {
                    self.close_tab();
                }
                return;
            }
            // Ctrl+= → zoom in (also Cmd+= on macOS)
            if self.check_keybinding(
                "zoom_in",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) || (cfg!(target_os = "macos") && self.mods.super_key && key_name == "=")
            {
                if self.font_zoom.zoom_in() {
                    self.apply_font_size();
                    self.show_toast(format!("{:.0}px", self.font_zoom.current_size()));
                }
                return;
            }
            // Ctrl+- → zoom out (also Cmd+- on macOS)
            if self.check_keybinding(
                "zoom_out",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) || (cfg!(target_os = "macos") && self.mods.super_key && key_name == "-")
            {
                if self.font_zoom.zoom_out() {
                    self.apply_font_size();
                    self.show_toast(format!("{:.0}px", self.font_zoom.current_size()));
                }
                return;
            }
            // Ctrl+0 → reset zoom (also Cmd+0 on macOS)
            if self.check_keybinding(
                "zoom_reset",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) || (cfg!(target_os = "macos") && self.mods.super_key && key_name == "0")
            {
                if self.font_zoom.reset() {
                    self.apply_font_size();
                    self.show_toast(format!("{:.0}px", self.font_zoom.current_size()));
                }
                return;
            }
            // Ctrl+Shift+Alt+] → increase background opacity
            if self.mods.ctrl && self.mods.shift && self.mods.alt && key_name == "]" {
                self.adjust_opacity(0.05);
                return;
            }
            // Ctrl+Shift+Alt+[ → decrease background opacity
            if self.mods.ctrl && self.mods.shift && self.mods.alt && key_name == "[" {
                self.adjust_opacity(-0.05);
                return;
            }
            // F11 → fullscreen
            if self.check_keybinding(
                "fullscreen",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                self.toggle_fullscreen();
                return;
            }
            // F1 → toggle debug overlay (P24-C)
            if key_name == "f1" && !self.mods.ctrl && !self.mods.shift && !self.mods.alt {
                self.debug_visible = !self.debug_visible;
                return;
            }
            // Ctrl+Shift+V → paste (also Cmd+V on macOS, Shift+Insert on all platforms)
            if (self.check_keybinding(
                "paste",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            )) || (cfg!(target_os = "macos") && self.mods.super_key && key_name == "v")
                || (self.mods.shift && !self.mods.ctrl && !self.mods.alt && key_name == "insert")
            {
                self.paste_from_clipboard();
                return;
            }
            // Ctrl+Shift+C → copy (also Cmd+C on macOS, Ctrl+Insert on Linux/Windows)
            if (self.check_keybinding(
                "copy",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            )) || (cfg!(target_os = "macos") && self.mods.super_key && key_name == "c")
                || (!cfg!(target_os = "macos")
                    && self.mods.ctrl
                    && !self.mods.shift
                    && key_name == "insert")
            {
                self.copy_selection_to_clipboard();
                return;
            }
            // Ctrl+Shift+K → clear screen + get fresh prompt (also Cmd+K on macOS)
            if self.check_keybinding(
                "clear",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) || (cfg!(target_os = "macos") && self.mods.super_key && key_name == "k")
            {
                crate::terminal_actions::clear_screen_and_scrollback(
                    self.active_session_mut().app_mut().grid_mut(),
                );
                // Send clear sequence so shell redraws a fresh prompt.
                self.write_to_pty(b"\x1b[H\x1b[2J\x0c");
                return;
            }
            // Ctrl+Shift+R → restart shell (full reset)
            if self.check_keybinding(
                "reset",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                self.active_session_mut().restart_active_shell();
                return;
            }
            // Ctrl+Shift+T → cycle theme
            if self.check_keybinding(
                "cycle_theme",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                self.cycle_theme();
                return;
            }
            // Ctrl+Shift+Alt+P → copy current working directory
            // (Ctrl+Shift+P without Alt is reserved for command palette)
            if self.mods.ctrl
                && self.mods.shift
                && self.mods.alt
                && let PhysicalKey::Code(KeyCode::KeyP) = &event.physical_key
            {
                if let Some(cwd) = self.active_session().cwd() {
                    crate::clipboard::set_clipboard_bytes(cwd.to_string_lossy().as_bytes());
                    self.show_toast("Copied path");
                } else {
                    self.show_toast("No path available");
                }
                return;
            }
            // Ctrl+Shift+F → toggle search
            if self.check_keybinding(
                "search",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                self.search.toggle();
                return;
            }
        }

        // ── P19-B: Split pane shortcuts (not configurable) ──

        // Ctrl+Shift+D → horizontal split (also Cmd+D on macOS)
        if let PhysicalKey::Code(KeyCode::KeyD) = &event.physical_key {
            let ctrl_shift = self.mods.ctrl && self.mods.shift && !self.mods.alt;
            let cmd = cfg!(target_os = "macos") && self.mods.super_key && !self.mods.shift;
            if ctrl_shift || cmd {
                self.split_pane_horizontal();
                return;
            }
        }

        // Ctrl+Shift+\ → vertical split (top / bottom)
        // (Ctrl+Shift+S is reserved for AI Suggest)
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::Backslash) = &event.physical_key
        {
            self.split_pane_vertical();
            return;
        }

        // Ctrl+Shift+] → focus next pane
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::BracketRight) = &event.physical_key
        {
            self.selection.clear();
            self.active_session_mut().focus_next_pane();
            return;
        }

        // Ctrl+Shift+[ → focus previous pane
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::BracketLeft) = &event.physical_key
        {
            self.selection.clear();
            self.active_session_mut().focus_prev_pane();
            return;
        }

        // Alt+h/j/k/l → vim-style pane navigation (tmux-navigator compatible)
        if !self.mods.ctrl && !self.mods.shift && self.mods.alt {
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::KeyJ) => {
                    self.selection.clear();
                    self.active_session_mut().focus_next_pane();
                    return;
                }
                PhysicalKey::Code(KeyCode::KeyK) => {
                    self.selection.clear();
                    self.active_session_mut().focus_prev_pane();
                    return;
                }
                PhysicalKey::Code(KeyCode::KeyH) => {
                    self.selection.clear();
                    self.active_session_mut().focus_prev_pane();
                    return;
                }
                PhysicalKey::Code(KeyCode::KeyL) => {
                    self.selection.clear();
                    self.active_session_mut().focus_next_pane();
                    return;
                }
                _ => {}
            }
        }

        // Ctrl+Shift+Z → toggle pane zoom (tmux-style maximize active pane)
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyZ) = &event.physical_key
        {
            self.toggle_pane_zoom();
            return;
        }

        // Ctrl+Shift+U → open URL at cursor position
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyU) = &event.physical_key
        {
            self.open_url_at_cursor();
            return;
        }

        // Ctrl+Shift+I → rename current tab
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyI) = &event.physical_key
        {
            self.renaming_tab = Some(self.active);
            self.rename_text = self.sessions[self.active].title().to_string();
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
            return;
        }

        // Ctrl+Shift+Alt+Arrows → adjust split ratio
        if self.mods.ctrl && self.mods.shift && self.mods.alt {
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::ArrowLeft) => {
                    self.active_session_mut()
                        .split_tree_mut()
                        .adjust_active_ratio(-0.05);
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowRight) => {
                    self.active_session_mut()
                        .split_tree_mut()
                        .adjust_active_ratio(0.05);
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowUp) => {
                    self.active_session_mut()
                        .split_tree_mut()
                        .adjust_active_ratio(0.05);
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowDown) => {
                    self.active_session_mut()
                        .split_tree_mut()
                        .adjust_active_ratio(-0.05);
                    return;
                }
                _ => {}
            }
        }

        // Ctrl+Shift+Arrows → extend selection by word (like editors).
        if self.mods.ctrl && self.mods.shift && !self.mods.alt {
            let cols = self.active_session().app().grid().width() as u16;
            let (cur_col, cur_row): (u16, u16) = self
                .selection
                .end
                .or(self.selection.start)
                .unwrap_or((0, 0));
            let extended: Option<(u16, u16)> = match &event.physical_key {
                PhysicalKey::Code(KeyCode::ArrowLeft) => {
                    // Jump to previous word boundary.
                    let grid = self.active_session().app().grid();
                    let mut c = cur_col;
                    for _ in 0..cols {
                        if c == 0 {
                            break;
                        }
                        c -= 1;
                        let ch = grid
                            .display_cell(c as usize, cur_row as usize)
                            .map(|cell| cell.ch)
                            .unwrap_or(' ');
                        if ch.is_whitespace() && c > 0 {
                            let prev_ch = grid
                                .display_cell((c - 1) as usize, cur_row as usize)
                                .map(|cell| cell.ch)
                                .unwrap_or(' ');
                            if !prev_ch.is_whitespace() {
                                break;
                            }
                        }
                    }
                    Some((c, cur_row))
                }
                PhysicalKey::Code(KeyCode::ArrowRight) => {
                    // Jump to next word boundary.
                    let grid = self.active_session().app().grid();
                    let mut c = cur_col;
                    let mut was_space = false;
                    for _ in 0..cols {
                        if c >= cols - 1 {
                            break;
                        }
                        let ch = grid
                            .display_cell(c as usize, cur_row as usize)
                            .map(|cell| cell.ch)
                            .unwrap_or(' ');
                        if was_space && !ch.is_whitespace() {
                            break;
                        }
                        was_space = ch.is_whitespace();
                        c += 1;
                    }
                    Some((c, cur_row))
                }
                _ => None,
            };
            if let Some((c, r)) = extended {
                if self.selection.start.is_none() {
                    self.selection.start = Some((cur_col, cur_row));
                }
                self.selection.extend(c, r);
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
                return;
            }
        }

        // Shift+Arrows (no Ctrl/Alt) → extend text selection (like editors).
        if self.mods.shift && !self.mods.ctrl && !self.mods.alt {
            let rows = self.active_session().app().grid().height() as u16;
            let (cur_col, cur_row) = self
                .selection
                .end
                .or(self.selection.start)
                .unwrap_or((0, 0));
            let extended = match &event.physical_key {
                PhysicalKey::Code(KeyCode::ArrowLeft) => Some((cur_col.saturating_sub(1), cur_row)),
                PhysicalKey::Code(KeyCode::ArrowRight) => Some((cur_col + 1, cur_row)),
                PhysicalKey::Code(KeyCode::ArrowUp) => Some((cur_col, cur_row.saturating_sub(1))),
                PhysicalKey::Code(KeyCode::ArrowDown) => {
                    Some((cur_col, (cur_row + 1).min(rows.saturating_sub(1))))
                }
                _ => None,
            };
            if let Some((c, r)) = extended {
                if self.selection.start.is_none() {
                    self.selection.start = Some((cur_col, cur_row));
                }
                self.selection.extend(c, r);
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
                return;
            }
        }

        // Ctrl+Shift+B → toggle status bar visibility (not configurable)
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyB) = &event.physical_key
        {
            self.status_bar_visible = !self.status_bar_visible;
            return;
        }

        // P28-G: Ctrl+Shift+M → toggle sound on/off
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyM) = &event.physical_key
        {
            self.sound_player.toggle();
            return;
        }

        // P28-A: Ctrl+Shift+G → toggle perf monitor
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyG) = &event.physical_key
        {
            self.perf_monitor.toggle();
            return;
        }

        // P28-H: Ctrl+Shift+L → toggle shell switcher
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyL) = &event.physical_key
        {
            self.shell_switcher.toggle();
            return;
        }

        // P28-C: Ctrl+Shift+Y → toggle command history sidebar
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyY) = &event.physical_key
        {
            self.cmd_history.toggle();
            return;
        }

        // P28-H: Shell switcher navigation when open
        if self.shell_switcher.open {
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::Escape) => {
                    self.shell_switcher.close();
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowUp) => {
                    self.shell_switcher.select_up();
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowDown) => {
                    self.shell_switcher.select_down();
                    return;
                }
                _ => {}
            }
        }

        // P28-A: Ctrl+Shift+W → cycle workspace forward
        if self.mods.ctrl
            && self.mods.shift
            && self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyW) = &event.physical_key
        {
            self.workspaces.cycle_next();
            self.animations.tab_switch();
            return;
        }

        // P31: Ctrl+Shift+Alt+P → cycle config profile
        if self.mods.ctrl
            && self.mods.shift
            && self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyP) = &event.physical_key
        {
            self.cycle_profile();
            return;
        }

        // P31: Ctrl+Shift+Alt+E → export config to clipboard
        if self.mods.ctrl
            && self.mods.shift
            && self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyE) = &event.physical_key
        {
            self.export_config();
            return;
        }

        // P33: Ctrl+Shift+Alt+I → import config from clipboard
        if self.mods.ctrl
            && self.mods.shift
            && self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyI) = &event.physical_key
        {
            self.import_config();
            return;
        }

        // P33: Ctrl+Shift+Alt+R → reset config to defaults
        if self.mods.ctrl
            && self.mods.shift
            && self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyR) = &event.physical_key
        {
            self.reset_config();
            return;
        }

        // P34: Ctrl+Shift+Alt+N → reset layout to single pane
        if self.mods.ctrl
            && self.mods.shift
            && self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyN) = &event.physical_key
        {
            self.reset_layout();
            return;
        }

        // Ctrl+Shift+Alt+S → save scrollback to file
        if self.mods.ctrl
            && self.mods.shift
            && self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyS) = &event.physical_key
        {
            self.save_scrollback_to_file();
            return;
        }

        // Ctrl+Shift+, → open config file in editor (like VS Code)
        // Cmd+, on macOS (standard "Preferences" shortcut)
        if let PhysicalKey::Code(KeyCode::Comma) = &event.physical_key {
            let want_open = (cfg!(target_os = "macos")
                && self.mods.super_key
                && !self.mods.ctrl
                && !self.mods.alt)
                || (self.mods.ctrl && self.mods.shift && !self.mods.alt);
            if want_open {
                self.open_config_file();
                return;
            }
        }

        // P29-A: Ctrl+Shift+/ → toggle shortcut help overlay.
        // Also handle quit confirm Esc/Enter.
        if self.quit_confirm {
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::Escape) => {
                    self.quit_confirm = false;
                    return;
                }
                PhysicalKey::Code(KeyCode::Enter) => {
                    // Enter = confirm quit.
                    self.should_quit = true;
                    return;
                }
                PhysicalKey::Code(KeyCode::KeyY) | PhysicalKey::Code(KeyCode::KeyS) => {
                    // Y or S = Yes/Sure
                    self.should_quit = true;
                    return;
                }
                PhysicalKey::Code(KeyCode::KeyN) => {
                    self.quit_confirm = false;
                    return;
                }
                _ => return, // swallow all other keys
            }
        }

        // P29-A: Ctrl+Shift+/ → toggle shortcut help overlay.
        if self.mods.ctrl
            && self.mods.shift
            && let PhysicalKey::Code(KeyCode::Slash) = &event.physical_key
        {
            self.shortcut_help.toggle();
            return;
        }

        // P29-A: When shortcut help is open, intercept keyboard input.
        if self.shortcut_help.visible {
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::Escape) => {
                    self.shortcut_help.close();
                    return;
                }
                PhysicalKey::Code(KeyCode::PageUp) => {
                    self.shortcut_help.scroll_up();
                    return;
                }
                PhysicalKey::Code(KeyCode::PageDown) => {
                    self.shortcut_help.scroll_down();
                    return;
                }
                PhysicalKey::Code(KeyCode::Backspace) => {
                    self.shortcut_help.backspace();
                    return;
                }
                _ => {}
            }
            if let Some(c) = event.text.as_ref().and_then(|t| t.chars().next())
                && !c.is_control()
            {
                self.shortcut_help.type_char(c);
                return;
            }
            return; // swallow all other keys when help is open
        }

        // P25-B: Ctrl+Shift+P → toggle command palette
        if self.mods.ctrl
            && self.mods.shift
            && let PhysicalKey::Code(KeyCode::KeyP) = &event.physical_key
        {
            self.command_palette.toggle();
            return;
        }

        // P25-B: When command palette is open, intercept keyboard input.
        if self.command_palette.visible {
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::Escape) => {
                    self.command_palette.toggle(); // close
                    return;
                }
                PhysicalKey::Code(KeyCode::Enter) => {
                    let registry = crate::command_palette::CommandRegistry::defaults();
                    let results = self.command_palette.results(&registry);
                    self.command_palette.confirm(&results);
                    // Execute the pending action.
                    if let Some(action_id) = self.command_palette.take_action() {
                        self.execute_command_palette_action(&action_id);
                    }
                    self.command_palette.toggle(); // close after confirm
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowUp) => {
                    let registry = crate::command_palette::CommandRegistry::defaults();
                    let results = self.command_palette.results(&registry);
                    self.command_palette.move_up(results.len());
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowDown) => {
                    let registry = crate::command_palette::CommandRegistry::defaults();
                    let results = self.command_palette.results(&registry);
                    self.command_palette.move_down(results.len());
                    return;
                }
                PhysicalKey::Code(KeyCode::Backspace) => {
                    self.command_palette.backspace();
                    return;
                }
                _ => {}
            }
            // Type printable characters into the palette query.
            if let Some(c) = event.text.as_ref().and_then(|t| t.chars().next())
                && !c.is_control()
            {
                self.command_palette.type_char(c);
                return;
            }
            return; // swallow all other keys when palette is open
        }

        // P25-D: Ctrl+Shift+Alt+B → cycle broadcast mode
        if self.mods.ctrl
            && self.mods.shift
            && self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyB) = &event.physical_key
        {
            self.broadcast.cycle();
            return;
        }

        // Ctrl+, (comma) → open settings window
        if self.mods.ctrl
            && !self.mods.shift
            && let PhysicalKey::Code(KeyCode::Comma) = &event.physical_key
        {
            self.pending_open_settings = true;
            return;
        }

        // P19-C: When settings overlay is open, intercept navigation keys.
        if self.settings.visible {
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::Escape) => {
                    self.apply_settings_on_close();
                    self.settings.close();
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowUp) => {
                    self.settings.move_up();
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowDown) => {
                    self.settings.move_down();
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowLeft) => {
                    self.handle_settings_left();
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowRight) => {
                    self.handle_settings_right();
                    return;
                }
                PhysicalKey::Code(KeyCode::Enter) => {
                    // Enter = save and close (same as Esc with save).
                    self.apply_settings_on_close();
                    self.settings.close();
                    return;
                }
                _ => {}
            }
        }

        // Ctrl+Shift+Return → toggle maximized (not configurable)
        if self.mods.ctrl
            && self.mods.shift
            && let PhysicalKey::Code(KeyCode::Enter) = &event.physical_key
        {
            self.toggle_maximized();
            return;
        }

        // Ctrl+Shift+End → scroll to bottom (reset viewport)
        if self.mods.ctrl
            && self.mods.shift
            && let PhysicalKey::Code(KeyCode::End) = &event.physical_key
        {
            self.active_session_mut()
                .app_mut()
                .terminal_mut()
                .grid_mut()
                .reset_viewport();
            self.smooth_scroll.reset();
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
            return;
        }

        // Shift+PageUp → scroll up one viewport height.
        // Shift+PageDown → scroll down one viewport height.
        // Shift+Home → scroll to top (oldest).
        // Shift+End → scroll to bottom (newest).
        if self.mods.shift && !self.mods.ctrl && !self.mods.alt {
            let grid_h = self.active_session().app().grid().height();
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::PageUp) => {
                    self.active_session_mut()
                        .app_mut()
                        .terminal_mut()
                        .grid_mut()
                        .scroll_up_viewport(grid_h);
                    self.smooth_scroll.reset();
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                    return;
                }
                PhysicalKey::Code(KeyCode::PageDown) => {
                    self.active_session_mut()
                        .app_mut()
                        .terminal_mut()
                        .grid_mut()
                        .scroll_down_viewport(grid_h);
                    self.smooth_scroll.reset();
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                    return;
                }
                PhysicalKey::Code(KeyCode::Home) => {
                    let scrollback_len = self.active_session().app().grid().scrollback_len();
                    self.active_session_mut()
                        .app_mut()
                        .terminal_mut()
                        .grid_mut()
                        .scroll_up_viewport(scrollback_len);
                    self.smooth_scroll.reset();
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                    return;
                }
                PhysicalKey::Code(KeyCode::End) => {
                    self.active_session_mut()
                        .app_mut()
                        .terminal_mut()
                        .grid_mut()
                        .reset_viewport();
                    self.smooth_scroll.reset();
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                    return;
                }
                _ => {}
            }
        }

        // Ctrl+Shift+Up/Down → scroll one line at a time (no Shift on macOS).
        if self.mods.ctrl && self.mods.shift && !self.mods.alt {
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::ArrowUp) => {
                    self.active_session_mut()
                        .app_mut()
                        .terminal_mut()
                        .grid_mut()
                        .scroll_up_viewport(1);
                    self.smooth_scroll.reset();
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowDown) => {
                    self.active_session_mut()
                        .app_mut()
                        .terminal_mut()
                        .grid_mut()
                        .scroll_down_viewport(1);
                    self.smooth_scroll.reset();
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                    return;
                }
                _ => {}
            }
        }

        // Ctrl+Shift+Alt+Up → scroll to mark (OSC 1337 SetMark).
        if self.mods.ctrl
            && self.mods.shift
            && self.mods.alt
            && let PhysicalKey::Code(KeyCode::ArrowUp) = &event.physical_key
        {
            let mark = self.active_session().app().terminal().mark_row();
            let scrollback_len = self.active_session().app().grid().scrollback_len();
            let current_offset = self.active_session().app().grid().display_offset();
            if let Some(mark_row) = mark {
                let target_offset = scrollback_len.saturating_sub(mark_row);
                let grid = self
                    .active_session_mut()
                    .app_mut()
                    .terminal_mut()
                    .grid_mut();
                if target_offset > current_offset {
                    grid.scroll_up_viewport(target_offset - current_offset);
                } else if target_offset < current_offset {
                    grid.scroll_down_viewport(current_offset - target_offset);
                }
                self.smooth_scroll.reset();
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
                self.show_toast(format!("Jumped to mark (line {})", mark_row));
            } else {
                self.show_toast("No mark set".to_string());
            }
            return;
        }

        // Alt+1-9 or Cmd+1-9 (macOS) → switch to tab N (not configurable)
        // macOS: Cmd+Up/Cmd+Down → scroll one line, Cmd+PageUp/PageDown → scroll one page.
        if cfg!(target_os = "macos") && self.mods.super_key && !self.mods.alt && !self.mods.ctrl {
            let grid = self
                .active_session_mut()
                .app_mut()
                .terminal_mut()
                .grid_mut();
            let mut scrolled = false;
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::ArrowUp) => {
                    grid.scroll_up_viewport(1);
                    scrolled = true;
                }
                PhysicalKey::Code(KeyCode::ArrowDown) => {
                    grid.scroll_down_viewport(1);
                    scrolled = true;
                }
                PhysicalKey::Code(KeyCode::PageUp) => {
                    grid.scroll_up_viewport(grid.height());
                    scrolled = true;
                }
                PhysicalKey::Code(KeyCode::PageDown) => {
                    grid.scroll_down_viewport(grid.height());
                    scrolled = true;
                }
                _ => {}
            }
            if scrolled {
                self.smooth_scroll.reset();
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
                return;
            }
        }

        // Alt+1-9 or Cmd+1-9 (macOS) → switch to tab N (not configurable)
        if (self.mods.alt && !self.mods.ctrl
            || cfg!(target_os = "macos") && self.mods.super_key && !self.mods.alt)
            && let PhysicalKey::Code(code) = &event.physical_key
        {
            let tab_idx = match code {
                KeyCode::Digit1 => Some(0),
                KeyCode::Digit2 => Some(1),
                KeyCode::Digit3 => Some(2),
                KeyCode::Digit4 => Some(3),
                KeyCode::Digit5 => Some(4),
                KeyCode::Digit6 => Some(5),
                KeyCode::Digit7 => Some(6),
                KeyCode::Digit8 => Some(7),
                KeyCode::Digit9 => Some(8),
                _ => None,
            };
            if let Some(idx) = tab_idx {
                self.switch_tab(idx);
                return;
            }
        }

        // Ctrl+Tab → next tab, Ctrl+Shift+Tab → prev tab (not configurable)
        // On macOS, Cmd+Tab/Cmd+Shift+Tab also cycles tabs.
        if (self.mods.ctrl || (cfg!(target_os = "macos") && self.mods.super_key))
            && let PhysicalKey::Code(KeyCode::Tab) = &event.physical_key
        {
            if self.mods.shift {
                self.prev_tab();
            } else {
                self.next_tab();
            }
            return;
        }

        // Ctrl+Shift+PageUp → move tab left, Ctrl+Shift+PageDown → move tab right
        if self.mods.ctrl && self.mods.shift && !self.mods.alt && self.sessions.len() > 1 {
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::PageUp) => {
                    if self.active > 0 {
                        self.move_tab(self.active, self.active - 1);
                    }
                    return;
                }
                PhysicalKey::Code(KeyCode::PageDown) => {
                    if self.active < self.sessions.len() - 1 {
                        self.move_tab(self.active, self.active + 1);
                    }
                    return;
                }
                _ => {}
            }
        }

        // Phase 8-D: Ctrl+Shift+Up/Down for command block navigation (not configurable)
        if self.mods.ctrl
            && self.mods.shift
            && let PhysicalKey::Code(code) = &event.physical_key
        {
            match code {
                KeyCode::ArrowUp => {
                    self.active_session_mut()
                        .app_mut()
                        .handle_event(AppEvent::PrevCommandBlock);
                    return;
                }
                KeyCode::ArrowDown => {
                    self.active_session_mut()
                        .app_mut()
                        .handle_event(AppEvent::NextCommandBlock);
                    return;
                }
                // Ctrl+Shift+A → select all text (also Cmd+A on macOS)
                KeyCode::KeyA => {
                    if self.mods.shift || (cfg!(target_os = "macos") && self.mods.super_key) {
                        let grid = self.active_session().app().grid();
                        let range = crate::terminal_actions::select_all_range(grid);
                        self.selection
                            .start(range.start_col as u16, range.start_row as u16);
                        self.selection
                            .extend(range.end_col as u16, range.end_row as u16);
                        self.selection.finish();
                        return;
                    }
                }
                // P10-C: AI assistant shortcuts (Ctrl+Shift+E/S/H/N, not configurable)
                #[cfg(feature = "ai")]
                KeyCode::KeyE => {
                    self.trigger_ai_request(ggterm_ai::Action::Explain);
                    return;
                }
                #[cfg(feature = "ai")]
                KeyCode::KeyS => {
                    self.trigger_ai_request(ggterm_ai::Action::Suggest);
                    return;
                }
                #[cfg(feature = "ai")]
                KeyCode::KeyH => {
                    self.trigger_ai_request(ggterm_ai::Action::ErrorHelp);
                    return;
                }
                #[cfg(feature = "ai")]
                KeyCode::KeyN => {
                    self.trigger_ai_request(ggterm_ai::Action::NL2Command);
                    return;
                }
                _ => {}
            }
        }

        // P10-D: When search bar is open, intercept all keyboard input.
        if self.search.visible {
            self.handle_search_input(event);
            return;
        }

        // P10-C: Esc dismisses AI overlay if visible.
        #[cfg(feature = "ai")]
        if self.ai_overlay.is_visible()
            && let PhysicalKey::Code(KeyCode::Escape) = &event.physical_key
        {
            self.ai_overlay.hide();
            return;
        }

        // Extract logical text for printable character support.
        let logical_text: Option<&str> = match &event.logical_key {
            Key::Character(s) => Some(s.as_ref()),
            _ => None,
        };

        // Use the shared keymap module for mapping.
        let mods: crate::input::KeyModifiers = self.mods.into();
        // Sync cursor/keypad application modes from terminal to encoder.
        let term = self.sessions[self.active].app().terminal();
        self.encoder.set_cursor_app_mode(term.cursor_keys_app());
        self.encoder.set_keypad_app_mode(term.keypad_app());
        if let Some(input_key) = map_winit_key(&event.physical_key, logical_text, &mods) {
            let bytes = self.encoder.encode(&input_key);
            if !bytes.is_empty() {
                self.write_to_pty(&bytes);
            }
        }
    }

    // ── Scrollback search (P10-D) ───────────────────────────────

    /// Handle keyboard input when the search bar is open.
    pub(super) fn handle_search_input(&mut self, event: &KeyEvent) {
        // Direct field access for disjoint borrows: self.sessions (immutable)
        // and self.search (mutable) are separate fields of self.
        let grid = self.sessions[self.active].app().grid();
        let search = &mut self.search;
        match &event.physical_key {
            PhysicalKey::Code(KeyCode::Escape) => {
                search.close();
            }
            PhysicalKey::Code(KeyCode::Enter) => {
                let matched = if self.mods.shift {
                    search.prev_match()
                } else {
                    search.next_match()
                };
                // Scroll viewport to show the matched position.
                if let Some(m) = matched {
                    let scrollback_len = self.sessions[self.active].app().grid().scrollback_len();
                    let grid_height = self.sessions[self.active].app().grid().height();
                    let visible_row = m.abs_row as isize - scrollback_len as isize;
                    if visible_row < 0 {
                        // Match is in scrollback — scroll up to show it at center.
                        let target_offset = scrollback_len - m.abs_row;
                        let grid = self.sessions[self.active]
                            .app_mut()
                            .terminal_mut()
                            .grid_mut();
                        let current_offset = grid.display_offset();
                        // Scroll up enough to make the match visible.
                        let desired_offset = target_offset
                            .min(scrollback_len)
                            .saturating_sub(grid_height / 3);
                        if desired_offset > current_offset {
                            grid.scroll_up_viewport(desired_offset - current_offset);
                        } else if desired_offset < current_offset {
                            grid.scroll_down_viewport(current_offset - desired_offset);
                        }
                    }
                }
            }
            PhysicalKey::Code(KeyCode::Backspace) => {
                search.backspace(grid);
            }
            PhysicalKey::Code(KeyCode::Tab) => {
                // Toggle case sensitivity.
                let grid2 = self.sessions[self.active].app().grid();
                self.search.toggle_case(grid2);
            }
            PhysicalKey::Code(KeyCode::ArrowUp) => {
                let grid2 = self.sessions[self.active].app().grid();
                self.search.history_prev(grid2);
            }
            PhysicalKey::Code(KeyCode::ArrowDown) => {
                let grid2 = self.sessions[self.active].app().grid();
                self.search.history_next(grid2);
            }
            _ => {
                if let Key::Character(s) = &event.logical_key
                    && let Some(c) = s.chars().next()
                    && !c.is_control()
                {
                    search.type_char(c, grid);
                }
            }
        }
    }

    // ── AI assistant (P10-C, ai feature) ──────────────────────────

    /// Trigger an AI request from the current terminal context.
    ///
    /// Convert pixel position to terminal cell coordinates.
    /// Compute the terminal content area in physical pixels.
    ///
    /// Accounts for tab bar height, status bar height, and content padding.
    /// Both the renderer and mouse handlers use this to ensure coordinates match.
    fn tab_font_size(&self) -> f32 {
        if let Some(ref renderer) = self.renderer {
            renderer.cell_height() as f32
        } else {
            28.0
        }
    }

    /// Get the full window width for tab bar layout calculations.
    /// MUST match the width used in render.rs for hit-testing to work.
    fn tab_layout_width(&self) -> f32 {
        if let Some(ref renderer) = self.renderer {
            renderer.resolution_width() as f32
        } else {
            self.config.cols as f32
        }
    }

    pub(super) fn content_area_bounds(&self) -> crate::splits::Rect {
        let cell_h = if let Some(ref renderer) = self.renderer {
            renderer.cell_height() as f32
        } else {
            self.config.cell_height
        };

        let (screen_w, screen_h) = if let Some(ref renderer) = self.renderer {
            (renderer.resolution_width(), renderer.resolution_height())
        } else {
            (self.config.cols as u32, self.config.rows as u32)
        };

        let tab_bar_h = if self.tab_bar.visible {
            ((cell_h + 8.0).max(28.0) + 6.0) as u32
        } else {
            0
        };
        let status_bar_h = if self.status_bar_visible {
            crate::desktop_config::STATUS_BAR_HEIGHT as u32
        } else {
            0
        };
        let pad = self.content_padding();

        let content_x = pad;
        let content_y = tab_bar_h + pad;
        let content_w = screen_w.saturating_sub(pad * 2);
        let content_h = screen_h
            .saturating_sub(tab_bar_h)
            .saturating_sub(status_bar_h)
            .saturating_sub(pad * 2);

        crate::splits::Rect::new(content_x, content_y, content_w, content_h)
    }

    pub(super) fn pixel_to_cell_pos(&self) -> (u16, u16) {
        // P18: Use actual renderer cell dimensions (DPI-aware, font-measured).
        let (cw, ch) = if let Some(ref renderer) = self.renderer {
            (renderer.cell_width() as f64, renderer.cell_height() as f64)
        } else {
            (
                self.config.cell_width as f64,
                self.config.cell_height as f64,
            )
        };

        // Subtract content area offset so cell coordinates are relative
        // to the pane's top-left, not the window's top-left.
        let bounds = self.content_area_bounds();
        let px = self.cursor_pos.0 - bounds.x as f64;
        let py = self.cursor_pos.1 - bounds.y as f64;
        crate::mouse::pixel_to_cell(px, py, cw, ch)
    }

    /// P20-D: Check if the cursor is over a different pane and switch focus.
    ///
    /// Returns `true` if focus changed (caller may want to redraw).
    pub(super) fn maybe_switch_pane_focus(&mut self) -> bool {
        let session = self.active_session();
        // Only relevant when there are multiple panes.
        if session.pane_count() <= 1 {
            return false;
        }
        // Skip when pane zoom is active (only active pane is visible).
        if self.pane_zoomed {
            return false;
        }

        // Use the same content area bounds as the renderer.
        let bounds = self.content_area_bounds();
        let (px, py) = (self.cursor_pos.0 as u32, self.cursor_pos.1 as u32);

        if let Some(hit_id) = session.split_tree().pane_at_point(px, py, bounds) {
            let active = session.split_tree().active();
            if hit_id != active {
                self.selection.clear();
                self.active_session_mut()
                    .split_tree_mut()
                    .set_active(hit_id);
                log::debug!("P20-D: pane focus → {hit_id}");
                return true;
            }
        }
        false
    }

    /// P21-A: Try to start dragging a split separator.
    ///
    /// Checks if cursor is near a separator line. If so, records the
    /// orientation and returns true (caller should skip normal mouse handling).
    pub(super) fn try_start_separator_drag(&mut self) -> bool {
        let session = self.active_session();
        if session.pane_count() <= 1 {
            return false;
        }
        // Skip when pane zoom is active.
        if self.pane_zoomed {
            return false;
        }

        let bounds = self.content_area_bounds();
        let (px, py) = (self.cursor_pos.0 as u32, self.cursor_pos.1 as u32);

        if let Some(orient) = session.split_tree().separator_at_point(px, py, bounds) {
            self.drag_resize = Some(orient);
            log::debug!("P21-A: separator drag started ({orient:?})");
            return true;
        }
        false
    }

    /// Handle winit MouseInput events (button press/release).
    pub(super) fn handle_mouse_input(
        &mut self,
        state: ElementState,
        button: winit::event::MouseButton,
    ) {
        // P21-A: Handle split separator drag.
        if button == winit::event::MouseButton::Left {
            if state == ElementState::Pressed {
                // Check if we're clicking on a separator.
                if self.try_start_separator_drag() {
                    return; // Don't process as pane click or selection
                }
            } else if state == ElementState::Released && self.drag_resize.is_some() {
                self.drag_resize = None;
                log::debug!("P21-A: separator drag ended");
                return;
            }
        }

        // P20-D: On left-click, switch pane focus to the pane under the cursor.
        if state == ElementState::Pressed
            && button == winit::event::MouseButton::Left
            && self.maybe_switch_pane_focus()
            && let Some(ref window) = self.window
        {
            window.request_redraw();
        }

        let mouse_button = match button {
            winit::event::MouseButton::Left => crate::mouse::MouseButton::Left,
            winit::event::MouseButton::Right => crate::mouse::MouseButton::Right,
            winit::event::MouseButton::Middle => crate::mouse::MouseButton::Middle,
            winit::event::MouseButton::Back => crate::mouse::MouseButton::Other(8),
            winit::event::MouseButton::Forward => crate::mouse::MouseButton::Other(16),
            winit::event::MouseButton::Other(n) => crate::mouse::MouseButton::Other(n as u8),
        };

        // P28: Tab bar right-click → open tab context menu.
        if state == ElementState::Pressed
            && button == winit::event::MouseButton::Right
            && self.tab_bar.visible
        {
            let (px, py) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
            let bounds = self.content_area_bounds();
            let tab_bar_h = bounds.y as f32;
            if py < tab_bar_h {
                let layout = self
                    .tab_bar
                    .compute_layout(self.tab_layout_width(), self.tab_font_size());
                if let Some(tab_idx) = self.tab_bar.tab_at_x(&layout, px) {
                    self.tab_context_menu.open(tab_idx, px, py);
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                    return;
                }
            }
        }

        // P28: Tab context menu item click or dismiss.
        if state == ElementState::Pressed && self.tab_context_menu.visible {
            let (px, py) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
            let hit_idx = self.tab_context_menu.hit_test(px, py);
            if let Some(idx) = hit_idx {
                let action = crate::tab_bar::TabMenuAction::all()[idx];
                self.execute_tab_menu_action(action);
            }
            self.tab_context_menu.close();
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
            if hit_idx.is_some() {
                return;
            }
        }

        let (col, row) = self.pixel_to_cell_pos();
        let mods = crate::mouse::MouseModifiers {
            shift: self.mods.shift,
            ctrl: self.mods.ctrl,
            alt: self.mods.alt,
        };

        // P30-A: Scrollbar click — start drag.
        if state == ElementState::Pressed
            && button == winit::event::MouseButton::Left
            && let Some(ref renderer) = self.renderer
        {
            let screen_w = renderer.resolution_width() as f32;
            let (px, py) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
            let bounds = self.content_area_bounds();
            if px >= screen_w - 12.0
                && px <= screen_w
                && py >= bounds.y as f32
                && py < (bounds.y + bounds.height) as f32
            {
                let scrollback_len = self.active_session().app().grid().scrollback_len();
                if scrollback_len > 0 {
                    self.scrollbar_drag = Some(py);
                    self.scroll_to_scrollbar_pos(py);
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                    return;
                }
            }
        }

        // P30-A: Scrollbar release — stop drag.
        if state == ElementState::Released && self.scrollbar_drag.is_some() {
            self.scrollbar_drag = None;
            return;
        }

        // P23-E: Tab drag release — stop dragging.
        if state == ElementState::Released && self.dragging_tab.is_some() {
            self.dragging_tab = None;
            return;
        }

        let term = self.active_session().app().terminal();

        // Check if mouse tracking is active.
        if term.mouse_tracking_enabled() {
            let sgr = term.mouse_sgr_enabled() || term.mouse_sgr_pixel_enabled();
            let sgr_pixel = term.mouse_sgr_pixel_enabled();
            let urxvt = term.mouse_urxvt_enabled();

            // For pixel mode, convert cell coords to pixel coords.
            let (mx, my) = if sgr_pixel && let Some(ref renderer) = self.renderer {
                (
                    (col as u32 * renderer.cell_width()) as u16,
                    (row as u32 * renderer.cell_height()) as u16,
                )
            } else {
                (col, row)
            };
            let mouse_ev = crate::mouse::MouseEvent {
                button: mouse_button,
                x: mx,
                y: my,
                mods,
            };

            match state {
                ElementState::Pressed => {
                    self.button_held = Some(mouse_button);
                    let bytes = if sgr_pixel {
                        crate::mouse::encode_mouse_event_pixel(&mouse_ev, mx, my, true)
                    } else {
                        crate::mouse::encode_mouse_event(&mouse_ev, sgr, urxvt, true)
                    };
                    if let Some(bytes) = bytes {
                        self.write_to_pty(&bytes);
                    }
                }
                ElementState::Released => {
                    self.button_held = None;
                    let bytes = if sgr_pixel {
                        crate::mouse::encode_mouse_event_pixel(&mouse_ev, mx, my, false)
                    } else {
                        crate::mouse::encode_mouse_event(&mouse_ev, sgr, urxvt, false)
                    };
                    if let Some(bytes) = bytes {
                        self.write_to_pty(&bytes);
                    }
                }
            }
            return;
        }
        match (mouse_button, state) {
            (crate::mouse::MouseButton::Left, ElementState::Pressed) => {
                // "+" dropdown menu dispatch.
                if self.new_tab_menu.visible {
                    let (px, py) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
                    if let Some(idx) = self.new_tab_menu.hit_test(px, py) {
                        self.execute_new_tab_menu_action(idx);
                    }
                    self.new_tab_menu.hide();
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                    return;
                }

                // P27-C: If context menu is open, handle item selection or close.
                if self.context_menu.visible {
                    let (px, py) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
                    if let Some(idx) = self.context_menu.hit_test(px, py) {
                        self.execute_context_menu_action(idx);
                    }
                    self.context_menu.hide();
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                    return;
                }

                self.button_held = Some(mouse_button);

                // P30-B: Tab bar left-click → switch tab.
                if self.tab_bar.visible {
                    let (px, py) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
                    let bounds = self.content_area_bounds();

                    // P32: Click scroll-to-bottom indicator → reset viewport.
                    if state == ElementState::Pressed
                        && button == winit::event::MouseButton::Left
                        && let Some(ref renderer) = self.renderer
                    {
                        let sw = renderer.resolution_width() as f32;
                        let sh = renderer.resolution_height() as f32;
                        let ix = sw - 80.0;
                        let iy = sh - 64.0;
                        if px >= ix && px <= ix + 60.0 && py >= iy && py <= iy + 24.0 {
                            self.active_session_mut()
                                .app_mut()
                                .terminal_mut()
                                .grid_mut()
                                .reset_viewport();
                            self.smooth_scroll.reset();
                            if let Some(ref window) = self.window {
                                window.request_redraw();
                            }
                            return;
                        }
                    }

                    if py < bounds.y as f32 {
                        let layout = self
                            .tab_bar
                            .compute_layout(self.tab_layout_width(), self.tab_font_size());
                        // New tab button (+) → open dropdown menu below button.
                        if self.tab_bar.is_new_tab_button_at(&layout, px, py) {
                            let btn = &layout.new_tab_button;
                            let menu_w = crate::new_tab_menu::NewTabMenuState::WIDTH;
                            // Right-align: menu's right edge = button's right edge,
                            // but clamp so it never goes off-screen on the left.
                            let mut menu_x = btn.cx + btn.size / 2.0 - menu_w;
                            if menu_x < 4.0 {
                                menu_x = 4.0;
                            }
                            let menu_y = btn.cy + btn.size / 2.0 + 2.0;
                            self.new_tab_menu.toggle(menu_x, menu_y);
                            if let Some(ref window) = self.window {
                                window.request_redraw();
                            }
                            return;
                        }
                        // Settings gear button → open settings window.
                        if self.tab_bar.is_settings_button_at(&layout, px, py) {
                            self.pending_open_settings = true;
                            return;
                        }
                        // Linux/Windows: window control buttons (minimize/maximize/close).
                        #[cfg(not(target_os = "macos"))]
                        {
                            let ctrl_layout = crate::titlebar::x11::compute_layout(
                                self.tab_layout_width(),
                                self.tab_font_size(),
                            );
                            if let Some(btn) = crate::titlebar::x11::hit_test(&ctrl_layout, px, py)
                            {
                                use crate::titlebar::WindowControlButton;
                                match btn {
                                    WindowControlButton::Close => {
                                        if let Some(ref window) = self.window {
                                            window.close();
                                        }
                                    }
                                    WindowControlButton::Minimize => {
                                        if let Some(ref window) = self.window {
                                            window.set_minimized(true);
                                        }
                                    }
                                    WindowControlButton::Maximize => {
                                        self.maximized = !self.maximized;
                                        if let Some(ref window) = self.window {
                                            window.set_maximized(self.maximized);
                                        }
                                    }
                                }
                                return;
                            }
                        }
                        // Close button (x) on a tab.
                        if let Some(tab_idx) = self.tab_bar.tab_at_x(&layout, px) {
                            if tab_idx < layout.tabs.len() {
                                let cb = &layout.tabs[tab_idx].close;
                                let hit = cb.size / 2.0 + 4.0; // generous touch target
                                let dx = px - cb.cx;
                                let dy = py - cb.cy;
                                if dx.abs() <= hit && dy.abs() <= hit && self.sessions.len() > 1 {
                                    self.close_tab();
                                    return;
                                }
                            }
                            self.switch_tab(tab_idx);
                            // P30-B: Double-click → start rename.
                            if self.click_count == 2 {
                                self.renaming_tab = Some(tab_idx);
                                self.rename_text = self.sessions[tab_idx].title().to_string();
                            }
                            // P23-E: Single click → start tab drag for reordering.
                            if self.click_count == 1 && self.sessions.len() > 1 {
                                self.dragging_tab = Some(tab_idx);
                            }
                            return;
                        }
                        // Empty area of tab bar → start window drag,
                        // or maximize on double-click (macOS convention).
                        if self.click_count == 2 {
                            self.maximized = !self.maximized;
                            if let Some(ref window) = self.window {
                                window.set_maximized(self.maximized);
                            }
                            return;
                        }
                        if let Some(ref window) = self.window {
                            let _ = window.drag_window();
                        }
                        return;
                    }
                }

                // P27-B: Double-click / triple-click detection.
                let now = std::time::Instant::now();
                let is_multi_click = self
                    .last_click_time
                    .is_some_and(|t| now.duration_since(t).as_millis() < 400)
                    && self.last_click_pos == (col, row);

                if is_multi_click {
                    self.click_count = (self.click_count % 3) + 1; // cycle 1→2→3→1
                } else {
                    self.click_count = 1;
                }
                self.last_click_time = Some(now);
                self.last_click_pos = (col, row);

                match self.click_count {
                    2 => {
                        // Double-click: select word at position.
                        self.select_word_at(col, row);
                    }
                    3 => {
                        // Triple-click: select entire line.
                        self.select_line_at(row);
                    }
                    _ => {
                        // Single click: start or extend selection.
                        // Close search bar and command palette (clicking away dismisses them).
                        if self.search.visible {
                            self.search.close();
                        }
                        if self.command_palette.visible {
                            self.command_palette.visible = false;
                        }
                        if self.context_menu.visible {
                            self.context_menu.hide();
                        }
                        if self.tab_context_menu.visible {
                            self.tab_context_menu.close();
                        }
                        if self.mods.shift && self.selection.start.is_some() {
                            // Shift+Click: extend existing selection to this point.
                            self.selection.extend(col, row);
                        } else if self.mods.alt {
                            // Alt+Click: start block (rectangular) selection.
                            self.selection.start_block(col, row);
                        } else {
                            // Normal click: start new selection.
                            self.selection.start(col, row);
                        }
                    }
                }
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            (crate::mouse::MouseButton::Left, ElementState::Released) => {
                // P17-C: Cmd+Click (macOS) or Ctrl+Click (other) opens hovered URL.
                let open_link = (cfg!(target_os = "macos") && self.mods.super_key)
                    || (!cfg!(target_os = "macos") && self.mods.ctrl);
                if open_link {
                    // First try OSC 8 hyperlink or detected URL.
                    if let Some((url, _, _, _)) = self.hovered_link.take() {
                        crate::mouse::open_url(&url);
                        return;
                    }
                    // Then try file path detection (compiler error lines).
                    let grid = self.sessions[self.active].app().grid();
                    if let Some(display_row) = grid.display_row(row as usize) {
                        let line_text: String = display_row.cells.iter().map(|c| c.ch).collect();
                        if let Some(path) = crate::mouse::find_file_path(&line_text, col as usize) {
                            crate::mouse::open_file_path(&path);
                            self.show_toast(format!("Opening: {}", path));
                            return;
                        }
                    }
                }

                self.button_held = None;
                self.selection.finish();
                self.selection_auto_scroll = 0;
                // Copy selection to clipboard if active.
                if self.selection.is_active() {
                    self.copy_selection_to_clipboard();
                }
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            (crate::mouse::MouseButton::Middle, ElementState::Pressed) => {
                // Middle-click on tab closes it (like browsers).
                // Otherwise, middle-click paste from system clipboard.
                let (px, py) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
                let bounds = self.content_area_bounds();
                if py < bounds.y as f32 && self.tab_bar.visible {
                    let layout = self
                        .tab_bar
                        .compute_layout(self.tab_layout_width(), self.tab_font_size());
                    if let Some(tab_idx) = self.tab_bar.tab_at_x(&layout, px)
                        && self.sessions.len() > 1
                    {
                        // Switch to the tab first, then close it.
                        self.switch_tab(tab_idx);
                        self.close_tab();
                        return;
                    }
                    // Middle-click on empty tab bar area (not on a tab)
                    // opens a new tab (browser-style).
                    if self.tab_bar.is_new_tab_button_at(&layout, px, py) {
                        self.open_tab();
                        return;
                    }
                }
                self.paste_from_clipboard();
            }
            (crate::mouse::MouseButton::Right, ElementState::Pressed) => {
                // P27-C: Show context menu at mouse position.
                let (px, py) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
                self.context_menu.show(px, py);
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    /// Handle cursor motion — extend selection or report mouse motion.
    pub(super) fn handle_cursor_moved(&mut self) {
        // P30-A: Scrollbar drag — scroll to cursor Y position.
        if self.scrollbar_drag.is_some() {
            let py = self.cursor_pos.1 as f32;
            self.scroll_to_scrollbar_pos(py);
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
            return;
        }

        // P23-E: Tab drag — reorder tabs as cursor moves between tab positions.
        if let Some(drag_idx) = self.dragging_tab
            && self.tab_bar.visible
        {
            let px = self.cursor_pos.0 as f32;
            let layout = self
                .tab_bar
                .compute_layout(self.tab_layout_width(), self.tab_font_size());
            if let Some(target_idx) = self.tab_bar.tab_at_x(&layout, px)
                && target_idx != drag_idx
            {
                self.move_tab(drag_idx, target_idx);
                self.dragging_tab = Some(target_idx);
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
                return;
            }
        }

        // P27-C: Update context menu hover state.
        if self.context_menu.visible {
            let (px, py) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
            self.context_menu.hovered = self.context_menu.hit_test(px, py);
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
        }
        // P21-A: If dragging a separator, adjust the ratio.
        if self.drag_resize.is_some() {
            let (px, py) = (self.cursor_pos.0 as u32, self.cursor_pos.1 as u32);
            let screen_w = self
                .renderer
                .as_ref()
                .map(|r| r.resolution_width())
                .unwrap_or(0);
            let screen_h = self
                .renderer
                .as_ref()
                .map(|r| r.resolution_height())
                .unwrap_or(0);
            let bounds = crate::splits::Rect::new(0, 0, screen_w, screen_h);
            let active = self.active;
            let tree = self.sessions[active].split_tree_mut();
            tree.set_ratio_at_point(px, py, bounds);
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
            return; // Skip normal cursor handling while dragging separator
        }

        let (col, row) = self.pixel_to_cell_pos();

        let (any_event, button_event) = {
            let term = self.active_session().app().terminal();
            (
                term.mouse_any_event_enabled(),
                term.mouse_button_event_enabled(),
            )
        };

        // If mouse motion tracking is on, report motion.
        if any_event || button_event {
            let held = self.button_held.is_some();
            if crate::mouse::should_report_motion(any_event, button_event, held) {
                let button = self.button_held.unwrap_or(crate::mouse::MouseButton::Left);
                let mods = crate::mouse::MouseModifiers {
                    shift: self.mods.shift,
                    ctrl: self.mods.ctrl,
                    alt: self.mods.alt,
                };
                let mouse_ev = crate::mouse::MouseEvent {
                    button,
                    x: col,
                    y: row,
                    mods,
                };

                let sgr = {
                    let term = self.active_session().app().terminal();
                    term.mouse_sgr_enabled() || term.mouse_sgr_pixel_enabled()
                };
                if sgr {
                    let bytes = crate::mouse::encode_sgr_motion(&mouse_ev, held);
                    self.write_to_pty(bytes.as_bytes());
                }
                return;
            }
        }

        // Extend selection while dragging.
        if self.selection.dragging {
            // Set auto-scroll direction based on proximity to edges.
            // The actual scrolling happens in about_to_wait via a timer.
            let bounds = self.content_area_bounds();
            let py = self.cursor_pos.1 as f32;
            let top_y = bounds.y as f32;
            let bottom_y = (bounds.y + bounds.height) as f32;
            let edge_zone = 40.0;

            if py <= top_y + edge_zone {
                self.selection_auto_scroll = -1;
            } else if py >= bottom_y - edge_zone {
                self.selection_auto_scroll = 1;
            } else {
                self.selection_auto_scroll = 0;
            }

            self.selection.extend(col, row);
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
        }

        // P17-C: Detect hovered URL (OSC 8 hyperlink or plain text).
        self.update_hovered_link(col, row);

        // P28-B: Update color picker hover state.
        self.update_color_picker_hover(col, row);

        // P28-F: Feed mouse position to cursor particle system.
        let (px, py) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
        self.cursor_particles.update_cursor(px, py);
        if self.cursor_particles.needs_render()
            && let Some(ref window) = self.window
        {
            window.request_redraw();
        }

        // Update system mouse cursor icon based on context.
        if let Some(ref window) = self.window {
            use winit::window::CursorIcon;
            let py = self.cursor_pos.1 as f32;
            let content_top = if self.tab_bar.visible { 30.0 } else { 0.0 };
            // Check if hovering over a pane separator.
            let on_separator = if let Some(is_horizontal) = self.drag_resize {
                Some(is_horizontal)
            } else if !self.pane_zoomed {
                let session = self.active_session();
                if session.pane_count() > 1 {
                    let bounds = self.content_area_bounds();
                    let (px, py) = (self.cursor_pos.0 as u32, self.cursor_pos.1 as u32);
                    session.split_tree().separator_at_point(px, py, bounds)
                } else {
                    None
                }
            } else {
                None
            };
            let icon = if let Some(is_horizontal) = on_separator {
                if is_horizontal {
                    CursorIcon::ColResize
                } else {
                    CursorIcon::RowResize
                }
            } else if self.hovered_link.is_some() {
                CursorIcon::Pointer
            } else if py > content_top || self.selection.dragging {
                CursorIcon::Text
            } else {
                CursorIcon::Default
            };
            window.set_cursor(icon);
        }
    }

    /// P17-C: Update `hovered_link` based on the cell under the cursor.
    ///
    /// Checks for OSC 8 hyperlinks first, then falls back to plain-text URL
    /// detection in the row's text content.
    pub(super) fn update_hovered_link(&mut self, col: u16, row: u16) {
        let col = col as usize;
        let row = row as usize;
        let grid = &self.sessions[self.active].app().grid();

        // Try OSC 8 hyperlink on the cell at (col, row).
        if let Some(cell_row) = grid.display_row(row)
            && col < cell_row.cells.len()
        {
            let cell = &cell_row.cells[col];
            if let Some(ref link) = cell.hyperlink {
                // Find extent of hyperlink cells in this row.
                let mut start = col;
                while start > 0 && cell_row.cells[start - 1].hyperlink.as_deref() == Some(link) {
                    start -= 1;
                }
                let mut end = col + 1;
                while end < cell_row.cells.len()
                    && cell_row.cells[end].hyperlink.as_deref() == Some(link)
                {
                    end += 1;
                }
                self.hovered_link = Some((link.clone(), start, end, row));
                return;
            }
        }

        // Fall back to plain-text URL detection.
        if let Some(cell_row) = grid.display_row(row) {
            let line: String = cell_row.cells.iter().map(|c| c.ch).collect();
            if let Some((start, end, url)) = crate::mouse::detect_url_at_position(&line, col) {
                self.hovered_link = Some((url, start, end, row));
                return;
            }
        }

        self.hovered_link = None;
    }

    /// P28-B: Update color picker hover state based on the cell under cursor.
    /// Scans the current row for hex/rgb color codes and checks if the cursor
    /// is hovering over one.
    pub(super) fn update_color_picker_hover(&mut self, col: u16, row: u16) {
        let col = col as usize;
        let row = row as usize;
        let grid = &self.sessions[self.active].app().grid();

        if let Some(cell_row) = grid.display_row(row) {
            let line: String = cell_row.cells.iter().map(|c| c.ch).collect();
            let mut matches = crate::color_picker::scan_line_for_colors(&line);
            for m in &mut matches {
                m.row = row;
            }
            if let Some(found) = crate::color_picker::find_color_at(&matches, col, row) {
                let hovered = found.clone();
                self.color_picker.hovered = Some(hovered);
                return;
            }
        }

        self.color_picker.clear();
    }

    /// P27-B: Select the word at the given cell position.
    ///
    /// A "word" is a run of non-whitespace characters. Finds the word
    /// boundaries by scanning left and right from the clicked cell.
    pub(super) fn select_word_at(&mut self, col: u16, row: u16) {
        let grid = &self.sessions[self.active].app().grid();
        let col_u = col as usize;

        // Get the display row to scan characters.
        let Some(display_row) = grid.display_row(row as usize) else {
            return;
        };

        // Character classification for word boundary detection.
        // Modern terminals (iTerm2, Alacritty, WezTerm) treat common path
        // and URL characters as word characters so that double-clicking
        // selects entire paths like /usr/local/bin or URLs like https://example.com
        let char_class = |c: char| -> u8 {
            if c.is_alphanumeric() || c == '_' {
                0 // word chars (letters, digits, underscore)
            } else if c.is_whitespace() {
                2 // whitespace separator
            } else if matches!(
                c,
                // Path and URL characters that should be part of words
                '.' | '-' | '/' | ':' | '@' | '~' | '+' | '#' | '?' | '=' | '&' | '%' | '$'
            ) {
                0 // path/URL chars — treat as word chars
            } else {
                1 // other punctuation/symbols
            }
        };

        // If the clicked cell is whitespace, select just that cell.
        let cells: Vec<char> = display_row.cells.iter().map(|c| c.ch).collect();
        if col_u >= cells.len() || char_class(cells[col_u]) == 2 {
            self.selection.start(col, row);
            self.selection.extend(col, row);
            self.selection.finish();
            return;
        }

        // Scan left for word start — stop at different char class.
        let target_class = char_class(cells[col_u]);
        let mut start = col_u;
        while start > 0 && char_class(cells[start - 1]) == target_class {
            start -= 1;
        }

        // Scan right for word end.
        let mut end = col_u;
        while end + 1 < cells.len() && char_class(cells[end + 1]) == target_class {
            end += 1;
        }

        self.selection.start(start as u16, row);
        self.selection.extend(end as u16, row);
        self.selection.finish();
    }

    /// P27-B: Select the entire line at the given row.
    pub(super) fn select_line_at(&mut self, row: u16) {
        let grid = &self.sessions[self.active].app().grid();
        let width = grid.width() as u16;
        if width == 0 {
            return;
        }
        self.selection.start(0, row);
        self.selection.extend(width - 1, row);
        self.selection.finish();
    }

    /// P27-C: Execute a context menu action by index.
    fn execute_context_menu_action(&mut self, index: usize) {
        let actions = crate::context_menu::ContextMenuAction::all();
        if index >= actions.len() {
            return;
        }
        match actions[index] {
            crate::context_menu::ContextMenuAction::Copy => {
                self.copy_selection_to_clipboard();
            }
            crate::context_menu::ContextMenuAction::Paste => {
                self.paste_from_clipboard();
            }
            crate::context_menu::ContextMenuAction::SelectAll => {
                let grid = &self.sessions[self.active].app().grid();
                let width = grid.width() as u16;
                let height = grid.height() as u16;
                if width > 0 && height > 0 {
                    self.selection.start(0, 0);
                    self.selection.extend(width - 1, height - 1);
                    self.selection.finish();
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                }
            }
            crate::context_menu::ContextMenuAction::Search => {
                self.search.toggle();
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            crate::context_menu::ContextMenuAction::SplitHorizontal => {
                self.split_pane_horizontal();
            }
            crate::context_menu::ContextMenuAction::SplitVertical => {
                self.split_pane_vertical();
            }
            crate::context_menu::ContextMenuAction::Clear => {
                // Clear screen + scrollback, then send Ctrl+L equivalent
                // (clear command) so the shell redraws a fresh prompt.
                crate::terminal_actions::clear_screen_and_scrollback(
                    self.active_session_mut().app_mut().grid_mut(),
                );
                // Send a newline + clear sequence to get a fresh prompt.
                self.write_to_pty(b"\x1b[H\x1b[2J\x0c");
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            crate::context_menu::ContextMenuAction::Reset => {
                // Full reinit: restart the shell process entirely.
                self.active_session_mut().restart_active_shell();
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
        }
    }

    /// Handle mouse wheel events — scroll scrollback or report to PTY.
    pub(super) fn handle_mouse_wheel(&mut self, delta: winit::event::MouseScrollDelta) {
        // Ctrl+Shift+Wheel → zoom font size (VS Code / iTerm2 style).
        if self.mods.ctrl && self.mods.shift && !self.mods.alt {
            let y = match delta {
                winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                winit::event::MouseScrollDelta::PixelDelta(pos) => (pos.y as f32 / 16.0).round(),
            };
            let changed = if y > 0.0 {
                self.font_zoom.zoom_in()
            } else {
                self.font_zoom.zoom_out()
            };
            if changed {
                self.apply_font_size();
                self.show_toast(format!("{:.0}px", self.font_zoom.current_size()));
            }
            return;
        }

        // P20-D: Route wheel events to the pane under the cursor.
        if self.maybe_switch_pane_focus()
            && let Some(ref window) = self.window
        {
            window.request_redraw();
        }

        let (col, row) = self.pixel_to_cell_pos();
        let mods = crate::mouse::MouseModifiers {
            shift: self.mods.shift,
            ctrl: self.mods.ctrl,
            alt: self.mods.alt,
        };

        let term = self.active_session().app().terminal();

        // When mouse tracking is on, send wheel as button events.
        if term.mouse_tracking_enabled() {
            let sgr = term.mouse_sgr_enabled();
            let urxvt = term.mouse_urxvt_enabled();

            let (dx, dy) = match delta {
                winit::event::MouseScrollDelta::LineDelta(x, y) => (x as i32, -(y as i32)),
                winit::event::MouseScrollDelta::PixelDelta(pos) => {
                    let x = (pos.x as f32 / 8.0).round() as i32;
                    let y = (pos.y as f32 / 16.0).round() as i32;
                    (x, -y)
                }
            };

            // Each scroll line = one wheel event.
            for _ in 0..dy.abs() {
                let button = if dy > 0 {
                    crate::mouse::MouseButton::WheelUp
                } else {
                    crate::mouse::MouseButton::WheelDown
                };
                let ev = crate::mouse::MouseEvent {
                    button,
                    x: col,
                    y: row,
                    mods,
                };
                if let Some(bytes) = crate::mouse::encode_mouse_event(&ev, sgr, urxvt, true) {
                    self.write_to_pty(&bytes);
                }
            }

            // Horizontal scroll.
            for _ in 0..dx.abs() {
                let button = if dx > 0 {
                    crate::mouse::MouseButton::WheelRight
                } else {
                    crate::mouse::MouseButton::WheelLeft
                };
                let ev = crate::mouse::MouseEvent {
                    button,
                    x: col,
                    y: row,
                    mods,
                };
                if let Some(bytes) = crate::mouse::encode_mouse_event(&ev, sgr, urxvt, true) {
                    self.write_to_pty(&bytes);
                }
            }
            return;
        }

        // Mouse tracking OFF — feed smooth scroller.
        let cell_h = if let Some(ref renderer) = self.renderer {
            renderer.cell_height() as f32
        } else {
            self.config.cell_height
        };

        match delta {
            winit::event::MouseScrollDelta::LineDelta(_x, y) => {
                // Line-based scroll (mouse wheel): add integer lines.
                let lines = -(y as i32); // up = positive
                self.smooth_scroll.add_lines(lines);
            }
            winit::event::MouseScrollDelta::PixelDelta(pos) => {
                // Pixel-based scroll (trackpad): precise with momentum.
                self.smooth_scroll.add_pixels(-(pos.y as f32), cell_h);
            }
        }

        // Process smooth scroll immediately on this frame.
        if let Some(delta_lines) = self.smooth_scroll.tick() {
            // P29-B: Shift+wheel → scroll all panes in the active tab.
            if self.mods.shift && self.active_session().split_tree().pane_count() > 1 {
                self.active_session_mut()
                    .scroll_all_panes_viewport(delta_lines);
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
                return;
            }

            let grid = self
                .active_session_mut()
                .app_mut()
                .terminal_mut()
                .grid_mut();
            if delta_lines > 0 {
                grid.scroll_up_viewport(delta_lines as usize);
            } else {
                grid.scroll_down_viewport((-delta_lines) as usize);
            }
        }

        if let Some(ref window) = self.window {
            window.request_redraw();
        }
    }
}
