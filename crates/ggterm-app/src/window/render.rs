//! Frame rendering — render_frame() with multi-pane and overlay support.

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
        // P32: Pre-compute scrolled state for indicator.
        let is_scrolled = self.sessions[active].app().terminal().grid().is_scrolled();
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
        let is_blink_style = matches!(
            session.app().terminal().cursor_style(),
            ggterm_core::CursorStyle::BlinkBlock
                | ggterm_core::CursorStyle::BlinkUnderline
                | ggterm_core::CursorStyle::BlinkBar
        );
        // Respect DECSET 12: even blink-style cursors stay steady when
        // the program has disabled cursor blinking (DECSET 12 = off).
        let is_blink = is_blink_style && session.app().terminal().cursor_blink_enabled();
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
            self.search
                .matches()
                .iter()
                .filter_map(|m| {
                    // visible_row = abs_row - (scrollback_len - display_offset)
                    // This maps absolute row to the row index currently shown on screen.
                    let base = scrollback_len.saturating_sub(display_offset);
                    let visible_row = m.abs_row.checked_sub(base)?;
                    if visible_row < grid_height {
                        Some((visible_row, m.col, m.col + m.len.saturating_sub(1)))
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            Vec::new()
        };

        let (gpu, surface, renderer) = match (&mut self.gpu, &self.surface, &mut self.renderer) {
            (Some(g), Some(s), Some(r)) => (g, s, r),
            _ => return,
        };

        // Apply search highlights before rendering.
        renderer.set_highlights(search_highlights);

        // Apply dynamic colors (OSC 10/11) if set on the terminal.
        let term = self.sessions[self.active].app().terminal();
        renderer.set_dynamic_fg(term.dynamic_fg().map(|c| match c {
            ggterm_core::Color::Rgb(r, g, b) => (*r, *g, *b),
            _ => unreachable!("dynamic_fg stores Rgb"),
        }));
        renderer.set_dynamic_bg(term.dynamic_bg().map(|c| match c {
            ggterm_core::Color::Rgb(r, g, b) => (*r, *g, *b),
            _ => unreachable!("dynamic_bg stores Rgb"),
        }));

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
        let cell_h = renderer.cell_height() as f32;
        let cell_w = renderer.cell_width() as f32;
        let screen_w = renderer.resolution_width() as f32;
        let screen_h = renderer.resolution_height() as f32;
        let overlay_rects: Vec<ggterm_render_wgpu::OverlayRect> = Vec::new();
        let mut overlay_texts: Vec<ggterm_render_wgpu::OverlayTextSpec> = Vec::new();
        let mut ui_rects: Vec<ggterm_render_wgpu::UiRect> = Vec::new();

        // Theme background as normalized f32 — used for tab bar/status bar
        // so they match the terminal content instead of hardcoded colors.
        let theme_bg = (br as f32 / 255.0, bg as f32 / 255.0, bb as f32 / 255.0);

        // Update tab bar data.
        let titles: Vec<&str> = self.sessions.iter().map(|s| s.title()).collect();
        self.tab_bar.update(&titles, self.active);

        // ── Tab bar: auto-fill width like browser tabs ─────────────────
        if self.tab_bar.visible {
            let tab_h = (cell_h + 8.0).max(28.0);
            let bar_h = tab_h + 6.0;
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
                // Reserve space for close button only when 2+ tabs exist.
                let reserved = if self.tab_bar.tabs.len() > 1 {
                    24.0 // close "x" + margin
                } else {
                    8.0 // just right padding
                };
                let max_chars = ((w - 16.0 - reserved) / cell_w).floor() as usize;
                let display_title: String = if title.chars().count() > max_chars {
                    format!(
                        "{}…",
                        title
                            .chars()
                            .take(max_chars.saturating_sub(1))
                            .collect::<String>()
                    )
                } else {
                    title
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
                });

                // Close button "x" — show on active tab, hovered tab, or when 2+ tabs.
                if self.tab_bar.tabs.len() > 1 {
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
                            color: (220.0, 80.0, 80.0, 0.2),
                            radius: 9.0,
                            stroke_width: 0.0,
                        });
                    }
                    let close_x = x + w - 16.0 - cell_w * 0.5;
                    overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                        text: "\u{00d7}".to_string(),
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
                    });
                }
            }

            // "+" multi-function button at the end.
            let btn_x = layout.new_tab_button.cx - layout.new_tab_button.size / 2.0;
            let btn_hovered = self.tab_bar.is_new_tab_button_at(
                &layout,
                self.cursor_pos.0 as f32,
                self.cursor_pos.1 as f32,
            );
            let btn_bg = if btn_hovered {
                (theme_bg.0 * 2.0, theme_bg.1 * 2.0, theme_bg.2 * 2.0, 0.7)
            } else {
                (theme_bg.0 * 1.3, theme_bg.1 * 1.3, theme_bg.2 * 1.3, 0.6)
            };
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: btn_x,
                y: 4.0,
                w: layout.new_tab_button.size,
                h: tab_h,
                color: btn_bg,
                radius: tab_radius,
                stroke_width: 0.0,
            });
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "+".to_string(),
                left: layout.new_tab_button.cx - cell_w * 0.5,
                top: 4.0 + 5.0,
                color: if btn_hovered {
                    (255, 255, 255)
                } else {
                    (180, 185, 200)
                },
            });

            // Settings gear button at the far right.
            let gear_hovered = self.tab_bar.is_settings_button_at(
                &layout,
                self.cursor_pos.0 as f32,
                self.cursor_pos.1 as f32,
            );
            let gear_bg = if gear_hovered {
                (theme_bg.0 * 2.0, theme_bg.1 * 2.0, theme_bg.2 * 2.0, 0.7)
            } else {
                (theme_bg.0 * 1.3, theme_bg.1 * 1.3, theme_bg.2 * 1.3, 0.6)
            };
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: layout.settings_button.cx - layout.settings_button.size / 2.0,
                y: 4.0,
                w: layout.settings_button.size,
                h: tab_h,
                color: gear_bg,
                radius: tab_radius,
                stroke_width: 0.0,
            });
            // Use a simple gear-like symbol: we use the Unicode gear character.
            // If it doesn't render, fallback to a colon-like icon.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "\u{2699}".to_string(), // ⚙ gear symbol
                left: layout.settings_button.cx - cell_w * 0.5,
                top: 4.0 + 5.0,
                color: if gear_hovered {
                    (255, 255, 255)
                } else {
                    (180, 185, 200)
                },
            });

            // ── Linux/Windows: window control buttons (minimize/maximize/close) ──
            #[cfg(not(target_os = "macos"))]
            {
                let layout = crate::titlebar::x11::compute_layout(screen_w, bar_h);
                let px = self.cursor_pos.0 as f32;
                let py = self.cursor_pos.1 as f32;

                for (glyph, &(bx, by, bsize), hover_color, normal_color) in [
                    (
                        "\u{2014}",
                        &layout.minimize,
                        (255, 200, 80u8),
                        (180, 180, 180u8),
                    ), // ─ minimize
                    (
                        "\u{25A1}",
                        &layout.maximize,
                        (100, 200, 100u8),
                        (180, 180, 180u8),
                    ), // □ maximize
                    (
                        "\u{2715}",
                        &layout.close,
                        (255, 100, 100u8),
                        (180, 180, 180u8),
                    ), // ✕ close
                ] {
                    let hovered = px >= bx && px <= bx + bsize && py >= by && py <= by + bsize;

                    // Button background on hover.
                    if hovered {
                        let bg = match glyph {
                            "\u{2715}" => (0.8, 0.15, 0.15, 0.8), // close → red
                            "\u{25A1}" => (0.15, 0.5, 0.15, 0.5), // maximize → green
                            _ => (0.3, 0.3, 0.3, 0.5),            // minimize → gray
                        };
                        ui_rects.push(ggterm_render_wgpu::UiRect {
                            x: bx - 4.0,
                            y: by - 4.0,
                            w: bsize + 8.0,
                            h: bsize + 8.0,
                            color: bg,
                            radius: 4.0,
                            stroke_width: 0.0,
                        });
                    }

                    overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                        text: glyph.to_string(),
                        left: bx,
                        top: by,
                        color: if hovered { hover_color } else { normal_color },
                    });
                }
            }
            // Close of `if self.tab_bar.visible` block.
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
            });

            // Error message if present.
            if let Some(err) = self.settings.error_text() {
                y_offset += cell_h;
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: format!("Error: {}", err),
                    left: px + 20.0,
                    top: y_offset,
                    color: (255, 100, 100),
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
                color: (103, 232, 249), // cyan
            });
            let about_text = self.about.format_text();
            for (i, line) in about_text.lines().enumerate() {
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: line.to_string(),
                    left: px + 20.0,
                    top: py + 16.0 + i as f32 * cell_h,
                    color: if i == 0 {
                        (122, 162, 247) // accent for title
                    } else {
                        (200, 200, 210)
                    },
                });
            }
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
            let segments = self.status_bar.format_segments();
            let cell_w = renderer.cell_width() as f32;
            let text_top = bar_y + 4.0;
            let mut x = pad_x + 8.0;

            for (text, color) in &segments {
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: text.clone(),
                    left: x,
                    top: text_top,
                    color: *color,
                });
                x += text.chars().count() as f32 * cell_w;
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

                // Separators before Split group and before Clear/Reset group.
                if action == &crate::context_menu::ContextMenuAction::SplitHorizontal
                    || action == &crate::context_menu::ContextMenuAction::Clear
                {
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

                // Hover highlight — inverted: bright bg + dark text.
                if is_hovered {
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

                // Item text — dark on hover, light otherwise.
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: action.label().to_string(),
                    left: mx + 16.0,
                    top: iy + 7.0,
                    color: if is_hovered {
                        (20, 20, 30)
                    } else {
                        (210, 215, 230)
                    },
                });
            }
            self.context_menu.effective_width = menu_w;
        }

        // ── P27-G: Scroll-to-bottom indicator ──────────────────────────
        // Show a "↓" indicator when scrolled up in scrollback.
        {
            let is_scrolled = self.sessions[active].app().terminal().grid().is_scrolled();
            if is_scrolled {
                let indicator_y =
                    content_bounds.y as f32 + content_bounds.height as f32 - cell_h - 4.0;
                let indicator_x =
                    content_bounds.x as f32 + content_bounds.width as f32 - cell_w * 3.0;
                // Pill background.
                ui_rects.push(ggterm_render_wgpu::UiRect {
                    x: indicator_x,
                    y: indicator_y,
                    w: cell_w * 2.5,
                    h: cell_h + 4.0,
                    color: (0.2, 0.4, 0.8, 0.7),
                    radius: 4.0,
                    stroke_width: 0.0,
                });
                // Arrow text.
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: "\u{2193}".to_string(), // ↓
                    left: indicator_x + cell_w * 0.5,
                    top: indicator_y + 2.0,
                    color: (255, 255, 255),
                });
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
            });

            let total = self.cmd_history.len();
            let failed = self.cmd_history.failed_count();
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: format!("{} cmds | {} failed", total, failed),
                left: sb_x + 12.0,
                top: sb_y + cell_h + 12.0,
                color: (120, 120, 140),
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
                });

                // Command text (truncated).
                let max_cmd_chars = ((sb_w - 70.0) / (renderer.cell_width() as f32)) as usize;
                let cmd_display = if entry.command.len() > max_cmd_chars {
                    format!("{}...", &entry.command[..max_cmd_chars])
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
            });

            // File name.
            let name_display = if preview.name.len() > 30 {
                format!("{}...", &preview.name[..27])
            } else {
                preview.name.clone()
            };
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: name_display,
                left: card_x + 68.0,
                top: card_y + 12.0,
                color: (240, 240, 250),
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
            });

            // Drop hint.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "Drop to insert path".to_string(),
                left: card_x + 12.0,
                top: card_y + card_h - cell_h - 8.0,
                color: (100, 180, 255),
            });
        }

        // ── P28-B: Color picker hover swatch ──────────────────────────
        if let Some(ref hovered) = self.color_picker.hovered {
            let (cx, cy) = self.cursor_pos;
            let swatch_x = cx as f32 + 16.0;
            let swatch_y = cy as f32 + 16.0;
            let swatch_size = 24.0_f32;

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
                });

                // Description.
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: entry.description.clone(),
                    left: px + 200.0,
                    top: ey + 3.0,
                    color: (200, 200, 210),
                });

                // Category label (right side).
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: entry.category.label().to_string(),
                    left: px + panel_w - 80.0,
                    top: ey + 3.0,
                    color: (r, g, b),
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
            });

            // Message.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "All running processes in your sessions will be".to_string(),
                left: dx + 24.0,
                top: dy + 52.0,
                color: (200, 200, 210),
            });
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "terminated. Session state will be saved.".to_string(),
                left: dx + 24.0,
                top: dy + 70.0,
                color: (200, 200, 210),
            });

            // Options.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "[Y] Yes, close   [N] Cancel".to_string(),
                left: dx + 24.0,
                top: dy + 110.0,
                color: (140, 180, 255),
            });

            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "Press Esc to cancel".to_string(),
                left: dx + 24.0,
                top: dy + 132.0,
                color: (120, 120, 140),
            });
        }

        // ── P30-C: Toast notification ─────────────────────────────────
        if let Some((msg, frames)) = &self.toast {
            let alpha = (*frames as f32 / 120.0).min(1.0);
            let toast_w = (msg.len() as f32 * cell_w + 24.0).max(80.0);
            let toast_h = 32.0;
            let tx = (screen_w - toast_w) / 2.0;
            let ty = screen_h - 50.0; // bottom of screen

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
                });
            }
        }

        // ── P32: Scroll-to-bottom indicator ──────────────────────────
        {
            if is_scrolled {
                let indicator_w = 60.0;
                let indicator_h = 24.0;
                let ix = screen_w - indicator_w - 20.0;
                let iy = screen_h - indicator_h - 40.0;

                ui_rects.push(ggterm_render_wgpu::UiRect {
                    x: ix,
                    y: iy,
                    w: indicator_w,
                    h: indicator_h,
                    color: (0.15, 0.25, 0.50, 0.9),
                    radius: 12.0,
                    stroke_width: 0.0,
                });
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: "↓ Bottom".to_string(),
                    left: ix + 12.0,
                    top: iy + 4.0,
                    color: (200, 220, 255),
                });
            }
        }

        // ── P30-A: Scrollbar ──────────────────────────────────────────
        // Show a thin scrollbar on the right edge when there's scrollback.
        {
            let scrollback_len = grid.scrollback_len();
            if scrollback_len > 0 {
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
                ui_rects.push(ggterm_render_wgpu::UiRect {
                    x: bar_x,
                    y: bar_track_y,
                    w: bar_w,
                    h: bar_track_h,
                    color: (1.0, 1.0, 1.0, 0.04),
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
                    color: (1.0, 1.0, 1.0, thumb_alpha),
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
            let bar_y = 8.0;

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
            // Accent border.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: bar_x,
                y: bar_y,
                w: bar_w,
                h: bar_h,
                color: (0.35, 0.42, 0.60, 0.8),
                radius: 10.0,
                stroke_width: 1.5,
            });

            // Search icon "🔍" → use ">" as simplified icon.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: ">".to_string(),
                left: bar_x + 12.0,
                top: bar_y + 10.0,
                color: (100, 140, 200),
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
            });

            // Search query text with cursor.
            let query_display = format!("{}_", self.search.query);
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: query_display,
                left: bar_x + 64.0,
                top: bar_y + 10.0,
                color: (230, 230, 240),
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
                let count_w = count_text.len() as f32 * cell_w;
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: count_text,
                    left: bar_x + bar_w - count_w - 14.0,
                    top: bar_y + 10.0,
                    color: count_color,
                });
            }

            // Hint text at bottom.
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "Tab: case  Enter: next  Esc: close".to_string(),
                left: bar_x + 12.0,
                top: bar_y + bar_h + 4.0,
                color: (110, 110, 130),
            });
        }

        // ── P33: URL hover tooltip + underline ────────────────────────
        if let Some((ref url, start_col, end_col, link_row)) = self.hovered_link {
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
            let tooltip_w = (url.len() as f32 * cell_w + 24.0).min(400.0);
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
            });
        }

        renderer.set_ui_rects(ui_rects);
        renderer.set_overlay_rects(overlay_rects);
        renderer.set_overlay_text(overlay_texts);

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
            let areas: Vec<(usize, crate::splits::Rect)> = if pane_zoomed {
                let active_id = tree.active();
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
                        needs_prepare: session.pane_needs_prepare(*pane_id),
                        reverse_video: app.terminal().reverse_video(),
                        dynamic_fg: app.terminal().dynamic_fg().map(|c| match c {
                            ggterm_core::Color::Rgb(r, g, b) => (*r, *g, *b),
                            _ => unreachable!("dynamic_fg stores Rgb"),
                        }),
                        dynamic_bg: app.terminal().dynamic_bg().map(|c| match c {
                            ggterm_core::Color::Rgb(r, g, b) => (*r, *g, *b),
                            _ => unreachable!("dynamic_bg stores Rgb"),
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

            // P21-D: Clear prepare flags after render.
            self.sessions[active].clear_prepare_flags();
        }

        // P19-C: Update tab bar display data.
        let titles: Vec<&str> = self.sessions.iter().map(|s| s.title()).collect();
        self.tab_bar.update(&titles, self.active);
        if self.tab_bar.visible {
            log::debug!("tab_bar: {}", self.tab_bar.format());
        }

        // P19-C: Settings overlay logging.
        if self.settings.visible {
            log::debug!("settings: {}", self.settings.format_summary());
        }
    }
}
