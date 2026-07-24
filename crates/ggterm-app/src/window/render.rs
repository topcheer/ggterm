//! Frame rendering — render_frame() with multi-pane and overlay support.

/// The "×" character used in tab close buttons — pre-allocated to avoid
/// a String allocation per tab per frame.
const CLOSE_BTN_CHAR: &str = "\u{00d7}";

use super::*;

impl DesktopApp {
    /// Render one frame.
    pub(super) fn render_frame(&mut self) {
        // P12-A/P12-C: Get theme background color for clear color,
        // and blend with visual bell flash if active.
        let active = self.active;

        // Compute content area bounds BEFORE any borrows of self.
        // This is shared with mouse handlers for coordinate consistency.
        let content_bounds = self.content_area_bounds();
        let (br, bg, bb) = {
            let session = &self.sessions[active];
            let theme = session.app().theme();
            theme.resolve_bg(&theme.default_bg)
        };
        let bg_color = if self.visual_bell_frames > 0 {
            let intensity = self.visual_bell_frames as f64 / VISUAL_BELL_DURATION_FRAMES as f64;
            let flash = 0.3 * intensity;
            [
                (br as f64 / 255.0) + flash * (1.0 - br as f64 / 255.0),
                (bg as f64 / 255.0) + flash * (1.0 - bg as f64 / 255.0),
                (bb as f64 / 255.0) + flash * (1.0 - bb as f64 / 255.0),
                self.background_opacity(),
            ]
        } else {
            [
                br as f64 / 255.0,
                bg as f64 / 255.0,
                bb as f64 / 255.0,
                self.background_opacity(),
            ]
        };

        // Decrement visual bell counter.
        if self.visual_bell_frames > 0 {
            self.visual_bell_frames -= 1;
        }

        // Now borrow session for grid data (cursor built per-pane below).
        let session = &self.sessions[active];
        let grid = session.app().grid();

        // P23-A: Cursor blink state — applied per-pane in the multi-pane loop.
        // Per xterm spec: Ps=0 (Default) = blinking block (same as Ps=1).
        let is_blink_style = matches!(
            session.app().terminal().cursor_style(),
            ggterm_core::CursorStyle::Default
                | ggterm_core::CursorStyle::BlinkBlock
                | ggterm_core::CursorStyle::BlinkUnderline
                | ggterm_core::CursorStyle::BlinkBar
        );
        // Respect DECSET 12: even blink-style cursors stay steady when
        // the program has disabled cursor blinking (DECSET 12 = off).
        // Also respect user config: cursor_blink = false disables all blinking.
        let config_blink = self
            .config_mgr
            .as_ref()
            .is_none_or(|m| m.config().appearance.cursor_blink);
        let is_blink =
            is_blink_style && session.app().terminal().cursor_blink_enabled() && config_blink;
        self.cursor_blink.set_enabled(is_blink);
        let blink_alpha = self.cursor_blink.alpha_focused(self.window_focused);
        let blink_visible = self.cursor_blink.is_visible();

        // P16-A: Wire search match highlights to renderer.
        // Convert SearchMatch(abs_row, col, len) → (visible_row, col_start, col_end).
        // Must account for display_offset (scrollback scroll position).
        let scrollback_len = grid.scrollback_len();
        let grid_height = grid.height();
        let display_offset = grid.display_offset();
        let search_highlights: Vec<(usize, usize, usize)> = if self.search.visible {
            let matches = self.search.matches();
            let mut highlights = Vec::with_capacity(matches.len());
            for m in matches {
                // visible_row = abs_row - (scrollback_len - display_offset)
                // This maps absolute row to the row index currently shown on screen.
                let base = scrollback_len.saturating_sub(display_offset);
                if let Some(visible_row) = m.abs_row.checked_sub(base)
                    && visible_row < grid_height
                {
                    highlights.push((visible_row, m.col, m.col + m.len.saturating_sub(1)));
                }
            }
            highlights
        } else {
            Vec::new()
        };

        // Extract the current/active match for distinct highlighting.
        let current_highlight: Option<(usize, usize, usize)> = if self.search.visible {
            let base = scrollback_len.saturating_sub(display_offset);
            self.search.current().and_then(|m| {
                m.abs_row.checked_sub(base).and_then(|visible_row| {
                    if visible_row < grid_height {
                        Some((visible_row, m.col, m.col + m.len.saturating_sub(1)))
                    } else {
                        None
                    }
                })
            })
        } else {
            None
        };

        // ── Cursor line highlight (Vim-style cursorline) ──────────────
        // Pre-compute before the mutable borrow at line 95.
        let cursor_line_rect = {
            let enabled = self
                .config_mgr
                .as_ref()
                .is_some_and(|m| m.config().appearance.cursor_line_highlight);
            if enabled {
                let cell_h = self
                    .renderer
                    .as_ref()
                    .map(|r| r.cell_height())
                    .unwrap_or(20) as f32;
                let bounds = self.content_area_bounds();
                let (_, row) = self.active_session().app().cursor();
                let y_pos = bounds.y as f32 + row as f32 * cell_h;
                Some((bounds.x as f32, y_pos, bounds.width as f32, cell_h))
            } else {
                None
            }
        };

        // Pre-compute IME preedit data before mutable borrow of renderer.
        let ime_data = if let Some(ref preedit) = self.ime_preedit {
            let (ccol, crow) = self.sessions[self.active].app().terminal().cursor();
            let bounds = self.content_area_bounds();
            Some((preedit.clone(), ccol, crow, bounds))
        } else {
            None
        };

        // Pre-compute running command text for close-tab confirmation dialog.
        // Must be done before the renderer borrow (which holds &mut self).
        let close_cmd_hint: Option<String> = if self.pending_close_tab.is_some() {
            let term = self.active_session().app().terminal();
            let blocks = term.command_blocks();
            let grid = term.grid();
            blocks
                .into_iter()
                .rev()
                .find(|b| b.end_row.is_none() && b.command_row.is_some())
                .and_then(|b| {
                    let cmd_row = b.command_row?;
                    let sl = grid.scrollback_len();
                    if cmd_row < sl {
                        return None;
                    }
                    let row = grid.row(cmd_row - sl)?;
                    let text: String = row
                        .visible_cells()
                        .skip_while(|(_, c)| c.ch.is_whitespace())
                        .map(|(_, c)| if c.ch == '\0' { ' ' } else { c.ch })
                        .collect::<String>()
                        .trim()
                        .to_string();
                    if text.is_empty() { None } else { Some(text) }
                })
        } else {
            None
        };

        let (gpu, surface, renderer) = match (&mut self.gpu, &self.surface, &mut self.renderer) {
            (Some(g), Some(s), Some(r)) => (g, s, r),
            _ => return,
        };

        // Apply search highlights before rendering.
        renderer.set_highlights(search_highlights);
        renderer.set_current_highlight(current_highlight);

        // Apply dynamic colors (OSC 10/11) if set on the terminal.
        let term = self.sessions[self.active].app().terminal();
        renderer.set_dynamic_fg(term.dynamic_fg().map(|c| match c {
            ggterm_core::Color::Rgb(r, g, b) => (*r, *g, *b),
            _ => (240, 240, 240), // defensive fallback
        }));
        renderer.set_dynamic_bg(term.dynamic_bg().map(|c| match c {
            ggterm_core::Color::Rgb(r, g, b) => (*r, *g, *b),
            _ => (20, 20, 20), // defensive fallback
        }));

        // OSC 4: Sync custom palette overrides from terminal to renderer.
        // Only clone when the map actually changed (avoid per-frame HashMap clone).
        let palette = term.palette_overrides();
        if palette != renderer.palette_overrides_ref() {
            renderer.set_palette_overrides(palette.clone());
        }

        // SGR 5: Blink text — share the cursor blink phase for text blink.
        renderer.set_blink_phase(blink_alpha);

        // DECSCNM: Reverse video mode — swap fg/bg globally.
        renderer.set_reverse_video(session.app().terminal().reverse_video());

        // SGR 58: Underline color override.
        renderer.set_underline_color(match session.app().terminal().underline_color_ref() {
            ggterm_core::Color::Rgb(r, g, b) => Some((*r, *g, *b)),
            ggterm_core::Color::Indexed(i) => {
                let (r, g, b) = ggterm_core::term::color_for_index(*i);
                Some((r, g, b))
            }
            ggterm_core::Color::Default => None,
        });

        // P19-G: Build overlay data (tab bar + settings + about).
        // Reuse Vecs from struct to avoid per-frame allocation.
        let cell_h = renderer.cell_height() as f32;
        let cell_w = renderer.cell_width() as f32;
        let screen_w = renderer.resolution_width() as f32;
        let screen_h = renderer.resolution_height() as f32;
        let mut overlay_texts = std::mem::take(&mut self.render_overlay_texts);
        let mut ui_rects = std::mem::take(&mut self.render_ui_rects);
        let mut status_segments = std::mem::take(&mut self.render_status_segs);
        overlay_texts.clear();
        ui_rects.clear();
        status_segments.clear();

        // Theme background as normalized f32 — used for tab bar/status bar
        // so they match the terminal content instead of hardcoded colors.
        let theme_bg = (br as f32 / 255.0, bg as f32 / 255.0, bb as f32 / 255.0);

        // Update tab bar data — only when multiple tabs exist (visible).
        // Single-tab mode skips this to avoid per-frame Vec allocations.
        if self.sessions.len() > 1 {
            // Reuse struct-level buffers to avoid per-frame allocation.
            self.render_tab_titles.clear();
            self.render_bell_flags.clear();
            self.render_cmd_done_flags.clear();
            for s in self.sessions.iter() {
                if s.is_pinned() {
                    self.render_tab_titles
                        .push(format!("\u{1f4cc}{}", s.title()));
                } else {
                    self.render_tab_titles.push(s.title().to_string());
                }
                self.render_bell_flags.push(s.has_bell());
                self.render_cmd_done_flags.push(s.command_completed());
            }
            let title_refs: Vec<&str> = self.render_tab_titles.iter().map(|s| s.as_str()).collect();
            self.tab_bar.update_with_bells(
                &title_refs,
                self.active,
                &self.render_bell_flags,
                &self.render_cmd_done_flags,
            );
        }

        // ── Tab bar: auto-fill width like browser tabs ─────────────────
        if self.tab_bar.visible {
            let tab_h = (cell_h + 26.0).max(48.0);
            let bar_h = tab_h + 4.0;
            let tab_radius = 6.0_f32;
            let cell_w = renderer.cell_width() as f32;

            // Use compute_layout for auto-fill width calculation.
            let layout = self.tab_bar.compute_layout(screen_w, cell_h);

            // Tab bar background — theme-aware.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: 0.0,
                y: 0.0,
                w: screen_w,
                h: bar_h,
                color: (
                    theme_bg.0 * 1.15,
                    theme_bg.1 * 1.15,
                    theme_bg.2 * 1.15,
                    0.95,
                ),
                radius: 0.0,
                stroke_width: 0.0,
            });

