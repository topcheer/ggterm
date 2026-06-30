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

        // Recreate renderer with surface dimensions — it computes cols/rows internally.
        if let Some(gpu) = &self.gpu {
            self.renderer = Some(gpu.create_renderer(width, height, self.scale_factor));
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

        self.active_session_mut().resize(new_cols, new_rows);

        true
    }

    /// Handle a winit key event using the existing keymap module.
    pub(super) fn handle_keyboard_input(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }

        // ── P14-D: Config-driven keybinding dispatch ──
        // All configurable actions are resolved through check_keybinding().
        // The resolved_keybindings map is populated from ConfigManager at
        // startup and falls back to default_keybindings() when no config exists.
        if let PhysicalKey::Code(code) = &event.physical_key {
            let key_name = keycode_to_name(code);

            // Ctrl+T → new tab
            if self.check_keybinding(
                "new_tab",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                self.open_tab();
                return;
            }
            // Ctrl+W → close tab (or close active pane if splits exist)
            if self.check_keybinding(
                "close_tab",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                if self.active_session().pane_count() > 1 {
                    // Multiple panes: close the active pane instead of the tab.
                    self.active_session_mut().remove_active_pane();
                } else {
                    self.close_tab();
                }
                return;
            }
            // Ctrl+= → zoom in
            if self.check_keybinding(
                "zoom_in",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                if self.font_zoom.zoom_in() {
                    self.apply_font_size();
                }
                return;
            }
            // Ctrl+- → zoom out
            if self.check_keybinding(
                "zoom_out",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                if self.font_zoom.zoom_out() {
                    self.apply_font_size();
                }
                return;
            }
            // Ctrl+0 → reset zoom
            if self.check_keybinding(
                "zoom_reset",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                if self.font_zoom.reset() {
                    self.apply_font_size();
                }
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
            // Ctrl+Shift+V → paste
            if self.check_keybinding(
                "paste",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                self.paste_from_clipboard();
                return;
            }
            // Ctrl+Shift+C → copy
            if self.check_keybinding(
                "copy",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                self.copy_selection_to_clipboard();
                return;
            }
            // Ctrl+Shift+K → clear
            if self.check_keybinding(
                "clear",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                crate::terminal_actions::clear_screen_and_scrollback(
                    self.active_session_mut().app_mut().grid_mut(),
                );
                return;
            }
            // Ctrl+Shift+R → reset terminal
            if self.check_keybinding(
                "reset",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                crate::terminal_actions::soft_reset(self.active_session_mut().app_mut().grid_mut());
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

        // Ctrl+Shift+D → horizontal split (left | right)
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyD) = &event.physical_key
        {
            self.split_pane_horizontal();
            return;
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
            self.active_session_mut().focus_next_pane();
            return;
        }

        // Ctrl+Shift+[ → focus previous pane
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::BracketLeft) = &event.physical_key
        {
            self.active_session_mut().focus_prev_pane();
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

        // Ctrl+Shift+B → toggle status bar visibility (not configurable)
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyB) = &event.physical_key
        {
            self.status_bar_visible = !self.status_bar_visible;
            return;
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
                    // TODO: execute pending action
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

        // Ctrl+, (comma) → toggle settings overlay (P19-C)
        if self.mods.ctrl
            && !self.mods.shift
            && let PhysicalKey::Code(KeyCode::Comma) = &event.physical_key
        {
            self.settings.toggle();
            return;
        }

        // P19-C: When settings overlay is open, intercept navigation keys.
        if self.settings.visible {
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::Escape) => {
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

        // Alt+1-9 → switch to tab N (not configurable)
        if self.mods.alt
            && !self.mods.ctrl
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
        if self.mods.ctrl
            && let PhysicalKey::Code(KeyCode::Tab) = &event.physical_key
        {
            if self.mods.shift {
                self.prev_tab();
            } else {
                self.next_tab();
            }
            return;
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
                // Ctrl+Shift+A → select all text (not configurable)
                KeyCode::KeyA => {
                    let grid = self.active_session().app().grid();
                    let range = crate::terminal_actions::select_all_range(grid);
                    self.selection
                        .start(range.start_col as u16, range.start_row as u16);
                    self.selection
                        .extend(range.end_col as u16, range.end_row as u16);
                    self.selection.finish();
                    return;
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
                if self.mods.shift {
                    search.prev_match();
                } else {
                    search.next_match();
                }
            }
            PhysicalKey::Code(KeyCode::Backspace) => {
                search.backspace(grid);
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
        crate::mouse::pixel_to_cell(self.cursor_pos.0, self.cursor_pos.1, cw, ch)
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

        // Need renderer to get screen dimensions for split area calculation.
        let Some(screen_w) = self.renderer.as_ref().map(|r| r.resolution_width()) else {
            return false;
        };
        let Some(screen_h) = self.renderer.as_ref().map(|r| r.resolution_height()) else {
            return false;
        };

        let bounds = crate::splits::Rect::new(0, 0, screen_w, screen_h);
        let (px, py) = (self.cursor_pos.0 as u32, self.cursor_pos.1 as u32);

        if let Some(hit_id) = session.split_tree().pane_at_point(px, py, bounds) {
            let active = session.split_tree().active();
            if hit_id != active {
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

        let Some(screen_w) = self.renderer.as_ref().map(|r| r.resolution_width()) else {
            return false;
        };
        let Some(screen_h) = self.renderer.as_ref().map(|r| r.resolution_height()) else {
            return false;
        };

        let bounds = crate::splits::Rect::new(0, 0, screen_w, screen_h);
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

        let (col, row) = self.pixel_to_cell_pos();
        let mods = crate::mouse::MouseModifiers {
            shift: self.mods.shift,
            ctrl: self.mods.ctrl,
            alt: self.mods.alt,
        };

        let term = self.active_session().app().terminal();

        // Check if mouse tracking is active.
        if term.mouse_tracking_enabled() {
            let mouse_ev = crate::mouse::MouseEvent {
                button: mouse_button,
                x: col,
                y: row,
                mods,
            };

            let sgr = term.mouse_sgr_enabled();
            let urxvt = term.mouse_urxvt_enabled();

            match state {
                ElementState::Pressed => {
                    self.button_held = Some(mouse_button);
                    if let Some(bytes) =
                        crate::mouse::encode_mouse_event(&mouse_ev, sgr, urxvt, true)
                    {
                        self.write_to_pty(&bytes);
                    }
                }
                ElementState::Released => {
                    self.button_held = None;
                    if let Some(bytes) =
                        crate::mouse::encode_mouse_event(&mouse_ev, sgr, urxvt, false)
                    {
                        self.write_to_pty(&bytes);
                    }
                }
            }
            return;
        }

        // Mouse tracking is OFF — handle selection and paste locally.
        match (mouse_button, state) {
            (crate::mouse::MouseButton::Left, ElementState::Pressed) => {
                self.button_held = Some(mouse_button);
                self.selection.start(col, row);
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            (crate::mouse::MouseButton::Left, ElementState::Released) => {
                // P17-C: Cmd+Click (macOS) or Ctrl+Click (other) opens hovered URL.
                let open_link = (cfg!(target_os = "macos") && self.mods.super_key)
                    || (!cfg!(target_os = "macos") && self.mods.ctrl);
                if open_link && let Some(ref url) = self.hovered_link.take() {
                    crate::mouse::open_url(url);
                    return;
                }

                self.button_held = None;
                self.selection.finish();
                // Copy selection to clipboard if active.
                if self.selection.is_active() {
                    self.copy_selection_to_clipboard();
                }
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            (crate::mouse::MouseButton::Middle, ElementState::Pressed) => {
                // Middle-click paste from system clipboard.
                self.paste_from_clipboard();
            }
            _ => {}
        }
    }

    /// Handle cursor motion — extend selection or report mouse motion.
    pub(super) fn handle_cursor_moved(&mut self) {
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
                    term.mouse_sgr_enabled()
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
            self.selection.extend(col, row);
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
        }

        // P17-C: Detect hovered URL (OSC 8 hyperlink or plain text).
        self.update_hovered_link(col, row);
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
                self.hovered_link = Some(link.clone());
                return;
            }
        }

        // Fall back to plain-text URL detection.
        if let Some(cell_row) = grid.display_row(row) {
            let line: String = cell_row.cells.iter().map(|c| c.ch).collect();
            if let Some((_, _, url)) = crate::mouse::detect_url_at_position(&line, col) {
                self.hovered_link = Some(url);
                return;
            }
        }

        self.hovered_link = None;
    }

    /// Handle mouse wheel events — scroll scrollback or report to PTY.
    pub(super) fn handle_mouse_wheel(&mut self, delta: winit::event::MouseScrollDelta) {
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

        // Mouse tracking OFF — scroll the scrollback buffer.
        let (lines, direction) = match delta {
            winit::event::MouseScrollDelta::LineDelta(_x, y) => (y.abs() as usize, y > 0.0),
            winit::event::MouseScrollDelta::PixelDelta(pos) => {
                let lines = (pos.y.abs() as f32 / 16.0).round() as usize;
                (lines.max(1), pos.y < 0.0) // Natural scroll: pixel up = scroll up
            }
        };

        let grid = self
            .active_session_mut()
            .app_mut()
            .terminal_mut()
            .grid_mut();
        if direction {
            grid.scroll_up_viewport(lines);
        } else {
            grid.scroll_down_viewport(lines);
        }

        if let Some(ref window) = self.window {
            window.request_redraw();
        }
    }
}
