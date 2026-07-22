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

        // Get actual cols/rows from renderer — but adjust for content area
        // (tab bar, status bar, padding). The renderer reports raw surface
        // cols/rows, which overcounts because the terminal content doesn't
        // start at (0,0).
        let (new_cols, new_rows) = {
            let r = self.renderer.as_ref();
            let raw_cols = r.map(|r| r.cols() as u16).unwrap_or(80).max(10);
            let raw_rows = r.map(|r| r.rows() as u16).unwrap_or(24).max(3);

            let cell_w = r.map(|r| r.cell_width()).unwrap_or(8);
            let cell_h = r.map(|r| r.cell_height()).unwrap_or(16);

            let bounds = self.content_area_bounds();
            let adj_cols = ((bounds.width / cell_w.max(1)) as u16).max(10);
            let adj_rows = ((bounds.height / cell_h.max(1)) as u16).max(3);
            (adj_cols.min(raw_cols), adj_rows.min(raw_rows))
        };

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

        // Clear selection — old coordinates may be outside the new grid.
        self.selection.clear();
        self.selection_auto_scroll = 0;

        // Show a brief size indicator toast.
        self.show_toast(format!("{}x{}", new_cols, new_rows));

        true
    }

    /// Handle a winit key event using the existing keymap module.
    pub(super) fn handle_keyboard_input(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }

        // --hold mode: when the hold message is shown, any key closes the window.
        if self.hold_message_shown {
            self.should_quit = true;
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

        // Close P2P share overlay on Escape.
        #[cfg(feature = "p2p")]
        if self.p2p_share.visible
            && let PhysicalKey::Code(KeyCode::Escape) = &event.physical_key
        {
            // If connected, just hide the overlay but keep the connection.
            if self.p2p_share.status == crate::p2p_share::P2pShareStatus::Connected {
                self.p2p_share.visible = false;
                self.show_toast("P2P: Sharing active in background (Esc to hide)");
            } else {
                // Not connected — stop sharing entirely.
                self.toggle_p2p_share();
            }
            return;
        }

        // Scrollback browse mode: toggle with Ctrl+Shift+Space.
        // When active, vim-style keys navigate scrollback instead of going to PTY.
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::Space) = &event.physical_key
        {
            self.scroll_mode = !self.scroll_mode;
            if self.scroll_mode {
                self.show_toast("Scroll mode: j/k scroll, G/g jump, q/Esc exit");
            } else {
                self.show_toast("Exited scroll mode");
            }
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
            return;
        }

        // Scrollback browse mode navigation.
        if self.scroll_mode && !self.mods.ctrl {
            let grid_h = self.active_session().app().grid().height();
            let scrollback_len = self.active_session().app().grid().scrollback_len();
            let mut handled = true;
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::KeyJ) | PhysicalKey::Code(KeyCode::ArrowDown) => {
                    self.active_session_mut()
                        .app_mut()
                        .grid_mut()
                        .scroll_down_viewport(1);
                }
                PhysicalKey::Code(KeyCode::KeyK) | PhysicalKey::Code(KeyCode::ArrowUp) => {
                    self.active_session_mut()
                        .app_mut()
                        .grid_mut()
                        .scroll_up_viewport(1);
                }
                PhysicalKey::Code(KeyCode::KeyG) => {
                    if self.mods.shift {
                        // Shift+G → jump to top (oldest)
                        self.active_session_mut()
                            .app_mut()
                            .grid_mut()
                            .scroll_up_viewport(scrollback_len);
                    } else {
                        // g → jump to bottom (newest)
                        self.active_session_mut()
                            .app_mut()
                            .grid_mut()
                            .reset_viewport();
                    }
                }
                PhysicalKey::Code(KeyCode::KeyU) => {
                    // u → half page up
                    self.active_session_mut()
                        .app_mut()
                        .grid_mut()
                        .scroll_up_viewport(grid_h / 2);
                }
                PhysicalKey::Code(KeyCode::KeyD) => {
                    // d → half page down
                    self.active_session_mut()
                        .app_mut()
                        .grid_mut()
                        .scroll_down_viewport(grid_h / 2);
                }
                PhysicalKey::Code(KeyCode::PageUp) => {
                    self.active_session_mut()
                        .app_mut()
                        .grid_mut()
                        .scroll_up_viewport(grid_h);
                }
                PhysicalKey::Code(KeyCode::PageDown) => {
                    self.active_session_mut()
                        .app_mut()
                        .grid_mut()
                        .scroll_down_viewport(grid_h);
                }
                PhysicalKey::Code(KeyCode::Space) => {
                    // Space → page down
                    self.active_session_mut()
                        .app_mut()
                        .grid_mut()
                        .scroll_down_viewport(grid_h);
                }
                PhysicalKey::Code(KeyCode::KeyB) => {
                    // b → page up
                    self.active_session_mut()
                        .app_mut()
                        .grid_mut()
                        .scroll_up_viewport(grid_h);
                }
                PhysicalKey::Code(KeyCode::KeyQ) | PhysicalKey::Code(KeyCode::Escape) => {
                    // q or Esc → exit scroll mode
                    self.scroll_mode = false;
                    self.show_toast("Exited scroll mode");
                }
                _ => {
                    // Any other key → exit scroll mode and fall through to normal processing.
                    self.scroll_mode = false;
                    handled = false;
                }
            }
            if handled {
                self.smooth_scroll.reset();
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
                return;
            }
        }

        // Block all keyboard input when a confirmation dialog is active.
        // Only Enter (confirm) and Escape (cancel) are processed.
        let has_pending_dialog =
            self.pending_large_paste.is_some() || self.pending_close_tab.is_some();

        // Cancel pending large paste on Escape.
        if has_pending_dialog
            && let PhysicalKey::Code(KeyCode::Escape) = &event.physical_key
            && !event.repeat
        {
            self.pending_large_paste = None;
            self.pending_close_tab = None;
            self.show_toast("Cancelled".to_string());
            return;
        }

        // Confirm pending large paste on Enter.
        if self.pending_large_paste.is_some()
            && let PhysicalKey::Code(KeyCode::Enter) = &event.physical_key
            && !event.repeat
        {
            self.paste_from_source(crate::clipboard::PasteSource::Confirmed);
            return;
        }

        // Confirm pending close tab on Enter or repeat close_tab shortcut.
        if self.pending_close_tab.is_some()
            && let PhysicalKey::Code(KeyCode::Enter) = &event.physical_key
            && !event.repeat
            && let Some(idx) = self.pending_close_tab.take()
        {
            self.pending_close_tab = None;
            if idx < self.sessions.len() {
                self.switch_tab(idx);
                self.close_tab();
            }
            return;
        }

        // Swallow all other keys while a dialog is active.
        if has_pending_dialog {
            return;
        }

        // Escape clears text selection (when no overlays are open).
        if !self.mods.ctrl
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::Escape) = &event.physical_key
            && self.selection.is_active()
        {
            self.selection.clear();
            self.selection_auto_scroll = 0;
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
            return;
        }

        // ── P14-D: Config-driven keybinding dispatch ──
        // All configurable actions are resolved through check_keybinding().
        // The resolved_keybindings map is populated from ConfigManager at
        // startup and falls back to default_keybindings() when no config exists.
        // Skip key-repeat for action shortcuts to prevent rapid-fire
        // (e.g. holding Ctrl+T creating dozens of tabs).
        let is_repeat = event.repeat;
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
                if !is_repeat {
                    self.open_tab();
                }
                return;
            }
            // Ctrl+Shift+T → reopen last closed tab (also Cmd+Shift+T on macOS)
            if (self.mods.ctrl && self.mods.shift && key_name == "t")
                || (cfg!(target_os = "macos")
                    && self.mods.super_key
                    && self.mods.shift
                    && key_name == "t")
            {
                if !is_repeat {
                    self.reopen_closed_tab();
                }
                return;
            }
            // Ctrl+Shift+N → open new ggterm window (also Cmd+Shift+N on macOS)
            if (self.mods.ctrl && self.mods.shift && key_name == "n")
                || (cfg!(target_os = "macos")
                    && self.mods.super_key
                    && self.mods.shift
                    && key_name == "n")
            {
                if !is_repeat {
                    self.open_new_window();
                }
                return;
            }
            // Ctrl+Shift+Alt+D → duplicate active tab
            if self.mods.ctrl && self.mods.shift && self.mods.alt && key_name == "d" {
                if !is_repeat {
                    self.duplicate_tab();
                }
                return;
            }
            // Ctrl+Shift+Alt+W → close all other tabs
            if self.mods.ctrl && self.mods.shift && self.mods.alt && key_name == "w" {
                if !is_repeat {
                    self.close_other_tabs();
                }
                return;
            }
            // Ctrl+Shift+Alt+Q → toggle P2P terminal sharing
            #[cfg(feature = "p2p")]
            if self.mods.ctrl && self.mods.shift && self.mods.alt && key_name == "q" {
                if !is_repeat {
                    self.toggle_p2p_share();
                }
                return;
            }
            // Ctrl+Shift+Alt+L → manually reload config from file
            if self.mods.ctrl && self.mods.shift && self.mods.alt && key_name == "l" {
                self.reload_configuration();
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
                if !is_repeat {
                    if self.active_session().pane_count() > 1 {
                        // Multiple panes: close the active pane instead of the tab.
                        self.show_toast("Pane closed");
                        self.active_session_mut().remove_active_pane();
                    } else {
                        self.close_tab();
                    }
                }
                return;
            }
            // Ctrl+= → zoom in (also Cmd+= on macOS, also Ctrl++ for keyboards that need Shift)
            if self.check_keybinding(
                "zoom_in",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) || (cfg!(target_os = "macos") && self.mods.super_key && key_name == "=")
                || (self.mods.ctrl && self.mods.shift && key_name == "+")
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
            // Alt+Enter → fullscreen (common alternate shortcut, matches xterm/iTerm2)
            if self.mods.alt && !self.mods.ctrl && !self.mods.shift && key_name == "enter" {
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
            // Ctrl+Shift+Alt+V → paste and execute (paste + Enter)
            if self.mods.ctrl && self.mods.shift && self.mods.alt && key_name == "v" {
                self.paste_and_run();
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
            // Ctrl+Shift+Alt+K → clear scrollback only (keep visible screen)
            if self.mods.ctrl && self.mods.shift && self.mods.alt && key_name == "k" {
                crate::terminal_actions::clear_scrollback_only(
                    self.active_session_mut().app_mut().grid_mut(),
                );
                self.show_toast("Scrollback cleared".to_string());
                return;
            }
            // Ctrl+Shift+Alt+H → copy selection as HTML (rich text with colors)
            if self.mods.ctrl && self.mods.shift && self.mods.alt && key_name == "h" {
                self.copy_selection_as_html();
                return;
            }
            // Ctrl+Shift+Alt+C → copy current working directory path to clipboard
            if self.mods.ctrl && self.mods.shift && self.mods.alt && key_name == "c" {
                self.execute_command_palette_action("terminal.copy_cwd_path");
                return;
            }
            // Ctrl+Shift+J → open shell rc file (.zshrc/.bashrc) in editor
            if self.mods.ctrl && self.mods.shift && !self.mods.alt && key_name == "j" {
                self.open_shell_config();
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
                if is_repeat {
                    return;
                }
                // If there's selected text, pre-fill the search query
                // with the first line of the selection.
                if !self.search.visible
                    && self.selection.is_active()
                    && let Some(((sx, sy), (ex, ey))) = self.selection.normalized()
                    && sy == ey
                {
                    // Single-line selection — use as search query.
                    let grid = self.active_session().app().grid();
                    let mut text = String::new();
                    for x in sx..=ex {
                        if let Some(cell) = grid.display_cell(x as usize, sy as usize)
                            && !cell.is_wide_spacer()
                        {
                            text.push(cell.ch);
                        }
                    }
                    let text = text.trim().to_string();
                    if !text.is_empty() {
                        self.search.query = text;
                    }
                }
                self.search.toggle();
                return;
            }
        }

        // ── F3 / Shift+F3 → continue search (next/prev match) ──
        if let PhysicalKey::Code(KeyCode::F3) = &event.physical_key {
            if is_repeat {
                return;
            }
            if self.search.visible {
                // Search bar open: just navigate.
                let matched = if self.mods.shift {
                    self.search.prev_match()
                } else {
                    self.search.next_match()
                };
                self.scroll_to_search_match(matched);
            } else {
                // Search bar closed: restore last query and navigate.
                // Disjoint borrow avoids cloning the entire grid.
                let grid = self.sessions[self.active].app().grid();
                if self.search.resume_from_last(grid) {
                    let matched = if self.mods.shift {
                        self.search.prev_match()
                    } else {
                        self.search.next_match()
                    };
                    self.scroll_to_search_match(matched);
                }
            }
            return;
        }

        // ── P19-B: Split pane shortcuts (not configurable) ──

        // Ctrl+Shift+D → horizontal split (also Cmd+D on macOS)
        if let PhysicalKey::Code(KeyCode::KeyD) = &event.physical_key {
            let ctrl_shift = self.mods.ctrl && self.mods.shift && !self.mods.alt;
            let cmd = cfg!(target_os = "macos") && self.mods.super_key && !self.mods.shift;
            if ctrl_shift || cmd {
                if !is_repeat {
                    self.split_pane_horizontal();
                }
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
            if !is_repeat {
                self.split_pane_vertical();
            }
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

        // Ctrl+Shift+X → swap active pane with next pane
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyX) = &event.physical_key
        {
            if !is_repeat {
                self.swap_active_pane();
            }
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
            if !is_repeat {
                self.toggle_pane_zoom();
            }
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

        // Ctrl+Shift+G → search web for selected text
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyG) = &event.physical_key
        {
            self.search_web_for_selection();
            return;
        }

        // Ctrl+Shift+O → open config file in default editor
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyO) = &event.physical_key
        {
            self.open_config_file();
            return;
        }

        // Ctrl+Shift+Alt+O → open current working directory in file manager
        if self.mods.ctrl
            && self.mods.shift
            && self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyO) = &event.physical_key
        {
            self.execute_command_palette_action("terminal.open_cwd_in_file_manager");
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

        // P28-A: Ctrl+Shift+Alt+G → toggle perf monitor (moved from Ctrl+Shift+G
        // which conflicts with search_web_for_selection).
        if self.mods.ctrl
            && self.mods.shift
            && self.mods.alt
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

        // Command history sidebar keyboard navigation
        if self.cmd_history.visible {
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::Escape) => {
                    if self.cmd_history.search_active {
                        self.cmd_history.toggle_search();
                    } else {
                        self.cmd_history.toggle();
                    }
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowUp) => {
                    self.cmd_history.select_up();
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowDown) => {
                    self.cmd_history.select_down();
                    return;
                }
                PhysicalKey::Code(KeyCode::Backspace) => {
                    if self.cmd_history.search_active {
                        self.cmd_history.search_backspace();
                    }
                    return;
                }
                PhysicalKey::Code(KeyCode::Enter) => {
                    // Re-run selected command
                    if let Some(idx) = self.cmd_history.selected {
                        let filtered = self.cmd_history.filtered_entries_rev();
                        if let Some(entry) = filtered.get(idx) {
                            let cmd = format!("{}\n", entry.command);
                            self.write_to_pty(cmd.as_bytes());
                            self.cmd_history.toggle();
                        }
                    }
                    return;
                }
                PhysicalKey::Code(KeyCode::Slash) => {
                    self.cmd_history.toggle_search();
                    return;
                }
                _ => {}
            }
            // If search is active, printable characters go to search query.
            // Use event.text for layout-aware input (not physical keycode).
            // Ignore keys when Ctrl or Alt is held — those are shortcuts, not text.
            if self.cmd_history.search_active
                && !self.mods.ctrl
                && !self.mods.alt
                && let Some(c) = event.text.as_ref().and_then(|t| t.chars().next())
                && !c.is_control()
            {
                self.cmd_history.search_push(c);
                return;
            }
        }

        // P28-A: Ctrl+Shift+Alt+Y → cycle workspace forward (moved from KeyW
        // which conflicts with close_other_tabs).
        if self.mods.ctrl
            && self.mods.shift
            && self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyY) = &event.physical_key
        {
            self.workspaces.cycle_next();
            self.animations.tab_switch();
            return;
        }

        // P31: Ctrl+Shift+Alt+F → cycle config profile (F = profile)
        // (Ctrl+Shift+Alt+P is reserved for copy cwd)
        if self.mods.ctrl
            && self.mods.shift
            && self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyF) = &event.physical_key
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

        // Ctrl+Alt+R → re-run last command (quick shortcut)
        if self.mods.ctrl
            && self.mods.alt
            && !self.mods.shift
            && let PhysicalKey::Code(KeyCode::KeyR) = &event.physical_key
        {
            self.rerun_last_command();
            return;
        }

        // Ctrl+Shift+Alt+P → Pipe selection to shell command
        if self.mods.ctrl
            && self.mods.shift
            && self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyP) = &event.physical_key
        {
            self.execute_command_palette_action("terminal.pipe_selection");
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

        // Ctrl+Shift+Alt+L → toggle terminal input lock
        if self.mods.ctrl
            && self.mods.shift
            && self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyL) = &event.physical_key
        {
            self.toggle_lock();
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

        // Ctrl+Shift+Alt+B → balance split panes
        if self.mods.ctrl
            && self.mods.shift
            && self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyB) = &event.physical_key
        {
            self.balance_panes();
            return;
        }

        // Ctrl+Shift+Alt+A → toggle always-on-top
        if self.mods.ctrl
            && self.mods.shift
            && self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyA) = &event.physical_key
        {
            self.toggle_always_on_top();
            return;
        }

        // Ctrl+Shift+Alt+H → export terminal as HTML (moved from KeyE which
        // conflicts with export_config).
        if self.mods.ctrl
            && self.mods.shift
            && self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyH) = &event.physical_key
        {
            self.export_html();
            return;
        }

        // Ctrl+Shift+Alt+H → import SSH hosts from ~/.ssh/config
        if self.mods.ctrl
            && self.mods.shift
            && self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyH) = &event.physical_key
        {
            self.import_ssh_hosts();
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
        // On macOS, Cmd+/ also works (more natural for Mac users).
        if (self.mods.ctrl && self.mods.shift || cfg!(target_os = "macos") && self.mods.super_key)
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
            // Ignore key-repeat for toggle/close actions to prevent flicker.
            if event.repeat {
                // Still allow character typing with repeat, just not toggle keys.
                match &event.physical_key {
                    PhysicalKey::Code(KeyCode::Escape) | PhysicalKey::Code(KeyCode::Enter) => {
                        return;
                    }
                    _ => {}
                }
            }
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::Escape) => {
                    self.command_palette.toggle(); // close
                    return;
                }
                PhysicalKey::Code(KeyCode::Enter) => {
                    let registry = &self.command_registry;
                    let results = self.command_palette.results(registry);
                    self.command_palette.confirm(&results);
                    // Execute the pending action.
                    if let Some(action_id) = self.command_palette.take_action() {
                        self.execute_command_palette_action(&action_id);
                    }
                    self.command_palette.toggle(); // close after confirm
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowUp) => {
                    let len = self.command_palette.results_len(&self.command_registry);
                    self.command_palette.move_up(len);
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowDown) => {
                    let len = self.command_palette.results_len(&self.command_registry);
                    self.command_palette.move_down(len);
                    return;
                }
                // PageUp/PageDown — jump by viewport page.
                PhysicalKey::Code(KeyCode::PageUp) => {
                    let len = self.command_palette.results_len(&self.command_registry);
                    let step = 8.min(len);
                    for _ in 0..step {
                        self.command_palette.move_up(len);
                    }
                    return;
                }
                PhysicalKey::Code(KeyCode::PageDown) => {
                    let len = self.command_palette.results_len(&self.command_registry);
                    let step = 8.min(len);
                    for _ in 0..step {
                        self.command_palette.move_down(len);
                    }
                    return;
                }
                // Home — jump to first result.
                PhysicalKey::Code(KeyCode::Home) => {
                    self.command_palette.selected = 0;
                    return;
                }
                // End — jump to last result.
                PhysicalKey::Code(KeyCode::End) => {
                    let len = self.command_palette.results_len(&self.command_registry);
                    if len > 0 {
                        self.command_palette.selected = len - 1;
                    }
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

        // P25-D: Ctrl+Shift+Alt+M → cycle broadcast mode (moved from KeyB
        // which conflicts with balance_panes).
        if self.mods.ctrl
            && self.mods.shift
            && self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyM) = &event.physical_key
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

        // Ctrl+Shift+Up/Down → jump between command prompts (OSC 133).
        if self.mods.ctrl && self.mods.shift && !self.mods.alt {
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::ArrowUp) => {
                    self.jump_to_prev_prompt();
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowDown) => {
                    self.jump_to_next_prompt();
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
            if !is_repeat {
                if self.mods.shift {
                    self.prev_tab();
                } else {
                    self.next_tab();
                }
            }
            return;
        }

        // Ctrl+Shift+` → toggle between current and last active tab
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::Backquote) = &event.physical_key
        {
            if !is_repeat {
                self.toggle_last_tab();
            }
            return;
        }

        // Alt+1..9 → switch to tab N (1-based, like browsers/IDEs)
        // Alt+0 → switch to last tab
        if self.mods.alt && !self.mods.ctrl && !self.mods.shift {
            let tab_idx = match &event.physical_key {
                PhysicalKey::Code(KeyCode::Digit1) => Some(0),
                PhysicalKey::Code(KeyCode::Digit2) => Some(1),
                PhysicalKey::Code(KeyCode::Digit3) => Some(2),
                PhysicalKey::Code(KeyCode::Digit4) => Some(3),
                PhysicalKey::Code(KeyCode::Digit5) => Some(4),
                PhysicalKey::Code(KeyCode::Digit6) => Some(5),
                PhysicalKey::Code(KeyCode::Digit7) => Some(6),
                PhysicalKey::Code(KeyCode::Digit8) => Some(7),
                PhysicalKey::Code(KeyCode::Digit9) => Some(8),
                PhysicalKey::Code(KeyCode::Digit0) => Some(self.sessions.len().saturating_sub(1)),
                _ => None,
            };
            if let Some(idx) = tab_idx {
                if idx < self.sessions.len() && !is_repeat {
                    self.switch_tab(idx);
                }
                return;
            }
        }

        // macOS: Cmd+Shift+] → next tab, Cmd+Shift+[ → prev tab (Safari/Chrome standard)
        if cfg!(target_os = "macos") && self.mods.super_key && self.mods.shift && !self.mods.alt {
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::BracketRight) => {
                    if !is_repeat {
                        self.next_tab();
                    }
                    return;
                }
                PhysicalKey::Code(KeyCode::BracketLeft) => {
                    if !is_repeat {
                        self.prev_tab();
                    }
                    return;
                }
                _ => {}
            }
        }

        // macOS: Cmd+Q → quit (standard macOS app quit shortcut)
        if cfg!(target_os = "macos")
            && self.mods.super_key
            && !self.mods.shift
            && !self.mods.alt
            && !self.mods.ctrl
            && let PhysicalKey::Code(KeyCode::KeyQ) = &event.physical_key
        {
            self.should_quit = true;
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

        // Phase 8-D: Ctrl+Alt+Up/Down for command block navigation.
        // (Moved from Ctrl+Shift+Up/Down which conflicts with scroll-one-line.)
        if self.mods.ctrl
            && self.mods.alt
            && !self.mods.shift
            && let PhysicalKey::Code(code) = &event.physical_key
        {
            match code {
                KeyCode::ArrowUp => {
                    let block_row = {
                        let app = self.active_session_mut().app_mut();
                        app.handle_event(AppEvent::PrevCommandBlock);
                        app.command_nav()
                            .navigator()
                            .current_block(app.terminal())
                            .map(|b| b.prompt_row)
                    };
                    if let Some(row) = block_row {
                        self.active_session_mut()
                            .app_mut()
                            .grid_mut()
                            .scroll_to_grid_row(row);
                    }
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                    return;
                }
                KeyCode::ArrowDown => {
                    let block_row = {
                        let app = self.active_session_mut().app_mut();
                        app.handle_event(AppEvent::NextCommandBlock);
                        app.command_nav()
                            .navigator()
                            .current_block(app.terminal())
                            .map(|b| b.prompt_row)
                    };
                    if let Some(row) = block_row {
                        self.active_session_mut()
                            .app_mut()
                            .grid_mut()
                            .scroll_to_grid_row(row);
                    }
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
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
                // Ctrl+Shift+Alt+O → copy last command's output
                KeyCode::KeyO => {
                    if self.mods.alt {
                        self.copy_last_command_output();
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
                    // NL2Command: show input mode instead of immediately requesting.
                    self.ai_overlay.start_nl2cmd_input();
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

        // Pipe-selection command input mode
        if self.pipe_command_active {
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::Escape) => {
                    self.pipe_command_active = false;
                    self.pipe_command_input.clear();
                    return;
                }
                PhysicalKey::Code(KeyCode::Enter) => {
                    let cmd = std::mem::take(&mut self.pipe_command_input);
                    self.pipe_command_active = false;
                    if cmd.is_empty() {
                        return;
                    }
                    // Guard: require an active selection to pipe.
                    // Without this, read_clipboard() would feed stale clipboard
                    // contents into the shell command — a potential data leak.
                    if self.selection.normalized().is_none() {
                        self.show_toast("Nothing selected — select text first");
                        return;
                    }
                    // Extract selection text directly from grid (don't overwrite clipboard).
                    let input = self.extract_selection_text();
                    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                    let result = std::process::Command::new(&shell)
                        .arg("-c")
                        .arg(&cmd)
                        .stdin(std::process::Stdio::piped())
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .spawn();
                    match result {
                        Ok(mut child) => {
                            use std::io::Write;
                            if let Some(stdin) = child.stdin.as_mut() {
                                let _ = stdin.write_all(input.as_bytes());
                            }
                            child.stdin.take();
                            // Spawn background thread — result sent via channel.
                            let (tx, rx) =
                                std::sync::mpsc::channel::<Result<(Vec<u8>, Vec<u8>), String>>();
                            std::thread::spawn(move || {
                                let output = child.wait_with_output();
                                let result = match output {
                                    Ok(o) => Ok((o.stdout, o.stderr)),
                                    Err(e) => Err(e.to_string()),
                                };
                                let _ = tx.send(result);
                            });
                            self.pending_pipe_result = Some(super::PipeCommandResult {
                                rx,
                                command: cmd.clone(),
                            });
                            self.show_toast(format!("Running '{cmd}'..."));
                        }
                        Err(e) => {
                            self.show_toast(format!("Failed to run '{cmd}': {e}"));
                        }
                    }
                    return;
                }
                PhysicalKey::Code(KeyCode::Backspace) => {
                    self.pipe_command_input.pop();
                    return;
                }
                _ => {}
            }
            // Printable characters go to command input (use winit text event
            // for proper Unicode support — same pattern as command palette).
            if let Some(c) = event.text.as_ref().and_then(|t| t.chars().next())
                && !c.is_control()
            {
                self.pipe_command_input.push(c);
            }
            return;
        }

        // NL2Command input mode: intercept keyboard for text input.
        #[cfg(feature = "ai")]
        if self.ai_overlay.is_nl2cmd_typing() {
            self.handle_nl2cmd_input(event);
            return;
        }

        // P10-C: Esc dismisses AI overlay if visible.
        #[cfg(feature = "ai")]
        if self.ai_overlay.is_visible() && !self.ai_overlay.is_nl2cmd_typing() {
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::Escape) => {
                    self.ai_overlay.hide();
                    return;
                }
                // Tab: insert AI response into terminal (for Suggest/NL2Cmd)
                PhysicalKey::Code(KeyCode::Tab) => {
                    if let Some(content) = self.ai_overlay.content() {
                        // Try to extract just the command (first code block or line)
                        let cmd = extract_ai_command(content);
                        if !cmd.is_empty() {
                            self.write_to_pty(cmd.as_bytes());
                            self.ai_overlay.hide();
                            self.show_toast(format!("Inserted: {}", truncate_str(&cmd, 40)));
                        }
                    }
                    return;
                }
                // Ctrl+Enter: insert AND execute the command
                PhysicalKey::Code(KeyCode::Enter) if self.mods.ctrl => {
                    if let Some(content) = self.ai_overlay.content() {
                        let cmd = extract_ai_command(content);
                        if !cmd.is_empty() {
                            let mut to_send = cmd.into_bytes();
                            to_send.push(b'\n');
                            self.write_to_pty(&to_send);
                            self.ai_overlay.hide();
                            self.show_toast("Command executed");
                        }
                    }
                    return;
                }
                _ => {}
            }
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
        self.encoder.set_modify_other_keys(term.modify_other_keys());
        if let Some(input_key) = map_winit_key(&event.physical_key, logical_text, &mods) {
            let bytes = self.encoder.encode(&input_key);
            if !bytes.is_empty() {
                // Clear text selection on keypress — standard terminal behavior
                // (selection is a transient visual state, not a persistent edit).
                self.selection.clear();
                // Auto-scroll to bottom on keypress (standard terminal behavior).
                // If the user scrolled up through scrollback and then starts
                // typing, jump back to the most recent output.
                let grid = self.sessions[self.active]
                    .app_mut()
                    .terminal_mut()
                    .grid_mut();
                if grid.display_offset() > 0 {
                    grid.reset_viewport();
                    self.new_output_while_scrolled = 0;
                }
                // cursor_blink.reset() is called inside write_to_pty().
                self.write_to_pty(&bytes);
            }
        }
    }

    // ── Scrollback search (P10-D) ───────────────────────────────

    /// Scroll viewport to show a search match (shared by F3 and Enter).
    fn scroll_to_search_match(&mut self, matched: Option<crate::search::SearchMatch>) {
        if self.search.last_wrapped() {
            self.show_toast("Search wrapped".to_string());
        }
        if let Some(m) = matched {
            let scrollback_len = self.sessions[self.active].app().grid().scrollback_len();
            let grid_height = self.sessions[self.active].app().grid().height();
            let visible_row = m.abs_row as isize - scrollback_len as isize;
            if visible_row < 0 {
                let target_offset = scrollback_len - m.abs_row;
                let grid = self.sessions[self.active]
                    .app_mut()
                    .terminal_mut()
                    .grid_mut();
                let current_offset = grid.display_offset();
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
                if search.last_wrapped() {
                    self.show_toast("Search wrapped".to_string());
                }
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
                if self.mods.shift {
                    // Shift+Tab toggles regex search mode.
                    let grid2 = self.sessions[self.active].app().grid();
                    self.search.toggle_regex(grid2);
                } else {
                    // Tab toggles case sensitivity.
                    let grid2 = self.sessions[self.active].app().grid();
                    self.search.toggle_case(grid2);
                }
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
                // Use event.text for layout-aware input; ignore Ctrl/Alt
                // (they're shortcuts, not text input).
                if !self.mods.ctrl
                    && !self.mods.alt
                    && let Some(c) = event.text.as_ref().and_then(|t| t.chars().next())
                    && !c.is_control()
                {
                    search.type_char(c, grid);
                }
            }
        }
    }

    /// Handle keyboard input for NL2Command text entry.
    #[cfg(feature = "ai")]
    pub(super) fn handle_nl2cmd_input(&mut self, event: &KeyEvent) {
        match &event.physical_key {
            PhysicalKey::Code(KeyCode::Escape) => {
                self.ai_overlay.hide();
            }
            PhysicalKey::Code(KeyCode::Enter) => {
                // Submit the natural language query.
                if let Some(query) = self.ai_overlay.nl2cmd_submit() {
                    self.trigger_ai_nl2cmd(&query);
                }
            }
            PhysicalKey::Code(KeyCode::Backspace) => {
                self.ai_overlay.nl2cmd_backspace();
            }
            _ => {
                // Type printable characters (layout-aware, ignore Ctrl/Alt).
                if !self.mods.ctrl
                    && !self.mods.alt
                    && let Some(c) = event.text.as_ref().and_then(|t| t.chars().next())
                    && !c.is_control()
                {
                    self.ai_overlay.nl2cmd_append(c);
                }
            }
        }
    }

    /// Trigger NL2Command with a natural language query.
    #[cfg(feature = "ai")]
    fn trigger_ai_nl2cmd(&mut self, query: &str) {
        self.ai_overlay.start_request(ggterm_ai::Action::NL2Command);
        let ctx = ggterm_ai::AIContext::from_terminal(self.active_session().app().terminal());
        let req = crate::ai_bridge::AIRequest {
            action: ggterm_ai::Action::NL2Command,
            context: ctx,
            natural_language: Some(query.to_string()),
            enable_tools: false,
        };
        if let Some(ref mut bridge) = self.ai_bridge {
            if !bridge.request(req) {
                self.ai_overlay.set_error("AI is busy, please wait...");
            }
        } else {
            self.ai_overlay
                .set_error("AI not configured (set ai.api_endpoint in config)");
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

        let tab_bar_h = if !self.tab_bar.tabs.is_empty() {
            // Both single-tab and multi-tab modes use the same bar height.
            ((cell_h + 26.0).max(48.0) + 4.0) as u32
        } else {
            0
        };
        let status_bar_h = if self.status_bar_visible {
            // Must match render.rs: bar_h = cell_h + 8.0
            (cell_h + 8.0) as u32
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
        let (col, row) = crate::mouse::pixel_to_cell(px, py, cw, ch);

        // Clamp to grid bounds — prevents out-of-range mouse tracking
        // events being sent to vim/htop/etc. when the cursor is in the
        // tab bar or status bar gap.
        let grid = self.active_session().app().grid();
        let max_col = (grid.width() - 1) as u16;
        let max_row = (grid.height() - 1) as u16;
        (col.min(max_col), row.min(max_row))
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
        // Window edge resize: when decorations are off, we handle resizing
        // ourselves via drag_resize_window(). Check if the mouse is at a
        // window edge and start a resize operation.
        #[cfg(not(target_os = "macos"))]
        if state == ElementState::Pressed
            && button == winit::event::MouseButton::Left
            && let Some(ref window) = self.window
            && let Some(dir) = self.window_edge_resize_direction()
        {
            let _ = window.drag_resize_window(dir);
            return;
        }

        // P21-A: Handle split separator drag.
        if button == winit::event::MouseButton::Left {
            if state == ElementState::Pressed {
                // Check P2P Share button click (right side of status bar).
                #[cfg(feature = "p2p")]
                if self.status_bar_visible {
                    let in_btn = self.is_in_share_button();
                    log::debug!(
                        "mouse click: status_bar_visible={}, in_share_button={}",
                        self.status_bar_visible,
                        in_btn
                    );
                    if in_btn {
                        self.toggle_p2p_share();
                        return;
                    }
                }
                // Check if we're clicking on a separator.
                if self.try_start_separator_drag() {
                    return; // Don't process as pane click or selection
                }

                // Check if clicking the scroll-to-bottom indicator pill.
                if let Some((x, y, w, h)) = self.scroll_indicator_rect {
                    let (mx, my) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
                    if mx >= x && mx < x + w && my >= y && my < y + h {
                        self.active_session_mut()
                            .app_mut()
                            .terminal_mut()
                            .grid_mut()
                            .reset_viewport();
                        self.new_output_while_scrolled = 0;
                        self.smooth_scroll.reset();
                        if let Some(ref window) = self.window {
                            window.request_redraw();
                        }
                        return;
                    }
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
                        // Protocol response: write directly to PTY to avoid
                        // auto-scroll/broadcast/blink-reset side effects.
                        self.active_session_mut().write_to_pty(&bytes);
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
                        self.active_session_mut().write_to_pty(&bytes);
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

                // Multi-click detection — done early so ALL click handlers
                // (title bar, tab bar, terminal area) can use click_count.
                let now = std::time::Instant::now();
                let is_multi_click = self
                    .last_click_time
                    .is_some_and(|t| now.duration_since(t).as_millis() < 400)
                    && self.last_click_pixel_pos.is_some_and(|(lx, ly)| {
                        (lx - self.cursor_pos.0).abs() < 5.0 && (ly - self.cursor_pos.1).abs() < 5.0
                    });
                if is_multi_click {
                    self.click_count = (self.click_count % 3) + 1; // cycle 1→2→3→1
                } else {
                    self.click_count = 1;
                }
                self.last_click_time = Some(now);
                self.last_click_pos = (col, row);
                self.last_click_pixel_pos = Some(self.cursor_pos);

                // Single-tab title bar: check + and gear buttons, plus
                // window drag on empty area.
                // This must be OUTSIDE the `if self.tab_bar.visible` block
                // because visible=false in single-tab mode.
                if !self.tab_bar.visible
                    && !self.tab_bar.tabs.is_empty()
                    && state == ElementState::Pressed
                    && button == winit::event::MouseButton::Left
                {
                    let (px, py) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
                    let (screen_w, cell_h) = if let Some(ref r) = self.renderer {
                        (r.resolution_width() as f32, r.cell_height() as f32)
                    } else {
                        (
                            self.config.cols as f32 * self.config.cell_width,
                            self.config.cell_height,
                        )
                    };
                    let layout = super::SingleTabButtonLayout::compute(screen_w, cell_h);

                    // "+" button → open dropdown menu.
                    if layout.is_on_plus(px, py) {
                        let menu_w = crate::new_tab_menu::NewTabMenuState::WIDTH;
                        let mut menu_x = layout.plus_x + layout.btn_size - menu_w;
                        if menu_x < 4.0 {
                            menu_x = 4.0;
                        }
                        let menu_y = layout.btn_y + layout.btn_size + 2.0;
                        self.new_tab_menu.toggle(menu_x, menu_y);
                        if let Some(ref window) = self.window {
                            window.request_redraw();
                        }
                        return;
                    }
                    // Settings gear button.
                    if layout.is_on_gear(px, py) {
                        self.pending_open_settings = true;
                        return;
                    }
                    // Linux/Windows: caption buttons (minimize/maximize/close).
                    #[cfg(not(target_os = "macos"))]
                    {
                        let ctrl_layout =
                            crate::titlebar::compute_caption_layout(screen_w, layout.bar_h);
                        if let Some(btn) = crate::titlebar::caption_hit_test(&ctrl_layout, px, py) {
                            use crate::titlebar::WindowControlButton;
                            match btn {
                                WindowControlButton::Close => {
                                    self.should_quit = true;
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
                    // Title bar drag and double-click maximize (all platforms).
                    {
                        if py < layout.bar_h {
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
                }

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
                        // Linux/Windows: caption buttons (minimize/maximize/close).
                        #[cfg(not(target_os = "macos"))]
                        {
                            let bar_h = (self.tab_font_size() + 26.0).max(48.0) + 4.0;
                            let ctrl_layout = crate::titlebar::compute_caption_layout(
                                self.tab_layout_width(),
                                bar_h,
                            );
                            if let Some(btn) =
                                crate::titlebar::caption_hit_test(&ctrl_layout, px, py)
                            {
                                use crate::titlebar::WindowControlButton;
                                match btn {
                                    WindowControlButton::Close => {
                                        self.should_quit = true;
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
                                let hit = cb.size / 2.0 + 4.0;
                                let dx = px - cb.cx;
                                let dy = py - cb.cy;
                                if dx.abs() <= hit
                                    && dy.abs() <= hit
                                    && self.sessions.len() > 1
                                    && !self.sessions[tab_idx].is_pinned()
                                {
                                    self.close_tab();
                                    return;
                                }
                            }
                            self.switch_tab(tab_idx);
                            if self.click_count == 2 {
                                self.renaming_tab = Some(tab_idx);
                                self.rename_text = self.sessions[tab_idx].title().to_string();
                            }
                            if self.click_count == 1 && self.sessions.len() > 1 {
                                self.dragging_tab = Some(tab_idx);
                            }
                            return;
                        }
                        // Empty area of tab bar → window drag or double-click maximize.
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

                // Terminal content area: multi-click already tracked above.
                match self.click_count {
                    2 => {
                        // Double-click: select word at position.
                        self.select_word_at(col, row);
                        self.drag_select_mode = crate::mouse::DragSelectMode::Word;
                        self.selection.resume_dragging();
                    }
                    3 => {
                        // Triple-click: select entire line.
                        self.select_line_at(row);
                        self.drag_select_mode = crate::mouse::DragSelectMode::Line;
                        self.selection.resume_dragging();
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
                        self.drag_select_mode = crate::mouse::DragSelectMode::Char;
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
                        let mut line_text = String::new();
                        for c in &display_row.cells {
                            if c.is_wide_spacer() {
                                continue;
                            }
                            line_text.push(c.ch);
                            for &mc in &c.combining {
                                line_text.push(mc);
                            }
                        }
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
                // Copy selection to clipboard if active and copy_on_select is enabled.
                if self.selection.is_active()
                    && self
                        .config_mgr
                        .as_ref()
                        .is_none_or(|m| m.config().terminal.copy_on_select)
                {
                    self.copy_selection_to_clipboard();
                }
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            (crate::mouse::MouseButton::Middle, ElementState::Pressed) => {
                // Middle-click on tab closes it (like browsers).
                // Otherwise, middle-click paste:
                // - Linux X11/Wayland: from PRIMARY selection (standard behavior)
                // - macOS/Windows: from CLIPBOARD
                let (px, py) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
                let bounds = self.content_area_bounds();
                if py < bounds.y as f32 && self.tab_bar.visible {
                    let layout = self
                        .tab_bar
                        .compute_layout(self.tab_layout_width(), self.tab_font_size());
                    if let Some(tab_idx) = self.tab_bar.tab_at_x(&layout, px)
                        && self.sessions.len() > 1
                        && !self.sessions[tab_idx].is_pinned()
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
                self.paste_from_source(crate::clipboard::PasteSource::MiddleClick);
            }
            (crate::mouse::MouseButton::Right, ElementState::Pressed) => {
                // P27-C: Show context menu at mouse position.
                let (px, py) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
                // Clamp menu position to window bounds so it's never
                // clipped off-screen at edges.
                let menu_w = crate::context_menu::ContextMenuState::WIDTH;
                let menu_h = self.context_menu.menu_height();
                let (sw, sh) = if let Some(ref renderer) = self.renderer {
                    (
                        renderer.resolution_width() as f32,
                        renderer.resolution_height() as f32,
                    )
                } else {
                    (800.0, 600.0)
                };
                let clamped_x = px.min(sw - menu_w - 4.0).max(4.0);
                let clamped_y = if py + menu_h > sh {
                    // Flip up if menu would overflow bottom edge.
                    (py - menu_h).max(4.0)
                } else {
                    py
                };
                self.context_menu.show(clamped_x, clamped_y);
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
                // Throttle: only report when cell position changes.
                // Without this, every pixel-level motion event floods the PTY.
                if self.last_mouse_cell == Some((col, row)) {
                    return;
                }
                self.last_mouse_cell = Some((col, row));
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
                    // Protocol response: write directly to PTY.
                    self.active_session_mut().write_to_pty(bytes.as_bytes());
                }
                return;
            }
        }

        // Extend selection while dragging.
        if self.selection.dragging {
            // Set auto-scroll direction and speed based on proximity to edges.
            // Closer to the edge = faster scroll (like iTerm2/VS Code).
            // The actual scrolling happens in about_to_wait via a timer.
            let bounds = self.content_area_bounds();
            let py = self.cursor_pos.1 as f32;
            let top_y = bounds.y as f32;
            let bottom_y = (bounds.y + bounds.height) as f32;
            let edge_zone = 60.0;

            if py <= top_y + edge_zone {
                // Distance into the top edge zone (0 at boundary, edge_zone at top).
                let dist = (top_y + edge_zone - py).max(0.0);
                // Speed: 1 line at the boundary, up to 5 at the very edge.
                let speed = 1 + (dist / edge_zone * 4.0) as i32;
                self.selection_auto_scroll = -speed;
            } else if py >= bottom_y - edge_zone {
                let dist = (py - (bottom_y - edge_zone)).max(0.0);
                let speed = 1 + (dist / edge_zone * 4.0) as i32;
                self.selection_auto_scroll = speed;
            } else {
                self.selection_auto_scroll = 0;
            }

            // Extend selection using the active drag mode.
            match self.drag_select_mode {
                crate::mouse::DragSelectMode::Word => {
                    self.extend_word_selection(col, row);
                }
                crate::mouse::DragSelectMode::Line => {
                    self.extend_line_selection(row);
                }
                crate::mouse::DragSelectMode::Char => {
                    self.selection.extend(col, row);
                }
            }
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
        }

        // P17-C: Detect hovered URL (OSC 8 hyperlink or plain text).
        // P28-B: Update color picker hover state.
        // Throttle: skip both when cell position hasn't changed (avoids
        // per-pixel grid iteration on mouse move).
        let cell_changed = self.last_mouse_cell != Some((col, row));
        if cell_changed {
            self.update_hovered_link(col, row);
            self.update_color_picker_hover(col, row);
        }

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
            let content_top = self.content_area_bounds().y as f32;
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
            } else if self.hovered_link.is_some()
                || (!self.tab_bar.visible
                    && !self.tab_bar.tabs.is_empty()
                    && self.is_on_floating_toolbar(py))
            {
                CursorIcon::Pointer
            } else if py > content_top || self.selection.dragging {
                CursorIcon::Text
            } else {
                // Check window edge resize zone before falling back to Default.
                #[cfg(not(target_os = "macos"))]
                if let Some(dir) = self.window_edge_resize_direction() {
                    use winit::window::ResizeDirection::*;
                    match dir {
                        North | South => CursorIcon::NsResize,
                        East | West => CursorIcon::EwResize,
                        NorthWest | SouthEast => CursorIcon::NwseResize,
                        NorthEast | SouthWest => CursorIcon::NeswResize,
                    }
                } else {
                    CursorIcon::Default
                }
                #[cfg(target_os = "macos")]
                {
                    CursorIcon::Default
                }
            };
            window.set_cursor(icon);
        }
    }

    /// Detect if cursor is at a window edge for resizing (decorations off).
    #[cfg(not(target_os = "macos"))]
    fn window_edge_resize_direction(&self) -> Option<winit::window::ResizeDirection> {
        use winit::window::ResizeDirection;

        let renderer = self.renderer.as_ref()?;
        let screen_w = renderer.resolution_width() as f32;
        let screen_h = renderer.resolution_height() as f32;

        let px = self.cursor_pos.0 as f32;
        let py = self.cursor_pos.1 as f32;
        const EDGE: f32 = 6.0;

        let at_left = px < EDGE;
        let at_right = px > screen_w - EDGE;
        let at_top = py < EDGE;
        let at_bottom = py > screen_h - EDGE;

        match (at_left, at_right, at_top, at_bottom) {
            (true, _, true, _) => Some(ResizeDirection::NorthWest),
            (_, true, true, _) => Some(ResizeDirection::NorthEast),
            (true, _, _, true) => Some(ResizeDirection::SouthWest),
            (_, true, _, true) => Some(ResizeDirection::SouthEast),
            (true, _, _, _) => Some(ResizeDirection::West),
            (_, true, _, _) => Some(ResizeDirection::East),
            (_, _, true, _) => Some(ResizeDirection::North),
            (_, _, _, true) => Some(ResizeDirection::South),
            _ => None,
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
            let line: String = cell_row.text();
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
            let line: String = cell_row.text();
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
    /// Get configured word characters for boundary detection.
    fn word_chars_config(&self) -> String {
        self.config_mgr
            .as_ref()
            .map(|m| m.config().terminal.word_chars.clone())
            .unwrap_or_else(|| ".-/:@~+#?=&%$".to_string())
    }

    pub(super) fn select_word_at(&mut self, col: u16, row: u16) {
        let grid = &self.sessions[self.active].app().grid();
        let col_u = col as usize;

        // Get the display row to scan characters.
        let Some(display_row) = grid.display_row(row as usize) else {
            return;
        };

        let word_chars = self.word_chars_config();
        // Classify a grid cell at a given column — skip wide spacers.
        let cell_class = |c: usize| -> Option<u8> {
            let cell = display_row.cell(c)?;
            if cell.is_wide_spacer() {
                return None;
            }
            let ch = if cell.ch == '\0' { ' ' } else { cell.ch };
            Some(if ch.is_alphanumeric() || ch == '_' {
                0
            } else if ch.is_whitespace() {
                2
            } else if word_chars.contains(ch) {
                0
            } else {
                1
            })
        };

        // If the clicked cell is whitespace, select just that cell.
        if cell_class(col_u) == Some(2) || display_row.cell(col_u).is_none() {
            self.selection.start(col, row);
            self.selection.extend(col, row);
            self.selection.finish();
            return;
        }

        // Scan left for word start — stop at different char class or wide spacer.
        let target_class = match cell_class(col_u) {
            Some(c) => c,
            None => {
                // Clicked on a wide spacer — back up to the lead cell.
                if col_u == 0 {
                    return;
                }
                match cell_class(col_u - 1) {
                    Some(c) => c,
                    None => return,
                }
            }
        };

        let mut start = col_u;
        while start > 0 && cell_class(start - 1) == Some(target_class) {
            start -= 1;
        }

        // Scan right for word end.
        let mut end = col_u;
        while cell_class(end + 1) == Some(target_class) {
            end += 1;
        }

        self.selection.start(start as u16, row);
        self.selection.extend(end as u16, row);
        self.selection.finish();
        // Copy on select (consistent with drag selection).
        if self.selection.is_active()
            && self
                .config_mgr
                .as_ref()
                .is_none_or(|m| m.config().terminal.copy_on_select)
        {
            self.copy_selection_to_clipboard();
        }
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
        self.selection.line_select = true;
        // Copy on select (consistent with drag selection).
        if self
            .config_mgr
            .as_ref()
            .is_none_or(|m| m.config().terminal.copy_on_select)
        {
            self.copy_selection_to_clipboard();
        }
    }

    /// Extend selection word-by-word during drag (after double-click).
    ///
    /// Finds word boundaries at the current cursor position and extends the
    /// selection end to encompass the full word at that position.
    fn extend_word_selection(&mut self, col: u16, row: u16) {
        let grid = &self.sessions[self.active].app().grid();
        let col_u = col as usize;

        let Some(display_row) = grid.display_row(row as usize) else {
            self.selection.extend(col, row);
            return;
        };

        let word_chars = self.word_chars_config();
        let cell_class = |c: usize| -> Option<u8> {
            let cell = display_row.cell(c)?;
            if cell.is_wide_spacer() {
                return None;
            }
            let ch = if cell.ch == '\0' { ' ' } else { cell.ch };
            Some(if ch.is_alphanumeric() || ch == '_' {
                0
            } else if ch.is_whitespace() {
                2
            } else if word_chars.contains(ch) {
                0
            } else {
                1
            })
        };

        // If cursor is on whitespace or past the line, just extend to cursor.
        if cell_class(col_u).is_none() || cell_class(col_u) == Some(2) {
            self.selection.extend(col, row);
            return;
        }

        // Find word boundaries at cursor position using grid cells directly.
        let target_class = cell_class(col_u).unwrap_or(1);
        let mut word_start = col_u;
        while word_start > 0 && cell_class(word_start - 1) == Some(target_class) {
            word_start -= 1;
        }
        let mut word_end = col_u;
        while cell_class(word_end + 1) == Some(target_class) {
            word_end += 1;
        }

        // Determine direction relative to the selection anchor.
        let anchor = self.selection.start.unwrap_or((0, 0));
        let is_forward = row > anchor.1 || (row == anchor.1 && word_end as u16 >= anchor.0);

        if is_forward {
            // Forward: extend end to the far edge of the current word.
            self.selection.extend(word_end as u16, row);
        } else {
            // Backward: extend end to the near edge of the current word.
            self.selection.extend(word_start as u16, row);
        }
    }

    /// Extend selection line-by-line during drag (after triple-click).
    ///
    /// Snaps the selection to full-width lines.
    fn extend_line_selection(&mut self, row: u16) {
        let grid = &self.sessions[self.active].app().grid();
        let width = grid.width() as u16;
        if width == 0 {
            return;
        }

        // Determine direction relative to the anchor line.
        let anchor = self.selection.start.unwrap_or((0, 0));
        let is_forward = row >= anchor.1;

        if is_forward {
            self.selection.extend(width - 1, row);
        } else {
            self.selection.extend(0, row);
        }
    }

    /// P27-C: Execute a context menu action by index.
    fn execute_context_menu_action(&mut self, index: usize) {
        let actions = crate::context_menu::ContextMenuAction::all();
        if index >= actions.len() {
            return;
        }
        let action = actions[index];
        // Guard: skip disabled items.
        let has_selection = self.selection.is_active();
        let has_url = self.hovered_link.is_some();
        if !action.is_enabled(has_selection, has_url, true) {
            return;
        }
        match action {
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
            crate::context_menu::ContextMenuAction::SearchWeb => {
                self.search_web_for_selection();
            }
            crate::context_menu::ContextMenuAction::OpenUrl => {
                self.open_url_at_cursor();
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
            crate::context_menu::ContextMenuAction::ExportScrollback => {
                self.save_scrollback_to_file();
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
                    // Protocol response: write directly to PTY.
                    self.active_session_mut().write_to_pty(&bytes);
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
                    self.active_session_mut().write_to_pty(&bytes);
                }
            }
            return;
        }

        // Mouse tracking OFF — check for alternate scroll mode (DECSET 7727).
        // When in the alternate screen (less, man, vim without mouse), convert
        // wheel events to Up/Down arrow key sequences so users can scroll.
        if term.is_alt_screen() && term.alternate_scroll_enabled() && !self.mods.shift {
            let lines = match delta {
                winit::event::MouseScrollDelta::LineDelta(_x, y) => -(y as i32),
                winit::event::MouseScrollDelta::PixelDelta(pos) => {
                    -(pos.y as f32 / 16.0).round() as i32
                }
            };
            // Send arrow keys for each scroll line.
            let key_bytes = if lines > 0 {
                // Scroll up → Up arrow
                if term.cursor_keys_app() {
                    b"\x1bOA".to_vec()
                } else {
                    b"\x1b[A".to_vec()
                }
            } else {
                // Scroll down → Down arrow
                if term.cursor_keys_app() {
                    b"\x1bOB".to_vec()
                } else {
                    b"\x1b[B".to_vec()
                }
            };
            // Send arrow keys for each scroll line — batch into a single
            // write to avoid N individual PTY writes (each locks a mutex).
            let count = lines.unsigned_abs() as usize;
            let mut batch = Vec::with_capacity(key_bytes.len() * count);
            for _ in 0..count {
                batch.extend_from_slice(&key_bytes);
            }
            self.write_to_pty(&batch);
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
                // Mouse wheel: scroll 3 lines per notch (standard terminal/editor behavior).
                let lines = -(y as i32 * 3);
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

    /// Check if the cursor is hovering over the floating toolbar (single-tab mode).
    /// The toolbar is in the top-right corner: "+" and settings gear buttons.
    pub(super) fn is_on_floating_toolbar(&self, py: f32) -> bool {
        if self.tab_bar.visible || self.tab_bar.tabs.is_empty() {
            return false;
        }
        let (screen_w, cell_h) = if let Some(ref r) = self.renderer {
            (r.resolution_width() as f32, r.cell_height() as f32)
        } else {
            (
                self.config.cols as f32 * self.config.cell_width,
                self.config.cell_height,
            )
        };
        let layout = super::SingleTabButtonLayout::compute(screen_w, cell_h);
        let px = self.cursor_pos.0 as f32;
        layout.is_on_plus(px, py) || layout.is_on_gear(px, py)
    }
}

/// Extract a shell command from AI response text.
///
/// Looks for code blocks (```...```) first, then falls back to lines
/// that look like commands (start with $ or contain common command patterns).
#[cfg(feature = "ai")]
fn extract_ai_command(text: &str) -> String {
    // Try to find a fenced code block.
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        // Skip optional language tag on first line
        let code_start = after.find('\n').map_or(0, |p| p + 1);
        let code = &after[code_start..];
        if let Some(close) = code.find("```") {
            return code[..close].trim().to_string();
        }
        // No closing fence — take the rest
        return code.trim().to_string();
    }

    // Try to find a line starting with $ (common command format)
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(cmd) = trimmed.strip_prefix("$ ") {
            return cmd.to_string();
        }
        if trimmed.starts_with("sudo ")
            || trimmed.starts_with("docker ")
            || trimmed.starts_with("git ")
            || trimmed.starts_with("cd ")
            || trimmed.starts_with("ls ")
            || trimmed.starts_with("cat ")
            || trimmed.starts_with("find ")
            || trimmed.starts_with("grep ")
            || trimmed.starts_with("rg ")
            || trimmed.starts_with("curl ")
            || trimmed.starts_with("wget ")
            || trimmed.starts_with("ssh ")
            || trimmed.starts_with("scp ")
            || trimmed.starts_with("npm ")
            || trimmed.starts_with("npx ")
            || trimmed.starts_with("yarn ")
            || trimmed.starts_with("pnpm ")
            || trimmed.starts_with("cargo ")
            || trimmed.starts_with("rustup ")
            || trimmed.starts_with("go ")
            || trimmed.starts_with("python ")
            || trimmed.starts_with("python3 ")
            || trimmed.starts_with("pip ")
            || trimmed.starts_with("make ")
            || trimmed.starts_with("kubectl ")
            || trimmed.starts_with("helm ")
            || trimmed.starts_with("systemctl ")
            || trimmed.starts_with("brew ")
            || trimmed.starts_with("apt ")
            || trimmed.starts_with("apt-get ")
            || trimmed.starts_with("tar ")
            || trimmed.starts_with("chmod ")
            || trimmed.starts_with("mkdir ")
            || trimmed.starts_with("cp ")
            || trimmed.starts_with("mv ")
            || trimmed.starts_with("echo ")
            || trimmed.starts_with("sed ")
            || trimmed.starts_with("awk ")
            || trimmed.starts_with("head ")
            || trimmed.starts_with("tail ")
            || trimmed.starts_with("diff ")
            || trimmed.starts_with("du ")
            || trimmed.starts_with("df ")
            || trimmed.starts_with("ps ")
            || trimmed.starts_with("kill ")
            || trimmed.starts_with("which ")
            || trimmed.starts_with("man ")
            || trimmed.starts_with("openssl ")
        {
            return trimmed.to_string();
        }
    }

    // If the entire response is a single line, treat it as a command
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() == 1 {
        return lines[0].trim().to_string();
    }

    String::new()
}

/// Truncate a string to `max` chars, appending "..." if truncated.
#[cfg(feature = "ai")]
fn truncate_str(s: &str, max: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{truncated}...")
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "ai")]
    use super::*;

    #[cfg(feature = "ai")]
    #[test]
    fn test_extract_ai_command_code_block() {
        let text = "Here's the command:\n```bash\ngit status\n```\nThat's it.";
        assert_eq!(extract_ai_command(text), "git status");
    }

    #[cfg(feature = "ai")]
    #[test]
    fn test_extract_ai_command_code_block_no_lang() {
        let text = "Try this:\n```\ndocker ps -a\n```";
        assert_eq!(extract_ai_command(text), "docker ps -a");
    }

    #[cfg(feature = "ai")]
    #[test]
    fn test_extract_ai_command_dollar_prompt() {
        let text = "You can run:\n$ cargo build --release\nTo compile.";
        assert_eq!(extract_ai_command(text), "cargo build --release");
    }

    #[cfg(feature = "ai")]
    #[test]
    fn test_extract_ai_command_known_prefix() {
        let text = "The solution is:\ngit push origin main\nDone.";
        assert_eq!(extract_ai_command(text), "git push origin main");
    }

    #[cfg(feature = "ai")]
    #[test]
    fn test_extract_ai_command_expanded_prefixes() {
        // Commands must be on their own line for prefix detection to work.
        // Single-line responses are returned as-is (see test_extract_ai_command_single_line).
        assert_eq!(
            extract_ai_command("Run this:\nkubectl get pods\nThat's it."),
            "kubectl get pods"
        );
        assert_eq!(
            extract_ai_command("Try:\nbrew install ripgrep\nDone."),
            "brew install ripgrep"
        );
        assert_eq!(
            extract_ai_command("Use this:\nrg 'pattern' src/\nGood luck."),
            "rg 'pattern' src/"
        );
    }

    #[cfg(feature = "ai")]
    #[test]
    fn test_extract_ai_command_single_line() {
        assert_eq!(extract_ai_command("ls -la"), "ls -la");
    }

    #[cfg(feature = "ai")]
    #[test]
    fn test_extract_ai_command_multi_line_no_command() {
        let text = "This command will list files.\nUse it carefully.\nGood luck.";
        assert_eq!(extract_ai_command(text), "");
    }

    #[cfg(feature = "ai")]
    #[test]
    fn test_extract_ai_command_empty() {
        assert_eq!(extract_ai_command(""), "");
    }

    #[cfg(feature = "ai")]
    #[test]
    fn test_truncate_str_short() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[cfg(feature = "ai")]
    #[test]
    fn test_truncate_str_long() {
        let result = truncate_str("abcdefghij", 5);
        assert_eq!(result, "abcde...");
    }

    #[cfg(feature = "ai")]
    #[test]
    fn test_truncate_str_utf8_safe() {
        // Multi-byte UTF-8 characters should not be split.
        let result = truncate_str("你好世界测试文字", 6);
        assert!(result.ends_with("..."));
        // Short enough — should NOT be truncated.
        let result2 = truncate_str("你好", 6);
        assert!(!result2.ends_with("..."));
        // Should not panic.
    }
}