            // Bottom border separator.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: 0.0,
                y: bar_h - 1.0,
                w: screen_w,
                h: 1.0,
                color: (theme_bg.0 * 1.6, theme_bg.1 * 1.6, theme_bg.2 * 1.6, 0.5),
                radius: 0.0,
                stroke_width: 0.0,
            });

            // macOS: subtle vertical divider after traffic lights.
            #[cfg(target_os = "macos")]
            {
                let tl_width = crate::titlebar::TRAFFIC_LIGHT_WIDTH;
                ui_rects.push(ggterm_render_wgpu::UiRect {
                    x: tl_width - 1.0,
                    y: 6.0,
                    w: 1.0,
                    h: bar_h - 12.0,
                    color: (theme_bg.0 * 2.0, theme_bg.1 * 2.0, theme_bg.2 * 2.0, 0.3),
                    radius: 0.0,
                    stroke_width: 0.0,
                });
            }

            // Linux/Windows: caption buttons (minimize/maximize/close).
            #[cfg(not(target_os = "macos"))]
            push_window_controls(
                &mut ui_rects,
                &mut overlay_texts,
                WindowControlParams {
                    screen_w,
                    bar_h,
                    cursor_x: self.cursor_pos.0 as f32,
                    cursor_y: self.cursor_pos.1 as f32,
                },
            );

            // Render each tab pill using layout positions.
            for (tab_idx, tl) in layout.tabs.iter().enumerate() {
                let tab = &tl.info;
                let x = tl.rect.x;
                let w = tl.rect.w;
                let tab_y = 4.0;

                // Unread output indicator: blue dot on non-active tabs.
                if !tab.active
                    && tab_idx < self.sessions.len()
                    && self.sessions[tab_idx].has_unread_output()
                {
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x: x + w - 12.0,
                        y: tab_y + 3.0,
                        w: 6.0,
                        h: 6.0,
                        color: (0.3, 0.6, 1.0, 0.9),
                        radius: 3.0,
                        stroke_width: 0.0,
                    });
                }

                // Running command indicator: green dot on non-active tabs.
                if !tab.active
                    && tab_idx < self.sessions.len()
                    && self.sessions[tab_idx].is_running()
                {
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x: x + w - 22.0,
                        y: tab_y + 3.0,
                        w: 6.0,
                        h: 6.0,
                        color: (0.3, 0.8, 0.4, 0.9),
                        radius: 3.0,
                        stroke_width: 0.0,
                    });
                }

                // Drag highlight: when dragging this tab, add a blue stroke.
                if self.dragging_tab == Some(tab_idx) {
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x: x - 1.0,
                        y: tab_y - 1.0,
                        w: w + 2.0,
                        h: tab_h + 2.0,
                        color: (0.48, 0.64, 0.97, 0.8),
                        radius: tab_radius + 1.0,
                        stroke_width: 2.0,
                    });
                }

                if tab.active {
                    // Active tab: brighter surface.
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x,
                        y: tab_y,
                        w,
                        h: tab_h,
                        color: (theme_bg.0 * 1.8, theme_bg.1 * 1.8, theme_bg.2 * 1.8, 0.95),
                        radius: tab_radius,
                        stroke_width: 0.0,
                    });
                    // Accent bottom border.
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x: x + tab_radius,
                        y: tab_y + tab_h - 2.0,
                        w: w - tab_radius * 2.0,
                        h: 2.0,
                        color: (0.48, 0.64, 0.97, 0.9),
                        radius: 0.0,
                        stroke_width: 0.0,
                    });
                } else {
                    // Inactive tab — brighten on hover.
                    let (cur_x, cur_y) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
                    let is_hovered =
                        cur_x >= x && cur_x < x + w && cur_y >= tab_y && cur_y < tab_y + tab_h;
                    let brightness = if is_hovered { 1.45 } else { 1.3 };
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x,
                        y: tab_y,
                        w,
                        h: tab_h,
                        color: (
                            theme_bg.0 * brightness,
                            theme_bg.1 * brightness,
                            theme_bg.2 * brightness,
                            0.7,
                        ),
                        radius: tab_radius,
                        stroke_width: 0.0,
                    });
                }

                // Tab title text — only truncate if genuinely too long.
                let title = tab.format();
                // Reserve space for close button only when 2+ tabs AND not pinned.
                let is_pinned = tab_idx < self.sessions.len() && self.sessions[tab_idx].is_pinned();
                // Prepend a pin indicator for pinned tabs so users understand
                // why the close button is absent.
                let title_with_pin = if is_pinned {
                    format!("\u{1F4CC} {title}") // 📌
                } else {
                    title
                };
                let reserved = if self.tab_bar.tabs.len() > 1 && !is_pinned {
                    24.0 // close "x" + margin
                } else {
                    8.0 // just right padding
                };
                let max_chars = ((w - 16.0 - reserved) / cell_w).floor() as usize;
                let display_title: String = if title_with_pin.chars().count() > max_chars {
                    format!(
                        "{}…",
                        title_with_pin
                            .chars()
                            .take(max_chars.saturating_sub(1))
                            .collect::<String>()
                    )
                } else {
                    title_with_pin
                };
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: display_title,
                    left: x + 12.0,
                    top: tab_y + 5.0,
                    color: if tab.active {
                        (210, 214, 232)
                    } else {
                        (120, 128, 154)
                    },
                    ..Default::default()
                });

                // Close button "x" — show on active tab, hovered tab, or when 2+ tabs.
                // Skip for pinned tabs (they can't be closed).
                if self.tab_bar.tabs.len() > 1 && !is_pinned {
                    // Check if mouse is hovering this tab.
                    let (cur_x, cur_y) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
                    let is_hovered =
                        cur_x >= x && cur_x < x + w && cur_y >= tab_y && cur_y < tab_y + tab_h;

                    // Check if hovering specifically the close button area.
                    let close_btn_hovered = is_hovered && cur_x >= x + w - 28.0 && cur_x < x + w;

                    // Close button background circle on hover.
                    if close_btn_hovered {
                        ui_rects.push(ggterm_render_wgpu::UiRect {
                            x: x + w - 22.0,
                            y: tab_y + 4.0,
                            w: 18.0,
                            h: tab_h - 8.0,
                            color: (0.86, 0.31, 0.31, 0.2),
                            radius: 9.0,
                            stroke_width: 0.0,
                        });
                    }
                    let close_x = x + w - 16.0 - cell_w * 0.5;
                    overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                        text: CLOSE_BTN_CHAR.into(),
                        left: close_x,
                        top: tab_y + 5.0,
                        color: if close_btn_hovered {
                            (220, 80, 80) // red on hover (browser-style)
                        } else if tab.active {
                            (190, 195, 210)
                        } else if is_hovered {
                            (160, 165, 180)
                        } else {
                            (80, 85, 100)
                        },
                        ..Default::default()
                    });
                }
            }

            // "+" multi-function button at the end.
            let btn_hovered = self.tab_bar.is_new_tab_button_at(
                &layout,
                self.cursor_pos.0 as f32,
                self.cursor_pos.1 as f32,
            );
            push_titlebar_button(
                &mut ui_rects,
                &mut overlay_texts,
                layout.new_tab_button.cx - layout.new_tab_button.size / 2.0,
                (bar_h - tab_h) / 2.0,
                layout.new_tab_button.size,
                tab_h,
                "+",
                btn_hovered,
                (0.35, 0.42, 0.55, 0.8),
                theme_bg,
                cell_w,
                cell_h,
                tab_radius,
            );

            // Settings gear button at the far right.
            let gear_hovered = self.tab_bar.is_settings_button_at(
                &layout,
                self.cursor_pos.0 as f32,
                self.cursor_pos.1 as f32,
            );
            push_titlebar_button(
                &mut ui_rects,
                &mut overlay_texts,
                layout.settings_button.cx - layout.settings_button.size / 2.0,
                (bar_h - tab_h) / 2.0,
                layout.settings_button.size,
                tab_h,
                "\u{2699}", // ⚙ gear symbol
                gear_hovered,
                (0.35, 0.42, 0.55, 0.8),
                theme_bg,
                cell_w,
                cell_h,
                tab_radius,
            );
            // Close of `if self.tab_bar.visible` block.
        } else if !self.tab_bar.tabs.is_empty() {
            // Single-tab mode: keep the title bar background strip but without
            // tab labels. Shows the current tab title on the left and "+"/"⚙"
            // buttons on the right. Taller bar and larger buttons for usability.
            let layout = super::SingleTabButtonLayout::compute(screen_w, cell_h);
            let bar_h = layout.bar_h;
            let btn_size = layout.btn_size;
            let cell_w = renderer.cell_width() as f32;

            // Title bar background — same theme-aware style as full tab bar.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: 0.0,
                y: 0.0,
                w: screen_w,
                h: bar_h,
                color: (
                    theme_bg.0 * 1.15,
                    theme_bg.1 * 1.15,
                    theme_bg.2 * 1.15,
                    0.95,
                ),
                radius: 0.0,
                stroke_width: 0.0,
            });

            // Bottom border separator — same as full tab bar.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: 0.0,
                y: bar_h - 1.0,
                w: screen_w,
                h: 1.0,
                color: (theme_bg.0 * 1.6, theme_bg.1 * 1.6, theme_bg.2 * 1.6, 0.5),
                radius: 0.0,
                stroke_width: 0.0,
            });

            // macOS: subtle vertical divider after traffic lights.
            #[cfg(target_os = "macos")]
            {
                let tl_width = crate::titlebar::TRAFFIC_LIGHT_WIDTH;
                ui_rects.push(ggterm_render_wgpu::UiRect {
                    x: tl_width - 1.0,
                    y: 6.0,
                    w: 1.0,
                    h: bar_h - 12.0,
                    color: (theme_bg.0 * 2.0, theme_bg.1 * 2.0, theme_bg.2 * 2.0, 0.3),
                    radius: 0.0,
                    stroke_width: 0.0,
                });
            }

            // Tab title on the left side (after traffic lights on macOS).
            #[cfg(target_os = "macos")]
            let title_x = crate::titlebar::TRAFFIC_LIGHT_WIDTH + 60.0;
            #[cfg(not(target_os = "macos"))]
            let title_x = 16.0;

            let title = self
                .tab_bar
                .tabs
                .first()
                .map(|t| t.title.as_str())
                .unwrap_or("ggterm");
            let display_title = if title.chars().count() > 50 {
                let truncated: String = title.chars().take(49).collect();
                format!("{truncated}…")
            } else {
                title.to_string()
            };
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: display_title,
                left: title_x,
                top: (bar_h - cell_h) / 2.0,
                color: (200, 205, 220),
                ..Default::default()
            });

            // "+" new-tab button.
            let plus_hovered =
                layout.is_on_plus(self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
            push_titlebar_button(
                &mut ui_rects,
                &mut overlay_texts,
                layout.plus_x,
                layout.btn_y,
                btn_size,
                btn_size,
                "+",
                plus_hovered,
                (0.35, 0.42, 0.55, 0.8),
                theme_bg,
                cell_w,
                cell_h,
                8.0,
            );

            // Settings gear button.
            let gear_hovered =
                layout.is_on_gear(self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
            push_titlebar_button(
                &mut ui_rects,
                &mut overlay_texts,
                layout.gear_x,
                layout.btn_y,
                btn_size,
                btn_size,
                "\u{2699}", // ⚙ gear symbol
                gear_hovered,
                (0.35, 0.42, 0.55, 0.8),
                theme_bg,
                cell_w,
                cell_h,
                8.0,
            );

            // ── Linux/Windows: caption buttons (minimize/maximize/close) ──
            #[cfg(not(target_os = "macos"))]
            push_window_controls(
                &mut ui_rects,
                &mut overlay_texts,
                WindowControlParams {
                    screen_w,
                    bar_h: layout.bar_h,
                    cursor_x: self.cursor_pos.0 as f32,
                    cursor_y: self.cursor_pos.1 as f32,
                },
            );
        }

        // ── Cursor line highlight (rendered if enabled) ───────────────
        if let Some((cx, cy, cw, ch)) = cursor_line_rect {
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: cx,
                y: cy,
                w: cw,
                h: ch,
                color: (1.0, 1.0, 1.0, 0.06), // very subtle white overlay
                radius: 0.0,
                stroke_width: 0.0,
            });
        }

        // ── P27-A: Text selection highlight ────────────────────────────
        // Draw semi-transparent blue rectangles over selected cells.
        // Block (rectangular) selection draws per-row rectangles.
        if self.selection.is_active()
            && self.selection.block_mode
            && let Some((x0, y0, x1, y1)) = self.selection.block_rect()
        {
            let (x0, y0, x1, y1) = (x0 as u32, y0 as u32, x1 as u32, y1 as u32);
            let pane_offset_x = content_bounds.x as f32;
            let pane_offset_y = content_bounds.y as f32;

            let (sr, sg, sb) = {
                let theme = self.sessions[self.active].app().theme();
                theme.resolve_bg(&theme.selection_bg)
            };
            let sel_color = (
                sr as f32 / 255.0,
                sg as f32 / 255.0,
                sb as f32 / 255.0,
                0.35,
            );

            for row in y0..=y1 {
                let px = pane_offset_x + x0 as f32 * cell_w;
                let py = pane_offset_y + row as f32 * cell_h;
                let pw = (x1 - x0 + 1) as f32 * cell_w;
                ui_rects.push(ggterm_render_wgpu::UiRect {
                    x: px,
                    y: py,
                    w: pw,
                    h: cell_h,
                    color: sel_color,
                    radius: 1.0,
                    stroke_width: 0.0,
                });
            }
        } else if self.selection.is_active()
            && let Some(((sx, sy), (ex, ey))) = self.selection.normalized()
        {
            let (sx, sy, ex, ey) = (sx as u32, sy as u32, ex as u32, ey as u32);

            // Selection coordinates are relative to the visible grid
            // (col, display_row). Convert to pixel positions in content area.
            let pane_offset_x = content_bounds.x as f32;
            let pane_offset_y = content_bounds.y as f32;

            // Selection color: use theme selection_bg with 30% alpha.
            let (sr, sg, sb) = {
                let theme = self.sessions[self.active].app().theme();
                theme.resolve_bg(&theme.selection_bg)
            };
            let sel_color = (
                sr as f32 / 255.0,
                sg as f32 / 255.0,
                sb as f32 / 255.0,
                0.35,
            );

            if sy == ey {
                // Single-row selection.
                let x = pane_offset_x + sx as f32 * cell_w;
                let y = pane_offset_y + sy as f32 * cell_h;
                let w = (ex - sx + 1) as f32 * cell_w;
                let h = cell_h;
                ui_rects.push(ggterm_render_wgpu::UiRect {
                    x,
                    y,
                    w,
                    h,
                    color: sel_color,
                    radius: 2.0,
                    stroke_width: 0.0,
                });
            } else {
                // Multi-row selection: first row (start to end of line).
                let x0 = pane_offset_x + sx as f32 * cell_w;
                let y0 = pane_offset_y + sy as f32 * cell_h;
                let w0 = content_bounds.width as f32 - sx as f32 * cell_w;
                ui_rects.push(ggterm_render_wgpu::UiRect {
                    x: x0,
                    y: y0,
                    w: w0,
                    h: cell_h,
                    color: sel_color,
                    radius: 2.0,
                    stroke_width: 0.0,
                });

                // Full rows in between.
                if ey > sy + 1 {
                    let full_x = pane_offset_x;
                    let full_y = pane_offset_y + (sy + 1) as f32 * cell_h;
                    let full_w = content_bounds.width as f32;
                    let full_h = (ey - sy - 1) as f32 * cell_h;
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x: full_x,
                        y: full_y,
                        w: full_w,
                        h: full_h,
                        color: sel_color,
                        radius: 0.0,
                        stroke_width: 0.0,
                    });
                }

                // Last row (start of line to end).
                let x1 = pane_offset_x;
                let y1 = pane_offset_y + ey as f32 * cell_h;
                let w1 = (ex + 1) as f32 * cell_w;
                ui_rects.push(ggterm_render_wgpu::UiRect {
                    x: x1,
                    y: y1,
                    w: w1,
                    h: cell_h,
                    color: sel_color,
                    radius: 2.0,
                    stroke_width: 0.0,
                });
            }
        }

        // ── Selection character count badge ──────────────────────────
        // When text is selected (not just a single cell), show a small
        // badge with the character count near the selection end.
        if let (Some((sx, sy)), Some((ex, ey))) = (self.selection.start, self.selection.end) {
            let (sx, sy, ex, ey) = if (sy, sx) <= (ey, ex) {
                (sx, sy, ex, ey)
            } else {
                (ex, ey, sx, sy)
            };
            // Skip if single-cell selection (no visible chars selected).
            if (sx, sy) != (ex, ey) {
                // Estimate selected character count from grid dimensions.
                let grid = self.sessions[self.active].app().grid();
                let cols = grid.width();
                let total_chars = if sy == ey {
                    (ex - sx + 1) as usize
                } else {
                    // First row: sx to end
                    let first = cols - sx as usize;
                    let last = ex as usize + 1;
                    let middle_rows = (ey - sy - 1) as usize * cols;
                    first + middle_rows + last
                };

                let badge_text = format!("{} chars", total_chars);
                let badge_w = badge_text.len() as f32 * cell_w + 16.0;
                let badge_h = cell_h + 6.0;
                let badge_x = content_bounds.x as f32 + (ex as f32 + 1.0) * cell_w + 4.0;
                let badge_y = content_bounds.y as f32 + (ey + 1) as f32 * cell_h + 2.0;

                // Clamp badge within content area.
                let max_x = content_bounds.x as f32 + content_bounds.width as f32 - badge_w;
                let badge_x = badge_x.min(max_x).max(content_bounds.x as f32);

                ui_rects.push(ggterm_render_wgpu::UiRect {
                    x: badge_x,
                    y: badge_y,
                    w: badge_w,
                    h: badge_h,
                    color: (0.2, 0.3, 0.5, 0.85),
                    radius: 4.0,
                    stroke_width: 0.0,
                });
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: badge_text,
                    left: badge_x + 8.0,
                    top: badge_y + 3.0,
                    color: (220, 220, 240),
                    ..Default::default()
                });
            }
        }

        // ── P26-D: Padded pane borders with rounded corners ───────────
        // Skip borders when pane zoom mode is active (only active pane visible).
        let active = self.active;
        let tree = &self.sessions[active].split_tree();
        if !tree.is_single() && !self.pane_zoomed {
            // Use the SAME content bounds as the pane grid rendering for
            // perfect alignment between borders and text content.
            let areas = tree.areas(content_bounds);
            let active_id = tree.active();
            let pane_radius = 4.0_f32;
            let pane_stroke_w = 2.0_f32;

            for (pane_id, rect) in &areas {
                let x = rect.x as f32;
                let y = rect.y as f32;
                let w = rect.width as f32;
                let h = rect.height as f32;

                // Active pane: glowing accent border (bright blue).
                // Inactive panes: dim subtle border.
                let border_color = if *pane_id == active_id {
                    (0.48, 0.64, 0.97, 0.8) // accent blue glow
                } else {
                    (theme_bg.0 * 2.5, theme_bg.1 * 2.5, theme_bg.2 * 2.5, 0.4) // dim theme-aware border
                };

                ui_rects.push(ggterm_render_wgpu::UiRect {
                    x,
                    y,
                    w,
                    h,
                    color: border_color,
                    radius: pane_radius,
                    stroke_width: pane_stroke_w,
                });

                // Pane background fill (slightly darker for depth).
                let bg_alpha = if *pane_id == active_id { 0.0 } else { 0.15 };
                if bg_alpha > 0.0 {
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x,
                        y,
                        w,
                        h,
                        color: (
                            theme_bg.0 * 0.5,
                            theme_bg.1 * 0.5,
                            theme_bg.2 * 0.5,
                            bg_alpha,
                        ),
                        radius: pane_radius,
                        stroke_width: 0.0,
                    });
                }
            }
        }

        // ── P26-F: Modern settings dialog with rounded corners ─────────
        if self.settings.visible {
            // Dark mask overlay.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: 0.0,
                y: 0.0,
                w: screen_w,
                h: screen_h,
                color: (0.02, 0.02, 0.04, 0.7),
                radius: 0.0,
                stroke_width: 0.0,
            });
            // Center panel with rounded corners.
            let pw = screen_w * 0.52;
            let ph = screen_h * 0.72;
            let px = (screen_w - pw) * 0.5;
            let py = (screen_h - ph) * 0.5;
            // Panel fill.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: px,
                y: py,
                w: pw,
                h: ph,
                color: (theme_bg.0 * 1.6, theme_bg.1 * 1.6, theme_bg.2 * 1.6, 0.98),
                radius: 12.0,
                stroke_width: 0.0,
            });
            // Panel border stroke.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: px,
                y: py,
                w: pw,
                h: ph,
                color: (0.45, 0.52, 0.68, 0.8),
                radius: 12.0,
                stroke_width: 1.0,
            });
            // Header accent bar.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: px,
                y: py,
                w: pw,
                h: 3.0,
                color: (0.26, 0.63, 0.95, 0.8),
                radius: 0.0,
                stroke_width: 0.0,
            });

            // Settings title.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "Settings".to_string(),
                left: px + 20.0,
                top: py + 18.0,
                color: (240, 240, 250),
                ..Default::default()
            });

            // Build field rows from SettingsState.
            let rows = self.settings.field_rows();
            let mut current_section: Option<crate::settings_ui::SettingsSection> = None;
            let mut y_offset = py + 44.0;

            for (section, label, value) in &rows {
                // Insert section header when section changes.
                if current_section != Some(*section) {
                    current_section = Some(*section);
                    y_offset += 6.0;
                    overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                        text: section.label().to_string(),
                        left: px + 20.0,
                        top: y_offset,
                        color: (90, 160, 230),
                        ..Default::default()
                    });
                    y_offset += cell_h;
                }

                // Check if this row is selected.
                let field_for_row = match (*section, *label) {
                    (crate::settings_ui::SettingsSection::Appearance, "Theme") => {
                        Some(crate::settings_ui::SettingsField::Theme)
                    }
                    (crate::settings_ui::SettingsSection::Appearance, "Font Size") => {
                        Some(crate::settings_ui::SettingsField::FontSize)
                    }
                    (crate::settings_ui::SettingsSection::Appearance, "Cursor Style") => {
                        Some(crate::settings_ui::SettingsField::CursorStyle)
                    }
                    (crate::settings_ui::SettingsSection::Appearance, "Font Family") => {
                        Some(crate::settings_ui::SettingsField::FontFamily)
                    }
                    (crate::settings_ui::SettingsSection::Terminal, "Scrollback") => {
                        Some(crate::settings_ui::SettingsField::Scrollback)
                    }
                    (crate::settings_ui::SettingsSection::Terminal, "Shell") => {
                        Some(crate::settings_ui::SettingsField::Shell)
                    }
                    (crate::settings_ui::SettingsSection::Terminal, "Restore Session") => {
                        Some(crate::settings_ui::SettingsField::RestoreSession)
                    }
                    (crate::settings_ui::SettingsSection::Ai, "AI Enabled") => {
                        Some(crate::settings_ui::SettingsField::AiEnabled)
                    }
                    (crate::settings_ui::SettingsSection::Ai, "AI Endpoint") => {
                        Some(crate::settings_ui::SettingsField::AiEndpoint)
                    }
                    (crate::settings_ui::SettingsSection::Ai, "AI Model") => {
                        Some(crate::settings_ui::SettingsField::AiModel)
                    }
                    _ => None,
                };
                let is_selected = field_for_row == Some(self.settings.selected);

                // Highlight selected row background.
                if is_selected {
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x: px + 12.0,
                        y: y_offset - 2.0,
                        w: pw - 24.0,
                        h: cell_h + 2.0,
                        color: (0.26, 0.52, 0.85, 0.25),
                        radius: 4.0,
                        stroke_width: 0.0,
                    });
                }

                // Row text: "  > Label: value" (selected) or "    Label: value"
                let prefix = if is_selected { "  > " } else { "    " };
                let row_color = if is_selected {
                    (255, 255, 255)
                } else {
                    (190, 190, 200)
                };
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: format!("{}{}: {}", prefix, label, value),
                    left: px + 20.0,
                    top: y_offset,
                    color: row_color,
                    ..Default::default()
                });
                y_offset += cell_h;
            }

            // Footer help text.
            y_offset += 8.0;
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: px + 12.0,
                y: y_offset,
                w: pw - 24.0,
                h: 1.0,
                color: (0.3, 0.3, 0.4, 0.5),
                radius: 0.0,
                stroke_width: 0.0,
            });
            y_offset += 6.0;
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "Up/Down: Navigate    Left/Right: Change    Esc: Close".to_string(),
                left: px + 20.0,
                top: y_offset,
                color: (120, 120, 140),
                ..Default::default()
            });

            // Error message if present.
            if let Some(err) = self.settings.error_text() {
                y_offset += cell_h;
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: format!("Error: {}", err),
                    left: px + 20.0,
                    top: y_offset,
                    color: (255, 100, 100),
                    ..Default::default()
                });
            }
        }

        // ── P26-F: Modern about dialog with rounded corners ──────────
        if self.about.visible {
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: 0.0,
                y: 0.0,
                w: screen_w,
                h: screen_h,
                color: (0.02, 0.02, 0.04, 0.7),
                radius: 0.0,
                stroke_width: 0.0,
            });
            let pw = screen_w * 0.4;
            let ph = screen_h * 0.32;
            let px = (screen_w - pw) * 0.5;
            let py = (screen_h - ph) * 0.5;
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: px,
                y: py,
                w: pw,
                h: ph,
                color: (theme_bg.0 * 1.6, theme_bg.1 * 1.6, theme_bg.2 * 1.6, 0.98),
                radius: 12.0,
                stroke_width: 0.0,
            });
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: px,
                y: py,
                w: pw,
                h: ph,
                color: (0.45, 0.52, 0.68, 0.8),
                radius: 12.0,
                stroke_width: 1.0,
            });
            // Header accent bar (gradient effect via overlapping rects).
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: px,
                y: py,
                w: pw,
                h: 3.0,
                color: (0.26, 0.63, 0.95, 0.8), // cyan-blue accent
                radius: 0.0,
                stroke_width: 0.0,
            });
            // ">_" terminal prompt symbol as visual logo.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: ">_".to_string(),
                left: px + 20.0,
                top: py + 16.0,
                color: (103, 232, 249), // cyan,
                ..Default::default()
            });
            let about_text = self.about.format_text();
            for (i, line) in about_text.lines().enumerate() {
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: line.to_string(),
                    left: px + 20.0,
                    top: py + 16.0 + (i + 1) as f32 * cell_h,
                    color: if i == 0 {
                        (122, 162, 247) // accent for title
                    } else {
                        (200, 200, 210)
                    },
                    ..Default::default()
                });
            }
        }

        // P2P share overlay — QR code + connection status.
        #[cfg(feature = "p2p")]
        if self.p2p_share.visible {
            let p2p = &self.p2p_share;

            // Dark mask background.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: 0.0,
                y: 0.0,
                w: screen_w,
                h: screen_h,
                color: (0.02, 0.02, 0.04, 0.85),
                radius: 0.0,
                stroke_width: 0.0,
            });

            // Panel dimensions.
            let pw = (screen_w * 0.55).min(520.0);
            // Extra height to fit wrapped ticket text without overflow.
            let ph = (screen_h * 0.78).min(680.0);
            let px = (screen_w - pw) * 0.5;
            let py = (screen_h - ph) * 0.5;
            let panel_pad = 24.0_f32;
            let inner_w = pw - panel_pad * 2.0;

            // Panel background.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: px,
                y: py,
                w: pw,
                h: ph,
                color: (theme_bg.0 * 1.4, theme_bg.1 * 1.4, theme_bg.2 * 1.4, 0.98),
                radius: 12.0,
                stroke_width: 0.0,
            });
            // Panel border.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: px,
                y: py,
                w: pw,
                h: ph,
                color: (0.45, 0.52, 0.68, 0.6),
                radius: 12.0,
                stroke_width: 1.0,
            });
            // Header accent bar.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: px,
                y: py,
                w: pw,
                h: 3.0,
                color: (0.26, 0.63, 0.95, 0.8),
                radius: 0.0,
                stroke_width: 0.0,
            });

            // ── Layout regions (top to bottom) ──
            // 1. Header: title + status + error      (~3 lines)
            // 2. QR code area
            // 3. Instructions + ticket + close hint   (~6 lines)
            let header_h = cell_h * 3.5 + 16.0;
            let footer_lines = 7.0; // max lines for instructions + ticket + close
            let footer_h = cell_h * footer_lines + 16.0;

            // QR code must fit between header and footer.
            let qr_max_h = (ph - header_h - footer_h).max(80.0);
            let qr_max_w = inner_w;
            let qr_max_px = qr_max_h.min(qr_max_w);

            // Title.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "P2P Terminal Sharing".to_string(),
                left: px + panel_pad,
                top: py + 20.0,
                color: (122, 162, 247),
                ..Default::default()
            });

            // Status text.
            let status_text = match p2p.status {
                crate::p2p_share::P2pShareStatus::Generating => "Generating ticket...",
                crate::p2p_share::P2pShareStatus::Waiting => "Waiting for device to connect...",
                crate::p2p_share::P2pShareStatus::Connected => "Device connected!",
                crate::p2p_share::P2pShareStatus::Error => "Error",
            };
            let status_color = match p2p.status {
                crate::p2p_share::P2pShareStatus::Generating => (200, 200, 210),
                crate::p2p_share::P2pShareStatus::Waiting => (103, 232, 249),
                crate::p2p_share::P2pShareStatus::Connected => (134, 239, 172),
                crate::p2p_share::P2pShareStatus::Error => (248, 113, 113),
            };
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: status_text.to_string(),
                left: px + panel_pad,
                top: py + 20.0 + cell_h,
                color: status_color,
                ..Default::default()
            });

            // Error message (if any) — wrap to fit panel width.
            if let Some(ref err) = p2p.error {
                let max_chars = (inner_w / cell_w).floor() as usize;
                for (i, line) in wrap_text(err, max_chars.max(10)).iter().enumerate() {
                    overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                        text: line.clone(),
                        left: px + panel_pad,
                        top: py + 20.0 + cell_h * (2.0 + i as f32),
                        color: (248, 113, 113),
                        ..Default::default()
                    });
                }
            }

            // QR code rendering: map QR modules to UiRects.
            let qr_y_start = py + header_h;
            if let Some(qr) = p2p.qr() {
                let qr_size = qr.len();
                let module_size = (qr_max_px / qr_size as f32).max(2.0);
                let qr_px = module_size * qr_size as f32;
                let qr_x = px + (pw - qr_px) * 0.5;
                let qr_y = qr_y_start + (qr_max_px - qr_px) * 0.5;

                // White background for QR code.
                ui_rects.push(ggterm_render_wgpu::UiRect {
                    x: qr_x - 8.0,
                    y: qr_y - 8.0,
                    w: qr_px + 16.0,
                    h: qr_px + 16.0,
                    color: (0.95, 0.95, 0.95, 1.0),
                    radius: 8.0,
                    stroke_width: 0.0,
                });

                // Render dark modules.
                for (row_idx, row) in qr.iter().enumerate() {
                    for (col_idx, &is_dark) in row.iter().enumerate() {
                        if is_dark {
                            ui_rects.push(ggterm_render_wgpu::UiRect {
                                x: qr_x + col_idx as f32 * module_size,
                                y: qr_y + row_idx as f32 * module_size,
                                w: module_size,
                                h: module_size,
                                color: (0.0, 0.0, 0.0, 1.0),
                                radius: 0.0,
                                stroke_width: 0.0,
                            });
                        }
                    }
                }
            }

            // Instructions — positioned right after QR area.
            let inst_y = py + header_h + qr_max_px + 8.0;
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "Scan QR code with GGTerm mobile app".to_string(),
                left: px + panel_pad,
                top: inst_y,
                color: (180, 180, 190),
                ..Default::default()
            });
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "or copy ticket manually:".to_string(),
                left: px + panel_pad,
                top: inst_y + cell_h,
                color: (180, 180, 190),
                ..Default::default()
            });

            // Ticket string — wrap to fit panel width to prevent overflow.
            let ticket = p2p.ticket();
            let max_ticket_chars = (inner_w / cell_w).floor() as usize;
            let ticket_lines = wrap_text(ticket, max_ticket_chars.max(20));
            for (i, line) in ticket_lines.iter().enumerate() {
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: line.clone(),
                    left: px + panel_pad,
                    top: inst_y + cell_h * (2.0 + i as f32),
                    color: (140, 160, 200),
                    ..Default::default()
                });
            }

            // Close hint.
            let hint_y = inst_y + cell_h * (2.0 + ticket_lines.len() as f32 + 0.5);
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "Press Esc or Ctrl+Shift+Alt+Q to close".to_string(),
                left: px + panel_pad,
                top: hint_y,
                color: (120, 120, 130),
                ..Default::default()
            });
        }

        // P26: Status bar overlay — modern rounded bottom bar with UiRect.
        if self.status_bar_visible {
            let bar_h = cell_h + 8.0;
            let bar_y = screen_h - bar_h;
            let pad_x = 12.0_f32;

            // Rounded background fill — theme-aware.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: pad_x,
                y: bar_y,
                w: screen_w - pad_x * 2.0,
                h: bar_h,
                color: (
                    theme_bg.0 * 1.15,
                    theme_bg.1 * 1.15,
                    theme_bg.2 * 1.15,
                    0.90,
                ),
                radius: 6.0,
                stroke_width: 0.0,
            });

            // Subtle top border stroke — theme-derived.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: pad_x,
                y: bar_y,
                w: screen_w - pad_x * 2.0,
                h: bar_h,
                color: (theme_bg.0 * 1.6, theme_bg.1 * 1.6, theme_bg.2 * 1.6, 0.5),
                radius: 6.0,
                stroke_width: 1.0,
            });

            // Render status bar text segments with individual colors.
            self.status_bar.format_segments_into(&mut status_segments);
            let cell_w = renderer.cell_width() as f32;
            let text_top = bar_y + 4.0;
            let mut x = pad_x + 8.0;

            // Right boundary: stop rendering segments that would overflow.
            let max_x = screen_w - pad_x - 8.0;
            for (text, color) in &status_segments {
                let text_w = text.chars().count() as f32 * cell_w;
                if x + text_w > max_x {
                    break;
                }
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: text.clone(),
                    left: x,
                    top: text_top,
                    color: *color,
                    ..Default::default()
                });
                x += text_w;
            }

            // Right-aligned "Share" button for P2P sharing.
            #[cfg(feature = "p2p")]
            {
                let share_text = if self.p2p_share.visible {
                    "Stop Share"
                } else {
                    "Share"
                };
                let share_text_len = share_text.chars().count() as f32;
                let share_w = share_text_len * cell_w + 24.0;
                let share_x = screen_w - pad_x - share_w - 8.0;
                let share_color = if self.p2p_share.visible {
                    (0.4, 0.6, 0.9, 0.3)
                } else {
                    (0.25, 0.28, 0.35, 0.6)
                };
                // Button background.
                ui_rects.push(ggterm_render_wgpu::UiRect {
                    x: share_x,
                    y: bar_y + 3.0,
                    w: share_w,
                    h: bar_h - 6.0,
                    color: share_color,
                    radius: 4.0,
                    stroke_width: 0.0,
                });
                // Button text.
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: share_text.to_string(),
                    left: share_x + 12.0,
                    top: text_top,
                    color: (180, 200, 240),
                    ..Default::default()
                });
            }
        }

        // ── P27-C: Context menu ───────────────────────────────────────
        if self.context_menu.visible {
            let cm = &self.context_menu;
            let (mx, my) = cm.pos;
            // Dynamic width: measure longest label.
            let max_label = crate::context_menu::ContextMenuAction::all()
                .iter()
                .map(|a| a.label().chars().count())
                .max()
                .unwrap_or(0);
            let menu_w = (max_label as f32 * cell_w + 32.0) // text + left/right padding
                .max(crate::context_menu::ContextMenuState::WIDTH);
            let mh = cm.menu_height();

            // Background — theme-aware.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: mx,
                y: my,
                w: menu_w,
                h: mh,
                color: (theme_bg.0 * 1.6, theme_bg.1 * 1.6, theme_bg.2 * 1.6, 0.97),
                radius: crate::context_menu::ContextMenuState::RADIUS,
                stroke_width: 0.0,
            });
            // Border — use a bright accent so it's clearly visible.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: mx,
                y: my,
                w: menu_w,
                h: mh,
                color: (0.45, 0.52, 0.68, 0.9),
                radius: crate::context_menu::ContextMenuState::RADIUS,
                stroke_width: 1.5,
            });

            // Menu items.
            for (i, action) in crate::context_menu::ContextMenuAction::all()
                .iter()
                .enumerate()
            {
                let (_, iy, _, _) = cm.item_rect(i);

                // Separators between action groups.
                if crate::context_menu::ContextMenuAction::separator_before(action) {
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x: mx + 8.0,
                        y: iy - 4.0,
                        w: menu_w - 16.0,
                        h: 1.0,
                        color: (theme_bg.0 * 2.0, theme_bg.1 * 2.0, theme_bg.2 * 2.0, 0.4),
                        radius: 0.0,
                        stroke_width: 0.0,
                    });
                }

                // Hover detection.
                let (cur_x, cur_y) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
                let is_hovered = cur_x >= mx
                    && cur_x < mx + menu_w
                    && cur_y >= iy
                    && cur_y < iy + crate::context_menu::ContextMenuState::ITEM_HEIGHT;

                // Context-aware enable/disable.
                let has_selection = self.selection.is_active();
                let has_url = self.hovered_link.is_some();
                let is_enabled = action.is_enabled(has_selection, has_url, true);

                // Hover highlight — only for enabled items.
                if is_hovered && is_enabled {
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x: mx + 4.0,
                        y: iy,
                        w: menu_w - 8.0,
                        h: crate::context_menu::ContextMenuState::ITEM_HEIGHT,
                        color: (0.35, 0.42, 0.60, 0.95),
                        radius: 4.0,
                        stroke_width: 0.0,
                    });
                }

                // Item text — dimmed if disabled, dark on hover, light otherwise.
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: action.label().to_string(),
                    left: mx + 16.0,
                    top: iy + 7.0,
                    color: if !is_enabled {
                        (90, 90, 100) // dimmed — action unavailable
                    } else if is_hovered {
                        (20, 20, 30)
                    } else {
                        (210, 215, 230)
                    },
                    ..Default::default()
                });

                // Right-aligned shortcut hint (dimmed, smaller spacing).
                if let Some(sc) = action.shortcut() {
                    let sc_w = sc.chars().count() as f32 * cell_w * 0.85;
                    overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                        text: sc.to_string(),
                        left: mx + menu_w - sc_w - 12.0,
                        top: iy + 7.0,
                        color: if !is_enabled {
                            (60, 60, 70)
                        } else if is_hovered {
                            (90, 90, 110)
                        } else {
                            (120, 125, 145)
                        },
                        ..Default::default()
                    });
                }
            }
            self.context_menu.effective_width = menu_w;
        }

        // ── P27-G: Scroll-to-bottom indicator ──────────────────────────
        // Show a "↓" indicator when scrolled up in scrollback.
        {
            let grid = self.sessions[active].app().terminal().grid();
            let is_scrolled = grid.is_scrolled();
            if is_scrolled {
                let offset = grid.display_offset();
                let indicator_y =
                    content_bounds.y as f32 + content_bounds.height as f32 - cell_h - 4.0;
                // Show "↓ N" for small offsets, "↓ N%" for larger ones.
                let scrollback_len = grid.scrollback_len();
                let total_lines = scrollback_len + grid.height();
                // Use float division so small scroll offsets still show a percentage.
                let pct = ((offset as f64 * 100.0) / total_lines.max(1) as f64) as u32;
                // When scrolled, always show position info — never "Bottom"
                // (the indicator only appears when is_scrolled is true).
                let label = if pct > 0 {
                    format!("\u{2193} {}%", pct)
                } else {
                    format!("\u{2193} {}L", offset)
                };

                // If new output arrived while scrolled up, show "+N" badge.
                let new_lines = self.new_output_while_scrolled;
                let label = if new_lines > 0 {
                    // Cap display to avoid overflow (e.g. "+9.9K").
                    let new_str = if new_lines >= 1000 {
                        format!("{}.{}K", new_lines / 1000, (new_lines % 1000) / 100)
                    } else {
                        new_lines.to_string()
                    };
                    format!("{label} +{new_str}")
                } else {
                    label
                };

                // Pill width adapts to label length.
                let label_len = label.chars().count().max(3) as f32;
                let pill_w = cell_w * (label_len + 1.5);
                let indicator_x =
                    content_bounds.x as f32 + content_bounds.width as f32 - pill_w - 4.0;
                // Store rect for click detection.
                self.scroll_indicator_rect = Some((indicator_x, indicator_y, pill_w, cell_h + 4.0));
                // Pill background — accent color when new output is pending.
                let pill_color = if new_lines > 0 {
                    (0.9, 0.5, 0.1, 0.85) // warm orange: attention needed
                } else {
                    (0.2, 0.4, 0.8, 0.7) // blue: informational
                };
                ui_rects.push(ggterm_render_wgpu::UiRect {
                    x: indicator_x,
                    y: indicator_y,
                    w: pill_w,
                    h: cell_h + 4.0,
                    color: pill_color,
                    radius: 4.0,
                    stroke_width: 0.0,
                });
                // Show scroll position: percentage for large offsets,
                // line count for small offsets.
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: label,
                    left: indicator_x + cell_w * 0.3,
                    top: indicator_y + 2.0,
                    color: (255, 255, 255),
                    ..Default::default()
                });
            } else {
                self.scroll_indicator_rect = None;
            }
        }

        // ── P28-F: Performance monitor overlay ─────────────────────────
        self.perf_monitor.record_frame();
        if self.perf_monitor.visible {
            let text = self.perf_monitor.format_display();
            let text_width = text.len() as f32 * (renderer.cell_width() as f32) + 16.0;
            let pm_y = 4.0_f32;
            let pm_x = screen_w - text_width - 12.0;

            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: pm_x,
                y: pm_y,
                w: text_width,
                h: cell_h + 8.0,
                color: (0.05, 0.05, 0.08, 0.8),
                radius: 6.0,
                stroke_width: 0.0,
            });
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text,
                left: pm_x + 8.0,
                top: pm_y + 4.0,
                color: (100, 200, 255),
                ..Default::default()
            });
        }

        // ── P28-H: Shell switcher dropdown ────────────────────────────
        if self.shell_switcher.open {
            let shells = self.shell_switcher.shells();
            // Dynamic width: measure longest shell label.
            let max_label = shells
                .iter()
                .map(|s| {
                    let base = if s.is_default { ">> " } else { "   " };
                    let ver = s.version.as_deref().unwrap_or("");
                    base.len() + s.name.len() + 1 + ver.len()
                })
                .max()
                .unwrap_or(10);
            let dd_w = (max_label as f32 * cell_w + 24.0).max(200.0);
            let dd_h = (shells.len() as f32 + 0.5) * (cell_h + 4.0);
            let dd_x = 12.0_f32;
            let dd_y = screen_h - dd_h - cell_h - 20.0;

            // Background — theme-aware.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: dd_x,
                y: dd_y,
                w: dd_w,
                h: dd_h,
                color: (theme_bg.0 * 1.6, theme_bg.1 * 1.6, theme_bg.2 * 1.6, 0.97),
                radius: 8.0,
                stroke_width: 0.0,
            });
            // Border — bright accent.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: dd_x,
                y: dd_y,
                w: dd_w,
                h: dd_h,
                color: (0.45, 0.52, 0.68, 0.9),
                radius: 8.0,
                stroke_width: 1.5,
            });

            for (i, shell) in shells.iter().enumerate() {
                let sy = dd_y + 4.0 + i as f32 * (cell_h + 4.0);
                let is_selected = i == self.shell_switcher.selected;
                let label = if shell.is_default {
                    format!(
                        ">> {} {}",
                        shell.name,
                        shell.version.as_deref().unwrap_or("")
                    )
                } else {
                    format!(
                        "   {} {}",
                        shell.name,
                        shell.version.as_deref().unwrap_or("")
                    )
                };
                // Hover detection (in addition to keyboard-selected).
                let (cur_x, cur_y) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
                let is_hovered = cur_x >= dd_x
                    && cur_x < dd_x + dd_w
                    && cur_y >= sy
                    && cur_y < sy + cell_h + 2.0;

                if is_selected || is_hovered {
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x: dd_x + 4.0,
                        y: sy,
                        w: dd_w - 8.0,
                        h: cell_h + 2.0,
                        color: if is_hovered && !is_selected {
                            (0.35, 0.42, 0.60, 0.95)
                        } else {
                            (theme_bg.0 * 2.0, theme_bg.1 * 2.0, theme_bg.2 * 2.0, 0.7)
                        },
                        radius: 4.0,
                        stroke_width: 0.0,
                    });
                }
                let color = if is_hovered && !is_selected {
                    (20, 20, 30)
                } else if is_selected {
                    (120, 200, 255)
                } else {
                    (200, 200, 200)
                };
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: label,
                    left: dd_x + 12.0,
                    top: sy + 2.0,
                    color,
                    ..Default::default()
                });
            }
        }

        // ── P28-C: Command history sidebar ────────────────────────────
        if self.cmd_history.visible {
            let sb_w = 280.0_f32;
            let sb_x = screen_w - sb_w - 4.0;
            let sb_y = 4.0_f32;
            let sb_h = screen_h - cell_h - 20.0; // leave room for status bar

            // Background — theme-aware.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: sb_x,
                y: sb_y,
                w: sb_w,
                h: sb_h,
                color: (theme_bg.0 * 1.6, theme_bg.1 * 1.6, theme_bg.2 * 1.6, 0.95),
                radius: 8.0,
                stroke_width: 0.0,
            });
            // Border.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: sb_x,
                y: sb_y,
                w: sb_w,
                h: sb_h,
                color: (0.45, 0.52, 0.68, 0.7),
                radius: 8.0,
                stroke_width: 1.0,
            });

            // Header.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "Command History".to_string(),
                left: sb_x + 12.0,
                top: sb_y + 8.0,
                color: (120, 200, 255),
                ..Default::default()
            });

            let total = self.cmd_history.len();
            let failed = self.cmd_history.failed_count();
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: format!("{} cmds | {} failed", total, failed),
                left: sb_x + 12.0,
                top: sb_y + cell_h + 12.0,
                color: (120, 120, 140),
                ..Default::default()
            });

            // Separator.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: sb_x + 8.0,
                y: sb_y + cell_h * 2.0 + 16.0,
                w: sb_w - 16.0,
                h: 1.0,
                color: (0.25, 0.27, 0.32, 0.6),
                radius: 0.0,
                stroke_width: 0.0,
            });

            // List recent commands (up to 20).
            let list_top = sb_y + cell_h * 2.0 + 24.0;
            let line_h = cell_h + 6.0;
            let max_items = ((sb_h - (list_top - sb_y) - 8.0) / line_h) as usize;
            let recent = self.cmd_history.recent(max_items);

            for (i, entry) in recent.iter().enumerate() {
                let ey = list_top + i as f32 * line_h;
                let is_selected = Some(entry.timestamp_ms)
                    == self
                        .cmd_history
                        .selected
                        .and_then(|idx| self.cmd_history.get(idx))
                        .map(|e| e.timestamp_ms);

                // Highlight selected row.
                if is_selected {
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x: sb_x + 4.0,
                        y: ey,
                        w: sb_w - 8.0,
                        h: line_h,
                        color: (0.15, 0.25, 0.45, 0.6),
                        radius: 4.0,
                        stroke_width: 0.0,
                    });
                }

                // Status indicator.
                let status_color = if entry.running {
                    (200, 200, 100)
                } else if entry.exit_code == Some(0) {
                    (100, 200, 120)
                } else {
                    (230, 80, 80)
                };
                let status_text = if entry.running {
                    "...".to_string()
                } else if entry.exit_code == Some(0) {
                    "OK".to_string()
                } else {
                    format!("E{}", entry.exit_code.unwrap_or(1))
                };
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: status_text,
                    left: sb_x + 10.0,
                    top: ey + 2.0,
                    color: status_color,
                    ..Default::default()
                });

                // Command text (truncated).
                let max_cmd_chars = ((sb_w - 70.0) / (renderer.cell_width() as f32)) as usize;
                let cmd_display = if entry.command.chars().count() > max_cmd_chars {
                    let truncated: String = entry.command.chars().take(max_cmd_chars).collect();
                    format!("{truncated}...")
                } else {
                    entry.command.clone()
                };
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: cmd_display,
                    left: sb_x + 50.0,
                    top: ey + 2.0,
                    color: if is_selected {
                        (255, 255, 255)
                    } else {
                        (200, 200, 210)
                    },
                    ..Default::default()
                });
            }
        }

        // ── P28-E: File drag-hover preview card ───────────────────────
        if self.file_preview.is_active()
            && let Some(ref preview) = self.file_preview.current
        {
            let card_w = 300.0_f32;
            let card_h = 80.0_f32;
            let card_x = self.file_preview.x;
            let card_y = self.file_preview.y;

            // Background.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: card_x,
                y: card_y,
                w: card_w,
                h: card_h,
                color: (0.1, 0.12, 0.18, 0.95),
                radius: 10.0,
                stroke_width: 0.0,
            });
            // Accent border.
            let (cr, cg, cb) = preview.category.color();
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: card_x,
                y: card_y,
                w: card_w,
                h: card_h,
                color: (
                    cr as f32 / 255.0 * 0.6,
                    cg as f32 / 255.0 * 0.6,
                    cb as f32 / 255.0 * 0.6,
                    0.8,
                ),
                radius: 10.0,
                stroke_width: 2.0,
            });

            // Category badge.
            let badge_w = 50.0_f32;
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: card_x + 8.0,
                y: card_y + 8.0,
                w: badge_w,
                h: 24.0,
                color: (
                    cr as f32 / 255.0 * 0.3,
                    cg as f32 / 255.0 * 0.3,
                    cb as f32 / 255.0 * 0.3,
                    0.9,
                ),
                radius: 4.0,
                stroke_width: 0.0,
            });
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: preview.category.icon_char().to_string(),
                left: card_x + 12.0,
                top: card_y + 12.0,
                color: (cr, cg, cb),
                ..Default::default()
            });

            // File name.
            let name_display = if preview.name.chars().count() > 30 {
                let truncated: String = preview.name.chars().take(27).collect();
                format!("{truncated}...")
            } else {
                preview.name.clone()
            };
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: name_display,
                left: card_x + 68.0,
                top: card_y + 12.0,
                color: (240, 240, 250),
                ..Default::default()
            });

            // File info: size + category.
            let size_str = preview
                .size
                .map(crate::file_preview::format_size)
                .unwrap_or_else(|| "—".to_string());
            let info = format!("{} | {}", size_str, preview.extension);
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: info,
                left: card_x + 68.0,
                top: card_y + 12.0 + cell_h + 4.0,
                color: (150, 150, 165),
                ..Default::default()
            });

            // Drop hint.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "Drop to insert path".to_string(),
                left: card_x + 12.0,
                top: card_y + card_h - cell_h - 8.0,
                color: (100, 180, 255),
                ..Default::default()
            });
        }

        // ── P28-B: Color picker hover swatch ──────────────────────────
        if let Some(ref hovered) = self.color_picker.hovered {
            let (cx, cy) = self.cursor_pos;
            let swatch_size = 24.0_f32;
            let hex_text_w = hovered.text.chars().count() as f32 * cell_w + 6.0;
            let total_w = swatch_size + hex_text_w;
            // Flip horizontally if near right edge.
            let swatch_x = if cx as f32 + 16.0 + total_w > screen_w {
                cx as f32 - total_w - 8.0
            } else {
                cx as f32 + 16.0
            };
            // Flip vertically if near bottom edge.
            let swatch_y = if cy as f32 + 16.0 + swatch_size > screen_h {
                cy as f32 - swatch_size - 8.0
            } else {
                cy as f32 + 16.0
            };

            // Color swatch (filled rounded rect).
            let (r, g, b) = hovered.rgb;
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: swatch_x,
                y: swatch_y,
                w: swatch_size,
                h: swatch_size,
                color: (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0),
                radius: 4.0,
                stroke_width: 0.0,
            });
            // Border.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: swatch_x,
                y: swatch_y,
                w: swatch_size,
                h: swatch_size,
                color: (0.8, 0.8, 0.8, 0.6),
                radius: 4.0,
                stroke_width: 1.0,
            });

            // Hex label next to the swatch.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: hovered.text.clone(),
                left: swatch_x + swatch_size + 6.0,
                top: swatch_y + 4.0,
                color: (r, g, b),
                ..Default::default()
            });
        }

        // ── P28-F: Cursor particle effects ────────────────────────────
        if self.cursor_particles.has_particles() {
            for p in self.cursor_particles.particles() {
                let alpha = p.alpha;
                ui_rects.push(ggterm_render_wgpu::UiRect {
                    x: p.x - p.size,
                    y: p.y - p.size,
                    w: p.size * 2.0,
                    h: p.size * 2.0,
                    color: (0.3, 0.6, 1.0, alpha),
                    radius: p.size,
                    stroke_width: 0.0,
                });
            }
        }

        // ── P28: Tab right-click context menu ─────────────────────────
        if self.tab_context_menu.visible {
            // Dynamic width: label_width + gap + shortcut_width + padding.
            let max_label = crate::tab_bar::TabMenuAction::all()
                .iter()
                .map(|a| a.label().chars().count())
                .max()
                .unwrap_or(0);
            let max_shortcut = crate::tab_bar::TabMenuAction::all()
                .iter()
                .map(|a| a.shortcut().chars().count())
                .max()
                .unwrap_or(0);
            // left_pad(12) + label + gap(2 cells) + shortcut + right_pad(12)
            let menu_w = max_label as f32 * cell_w
                + max_shortcut as f32 * cell_w
                + cell_w * 2.0 // gap between label and shortcut
                + 24.0; // left + right padding
            let menu_h = self.tab_context_menu.menu_height();
            let mx = self.tab_context_menu.x;
            let my = self.tab_context_menu.y;

            // Background — theme-aware.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: mx,
                y: my,
                w: menu_w,
                h: menu_h,
                color: (theme_bg.0 * 1.6, theme_bg.1 * 1.6, theme_bg.2 * 1.6, 0.97),
                radius: 8.0,
                stroke_width: 0.0,
            });
            // Border — bright accent.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: mx,
                y: my,
                w: menu_w,
                h: menu_h,
                color: (0.45, 0.52, 0.68, 0.9),
                radius: 8.0,
                stroke_width: 1.5,
            });

            for (i, action) in crate::tab_bar::TabMenuAction::all().iter().enumerate() {
                let iy = my
                    + 4.0
                    + i as f32
                        * (crate::tab_bar::TabContextMenuState::ITEM_HEIGHT
                            + crate::tab_bar::TabContextMenuState::ITEM_GAP);

                // Hover detection.
                let (px, py) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
                let is_hovered = px >= mx
                    && px < mx + menu_w
                    && py >= iy
                    && py < iy + crate::tab_bar::TabContextMenuState::ITEM_HEIGHT;

                if is_hovered {
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x: mx + 4.0,
                        y: iy,
                        w: menu_w - 8.0,
                        h: crate::tab_bar::TabContextMenuState::ITEM_HEIGHT,
                        color: (0.35, 0.42, 0.60, 0.95),
                        radius: 4.0,
                        stroke_width: 0.0,
                    });
                }

                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: action.label().to_string(),
                    left: mx + 12.0,
                    top: iy + 5.0,
                    color: if is_hovered {
                        (20, 20, 30)
                    } else {
                        (220, 220, 230)
                    },
                    ..Default::default()
                });

                // Shortcut hint (right-aligned).
                let shortcut = action.shortcut();
                if !shortcut.is_empty() {
                    overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                        text: shortcut.to_string(),
                        left: mx + menu_w - shortcut.len() as f32 * cell_w - 12.0,
                        top: iy + 5.0,
                        color: if is_hovered {
                            (40, 40, 60)
                        } else {
                            (120, 120, 140)
                        },
                        ..Default::default()
                    });
                }
            }
        }

        // ── "+" dropdown menu rendering ──────────────────────────────
        if self.new_tab_menu.visible {
            use crate::new_tab_menu::{NewTabMenuAction, NewTabMenuState};
            // Dynamic width: measure longest label.
            let max_label = NewTabMenuAction::all()
                .iter()
                .map(|a| a.label().chars().count())
                .max()
                .unwrap_or(0);
            let menu_w = (max_label as f32 * cell_w + 32.0).max(NewTabMenuState::WIDTH);
            self.new_tab_menu.effective_width = menu_w;
            let menu_h = self.new_tab_menu.menu_height();
            let mx = self.new_tab_menu.pos.0;
            let my = self.new_tab_menu.pos.1;

            // Background — theme-aware, slightly brighter than terminal bg.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: mx,
                y: my,
                w: menu_w,
                h: menu_h,
                color: (theme_bg.0 * 1.6, theme_bg.1 * 1.6, theme_bg.2 * 1.6, 0.97),
                radius: NewTabMenuState::RADIUS,
                stroke_width: 0.0,
            });
            // Border — bright accent for clear visibility.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: mx,
                y: my,
                w: menu_w,
                h: menu_h,
                color: (0.45, 0.52, 0.68, 0.9),
                radius: NewTabMenuState::RADIUS,
                stroke_width: 1.5,
            });

            for (i, action) in NewTabMenuAction::all().iter().enumerate() {
                let (_, iy, _, _) = self.new_tab_menu.item_rect(i);

                // Separator before Split items (after New Tab).
                if i == 1 {
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x: mx + 8.0,
                        y: iy - 4.0,
                        w: menu_w - 16.0,
                        h: 1.0,
                        color: (theme_bg.0 * 2.0, theme_bg.1 * 2.0, theme_bg.2 * 2.0, 0.4),
                        radius: 0.0,
                        stroke_width: 0.0,
                    });
                }

                // Hover detection.
                let (px, py) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
                let is_hovered = px >= mx
                    && px < mx + menu_w
                    && py >= iy
                    && py < iy + NewTabMenuState::ITEM_HEIGHT;

                if is_hovered {
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x: mx + 4.0,
                        y: iy,
                        w: menu_w - 8.0,
                        h: NewTabMenuState::ITEM_HEIGHT,
                        color: (0.35, 0.42, 0.60, 0.95),
                        radius: 4.0,
                        stroke_width: 0.0,
                    });
                }

                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: action.label().to_string(),
                    left: mx + 16.0,
                    top: iy + 7.0,
                    color: if is_hovered {
                        (20, 20, 30)
                    } else {
                        (210, 215, 230)
                    },
                    ..Default::default()
                });
            }
        }

        // ── P29-A: Shortcut help overlay ──────────────────────────────
        if self.shortcut_help.visible {
            let panel_w = 520.0;
            let panel_h = 480.0;
            let win_w = content_bounds.width as f32;
            let win_h = content_bounds.height as f32 + content_bounds.y as f32;
            let px = (win_w - panel_w) / 2.0;
            let py = (win_h - panel_h) / 2.0;

            // Dark mask over entire window.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: 0.0,
                y: 0.0,
                w: win_w,
                h: win_h,
                color: (0.0, 0.0, 0.0, 0.5),
                radius: 0.0,
                stroke_width: 0.0,
            });

            // Panel background.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: px,
                y: py,
                w: panel_w,
                h: panel_h,
                color: (theme_bg.0 * 1.6, theme_bg.1 * 1.6, theme_bg.2 * 1.6, 0.97),
                radius: 12.0,
                stroke_width: 0.0,
            });
            // Panel border.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: px,
                y: py,
                w: panel_w,
                h: panel_h,
                color: (0.45, 0.52, 0.68, 0.7),
                radius: 12.0,
                stroke_width: 1.0,
            });

            // Title.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "Keyboard Shortcuts".to_string(),
                left: px + 20.0,
                top: py + 16.0,
                color: (240, 240, 250),
                ..Default::default()
            });

            // Search field background.
            let search_y = py + 44.0;
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: px + 16.0,
                y: search_y,
                w: panel_w - 32.0,
                h: 28.0,
                color: (0.15, 0.17, 0.22, 0.8),
                radius: 6.0,
                stroke_width: 0.0,
            });

            // Search placeholder / query text.
            let search_display = if self.shortcut_help.query.is_empty() {
                "Type to search...".to_string()
            } else {
                self.shortcut_help.query.clone()
            };
            let search_color = if self.shortcut_help.query.is_empty() {
                (100, 100, 120)
            } else {
                (200, 220, 255)
            };
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: format!("> {}", search_display),
                left: px + 24.0,
                top: search_y + 5.0,
                color: search_color,
                ..Default::default()
            });

            // Shortcut entries.
            let entries = self.shortcut_help.filtered();
            let line_h = 22.0_f32;
            let max_rows = 14_usize;
            let scroll = self.shortcut_help.scroll.min(entries.len());
            let visible_entries = entries.iter().skip(scroll).take(max_rows);

            for (i, entry) in visible_entries.enumerate() {
                let ey = search_y + 40.0 + i as f32 * line_h;

                // Alternating row background for readability.
                if i % 2 == 0 {
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x: px + 12.0,
                        y: ey,
                        w: panel_w - 24.0,
                        h: line_h,
                        color: (0.12, 0.13, 0.17, 0.4),
                        radius: 0.0,
                        stroke_width: 0.0,
                    });
                }

                // Category badge.
                let (r, g, b) = entry.category.color();
                ui_rects.push(ggterm_render_wgpu::UiRect {
                    x: px + 20.0,
                    y: ey + 4.0,
                    w: 6.0,
                    h: line_h - 8.0,
                    color: (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 0.8),
                    radius: 2.0,
                    stroke_width: 0.0,
                });

                // Keys (monospace, highlighted).
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: entry.keys.clone(),
                    left: px + 36.0,
                    top: ey + 3.0,
                    color: (140, 200, 255),
                    ..Default::default()
                });

                // Description.
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: entry.description.clone(),
                    left: px + 200.0,
                    top: ey + 3.0,
                    color: (200, 200, 210),
                    ..Default::default()
                });

                // Category label (right side).
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: entry.category.label().to_string(),
                    left: px + panel_w - 80.0,
                    top: ey + 3.0,
                    color: (r, g, b),
                    ..Default::default()
                });
            }

            // Result count footer.
            let footer = format!(
                "{} shortcuts{} — Esc to close",
                entries.len(),
                if self.shortcut_help.query.is_empty() {
                    String::new()
                } else {
                    format!(" match \"{}\"", self.shortcut_help.query)
                }
            );
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: footer,
                left: px + 20.0,
                top: py + panel_h - 24.0,
                color: (120, 120, 140),
                ..Default::default()
            });
        }

        // ── P29-C: Quit confirmation dialog ───────────────────────────
        if self.quit_confirm {
            let win_w = content_bounds.width as f32;
            let win_h = content_bounds.height as f32 + content_bounds.y as f32;

            // Dark mask.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: 0.0,
                y: 0.0,
                w: win_w,
                h: win_h,
                color: (0.0, 0.0, 0.0, 0.6),
                radius: 0.0,
                stroke_width: 0.0,
            });

            let dlg_w = 400.0;
            let dlg_h = 160.0;
            let dx = (win_w - dlg_w) / 2.0;
            let dy = (win_h - dlg_h) / 2.0;

            // Dialog background.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: dx,
                y: dy,
                w: dlg_w,
                h: dlg_h,
                color: (0.1, 0.11, 0.15, 0.98),
                radius: 12.0,
                stroke_width: 0.0,
            });
            // Dialog border.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: dx,
                y: dy,
                w: dlg_w,
                h: dlg_h,
                color: (0.4, 0.3, 0.3, 0.5),
                radius: 12.0,
                stroke_width: 1.0,
            });

            // Title.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "Close Terminal?".to_string(),
                left: dx + 24.0,
                top: dy + 20.0,
                color: (255, 180, 100),
                ..Default::default()
            });

            // Count running processes across all sessions.
            let running_count = self
                .sessions
                .iter()
                .filter(|s| s.app().terminal().is_command_running())
                .count();

            // Message — adapt based on whether processes are running.
            let msg1 = if running_count > 0 {
                format!("{running_count} session(s) have running processes.")
            } else {
                "All sessions are at a shell prompt.".to_string()
            };
            let msg2 = "Session state will be saved.".to_string();

            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: msg1,
                left: dx + 24.0,
                top: dy + 52.0,
                color: (200, 200, 210),
                ..Default::default()
            });
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: msg2,
                left: dx + 24.0,
                top: dy + 70.0,
                color: (160, 160, 170),
                ..Default::default()
            });

            // Options.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "[Y] Yes, close   [N] Cancel".to_string(),
                left: dx + 24.0,
                top: dy + 110.0,
                color: (140, 180, 255),
                ..Default::default()
            });

            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "Press Esc to cancel".to_string(),
                left: dx + 24.0,
                top: dy + 132.0,
                color: (120, 120, 140),
                ..Default::default()
            });
        }

        // ── P30-C: Toast notification ─────────────────────────────────
        if let Some((msg, frames)) = &self.toast {
            // Total lifetime = 120 frames (~2s at 60fps).
            // Fade-in: first 12 frames (frames > 108).
            // Full opacity: 108 >= frames > 24.
            // Fade-out: last 24 frames.
            let fade_in = ((120 - *frames) as f32 / 12.0).min(1.0);
            let fade_out = if *frames > 24 {
                1.0
            } else {
                *frames as f32 / 24.0
            };
            let alpha = fade_in.min(fade_out);

            // Slide-up offset: start 10px lower during fade-in.
            let slide = (1.0 - fade_in) * 10.0;

            // Use char count for width (not byte length — CJK/emoji are multi-byte).
            let char_count = msg.chars().count() as f32;
            let toast_w = (char_count * cell_w + 24.0).max(80.0);
            let toast_h = 32.0;
            let tx = (screen_w - toast_w) / 2.0;
            // Position above status bar to avoid overlap.
            let bottom_margin = if self.status_bar_visible {
                cell_h + 8.0 + 6.0 // status bar height + gap
            } else {
                16.0
            };
            let ty = screen_h - bottom_margin - toast_h + slide;

            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: tx,
                y: ty,
                w: toast_w,
                h: toast_h,
                color: (0.12, 0.14, 0.20, 0.95 * alpha),
                radius: 8.0,
                stroke_width: 0.0,
            });
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: tx,
                y: ty,
                w: toast_w,
                h: toast_h,
                color: (0.3, 0.5, 0.9, 0.5 * alpha),
                radius: 8.0,
                stroke_width: 1.0,
            });
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: msg.clone(),
                left: tx + 12.0,
                top: ty + 7.0,
                color: (200, 220, 255),
                ..Default::default()
            });
        }

        // ── P30-B: Tab rename input field ─────────────────────────────
        if let Some(rename_idx) = self.renaming_tab
            && self.tab_bar.visible
        {
            let layout = self.tab_bar.compute_layout(screen_w, cell_h);
            if let Some(tab_layout) = layout.tabs.get(rename_idx) {
                let rx = tab_layout.rect.x;
                let ry = tab_layout.rect.y;
                let rw = tab_layout.rect.w;
                let rh = tab_layout.rect.h;

                // Input field background.
                ui_rects.push(ggterm_render_wgpu::UiRect {
                    x: rx,
                    y: ry,
                    w: rw,
                    h: rh,
                    color: (theme_bg.0 * 1.6, theme_bg.1 * 1.6, theme_bg.2 * 1.6, 0.98),
                    radius: 6.0,
                    stroke_width: 0.0,
                });
                // Input field border.
                ui_rects.push(ggterm_render_wgpu::UiRect {
                    x: rx,
                    y: ry,
                    w: rw,
                    h: rh,
                    color: (0.3, 0.55, 0.95, 0.7),
                    radius: 6.0,
                    stroke_width: 1.0,
                });

                // Text with cursor.
                let display = format!("{}_ ", self.rename_text);
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: display,
                    left: rx + 8.0,
                    top: ry + 5.0,
                    color: (240, 240, 250),
                    ..Default::default()
                });
            }
        }

        // ── P30-A: Scrollbar ──────────────────────────────────────────
        // Show a thin scrollbar on the right edge when there's scrollback
        // and we're NOT in alternate screen mode (vim, less, htop, etc).
        {
            let is_alt = self.sessions[self.active].app().terminal().is_alt_screen();
            let scrollback_len = grid.scrollback_len();
            if scrollback_len > 0 && !is_alt {
                let total_rows = scrollback_len + grid.height();
                let visible_ratio = grid.height() as f32 / total_rows as f32;
                let visible_ratio = visible_ratio.clamp(0.05, 1.0);

                // display_offset: 0 = at bottom (most recent), scrollback_len = at top.
                // Scroll position from top: scrollback_len - display_offset.
                let scroll_from_top =
                    (scrollback_len - grid.display_offset()) as f32 / total_rows as f32;

                // Detect hover over scrollbar area.
                let cursor_near_right = self.cursor_pos.0 as f32 > screen_w - 24.0;
                let bar_w = if cursor_near_right { 6.0 } else { 4.0 };
                let bar_x = screen_w - bar_w - 2.0;
                let bar_track_y = content_bounds.y as f32;
                let bar_track_h = content_bounds.height as f32;
                let thumb_h = (bar_track_h * visible_ratio).max(20.0);
                let thumb_y = bar_track_y + scroll_from_top * (bar_track_h - thumb_h);

                // Track (faint background).
                // Theme-aware: use white on dark themes, dark on light themes.
                let theme_ref = self.sessions[self.active].app().theme();
                let is_light_theme =
                    matches!(theme_ref.default_bg, ggterm_core::Color::Rgb(r, _, _) if r > 180);
                let (track_rgb, thumb_rgb) = if is_light_theme {
                    ((0.0, 0.0, 0.0), (0.0, 0.0, 0.0))
                } else {
                    ((1.0, 1.0, 1.0), (1.0, 1.0, 1.0))
                };
                ui_rects.push(ggterm_render_wgpu::UiRect {
                    x: bar_x,
                    y: bar_track_y,
                    w: bar_w,
                    h: bar_track_h,
                    color: (track_rgb.0, track_rgb.1, track_rgb.2, 0.04),
                    radius: 3.0,
                    stroke_width: 0.0,
                });

                // Thumb — brighter when scrolled or hovered.
                let thumb_alpha = if grid.is_scrolled() {
                    if cursor_near_right { 0.7 } else { 0.5 }
                } else if cursor_near_right {
                    0.4
                } else {
                    0.2
                };
                ui_rects.push(ggterm_render_wgpu::UiRect {
                    x: bar_x,
                    y: thumb_y,
                    w: bar_w,
                    h: thumb_h,
                    color: (thumb_rgb.0, thumb_rgb.1, thumb_rgb.2, thumb_alpha),
                    radius: 3.0,
                    stroke_width: 0.0,
                });
            }
        }

        // ── P32: Floating search bar overlay ─────────────────────────
        if self.search.visible {
            let bar_w = 420.0;
            let bar_h = 40.0;
            let bar_x = screen_w - bar_w - 16.0;
            let bar_y = content_bounds.y as f32 + 8.0;

            // Background — theme-aware.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: bar_x,
                y: bar_y,
                w: bar_w,
                h: bar_h,
                color: (theme_bg.0 * 1.6, theme_bg.1 * 1.6, theme_bg.2 * 1.6, 0.97),
                radius: 10.0,
                stroke_width: 0.0,
            });
            // Accent border — red when no matches found, blue otherwise.
            let no_match = !self.search.query.is_empty() && self.search.match_count() == 0;
            let border_color = if no_match {
                (0.8, 0.3, 0.3, 0.8)
            } else {
                (0.35, 0.42, 0.60, 0.8)
            };
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: bar_x,
                y: bar_y,
                w: bar_w,
                h: bar_h,
                color: border_color,
                radius: 10.0,
                stroke_width: 1.5,
            });

            // Search icon "🔍" → use ">" as simplified icon.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: ">".to_string(),
                left: bar_x + 12.0,
                top: bar_y + 10.0,
                color: (100, 140, 200),
                ..Default::default()
            });

            // Mode indicator: "Aa" = case-insensitive, "AA" = case-sensitive.
            let mode_label = if self.search.case_insensitive {
                "Aa"
            } else {
                "AA"
            };
            let mode_color = if self.search.case_insensitive {
                (120u8, 160, 200)
            } else {
                (200u8, 180, 100)
            };
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: mode_label.to_string(),
                left: bar_x + 30.0,
                top: bar_y + 10.0,
                color: mode_color,
                ..Default::default()
            });

            // Regex mode indicator: ".*" = regex on (green), "Ab" = literal (dim).
            let regex_label = if self.search.regex_mode { ".*" } else { "Ab" };
            let regex_color = if self.search.regex_mode {
                (100u8, 220, 120)
            } else {
                (100u8, 100, 110)
            };
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: regex_label.to_string(),
                left: bar_x + 48.0,
                top: bar_y + 10.0,
                color: regex_color,
                ..Default::default()
            });

            // Search query text with cursor (red if no matches).
            let query_display = format!("{}_", self.search.query);
            let query_color = if !self.search.query.is_empty() && self.search.match_count() == 0 {
                (220u8, 100, 100) // Red when no matches.
            } else {
                (230u8, 230, 240) // Normal.
            };
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: query_display,
                left: bar_x + 72.0,
                top: bar_y + 10.0,
                color: query_color,
                ..Default::default()
            });

            // Match count — right-aligned.
            let match_info = self.search.match_count();
            let count_text = if match_info > 0 {
                let current = self.search.current_index().map(|i| i + 1).unwrap_or(0);
                format!("{}/{}", current, match_info)
            } else if !self.search.query.is_empty() {
                "No results".to_string()
            } else {
                String::new()
            };
            if !count_text.is_empty() {
                let count_color = if match_info > 0 {
                    (100u8, 140, 180)
                } else {
                    (200u8, 100, 100)
                };
                let count_w = count_text.chars().count() as f32 * cell_w;
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: count_text,
                    left: bar_x + bar_w - count_w - 14.0,
                    top: bar_y + 10.0,
                    color: count_color,
                    ..Default::default()
                });
            }

            // Hint text at bottom.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "\u{21b5}next  Shift+\u{21b5}prev  Tab: case  Shift+Tab: regex  Esc: close"
                    .to_string(),
                left: bar_x + 12.0,
                top: bar_y + bar_h + 4.0,
                color: (110, 110, 130),
                ..Default::default()
            });
        }

        // ── Pipe Selection to Shell Command input overlay ────────────
        if self.pipe_command_active {
            let bar_w = 480.0;
            let bar_h = 40.0;
            let bar_x = (screen_w - bar_w) / 2.0;
            let bar_y = content_bounds.y as f32 + 8.0;

            // Background — theme-aware.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: bar_x,
                y: bar_y,
                w: bar_w,
                h: bar_h,
                color: (theme_bg.0 * 1.6, theme_bg.1 * 1.6, theme_bg.2 * 1.6, 0.97),
                radius: 10.0,
                stroke_width: 0.0,
            });
            // Accent border — green to distinguish from search (blue).
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: bar_x,
                y: bar_y,
                w: bar_w,
                h: bar_h,
                color: (0.25, 0.60, 0.35, 0.8),
                radius: 10.0,
                stroke_width: 1.5,
            });

            // Prompt icon "$ |"
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "$ |".to_string(),
                left: bar_x + 12.0,
                top: bar_y + 10.0,
                color: (100, 200, 120),
                ..Default::default()
            });

            // Command input with block cursor.
            let cmd_display = format!("{}▎", self.pipe_command_input);
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: cmd_display,
                left: bar_x + 36.0,
                top: bar_y + 10.0,
                color: (230, 230, 240),
                ..Default::default()
            });

            // Hint text at bottom.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "Enter: run  Esc: cancel  (selection piped to stdin, result to clipboard)"
                    .to_string(),
                left: bar_x + 12.0,
                top: bar_y + bar_h + 4.0,
                color: (110, 110, 130),
                ..Default::default()
            });
        }

        // ── Command Palette overlay ──────────────────────────────────
        if self.command_palette.visible {
            let results = self.command_palette.results(&self.command_registry);
            let palette_w = (520.0_f32).min(screen_w - 40.0);
            let palette_h = 360.0;
            let palette_x = (screen_w - palette_w) / 2.0;
            let palette_y = 60.0;

            // Background.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: palette_x,
                y: palette_y,
                w: palette_w,
                h: palette_h,
                color: (theme_bg.0 * 1.6, theme_bg.1 * 1.6, theme_bg.2 * 1.6, 0.97),
                radius: 12.0,
                stroke_width: 0.0,
            });
            // Accent border.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: palette_x,
                y: palette_y,
                w: palette_w,
                h: palette_h,
                color: (0.35, 0.45, 0.65, 0.8),
                radius: 12.0,
                stroke_width: 1.5,
            });

            // Query input row background.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: palette_x + 8.0,
                y: palette_y + 8.0,
                w: palette_w - 16.0,
                h: 36.0,
                color: (theme_bg.0 * 2.0, theme_bg.1 * 2.0, theme_bg.2 * 2.0, 0.5),
                radius: 8.0,
                stroke_width: 0.0,
            });

            // ">" prompt icon.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: ">".to_string(),
                left: palette_x + 16.0,
                top: palette_y + 16.0,
                color: (100, 140, 200),
                ..Default::default()
            });

            // Query text with block cursor.
            let query_display = format!("{}▎", self.command_palette.query);
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: query_display,
                left: palette_x + 32.0,
                top: palette_y + 16.0,
                color: (230, 230, 240),
                ..Default::default()
            });

            // Result items.
            let item_h = 28.0;
            let list_y = palette_y + 52.0;
            for (i, (cmd, _score)) in results.iter().enumerate().take(10) {
                let item_y = list_y + i as f32 * item_h;
                let is_selected = i == self.command_palette.selected;

                if is_selected {
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x: palette_x + 8.0,
                        y: item_y,
                        w: palette_w - 16.0,
                        h: item_h - 2.0,
                        color: (0.3, 0.45, 0.7, 0.3),
                        radius: 6.0,
                        stroke_width: 0.0,
                    });
                }

                let label_color = if is_selected {
                    (255, 255, 255)
                } else {
                    (200, 200, 210)
                };

                // Command label.
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: cmd.label.clone(),
                    left: palette_x + 20.0,
                    top: item_y + 5.0,
                    color: label_color,
                    ..Default::default()
                });

                // Shortcut on the right (if any).
                if let Some(ref shortcut) = cmd.shortcut {
                    overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                        text: shortcut.clone(),
                        left: palette_x + palette_w - shortcut.len() as f32 * 8.0 - 20.0,
                        top: item_y + 5.0,
                        color: (100, 140, 200),
                        ..Default::default()
                    });
                }
            }

            // Show "No results" when search yields nothing.
            if results.is_empty() && !self.command_palette.query.is_empty() {
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: "No matching commands".to_string(),
                    left: palette_x + 20.0,
                    top: list_y + 5.0,
                    color: (130, 130, 145),
                    ..Default::default()
                });
            }

            // Hint at bottom.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "↑↓ navigate  Enter: run  Esc: close".to_string(),
                left: palette_x + 16.0,
                top: palette_y + palette_h - 22.0,
                color: (110, 110, 130),
                ..Default::default()
            });
        }

        // ── P33: URL hover tooltip + underline ────────────────────────
        if let Some((ref url, start_col, end_col, link_row)) = self.hovered_link {
            // Draw background highlight beneath the URL text.
            let hl_x = content_bounds.x as f32 + start_col as f32 * cell_w;
            let hl_w = (end_col - start_col) as f32 * cell_w;
            let hl_y = content_bounds.y as f32 + link_row as f32 * cell_h;
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: hl_x,
                y: hl_y,
                w: hl_w,
                h: cell_h,
                color: (0.3, 0.5, 0.9, 0.15),
                radius: 2.0,
                stroke_width: 0.0,
            });

            // Draw underline beneath the URL text.
            let under_x = content_bounds.x as f32 + start_col as f32 * cell_w;
            let under_w = (end_col - start_col) as f32 * cell_w;
            let under_y = content_bounds.y as f32 + (link_row + 1) as f32 * cell_h - 2.0;
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: under_x,
                y: under_y,
                w: under_w,
                h: 1.5,
                color: (0.4, 0.6, 1.0, 0.8),
                radius: 0.0,
                stroke_width: 0.0,
            });

            // Tooltip showing the full URL.
            let tooltip_w = (url.chars().count() as f32 * cell_w + 24.0).min(400.0);
            let tooltip_h = 24.0;
            let px = self.cursor_pos.0 as f32 + 14.0;
            let py = self.cursor_pos.1 as f32 + 14.0;
            let px = px.min(screen_w - tooltip_w - 8.0);

            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: px,
                y: py,
                w: tooltip_w,
                h: tooltip_h,
                color: (theme_bg.0 * 1.6, theme_bg.1 * 1.6, theme_bg.2 * 1.6, 0.95),
                radius: 6.0,
                stroke_width: 0.0,
            });
            let display_url = if url.chars().count() > 55 {
                format!("{}...", url.chars().take(52).collect::<String>())
            } else {
                url.clone()
            };
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: display_url,
                left: px + 10.0,
                top: py + 5.0,
                color: (100, 180, 255),
                ..Default::default()
            });
        }

        // AI overlay panel — bottom of screen, shows AI responses.
        #[cfg(feature = "ai")]
        if self.ai_overlay.is_visible() {
            let ai = &self.ai_overlay;
            let panel_h = (screen_h * 0.35).clamp(120.0, 300.0);
            let panel_y = screen_h - panel_h - cell_h; // above status bar
            let panel_w = (screen_w * 0.85).min(800.0);
            let panel_x = (screen_w - panel_w) * 0.5;

            // Background.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: panel_x,
                y: panel_y,
                w: panel_w,
                h: panel_h,
                color: (0.06, 0.08, 0.12, 0.95),
                radius: 10.0,
                stroke_width: 0.0,
            });
            // Border — accent blue.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: panel_x,
                y: panel_y,
                w: panel_w,
                h: panel_h,
                color: (0.30, 0.58, 0.95, 0.5),
                radius: 10.0,
                stroke_width: 1.0,
            });
            // Header accent bar.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: panel_x,
                y: panel_y,
                w: panel_w,
                h: 3.0,
                color: (0.26, 0.63, 0.95, 0.8),
                radius: 0.0,
                stroke_width: 0.0,
            });

            // Header: AI label + action name + tool indicator.
            let header_text = match ai.action() {
                Some(action) => format!("AI · {} · tools on", action.label()),
                None => "AI".to_string(),
            };
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: header_text,
                left: panel_x + 16.0,
                top: panel_y + 12.0,
                color: (122, 162, 247),
                ..Default::default()
            });

            // Close hint on the right.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "Esc".to_string(),
                left: panel_x + panel_w - 40.0,
                top: panel_y + 12.0,
                color: (100, 100, 110),
                ..Default::default()
            });

            // Content area.
            let content_y = panel_y + 14.0 + cell_h;
            let content_w = panel_w - 32.0;
            let max_chars = (content_w / cell_w).floor() as usize;

            if ai.is_nl2cmd_typing() {
                // NL2Command input mode: show prompt with cursor.
                let prompt_text = format!("> {}_", ai.nl2cmd_input());
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: prompt_text,
                    left: panel_x + 16.0,
                    top: content_y,
                    color: (200, 220, 255),
                    ..Default::default()
                });
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: "Type what you want to do, then press Enter".to_string(),
                    left: panel_x + 16.0,
                    top: content_y + cell_h * 1.5,
                    color: (120, 120, 135),
                    ..Default::default()
                });
            } else if ai.is_busy() && ai.content().is_none() {
                // Show "Thinking" with animated dots.
                let dots = match self.frame_count / 20 % 4 {
                    0 => ".  ",
                    1 => ".. ",
                    _ => "...",
                };
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: format!("Thinking{dots}"),
                    left: panel_x + 16.0,
                    top: content_y,
                    color: (130, 130, 140),
                    ..Default::default()
                });
            } else if let Some(content) = ai.content() {
                // Display content, wrapped to fit panel width.
                let is_error = content.starts_with("Error:");
                let text_color = if is_error {
                    (248, 113, 113)
                } else {
                    (200, 205, 220)
                };

                #[cfg(feature = "p2p")]
                let lines = wrap_text(content, max_chars.max(20));
                #[cfg(not(feature = "p2p"))]
                let lines: Vec<String> = {
                    let mut result = Vec::new();
                    for line in content.lines() {
                        let chars: Vec<char> = line.chars().collect();
                        let mut start = 0;
                        while start < chars.len() {
                            let end = (start + max_chars.max(20)).min(chars.len());
                            result.push(chars[start..end].iter().collect());
                            start = end;
                        }
                    }
                    if result.is_empty() {
                        result.push(String::new());
                    }
                    result
                };

                let max_lines = ((panel_h - 14.0 - cell_h * 2.0) / cell_h).floor() as usize;
                for (i, line) in lines.iter().enumerate().take(max_lines.max(3)) {
                    // Markdown-aware coloring: code blocks and inline code get distinct colors.
                    let is_error_line = line.starts_with("Error:");
                    let is_code_block_delim = line.trim_start().starts_with("```");
                    let is_command_line = line.starts_with("$ ");
                    let is_header = line.starts_with("# ") || line.starts_with("## ");

                    let color = if is_error_line {
                        (248, 113, 113) // red
                    } else if is_code_block_delim {
                        (80, 85, 95) // dim grey for fence markers
                    } else if is_command_line {
                        (130, 200, 255) // cyan for commands
                    } else if is_header {
                        (180, 190, 210) // bright for headers
                    } else if text_color == (200, 205, 220) {
                        // Normal text — check for inline code markers.
                        (200, 205, 220)
                    } else {
                        text_color
                    };

                    // Strip markdown artifacts for display.
                    let display = if is_code_block_delim {
                        // Don't show ``` markers, show a separator line instead.
                        "─".repeat(((panel_w - 32.0) / cell_w) as usize)
                    } else {
                        line.clone()
                    };

                    overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                        text: display,
                        left: panel_x + 16.0,
                        top: content_y + cell_h * i as f32,
                        color,
                        ..Default::default()
                    });
                }
            }
        }

        // AI overlay: footer hint for Tab/Ctrl+Enter actions.
        #[cfg(feature = "ai")]
        if self.ai_overlay.is_visible()
            && !self.ai_overlay.is_busy()
            && !self.ai_overlay.is_nl2cmd_typing()
        {
            let hint_y = screen_h - cell_h - 4.0;
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "Tab: insert command | Ctrl+Enter: run | Esc: close".to_string(),
                left: (screen_w - 52.0 * cell_w) * 0.5,
                top: hint_y,
                color: (100, 110, 125),
                ..Default::default()
            });
        }

        // Clipboard feedback: green border flash on copy/paste.
        let cb_intensity = self.clipboard_feedback.intensity();
        if cb_intensity > 0.0 {
            let (cb_x, cb_y, cb_w, cb_h) = (
                content_bounds.x as f32,
                content_bounds.y as f32,
                content_bounds.width as f32,
                content_bounds.height as f32,
            );
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: cb_x,
                y: cb_y,
                w: cb_w,
                h: cb_h,
                color: (0.3, 0.8, 0.4, 0.5 * cb_intensity),
                radius: 4.0,
                stroke_width: 3.0,
            });
        }

        // ── Terminal lock indicator: persistent amber border ──
        // Shows when input is locked so the user never wonders why
        // keyboard input is being ignored.
        if self.locked {
            let (lx, ly, lw, lh) = (
                content_bounds.x as f32,
                content_bounds.y as f32,
                content_bounds.width as f32,
                content_bounds.height as f32,
            );
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: lx,
                y: ly,
                w: lw,
                h: lh,
                color: (0.9, 0.6, 0.1, 0.35),
                radius: 4.0,
                stroke_width: 2.0,
            });
        }

        // IME preedit text: add to overlay_texts (not replace) so that
        // toast/status bar/search bar remain visible during CJK composition.
        if let Some((ref preedit, ime_ccol, ime_crow, ref ime_bounds)) = ime_data {
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: preedit.clone(),
                left: ime_bounds.x as f32 + ime_ccol as f32 * cell_w,
                top: ime_bounds.y as f32 + ime_crow as f32 * cell_h,
                color: (255, 255, 255),
                ..Default::default()
            });
        }

        renderer.set_ui_rects(ui_rects);
        renderer.set_overlay_text(overlay_texts);

        // ── Large paste confirmation dialog ──────────────────────
        if let Some(ref pending) = self.pending_large_paste {
            let kb = pending.len() / 1024;
            let msg = format!("Paste {kb}KB? Enter=confirm  Esc=cancel");

            // Dim overlay covering the full screen.
            let mut paste_ui: Vec<ggterm_render_wgpu::UiRect> = Vec::with_capacity(3);
            paste_ui.push(ggterm_render_wgpu::UiRect {
                x: 0.0,
                y: 0.0,
                w: screen_w,
                h: screen_h,
                color: (0.0, 0.0, 0.0, 0.6),
                radius: 0.0,
                stroke_width: 0.0,
            });

            // Centered dialog panel.
            let char_w = renderer.cell_width() as f32;
            let panel_w = msg.chars().count() as f32 * char_w + 48.0;
            let panel_h = cell_h + 32.0;
            let panel_x = (screen_w - panel_w) / 2.0;
            let panel_y = (screen_h - panel_h) / 2.0;
            paste_ui.push(ggterm_render_wgpu::UiRect {
                x: panel_x,
                y: panel_y,
                w: panel_w,
                h: panel_h,
                color: (0.12, 0.13, 0.16, 0.95),
                radius: 8.0,
                stroke_width: 0.0,
            });
            // Accent border.
            paste_ui.push(ggterm_render_wgpu::UiRect {
                x: panel_x,
                y: panel_y,
                w: panel_w,
                h: panel_h,
                color: (0.35, 0.55, 0.95, 0.0),
                radius: 8.0,
                stroke_width: 1.5,
            });

            renderer.set_ui_rects(paste_ui);
            renderer.set_overlay_text(vec![ggterm_render_wgpu::OverlayTextSpec {
                text: msg,
                left: panel_x + 24.0,
                top: panel_y + 16.0,
                color: (240, 240, 250),
                ..Default::default()
            }]);
        }

        // ── Tab close confirmation dialog ─────────────────────────
        if self.pending_close_tab.is_some() {
            let msg = if let Some(ref cmd) = close_cmd_hint {
                let display: String = cmd.chars().take(40).collect();
                let suffix = if cmd.chars().count() > 40 { "…" } else { "" };
                format!("Process running: {display}{suffix} — close again to confirm")
            } else {
                "Process still running. Close tab again to confirm.".to_string()
            };
            let char_w = renderer.cell_width() as f32;
            let mut close_ui: Vec<ggterm_render_wgpu::UiRect> = Vec::with_capacity(3);
            close_ui.push(ggterm_render_wgpu::UiRect {
                x: 0.0,
                y: 0.0,
                w: screen_w,
                h: screen_h,
                color: (0.0, 0.0, 0.0, 0.5),
                radius: 0.0,
                stroke_width: 0.0,
            });
            let panel_w = msg.chars().count() as f32 * char_w + 48.0;
            let panel_h = cell_h + 32.0;
            let panel_x = (screen_w - panel_w) / 2.0;
            let panel_y = (screen_h - panel_h) / 2.0;
            close_ui.push(ggterm_render_wgpu::UiRect {
                x: panel_x,
                y: panel_y,
                w: panel_w,
                h: panel_h,
                color: (0.12, 0.13, 0.16, 0.95),
                radius: 8.0,
                stroke_width: 0.0,
            });
            close_ui.push(ggterm_render_wgpu::UiRect {
                x: panel_x,
                y: panel_y,
                w: panel_w,
                h: panel_h,
                color: (0.9, 0.6, 0.2, 0.0),
                radius: 8.0,
                stroke_width: 1.5,
            });
            renderer.set_ui_rects(close_ui);
            renderer.set_overlay_text(vec![ggterm_render_wgpu::OverlayTextSpec {
                text: msg.to_string(),
                left: panel_x + 24.0,
                top: panel_y + 16.0,
                color: (240, 220, 180),
                ..Default::default()
            }]);
        }

        // IME preedit text overlay — show in-progress CJK composition text
        // as an underline-styled overlay at the cursor position.
        if let Some((preedit, ime_ccol, ime_crow, ime_bounds)) = ime_data {
            let char_count = preedit.chars().count();
            // Use content area x/y offset so preedit aligns with the actual
            // cursor position. bounds.y already includes tab bar height.
            let ime_rects: Vec<ggterm_render_wgpu::OverlayRect> = (0..char_count)
                .map(|i| ggterm_render_wgpu::OverlayRect {
                    x: ime_bounds.x as f32 + (ime_ccol + i) as f32 * cell_w,
                    y: ime_bounds.y as f32 + (ime_crow + 1) as f32 * cell_h - 2.0,
                    w: cell_w,
                    h: 2.0,
                    color: (1.0, 1.0, 1.0),
                })
                .collect();
            renderer.set_overlay_rects(ime_rects);
            // Preedit text is already in overlay_texts (added before set_overlay_text).
        } else {
            // Clear IME overlay rects when preedit is not active,
            // preventing stale underline bars from persisting on screen.
            renderer.set_overlay_rects(Vec::new());
        }

        // P20-A: Multi-pane viewport rendering.
        // When the active session has multiple panes, render each pane's grid
        // at its SplitTree area offset within a single render pass.

        let cell_h_px = renderer.cell_height();
        let cell_w_px = renderer.cell_width();
        let bounds = content_bounds;

        // Always use multi-pane rendering path for consistent overlay text
        // rendering (Issue: single-pane used render_to_pass which merges
        // overlay text with grid text; multi-pane renders them separately,
        // causing subtle font differences).
        {
            // When pane zoom is active, render only the active pane at full bounds.
            let pane_zoomed = self.pane_zoomed;

            // Resize panes to match their areas BEFORE rendering.
            {
                let session = &mut self.sessions[active];
                if pane_zoomed {
                    // Zoom: resize active pane to full content area.
                    let active_id = session.split_tree().active();
                    let zoom_areas = vec![(
                        active_id,
                        crate::splits::Rect {
                            x: bounds.x,
                            y: bounds.y,
                            width: bounds.width,
                            height: bounds.height,
                        },
                    )];
                    session.resize_panes_to_areas(&zoom_areas, cell_w_px, cell_h_px);
                } else {
                    let tree = session.split_tree().clone();
                    let areas = tree.areas(bounds);
                    session.resize_panes_to_areas(&areas, cell_w_px, cell_h_px);
                }
            }

            let session = &self.sessions[active];
            let tree = session.split_tree();

            // When zoomed, render only the active pane at full bounds.
            let active_id = tree.active();
            let areas: Vec<(usize, crate::splits::Rect)> = if pane_zoomed {
                vec![(
                    active_id,
                    crate::splits::Rect {
                        x: bounds.x,
                        y: bounds.y,
                        width: bounds.width,
                        height: bounds.height,
                    },
                )]
            } else {
                tree.areas(bounds)
            };

            // Build cursor states per pane (owned values, no borrow issues).
            let cursors: Vec<_> = areas
                .iter()
                .filter_map(|(id, _)| {
                    session.pane_app(*id).map(|app| {
                        let mut cs = cursor_state(app);
                        // Hide cursor when scrolled into scrollback history.
                        if app.terminal().grid().is_scrolled() {
                            cs.visible = false;
                        }
                        // Dim cursor when window is unfocused or pane is inactive (hollow outline).
                        cs.focused = self.window_focused && *id == active_id;
                        // Apply blink from DesktopApp's cursor_blink state.
                        if cs.visible {
                            cs.blink_alpha = blink_alpha;
                            cs.visible = blink_visible;
                        }
                        cs
                    })
                })
                .collect();

            // Build PaneRenderSpec list (grid refs borrow session, cursors from local vec).
            let mut specs: Vec<crate::gpu::PaneRenderSpec> = Vec::new();
            for ((pane_id, rect), cursor) in areas.iter().zip(cursors.iter()) {
                if let Some(app) = session.pane_app(*pane_id) {
                    specs.push(crate::gpu::PaneRenderSpec {
                        grid: app.grid(),
                        cursor,
                        offset_x: rect.x,
                        offset_y: rect.y,
                        width: rect.width,
                        height: rect.height,
                        reverse_video: app.terminal().reverse_video(),
                        dynamic_fg: app.terminal().dynamic_fg().map(|c| match c {
                            ggterm_core::Color::Rgb(r, g, b) => (*r, *g, *b),
                            _ => (240, 240, 240),
                        }),
                        dynamic_bg: app.terminal().dynamic_bg().map(|c| match c {
                            ggterm_core::Color::Rgb(r, g, b) => (*r, *g, *b),
                            _ => (20, 20, 20),
                        }),
                        underline_color: match app.terminal().underline_color_ref() {
                            ggterm_core::Color::Rgb(r, g, b) => Some((*r, *g, *b)),
                            ggterm_core::Color::Indexed(i) => {
                                let (r, g, b) = ggterm_core::term::color_for_index(*i);
                                Some((r, g, b))
                            }
                            ggterm_core::Color::Default => None,
                        },
                    });
                }
            }

            if let Err(e) = gpu.render_multi_pane_frame(surface, renderer, &specs, bg_color) {
                log::error!("Render error: {e}");
            }

            // Clear content_dirty to prevent 100% CPU on idle.
            self.sessions[active].clear_content_dirty();
        }

        // P19-C: Debug log tab bar state (data already updated before render).
        // Guard with log_enabled! to avoid format() allocation when debug logging is off.
        if self.tab_bar.visible && log::log_enabled!(log::Level::Debug) {
            log::debug!("tab_bar: {}", self.tab_bar.format());
        }

        // P19-C: Settings overlay logging.
        if self.settings.visible && log::log_enabled!(log::Level::Debug) {
            log::debug!("settings: {}", self.settings.format_summary());
        }

        // status_segments was a local scratch buffer — move it back so
        // next frame can reuse the capacity. overlay_texts/ui_rects were
        // consumed by renderer via set_ui_rects()/set_overlay_text();
        // std::mem::take left empty Vecs in the struct fields.
        self.render_status_segs = status_segments;
    }
}

