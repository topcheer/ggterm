//! Frame rendering — render_frame() with multi-pane and overlay support.

use super::*;

impl DesktopApp {
    /// Render one frame.
    pub(super) fn render_frame(&mut self) {
        // P12-A/P12-C: Get theme background color for clear color,
        // and blend with visual bell flash if active.
        let active = self.active;
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
            ]
        } else {
            [br as f64 / 255.0, bg as f64 / 255.0, bb as f64 / 255.0]
        };

        // Decrement visual bell counter.
        if self.visual_bell_frames > 0 {
            self.visual_bell_frames -= 1;
        }

        // Now borrow session for grid + cursor data.
        let session = &self.sessions[active];
        let grid = session.app().grid();
        let mut cursor = cursor_state(session.app());

        // P23-A: Apply cursor blink alpha.
        let is_blink = matches!(
            session.app().terminal().cursor_style(),
            ggterm_core::CursorStyle::BlinkBlock
                | ggterm_core::CursorStyle::BlinkUnderline
                | ggterm_core::CursorStyle::BlinkBar
        );
        self.cursor_blink.set_enabled(is_blink);
        if cursor.visible {
            cursor.blink_alpha = self.cursor_blink.alpha();
            cursor.visible = self.cursor_blink.is_visible();
        }

        // P16-A: Wire search match highlights to renderer.
        // Convert SearchMatch(abs_row, col, len) → (visible_row, col_start, col_end).
        let scrollback_len = grid.scrollback_len();
        let grid_height = grid.height();
        let search_highlights: Vec<(usize, usize, usize)> = if self.search.visible {
            self.search
                .matches()
                .iter()
                .filter_map(|m| {
                    let visible_row = m.abs_row.checked_sub(scrollback_len)?;
                    // Only highlight rows within the visible grid.
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

        // P19-G: Build overlay data (tab bar + settings + about).
        let cell_h = renderer.cell_height() as f32;
        let screen_w = renderer.resolution_width() as f32;
        let screen_h = renderer.resolution_height() as f32;
        #[allow(unused_variables)]
        let overlay_rects: Vec<ggterm_render_wgpu::OverlayRect> = Vec::new();
        let mut overlay_texts: Vec<ggterm_render_wgpu::OverlayTextSpec> = Vec::new();
        let mut ui_rects: Vec<ggterm_render_wgpu::UiRect> = Vec::new();

        // Update tab bar data.
        let titles: Vec<&str> = self.sessions.iter().map(|s| s.title()).collect();
        self.tab_bar.update(&titles, self.active);

        // ── P26-C: Modern pill-shaped tab bar ──────────────────────────
        if self.tab_bar.visible {
            let tab_h = (cell_h + 8.0).max(28.0); // comfortable tab height
            let bar_h = tab_h + 6.0; // top padding + tab + bottom padding
            let pad_x = 8.0_f32;
            let tab_gap = 4.0_f32;
            let tab_radius = 6.0_f32;

            // Tab bar background — semi-transparent dark surface.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: 0.0,
                y: 0.0,
                w: screen_w,
                h: bar_h,
                color: (0.07, 0.07, 0.10, 0.92), // rich dark, slightly transparent
                radius: 0.0,
                stroke_width: 0.0,
            });

            // Bottom border line (subtle separator from terminal content).
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: 0.0,
                y: bar_h - 1.0,
                w: screen_w,
                h: 1.0,
                color: (0.15, 0.17, 0.23, 0.6),
                radius: 0.0,
                stroke_width: 0.0,
            });

            // Calculate tab widths based on content.
            let cell_w = renderer.cell_width() as f32;
            let tab_paddings = 12.0_f32; // horizontal padding inside each tab pill

            // First pass: compute widths.
            let tab_widths: Vec<f32> = self
                .tab_bar
                .tabs
                .iter()
                .map(|t| {
                    let text_w = t.format().chars().count() as f32 * cell_w;
                    (text_w + tab_paddings * 2.0).min(200.0) // cap at 200px
                })
                .collect();

            // Second pass: render tabs.
            let mut x = pad_x;
            for (i, tab) in self.tab_bar.tabs.iter().enumerate() {
                let w = tab_widths[i];
                let tab_y = 4.0; // top padding
                let title = tab.format();

                if tab.active {
                    // Active tab: brighter surface with accent bottom border.
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x,
                        y: tab_y,
                        w,
                        h: tab_h,
                        color: (0.14, 0.15, 0.22, 0.95), // surface_active
                        radius: tab_radius,
                        stroke_width: 0.0,
                    });
                    // Accent bottom border (glow effect).
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x: x + tab_radius,
                        y: tab_y + tab_h - 2.0,
                        w: w - tab_radius * 2.0,
                        h: 2.0,
                        color: (0.48, 0.64, 0.97, 0.9), // accent blue glow
                        radius: 0.0,
                        stroke_width: 0.0,
                    });
                } else {
                    // Inactive tab: subtle hover surface.
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x,
                        y: tab_y,
                        w,
                        h: tab_h,
                        color: (0.10, 0.10, 0.14, 0.7),
                        radius: tab_radius,
                        stroke_width: 0.0,
                    });
                }

                // Tab title text.
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: title,
                    left: x + tab_paddings,
                    top: tab_y + 5.0,
                    color: if tab.active {
                        (210, 214, 232) // text_primary (bright)
                    } else {
                        (120, 128, 154) // text_secondary (muted)
                    },
                });

                x += w + tab_gap;
            }

            // "+" new tab button at the end.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x,
                y: 4.0,
                w: tab_h,
                h: tab_h,
                color: (0.10, 0.10, 0.14, 0.6),
                radius: tab_radius,
                stroke_width: 0.0,
            });
            overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                text: "+".to_string(),
                left: x + tab_h * 0.5 - cell_w * 0.5,
                top: 4.0 + 5.0,
                color: (120, 128, 154),
            });
        }

        // ── P26-D: Padded pane borders with rounded corners ───────────
        let active = self.active;
        let tree = &self.sessions[active].split_tree();
        if !tree.is_single() {
            let tab_bar_h = if self.tab_bar.visible {
                (cell_h + 8.0).max(28.0) + 6.0
            } else {
                0.0
            };
            let bounds = crate::splits::Rect::new(
                4, // content padding
                tab_bar_h as u32 + 4,
                screen_w as u32 - 8, // minus padding both sides
                screen_h as u32 - tab_bar_h as u32 - 8,
            );
            let areas = tree.areas(bounds);
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
                    (0.20, 0.22, 0.28, 0.5) // dim border
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

                // Pane background fill (slightly different shade for depth).
                let bg_alpha = if *pane_id == active_id { 0.0 } else { 0.15 };
                if bg_alpha > 0.0 {
                    ui_rects.push(ggterm_render_wgpu::UiRect {
                        x,
                        y,
                        w,
                        h,
                        color: (0.04, 0.04, 0.06, bg_alpha),
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
            let pw = screen_w * 0.5;
            let ph = screen_h * 0.55;
            let px = (screen_w - pw) * 0.5;
            let py = (screen_h - ph) * 0.5;
            // Panel fill.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: px,
                y: py,
                w: pw,
                h: ph,
                color: (0.12, 0.13, 0.19, 0.98),
                radius: 12.0,
                stroke_width: 0.0,
            });
            // Panel border stroke.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: px,
                y: py,
                w: pw,
                h: ph,
                color: (0.22, 0.24, 0.32, 0.8),
                radius: 12.0,
                stroke_width: 1.0,
            });
            // Settings text lines.
            let theme_str = self.settings.theme.clone();
            let font_str = self.settings.font_size.to_string();
            let scrollback_str = self.settings.scrollback_lines.to_string();
            let shell_str = self.settings.shell.clone();
            let ai_str = (if self.settings.ai_enabled {
                "on"
            } else {
                "off"
            })
            .to_string();
            let endpoint_str = self.settings.ai_endpoint.clone();
            let model_str = self.settings.ai_model.clone();
            let fields: [(&str, &str); 7] = [
                ("Theme", &theme_str),
                ("Font Size", &font_str),
                ("Scrollback", &scrollback_str),
                ("Shell", &shell_str),
                ("AI", &ai_str),
                ("AI Endpoint", &endpoint_str),
                ("AI Model", &model_str),
            ];
            for (i, (label, value)) in fields.iter().enumerate() {
                let line = format!(
                    "  {}  {}: {}",
                    if i as u8 == self.settings.selected as u8 {
                        ">"
                    } else {
                        " "
                    },
                    label,
                    value
                );
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: line,
                    left: px + 20.0,
                    top: py + 40.0 + i as f32 * cell_h,
                    color: (200, 200, 210),
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
                color: (0.12, 0.13, 0.19, 0.98),
                radius: 12.0,
                stroke_width: 0.0,
            });
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: px,
                y: py,
                w: pw,
                h: ph,
                color: (0.22, 0.24, 0.32, 0.8),
                radius: 12.0,
                stroke_width: 1.0,
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

            // Rounded background fill (dark, semi-transparent).
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: pad_x,
                y: bar_y,
                w: screen_w - pad_x * 2.0,
                h: bar_h,
                color: (0.1, 0.1, 0.13, 0.85),
                radius: 6.0,
                stroke_width: 0.0,
            });

            // Subtle top border stroke.
            ui_rects.push(ggterm_render_wgpu::UiRect {
                x: pad_x,
                y: bar_y,
                w: screen_w - pad_x * 2.0,
                h: bar_h,
                color: (0.25, 0.27, 0.32, 0.6),
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

        renderer.set_ui_rects(ui_rects);
        renderer.set_overlay_rects(overlay_rects);
        renderer.set_overlay_text(overlay_texts);

        // P20-A: Multi-pane viewport rendering.
        // When the active session has multiple panes, render each pane's grid
        // at its SplitTree area offset within a single render pass.
        let pane_count = self.sessions[active].pane_count();
        if pane_count > 1 {
            let session = &self.sessions[active];
            let tree = session.split_tree();
            let bounds = crate::splits::Rect::new(0, 0, screen_w as u32, screen_h as u32);
            let areas = tree.areas(bounds);

            // Build cursor states per pane (owned values, no borrow issues).
            let cursors: Vec<_> = areas
                .iter()
                .filter_map(|(id, _)| session.pane_app(*id).map(cursor_state))
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
                    });
                }
            }

            if let Err(e) = gpu.render_multi_pane_frame(surface, renderer, &specs, bg_color) {
                log::error!("Render error: {e}");
            }

            // P21-D: Clear prepare flags after render (mutable borrow, disjoint from gpu).
            self.sessions[active].clear_prepare_flags();
        } else if let Err(e) = gpu.render_frame(surface, renderer, grid, &cursor, bg_color) {
            log::error!("Render error: {e}");
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
