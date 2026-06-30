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
        let mut overlay_rects: Vec<ggterm_render_wgpu::OverlayRect> = Vec::new();
        let mut overlay_texts: Vec<ggterm_render_wgpu::OverlayTextSpec> = Vec::new();

        // Update tab bar data.
        let titles: Vec<&str> = self.sessions.iter().map(|s| s.title()).collect();
        self.tab_bar.update(&titles, self.active);

        // Tab bar overlay: backgrounds + text.
        if self.tab_bar.visible {
            let bar_h = cell_h;
            // Dark background strip
            overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                x: 0.0,
                y: 0.0,
                w: screen_w,
                h: bar_h,
                color: (0.12, 0.12, 0.15),
            });
            let tab_max_w = screen_w / self.tab_bar.tabs.len() as f32;
            for (i, tab) in self.tab_bar.tabs.iter().enumerate() {
                let x = i as f32 * tab_max_w;
                if tab.active {
                    overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                        x,
                        y: 0.0,
                        w: tab_max_w,
                        h: bar_h,
                        color: (0.2, 0.2, 0.3),
                    });
                }
                // Separator line
                if i > 0 {
                    overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                        x,
                        y: 0.0,
                        w: 1.0,
                        h: bar_h,
                        color: (0.3, 0.3, 0.35),
                    });
                }
                // Tab title text
                let title = tab.format();
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: title,
                    left: x + 4.0,
                    top: 0.0,
                    color: if tab.active {
                        (220, 220, 220)
                    } else {
                        (160, 160, 160)
                    },
                });
            }
        }

        // P20-B: Pane border overlays — draw 1px separators between split panes.
        let active = self.active;
        let tree = &self.sessions[active].split_tree();
        if !tree.is_single() {
            let bounds = crate::splits::Rect::new(
                0,
                if self.tab_bar.visible {
                    cell_h as u32
                } else {
                    0
                },
                screen_w as u32,
                screen_h as u32,
            );
            let areas = tree.areas(bounds);
            let active_id = tree.active();
            let border_active = (0.4, 0.55, 0.85_f32);
            let border_inactive = (0.15, 0.15, 0.2_f32);

            for (pane_id, rect) in &areas {
                let x = rect.x as f32;
                let y = rect.y as f32;
                let w = rect.width as f32;
                let h = rect.height as f32;
                let c = if *pane_id == active_id {
                    border_active
                } else {
                    border_inactive
                };
                overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                    x,
                    y,
                    w,
                    h: 1.0,
                    color: c,
                });
                overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                    x,
                    y: y + h - 1.0,
                    w,
                    h: 1.0,
                    color: c,
                });
                overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                    x,
                    y,
                    w: 1.0,
                    h,
                    color: c,
                });
                overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                    x: x + w - 1.0,
                    y,
                    w: 1.0,
                    h,
                    color: c,
                });
            }
        }

        // Settings overlay: semi-transparent mask + panel.
        if self.settings.visible {
            // Dark mask
            overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                x: 0.0,
                y: 0.0,
                w: screen_w,
                h: screen_h,
                color: (0.05, 0.05, 0.05),
            });
            // Center panel
            let pw = screen_w * 0.6;
            let ph = screen_h * 0.5;
            let px = (screen_w - pw) * 0.5;
            let py = (screen_h - ph) * 0.5;
            overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                x: px,
                y: py,
                w: pw,
                h: ph,
                color: (0.1, 0.1, 0.12),
            });
            // Panel border (top + bottom)
            overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                x: px,
                y: py,
                w: pw,
                h: 2.0,
                color: (0.35, 0.35, 0.4),
            });
            overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                x: px,
                y: py + ph - 2.0,
                w: pw,
                h: 2.0,
                color: (0.35, 0.35, 0.4),
            });
            // Settings text lines
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
                    left: px + 10.0,
                    top: py + 10.0 + i as f32 * cell_h,
                    color: (200, 200, 200),
                });
            }
        }

        // About dialog overlay
        if self.about.visible {
            overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                x: 0.0,
                y: 0.0,
                w: screen_w,
                h: screen_h,
                color: (0.05, 0.05, 0.05),
            });
            let pw = screen_w * 0.4;
            let ph = screen_h * 0.3;
            let px = (screen_w - pw) * 0.5;
            let py = (screen_h - ph) * 0.5;
            overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                x: px,
                y: py,
                w: pw,
                h: ph,
                color: (0.1, 0.1, 0.12),
            });
            let about_text = self.about.format_text();
            for (i, line) in about_text.lines().enumerate() {
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: line.to_string(),
                    left: px + 10.0,
                    top: py + 10.0 + i as f32 * cell_h,
                    color: (200, 200, 200),
                });
            }
        }

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

        // P17-D: Update status bar and log it.
        if self.status_bar_visible {
            let status_text = self.status_bar.format();
            log::debug!("status: {}", status_text);
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