/// Render a title bar button with background container and centered icon.
/// Parameters for rendering Linux/Windows window control buttons.
#[cfg(not(target_os = "macos"))]
struct WindowControlParams {
    screen_w: f32,
    bar_h: f32,
    cursor_x: f32,
    cursor_y: f32,
}

/// Render Linux/Windows window caption buttons (minimize/maximize/close).
///
/// Matches Windows 10/11 caption button conventions:
/// - Full-height buttons, flush right, no gaps
/// - Order left-to-right: Minimize, Maximize, Close
/// - Hover: close → red (#E81123), others → semi-transparent white
/// - Icons drawn as Unicode glyphs, centered in each button
#[cfg(not(target_os = "macos"))]
fn push_window_controls(
    ui_rects: &mut Vec<ggterm_render_wgpu::UiRect>,
    overlay_texts: &mut Vec<ggterm_render_wgpu::OverlayTextSpec>,
    p: WindowControlParams,
) {
    use crate::titlebar::WindowControlButton;

    let layout = crate::titlebar::compute_caption_layout(p.screen_w, p.bar_h);

    for (btn, b, glyph) in [
        (WindowControlButton::Minimize, &layout.minimize, "\u{2014}"), // ─
        (WindowControlButton::Maximize, &layout.maximize, "\u{25A1}"), // □
        (WindowControlButton::Close, &layout.close, "\u{2715}"),       // ✕
    ] {
        let hovered = p.cursor_x >= b.x
            && p.cursor_x <= b.x + b.w
            && p.cursor_y >= b.y
            && p.cursor_y <= b.y + b.h;

        // Hover background — Windows-style full-button highlight.
        if hovered {
            let bg = match btn {
                WindowControlButton::Close => (0.91, 0.07, 0.14, 1.0), // #E81123
                _ => (1.0, 1.0, 1.0, 0.12),                            // semi-transparent white
            };
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: b.x,
                y: b.y,
                w: b.w,
                h: b.h,
                color: bg,
                radius: 0.0,
                stroke_width: 0.0,
            });
        }

        // Icon color: white normally, white on close hover.
        let icon_color = if hovered && btn == WindowControlButton::Close {
            (255, 255, 255u8)
        } else {
            (200, 200, 200u8)
        };

        // Center the glyph in the button.
        let glyph_x = b.x + (b.w - 12.0) / 2.0; // approx glyph width
        let glyph_y = b.y + (b.h - 16.0) / 2.0;

        overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
            text: glyph.to_string(),
            left: glyph_x,
            top: glyph_y,
            color: icon_color,
            ..Default::default()
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn push_titlebar_button(
    ui_rects: &mut Vec<ggterm_render_wgpu::UiRect>,
    overlay_texts: &mut Vec<ggterm_render_wgpu::OverlayTextSpec>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    icon: &str,
    hovered: bool,
    hover_color: (f32, f32, f32, f32),
    theme_bg: (f32, f32, f32),
    cell_w: f32,
    cell_h: f32,
    radius: f32,
) {
    // Background container.
    ui_rects.push(ggterm_render_wgpu::UiRect {
        x,
        y,
        w,
        h,
        color: if hovered {
            hover_color
        } else {
            (theme_bg.0 * 1.8, theme_bg.1 * 1.8, theme_bg.2 * 1.8, 0.5)
        },
        radius,
        stroke_width: 0.0,
    });

    // Centered icon.
    let scale = (w.min(h) / cell_w * 0.55).clamp(1.0, 2.5);
    overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
        text: icon.to_string(),
        left: x + w / 2.0 - cell_w * scale / 2.0,
        top: y + h / 2.0 - cell_h * scale / 2.0,
        color: if hovered {
            (240, 240, 250)
        } else {
            (200, 205, 220)
        },
        scale,
    });
}

/// Word-wrap a string to fit within `max_chars` characters per line.
/// Breaks at character boundaries (monospace terminal text).
#[cfg(feature = "p2p")]
fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    if max_chars == 0 {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    for line in text.lines() {
        let chars: Vec<char> = line.chars().collect();
        let mut start = 0;
        while start < chars.len() {
            let end = (start + max_chars).min(chars.len());
            lines.push(chars[start..end].iter().collect());
            start = end;
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}
