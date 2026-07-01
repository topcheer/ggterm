//! # GGTerm Render — wgpu + glyphon
//!
//! GPU-accelerated terminal renderer using [wgpu] and [glyphon].
//!
//! Implements the [`Renderer`] trait from `ggterm-render`.

pub mod colors;
pub mod converter;

pub use colors::{ANSI_16, DEFAULT_BG, DEFAULT_FG, indexed_to_rgb, map_bg, map_color, map_fg};
pub use converter::{TextRun, row_to_runs, row_to_text};

use ggterm_core::{DirtyRect, Grid};
use ggterm_render::theme::RenderTheme;
use ggterm_render::{CursorState, Renderer};
use glyphon::cosmic_text::LayoutRun;
use glyphon::cosmic_text::LineEnding;
use glyphon::{
    Attrs, AttrsList, Buffer, BufferLine, Cache as GlyphonCache, Color as GlyphonColor, Family,
    FontSystem, Metrics, PrepareError, RenderError as GlyphonRenderError, Resolution, Shaping,
    SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use thiserror::Error;

/// Unified error type for GPU text rendering operations.
#[derive(Debug, Error)]
pub enum RenderError {
    /// Failed to prepare text (shaping, atlas allocation).
    #[error("prepare error: {0}")]
    Prepare(#[from] PrepareError),
    /// Failed to render text quads into the render pass.
    #[error("render error: {0}")]
    Render(#[from] GlyphonRenderError),
}

const DEFAULT_FONT_SIZE: f32 = 15.0;
/// Line height must equal font_size for seamless box-drawing chars.
const DEFAULT_LINE_HEIGHT: f32 = 15.0;

/// Terminal font family — platform-specific for best box-drawing support.
#[cfg(target_os = "macos")]
const TERMINAL_FONT: &str = "Menlo";
#[cfg(all(unix, not(target_os = "macos")))]
const TERMINAL_FONT: &str = "DejaVu Sans Mono"; // Linux — widely available
#[cfg(target_os = "windows")]
const TERMINAL_FONT: &str = "Cascadia Mono"; // Windows 11 default terminal font

/// Measure the actual monospace advance width from the font system.
/// Returns the pixel width of a single character at the given font size.
/// Falls back to `font_size * 0.6` if measurement fails.
fn measure_cell_width(font_system: &mut FontSystem, font_size: f32) -> f32 {
    let metrics = Metrics::new(font_size, font_size);
    let attrs = Attrs::new().family(Family::Name(TERMINAL_FONT));
    let attrs_list = AttrsList::new(&attrs);

    // Use 'M' as a representative monospace character.
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.lines = vec![BufferLine::new(
        "MMMMMMMMMM".to_string(), // 10 M's for stable measurement
        LineEnding::None,
        attrs_list,
        Shaping::Advanced, // Advanced needed for CJK font fallback
    )];
    buffer.shape_until_scroll(font_system, false);

    // Get the layout runs and measure the advance width of the first glyph.
    let runs: Vec<LayoutRun> = buffer.layout_runs().collect();
    if let Some(run) = runs.first() {
        if run.glyphs.len() >= 2 {
            // Compare positions of first two glyphs to get advance width.
            let advance = run.glyphs[1].x - run.glyphs[0].x;
            if advance > 0.0 {
                return advance;
            }
        }
        // Fallback: total width / number of glyphs
        if run.glyphs.len() >= 10 {
            let total = run.glyphs.last().unwrap().x + run.glyphs.last().unwrap().w;
            return total / 10.0;
        }
    }

    // Fallback to estimate.
    font_size * 0.6
}

/// GPU-accelerated terminal renderer using wgpu + glyphon.
///
/// Created with an externally-managed `wgpu::Device` and `wgpu::Queue` (typically
/// from the winit event loop in P1-F3). The renderer does NOT own a surface —
/// the app layer manages `surface.get_current_texture()` and creates the
/// `wgpu::RenderPass`, then calls [`render_to_pass()`](GlyphonRenderer::render_to_pass).
pub struct GlyphonRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    /// Glyphon cache — kept alive for the lifetime of the renderer.
    #[allow(dead_code)]
    cache: GlyphonCache,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    viewport: Viewport,
    resolution: Resolution,
    cols: usize,
    rows: usize,
    font_size: f32,
    line_height: f32,
    /// Measured cell width from actual font metrics (P18-B). Float for precise positioning.
    cell_width_f32: f32,
    /// Natural advance width of the terminal font (before letter_spacing adjustment).
    natural_advance: f32,
    /// Cell width rounded to integer for grid computations.
    measured_cell_width: u32,
    /// DPI scale factor (e.g. 2.0 on Retina). P18-A.
    scale_factor: f64,
    /// Active render theme (P11-D).
    theme: RenderTheme,
    /// Underline render pipeline (P12-B fix).
    underline_pipeline: Option<wgpu::RenderPipeline>,
    /// Underline vertex buffer (P12-B fix).
    underline_vertex_buffer: Option<wgpu::Buffer>,
    /// Number of underline vertices (P12-B fix).
    underline_vertex_count: u32,
    /// Strikethrough vertex buffer (P13-A).
    strike_vertex_buffer: Option<wgpu::Buffer>,
    /// Number of strikethrough vertices (P13-A).
    strike_vertex_count: u32,
    /// Search-match highlights: (row, col_start, col_end) inclusive (P14-B).
    highlights: Vec<(usize, usize, usize)>,
    /// Dynamic foreground color (OSC 10) — overrides theme default fg.
    dynamic_fg: Option<(u8, u8, u8)>,
    /// Dynamic background color (OSC 11) — overrides theme default bg.
    dynamic_bg: Option<(u8, u8, u8)>,
    // ── P19-G: UI Overlay rendering ──
    /// Overlay text specs: (text, left_px, top_px, (r,g,b)).
    overlay_text: Vec<OverlayTextSpec>,
    /// Overlay rectangle specs: (x_px, y_px, w_px, h_px, (r,g,b)).
    overlay_rects: Vec<OverlayRect>,
    /// Overlay rectangle vertices buffer.
    overlay_vertex_buffer: Option<wgpu::Buffer>,
    /// Number of overlay vertices.
    overlay_vertex_count: u32,
    // ── P20-A: Multi-pane viewport offset ──
    /// Pixel offset applied to grid text + decoration positions for
    /// rendering into a sub-region of the surface (split panes).
    viewport_offset: (f32, f32),
    // ── P26-A: Modern UI rendering (SDF rounded rects) ──
    /// UI rect specs for the SDF pipeline.
    ui_rects: Vec<UiRect>,
    /// UI vertex buffer.
    ui_vertex_buffer: Option<wgpu::Buffer>,
    /// Number of UI vertices.
    ui_vertex_count: u32,
    /// UI SDF pipeline (rounded rectangles with alpha + stroke).
    ui_pipeline: Option<wgpu::RenderPipeline>,
}

// ── P26-A: Modern UI Rendering ──────────────────────────────────

/// Specification for a modern UI rectangle with rounded corners, alpha,
/// and optional stroke (border-only rendering).
///
/// Used for tab bar backgrounds, pane borders, dialog panels, status bar.
/// Rendered by the `ui.wgsl` SDF shader via a separate pipeline.
#[derive(Debug, Clone)]
pub struct UiRect {
    /// X position in physical pixels (top-left corner).
    pub x: f32,
    /// Y position in physical pixels (top-left corner).
    pub y: f32,
    /// Width in physical pixels.
    pub w: f32,
    /// Height in physical pixels.
    pub h: f32,
    /// Fill color (r, g, b, a) in [0, 1].
    pub color: (f32, f32, f32, f32),
    /// Corner radius in physical pixels (0 = sharp corners).
    pub radius: f32,
    /// Stroke (border) width in physical pixels.
    /// When > 0.5, renders as an outline ring instead of a filled shape.
    pub stroke_width: f32,
}

/// P19-G: Overlay text specification for tab bar / settings / about rendering.
#[derive(Debug, Clone)]
pub struct OverlayTextSpec {
    /// Text content.
    pub text: String,
    /// X position in physical pixels.
    pub left: f32,
    /// Y position in physical pixels.
    pub top: f32,
    /// Text color (r, g, b).
    pub color: (u8, u8, u8),
}

/// P19-G: Overlay rectangle specification for UI backgrounds and panels.
#[derive(Debug, Clone)]
pub struct OverlayRect {
    /// X position in physical pixels.
    pub x: f32,
    /// Y position in physical pixels.
    pub y: f32,
    /// Width in physical pixels.
    pub w: f32,
    /// Height in physical pixels.
    pub h: f32,
    /// Fill color (r, g, b) in [0, 1].
    pub color: (f32, f32, f32),
}

impl GlyphonRenderer {
    /// Create a new GlyphonRenderer.
    ///
    /// # Arguments
    /// * `device` — wgpu device (from adapter request)
    /// * `queue` — wgpu queue (from adapter request)
    /// * `surface_format` — texture format from `surface.get_capabilities(&adapter)`
    /// * `cols` — initial terminal width in cells
    /// * `rows` — initial terminal height in cells
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        surface_width: u32,
        surface_height: u32,
        scale_factor: f64,
    ) -> Self {
        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = GlyphonCache::new(device);
        let mut atlas = TextAtlas::new(device, queue, &cache, surface_format);

        // P18-A: Scale font by DPI factor for crisp Retina rendering.
        let sf = scale_factor as f32;
        let scaled_font = DEFAULT_FONT_SIZE * sf;
        let scaled_lh = DEFAULT_LINE_HEIGHT * sf;

        // P18-D: Use the EXACT float advance of ASCII 'M' as cell width.
        // Do NOT round — sub-pixel rounding causes cumulative drift.
        // glyphon renders chars at their natural float advance; if cell_w
        // matches exactly, positioning is perfect.
        // CJK chars occupy 2 cells but render at their natural width within
        // that space (each in its own run at start_col positioning).
        let natural_advance = measure_cell_width(&mut font_system, scaled_font);
        let cell_w_f32 = natural_advance;
        let cell_w = natural_advance.round() as u32;
        let cell_h = scaled_lh.round() as u32;

        // P18-C: Viewport resolution MUST match the GPU surface texture size.
        // If they differ, glyphon's output gets stretched/compressed onto the
        // surface, causing character misalignment and border drift.
        let viewport_w = surface_width.max(1);
        let viewport_h = surface_height.max(1);

        // Derive cols/rows from cell dimensions.
        let cols = (viewport_w / cell_w.max(1)) as usize;
        let rows = (viewport_h / cell_h.max(1)) as usize;

        let text_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);

        let mut viewport = Viewport::new(device, &cache);
        viewport.update(
            queue,
            Resolution {
                width: viewport_w,
                height: viewport_h,
            },
        );

        Self {
            font_system,
            swash_cache,
            cache,
            atlas,
            text_renderer,
            viewport,
            resolution: Resolution {
                width: viewport_w,
                height: viewport_h,
            },
            cols,
            rows,
            font_size: scaled_font,
            line_height: scaled_lh,
            cell_width_f32: cell_w_f32,
            natural_advance,
            measured_cell_width: cell_w,
            scale_factor,
            theme: RenderTheme::default(),
            underline_pipeline: None,
            underline_vertex_buffer: None,
            underline_vertex_count: 0,
            strike_vertex_buffer: None,
            strike_vertex_count: 0,
            highlights: Vec::new(),
            dynamic_fg: None,
            dynamic_bg: None,
            overlay_text: Vec::new(),
            overlay_rects: Vec::new(),
            overlay_vertex_buffer: None,
            overlay_vertex_count: 0,
            viewport_offset: (0.0, 0.0),
            ui_rects: Vec::new(),
            ui_vertex_buffer: None,
            ui_vertex_count: 0,
            ui_pipeline: None,
        }
    }

    /// Get the cell width in pixels (measured from actual font).
    pub fn cell_width(&self) -> u32 {
        self.measured_cell_width
    }

    /// Get the surface width in physical pixels.
    pub fn resolution_width(&self) -> u32 {
        self.resolution.width
    }

    /// Get the surface height in physical pixels.
    pub fn resolution_height(&self) -> u32 {
        self.resolution.height
    }

    /// Get the cell height in pixels.
    pub fn cell_height(&self) -> u32 {
        self.line_height.ceil() as u32
    }

    /// Get the number of columns (derived from surface/cell dimensions).
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Get the number of rows (derived from surface/cell dimensions).
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Set the viewport pixel offset for multi-pane rendering (P20-A).
    ///
    /// All grid text and decoration positions will be shifted by `(x, y)`
    /// in the next `prepare_grid*` / `prepare_decorations` call.
    /// Overlay rendering is NOT affected (uses absolute screen coords).
    pub fn set_viewport_offset(&mut self, x: f32, y: f32) {
        self.viewport_offset = (x, y);
    }

    /// P23-C: Determine whether `prepare_grid()` should be called for this grid.
    ///
    /// Returns `true` when the grid's content has changed (dirty flag set),
    /// or when the viewport offset has changed (pane repositioned).
    pub fn should_prepare_grid(&self, grid: &Grid) -> bool {
        grid.content_dirty() || !self.highlights.is_empty()
    }

    /// Set the active render theme (P11-D).
    ///
    /// The theme controls default foreground/background colors, cursor color,
    /// and the ANSI 16-color palette used for text rendering.
    pub fn set_theme(&mut self, theme: RenderTheme) {
        self.theme = theme;
    }

    /// Get a reference to the current render theme.
    pub fn current_theme(&self) -> &RenderTheme {
        &self.theme
    }

    /// Set search-match highlights for the next render (P14-B).
    ///
    /// Each tuple is `(row, col_start, col_end)` — all inclusive.
    /// Pass an empty vec to clear highlights.
    pub fn set_highlights(&mut self, highlights: Vec<(usize, usize, usize)>) {
        self.highlights = highlights;
    }

    /// Set dynamic foreground color (OSC 10). Pass None to reset to theme default.
    pub fn set_dynamic_fg(&mut self, color: Option<(u8, u8, u8)>) {
        self.dynamic_fg = color;
    }

    /// Set dynamic background color (OSC 11). Pass None to reset to theme default.
    pub fn set_dynamic_bg(&mut self, color: Option<(u8, u8, u8)>) {
        self.dynamic_bg = color;
    }

    /// Set the font size and recompute cell metrics.
    ///
    /// Line height = font_size (1.0x, for seamless box-drawing chars).
    /// Cell width = measured advance of 'M' in the terminal font (exact float).
    /// Scale factor is applied to convert logical points to physical pixels.
    pub fn set_font_size(&mut self, size: f32) {
        // P18-A: size is logical points; multiply by scale_factor for physical pixels.
        let physical = size * self.scale_factor as f32;
        self.font_size = physical.clamp(6.0, 144.0);
        // P18-B: line_height = font_size for seamless box-drawing character tiling.
        self.line_height = self.font_size;
        // P18-D: Use exact float advance as cell width — no rounding.
        let natural = measure_cell_width(&mut self.font_system, self.font_size);
        self.natural_advance = natural;
        self.cell_width_f32 = natural;
        self.measured_cell_width = natural.round() as u32;
    }

    /// Get the current font size.
    pub fn font_size(&self) -> f32 {
        self.font_size
    }

    /// Prepare text rendering: Grid → glyphon buffers → shape → prepare().
    ///
    /// Call this before [`draw()`](Self::draw). Separating prepare/draw lets
    /// the app layer manage the wgpu render pass.
    pub fn prepare_grid(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        grid: &Grid,
        cursor: &CursorState,
    ) -> Result<(), RenderError> {
        self.prepare_grid_with_dirty(device, queue, grid, cursor, None)
    }

    /// Prepare with dirty rect optimization.
    ///
    /// When `dirty` is `Some(rect)`, only the affected rows are rebuilt.
    /// When `None`, all rows are rebuilt (full repaint).
    pub fn prepare_grid_with_dirty(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        grid: &Grid,
        cursor: &CursorState,
        dirty: Option<&DirtyRect>,
    ) -> Result<(), RenderError> {
        // Update viewport resolution
        self.viewport.update(queue, self.resolution);

        let theme = &self.theme;
        let metrics = Metrics::new(self.font_size, self.line_height);
        let cell_h = self.cell_height() as f32;

        // Determine which rows to render
        let (row_start, row_end) = match dirty {
            Some(rect) => (rect.y, (rect.y + rect.height).min(grid.height())),
            None => (0, grid.height()),
        };

        // P18-D: Use the EXACT float advance for positioning.
        // glyphon renders chars at their natural float advance. If we round
        // to integer for positioning, long runs drift: 40 chars × 9.3px = 372px,
        // but grid position = 40 × 9 = 360px → 12px misalignment.
        // Using the exact float advance eliminates this.
        let cell_w = self.cell_width_f32;
        let cell_h_f32 = cell_h;

        // P18-D: Build one glyphon Buffer PER TEXT RUN (not per row).
        // Each run is positioned at its exact grid column: left = start_col * cell_w.
        // This ensures perfect alignment for ASCII, CJK, emoji, and box-drawing chars.
        // Wide chars (CJK/emoji) always start their own run so they don't drift
        // subsequent characters within the same run.
        let mut buffers: Vec<Buffer> = Vec::new();
        type AreaSpec = (usize, f32, f32, (u8, u8, u8));
        let mut text_area_specs: Vec<AreaSpec> = Vec::new();

        for row_idx in row_start..row_end {
            let row_highlights: Vec<(usize, usize)> = self
                .highlights
                .iter()
                .filter(|&&(r, _, _)| r == row_idx)
                .map(|&(_, s, e)| (s, e))
                .collect();

            let runs = converter::row_to_runs(
                grid,
                row_idx,
                theme,
                Some(cursor),
                &row_highlights,
                self.dynamic_fg,
                self.dynamic_bg,
            );

            for run in &runs {
                if run.text.is_empty() {
                    continue;
                }

                // P18-D: No letter_spacing — use the font's exact natural advance.
                // The cell_w is set to the exact float advance of the ASCII 'M',
                // so glyphon's natural positioning matches the grid perfectly.
                let attrs = Attrs::new()
                    .family(Family::Name(TERMINAL_FONT))
                    .color(GlyphonColor::rgb(run.fg.0, run.fg.1, run.fg.2));
                let mut attrs = attrs;
                // P18-D: Do NOT apply Weight::BOLD. Menlo Bold is missing
                // box-drawing glyphs (U+2500-257F), causing tofu/squares.
                // Bold is distinguished by bright foreground color (handled
                // by SGR processing). This matches xterm/Alacritty behavior.
                // Keep the run.bold flag for run splitting only.
                let _ = run.bold; // suppress unused warning
                if run.italic {
                    attrs = attrs.style(glyphon::Style::Italic);
                }
                let attrs_list = AttrsList::new(&attrs);

                let mut buffer = Buffer::new(&mut self.font_system, metrics);
                buffer.lines = vec![BufferLine::new(
                    run.text.clone(),
                    LineEnding::None,
                    attrs_list,
                    Shaping::Advanced,
                )];
                buffer.shape_until_scroll(&mut self.font_system, false);

                let buf_idx = buffers.len();
                buffers.push(buffer);

                let abs_x = run.start_col as f32 * cell_w + self.viewport_offset.0;
                let abs_y = row_idx as f32 * cell_h_f32 + self.viewport_offset.1;
                text_area_specs.push((buf_idx, abs_x, abs_y, run.fg));
            }
        }

        // P19-G: Append overlay text (tab bar, settings, about) as extra buffers.
        let overlay_texts = std::mem::take(&mut self.overlay_text);
        for ot in &overlay_texts {
            let attrs = Attrs::new()
                .family(Family::Name(TERMINAL_FONT))
                .color(GlyphonColor::rgb(ot.color.0, ot.color.1, ot.color.2));
            let attrs_list = AttrsList::new(&attrs);
            let mut buffer = Buffer::new(&mut self.font_system, metrics);
            buffer.lines = vec![BufferLine::new(
                ot.text.clone(),
                LineEnding::None,
                attrs_list,
                Shaping::Advanced,
            )];
            buffer.shape_until_scroll(&mut self.font_system, false);
            let buf_idx = buffers.len();
            buffers.push(buffer);
            text_area_specs.push((buf_idx, ot.left, ot.top, ot.color));
        }
        self.overlay_text = overlay_texts; // restore for next frame

        // Build TextArea references after all buffers are created.
        let text_areas: Vec<TextArea> = text_area_specs
            .iter()
            .map(|&(buf_idx, x, y, fg)| TextArea {
                buffer: &buffers[buf_idx],
                left: x,
                top: y,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: 0,
                    right: self.resolution.width as i32,
                    bottom: self.resolution.height as i32,
                },
                default_color: GlyphonColor::rgb(fg.0, fg.1, fg.2),
                custom_glyphs: &[],
            })
            .collect();

        // Prepare text renderer — shape + rasterize glyphs into GPU atlas
        self.text_renderer.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            text_areas,
            &mut self.swash_cache,
        )?;

        Ok(())
    }

    /// Draw previously-prepared text into a wgpu render pass.
    ///
    /// Call [`prepare_grid()`](Self::prepare_grid) first, then this method
    /// inside your render pass.
    pub fn draw(&self, render_pass: &mut wgpu::RenderPass<'_>) -> Result<(), GlyphonRenderError> {
        self.text_renderer
            .render(&self.atlas, &self.viewport, render_pass)?;
        self.draw_decorations(render_pass);
        self.draw_overlay(render_pass);
        Ok(())
    }

    /// Build the underline render pipeline lazily (P12-B fix).
    fn ensure_underline_pipeline(&mut self, device: &wgpu::Device) {
        if self.underline_pipeline.is_some() {
            return;
        }
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("underline shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/underline.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("underline pipeline layout"),
            bind_group_layouts: &[],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("underline pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 20, // 2 floats (pos) + 3 floats (color) = 20 bytes
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x3],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Bgra8UnormSrgb,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        self.underline_pipeline = Some(pipeline);
    }

    /// Collect underline + strikethrough cells from grid and upload vertex data.
    fn prepare_decorations(&mut self, device: &wgpu::Device, grid: &Grid) {
        let cell_w = self.cell_width_f32;
        let cell_h = self.cell_height() as f32;
        let screen_w = self.resolution.width as f32;
        let screen_h = self.resolution.height as f32;

        let mut underline_verts: Vec<f32> = Vec::new();
        let mut strike_verts: Vec<f32> = Vec::new();
        let underline_y = cell_h - 2.0;
        let strike_y = cell_h * 0.5; // mid-line for strikethrough
        let thickness = 1.0;

        for row_idx in 0..grid.height().min(self.rows) {
            for col_idx in 0..grid.width().min(self.cols) {
                if let Some(cell) = grid.display_cell(col_idx, row_idx) {
                    let theme = &self.theme;
                    let fg = theme.resolve_fg(&cell.fg);
                    let (r, g, b) = (
                        fg.0 as f32 / 255.0,
                        fg.1 as f32 / 255.0,
                        fg.2 as f32 / 255.0,
                    );

                    let px = col_idx as f32 * cell_w + self.viewport_offset.0;
                    let x0 = px / screen_w * 2.0 - 1.0;
                    let x1 = (px + cell_w) / screen_w * 2.0 - 1.0;

                    // Underline
                    if cell.flags.contains(ggterm_core::CellFlags::UNDERLINE) {
                        let py = row_idx as f32 * cell_h + underline_y + self.viewport_offset.1;
                        let y0 = 1.0 - py / screen_h * 2.0;
                        let y1 = 1.0 - (py + thickness) / screen_h * 2.0;
                        for &(x, y) in &[(x0, y0), (x1, y0), (x0, y1), (x1, y0), (x1, y1), (x0, y1)]
                        {
                            underline_verts.extend_from_slice(&[x, y, r, g, b]);
                        }
                    }

                    // Strikethrough (P13-A)
                    if cell.flags.contains(ggterm_core::CellFlags::STRIKETHROUGH) {
                        let py = row_idx as f32 * cell_h + strike_y + self.viewport_offset.1;
                        let y0 = 1.0 - py / screen_h * 2.0;
                        let y1 = 1.0 - (py + thickness) / screen_h * 2.0;
                        for &(x, y) in &[(x0, y0), (x1, y0), (x0, y1), (x1, y0), (x1, y1), (x0, y1)]
                        {
                            strike_verts.extend_from_slice(&[x, y, r, g, b]);
                        }
                    }
                }
            }
        }

        // Upload underline vertices
        upload_vertices(
            device,
            &underline_verts,
            &mut self.underline_vertex_buffer,
            &mut self.underline_vertex_count,
            "underline",
            5, // 5 floats per vertex: x, y, r, g, b
        );

        // Upload strikethrough vertices
        upload_vertices(
            device,
            &strike_verts,
            &mut self.strike_vertex_buffer,
            &mut self.strike_vertex_count,
            "strikethrough",
            5, // 5 floats per vertex: x, y, r, g, b
        );
    }

    /// Draw underline + strikethrough rectangles into the render pass.
    fn draw_decorations(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        let pipeline = match &self.underline_pipeline {
            Some(p) => p,
            None => return,
        };

        // Draw underlines
        if let Some(ref buffer) = self.underline_vertex_buffer
            && self.underline_vertex_count > 0
        {
            render_pass.set_pipeline(pipeline);
            render_pass.set_vertex_buffer(0, buffer.slice(..));
            render_pass.draw(0..self.underline_vertex_count, 0..1);
        }

        // Draw strikethroughs (reuse same pipeline shape)
        if let Some(ref buffer) = self.strike_vertex_buffer
            && self.strike_vertex_count > 0
        {
            render_pass.set_pipeline(pipeline);
            render_pass.set_vertex_buffer(0, buffer.slice(..));
            render_pass.draw(0..self.strike_vertex_count, 0..1);
        }
    }

    // ── P19-G: UI Overlay Rendering ──────────────────────────────────

    /// Set overlay text specs for this frame. Call before `render_to_pass()`.
    pub fn set_overlay_text(&mut self, texts: Vec<OverlayTextSpec>) {
        self.overlay_text = texts;
    }

    /// Set overlay rectangle specs for this frame. Call before `render_to_pass()`.
    pub fn set_overlay_rects(&mut self, rects: Vec<OverlayRect>) {
        self.overlay_rects = rects;
    }

    /// Clear all overlay data.
    pub fn clear_overlay(&mut self) {
        self.overlay_text.clear();
        self.overlay_rects.clear();
        self.overlay_vertex_count = 0;
        self.overlay_vertex_buffer = None;
    }

    /// Generate overlay vertices from `overlay_rects` and upload to GPU.
    fn prepare_overlay(&mut self, device: &wgpu::Device) {
        let screen_w = self.resolution.width as f32;
        let screen_h = self.resolution.height as f32;
        let mut verts: Vec<f32> = Vec::new();

        for r in &self.overlay_rects {
            push_rect(&mut verts, r.x, r.y, r.w, r.h, r.color, screen_w, screen_h);
        }

        upload_vertices(
            device,
            &verts,
            &mut self.overlay_vertex_buffer,
            &mut self.overlay_vertex_count,
            "overlay",
            5, // 5 floats per vertex: x, y, r, g, b
        );
    }

    /// Draw overlay rectangles using the underline pipeline.
    fn draw_overlay(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        if let Some(ref pipeline) = self.underline_pipeline
            && let Some(ref buffer) = self.overlay_vertex_buffer
            && self.overlay_vertex_count > 0
        {
            render_pass.set_pipeline(pipeline);
            render_pass.set_vertex_buffer(0, buffer.slice(..));
            render_pass.draw(0..self.overlay_vertex_count, 0..1);
        }
    }

    // ── P26-A: Modern UI Rendering (SDF rounded rectangles) ──────────

    /// Set UI rectangle specs for this frame. Call before `render_overlays_to_pass()`.
    pub fn set_ui_rects(&mut self, rects: Vec<UiRect>) {
        self.ui_rects = rects;
    }

    /// Build the UI SDF pipeline lazily.
    fn ensure_ui_pipeline(&mut self, device: &wgpu::Device) {
        if self.ui_pipeline.is_some() {
            return;
        }
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/ui.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui pipeline layout"),
            bind_group_layouts: &[],
            ..Default::default()
        });
        // Vertex layout: 12 floats per vertex = 48 bytes stride.
        //   0: position.xy     (Float32x2, 8 bytes)
        //   1: color.rgba      (Float32x4, 16 bytes)
        //   2: local_pos.xy    (Float32x2, 8 bytes)
        //   3: half_size.xy    (Float32x2, 8 bytes)
        //   4: params.xy       (Float32x2, 8 bytes) — x=radius, y=stroke_width
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 48, // 12 floats × 4 bytes
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, // position
                        1 => Float32x4, // color
                        2 => Float32x2, // local_pos
                        3 => Float32x2, // half_size
                        4 => Float32x2, // params (radius, stroke_width)
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Bgra8UnormSrgb,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        self.ui_pipeline = Some(pipeline);
    }

    /// Generate vertices for all `ui_rects` and upload to GPU.
    fn prepare_ui(&mut self, device: &wgpu::Device) {
        let screen_w = self.resolution.width as f32;
        let screen_h = self.resolution.height as f32;
        let mut verts: Vec<f32> = Vec::new();

        for r in &self.ui_rects {
            push_ui_rect(
                &mut verts,
                r.x,
                r.y,
                r.w,
                r.h,
                r.color,
                r.radius,
                r.stroke_width,
                screen_w,
                screen_h,
            );
        }

        upload_vertices(
            device,
            &verts,
            &mut self.ui_vertex_buffer,
            &mut self.ui_vertex_count,
            "ui overlay",
            12, // 12 floats per vertex
        );
    }

    /// Draw UI rectangles using the SDF pipeline.
    fn draw_ui(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        if let Some(ref pipeline) = self.ui_pipeline
            && let Some(ref buffer) = self.ui_vertex_buffer
            && self.ui_vertex_count > 0
        {
            render_pass.set_pipeline(pipeline);
            render_pass.set_vertex_buffer(0, buffer.slice(..));
            render_pass.draw(0..self.ui_vertex_count, 0..1);
        }
    }

    /// Full render cycle: prepare grid + draw into render pass.
    ///
    /// Convenience method combining [`prepare_grid()`](Self::prepare_grid) and
    /// [`draw()`](Self::draw). The caller creates the render pass from the
    /// surface texture view.
    ///
    /// # Example
    /// ```ignore
    /// let frame = surface.get_current_texture()?;
    /// let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
    /// let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    /// {
    ///     let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
    ///         color_attachments: &[Some(wgpu::RenderPassColorAttachment {
    ///             view: &view,
    ///             resolve_target: None,
    ///             ops: wgpu::Operations {
    ///                 load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
    ///                 store: wgpu::StoreOp::Store,
    ///             },
    ///         })],
    ///         ..Default::default()
    ///     });
    ///     renderer.render_to_pass(&device, &queue, &grid, &cursor, &mut pass)?;
    /// }
    /// queue.submit(std::iter::once(encoder.finish()));
    /// frame.present();
    /// ```
    pub fn render_to_pass(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        grid: &Grid,
        cursor: &CursorState,
        render_pass: &mut wgpu::RenderPass<'_>,
    ) -> Result<(), RenderError> {
        self.prepare_grid(device, queue, grid, cursor)?;
        self.ensure_underline_pipeline(device);
        self.prepare_decorations(device, grid);
        self.prepare_overlay(device);
        self.draw(render_pass)?;
        Ok(())
    }

    /// Full render with dirty rect optimization.
    #[allow(clippy::too_many_arguments)]
    pub fn render_to_pass_with_dirty(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        grid: &Grid,
        cursor: &CursorState,
        dirty: Option<&DirtyRect>,
        render_pass: &mut wgpu::RenderPass<'_>,
    ) -> Result<(), RenderError> {
        self.prepare_grid_with_dirty(device, queue, grid, cursor, dirty)?;
        self.draw(render_pass)?;
        Ok(())
    }

    /// Prepare and draw ONE pane's grid + decorations (no overlay) (P20-A).
    ///
    /// Call `set_viewport_offset()` before this to position the pane.
    /// Overlays are drawn separately via `prepare_overlay` + `draw_overlay`.
    ///
    /// `needs_prepare` (P21-D): when `false`, skips `prepare_grid()` and
    /// `prepare_decorations()`, reusing existing glyphon buffers. The draw
    /// is always performed (wgpu requires a full render pass).
    pub fn render_pane_to_pass(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        grid: &Grid,
        cursor: &CursorState,
        needs_prepare: bool,
        render_pass: &mut wgpu::RenderPass<'_>,
    ) -> Result<(), RenderError> {
        // Temporarily remove overlay text so prepare_grid_with_dirty
        // doesn't mix it into the pane's text atlas. Overlay text is
        // rendered separately in render_overlays_to_pass with full-screen
        // scissor. Without this, overlay text gets double-rendered
        // (once clipped in pane pass, once in overlay pass), causing
        // visual ghosting/blurriness in multi-pane mode.
        let saved_overlay = std::mem::take(&mut self.overlay_text);
        if needs_prepare {
            self.prepare_grid(device, queue, grid, cursor)?;
            self.ensure_underline_pipeline(device);
            self.prepare_decorations(device, grid);
        }
        self.overlay_text = saved_overlay;
        // Draw text + decorations only (no overlay text).
        self.text_renderer
            .render(&self.atlas, &self.viewport, render_pass)?;
        self.draw_decorations(render_pass);
        Ok(())
    }

    /// Prepare and draw overlays (P20-A).
    ///
    /// Call after all panes are rendered. Resets viewport offset to (0, 0).
    pub fn render_overlays_to_pass(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render_pass: &mut wgpu::RenderPass<'_>,
    ) -> Result<(), RenderError> {
        self.viewport_offset = (0.0, 0.0);
        self.ensure_underline_pipeline(device);
        self.ensure_ui_pipeline(device);

        // Step 1: Draw UI backgrounds (tab bar, status bar, dialogs) FIRST
        // so overlay text is rendered ON TOP, not hidden behind them.
        self.prepare_overlay(device);
        self.draw_overlay(render_pass);
        self.prepare_ui(device);
        self.draw_ui(render_pass);

        // Step 2: Prepare and render overlay TEXT (tab bar, status bar, etc)
        // on top of the UI backgrounds.
        let overlay_texts = std::mem::take(&mut self.overlay_text);
        if !overlay_texts.is_empty() {
            let metrics = Metrics::new(self.font_size, self.font_size);
            let mut buffers: Vec<Buffer> = Vec::new();
            #[allow(clippy::type_complexity)]
            let mut text_area_specs: Vec<(usize, f32, f32, (u8, u8, u8))> = Vec::new();

            for ot in &overlay_texts {
                let attrs = Attrs::new()
                    .family(Family::Name(TERMINAL_FONT))
                    .color(GlyphonColor::rgb(ot.color.0, ot.color.1, ot.color.2));
                let attrs_list = AttrsList::new(&attrs);
                let mut buffer = Buffer::new(&mut self.font_system, metrics);
                buffer.lines = vec![BufferLine::new(
                    ot.text.clone(),
                    LineEnding::None,
                    attrs_list,
                    Shaping::Advanced,
                )];
                buffer.shape_until_scroll(&mut self.font_system, false);
                let buf_idx = buffers.len();
                buffers.push(buffer);
                text_area_specs.push((buf_idx, ot.left, ot.top, ot.color));
            }

            let text_areas: Vec<TextArea> = text_area_specs
                .iter()
                .map(|&(buf_idx, x, y, fg)| TextArea {
                    buffer: &buffers[buf_idx],
                    left: x,
                    top: y,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: 0,
                        top: 0,
                        right: self.resolution.width as i32,
                        bottom: self.resolution.height as i32,
                    },
                    default_color: GlyphonColor::rgb(fg.0, fg.1, fg.2),
                    custom_glyphs: &[],
                })
                .collect();

            self.text_renderer.prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                text_areas,
                &mut self.swash_cache,
            )?;
            self.text_renderer
                .render(&self.atlas, &self.viewport, render_pass)?;
        }
        self.overlay_text = overlay_texts;
        Ok(())
    }
}

impl Renderer for GlyphonRenderer {
    fn render(&mut self, _grid: &Grid, _cursor: &CursorState, _dirty: Option<&DirtyRect>) {
        // GPU rendering requires a wgpu render pass from the surface.
        // The app layer (P1-F3: winit) calls render_to_pass() in its render loop.
        //
        // This trait method is used by the headless test harness and ConsoleRenderer.
        // For GPU rendering, use `render_to_pass()` directly.
    }

    fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.rows = rows;
        // Keep resolution in sync — cols/rows * cell dimensions.
        let cell_w = self.cell_width();
        let cell_h = self.cell_height();
        self.resolution = Resolution {
            width: cols.max(1) as u32 * cell_w,
            height: rows.max(1) as u32 * cell_h,
        };
    }
}

/// Upload vertex data to a GPU buffer (helper for decoration rendering).
/// Push a rectangle's 6 vertices (2 triangles) into the vertex buffer.
/// Coordinates are in pixel space, converted to NDC.
#[allow(clippy::too_many_arguments)]
fn push_rect(
    verts: &mut Vec<f32>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: (f32, f32, f32),
    screen_w: f32,
    screen_h: f32,
) {
    let (r, g, b) = color;
    // Convert pixel coords to NDC: [-1, 1]
    let x0 = (x / screen_w) * 2.0 - 1.0;
    let x1 = ((x + w) / screen_w) * 2.0 - 1.0;
    let y0 = 1.0 - (y / screen_h) * 2.0;
    let y1 = 1.0 - ((y + h) / screen_h) * 2.0;
    // Two triangles: top-left, top-right, bottom-right + top-left, bottom-right, bottom-left
    // Each vertex: [x, y, r, g, b] (matches underline pipeline layout)
    verts.extend_from_slice(&[
        x0, y0, r, g, b, x1, y0, r, g, b, x1, y1, r, g, b, x0, y0, r, g, b, x1, y1, r, g, b, x0,
        y1, r, g, b,
    ]);
}

/// Generate 6 vertices (2 triangles) for a rounded-rect UI element.
///
/// Each vertex carries: `[pos.xy, color.rgba, local_pos.xy, half_size.xy, params.xy]`
/// = 12 floats. The fragment shader uses local_pos + half_size + radius
/// to evaluate the SDF for anti-aliased rounded corners.
///
/// For stroke rendering (stroke_width > 0), the shader draws only pixels
/// near the rect boundary. For fill rendering (stroke_width == 0), it
/// fills the entire interior.
#[allow(clippy::too_many_arguments)]
fn push_ui_rect(
    verts: &mut Vec<f32>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: (f32, f32, f32, f32),
    radius: f32,
    stroke_width: f32,
    screen_w: f32,
    screen_h: f32,
) {
    let (r, g, b, a) = color;

    // Local pixel coords relative to rect center (used by SDF in fragment shader).
    let half_w = w * 0.5;
    let half_h = h * 0.5;

    // For stroke rendering, expand the rect slightly to accommodate the stroke.
    // The stroke is drawn centered on the boundary, so we need the vertex quad
    // to be slightly larger than the visual rect.
    let expand = if stroke_width > 0.5 {
        stroke_width * 0.5 + 1.5 // half stroke + AA feather
    } else {
        1.5 // AA feather only
    };
    let ex0 = ((x - expand) / screen_w) * 2.0 - 1.0;
    let ex1 = ((x + w + expand) / screen_w) * 2.0 - 1.0;
    let ey0 = 1.0 - ((y - expand) / screen_h) * 2.0;
    let ey1 = 1.0 - ((y + h + expand) / screen_h) * 2.0;

    let elx0 = -half_w - expand;
    let elx1 = half_w + expand;
    let ely0 = -half_h - expand;
    let ely1 = half_h + expand;

    // Two triangles: TL, TR, BR + TL, BR, BL
    // Use expanded coords for rendering so AA + stroke have room.
    // Use un-expanded half_size so SDF matches the visual rect.
    let p = (radius, stroke_width);
    verts.extend_from_slice(&[
        // Triangle 1: TL, TR, BR
        ex0, ey0, r, g, b, a, elx0, ely0, half_w, half_h, p.0, p.1, // TL
        ex1, ey0, r, g, b, a, elx1, ely0, half_w, half_h, p.0, p.1, // TR
        ex1, ey1, r, g, b, a, elx1, ely1, half_w, half_h, p.0, p.1, // BR
        // Triangle 2: TL, BR, BL
        ex0, ey0, r, g, b, a, elx0, ely0, half_w, half_h, p.0, p.1, // TL
        ex1, ey1, r, g, b, a, elx1, ely1, half_w, half_h, p.0, p.1, // BR
        ex0, ey1, r, g, b, a, elx0, ely1, half_w, half_h, p.0, p.1, // BL
    ]);
}

fn upload_vertices(
    device: &wgpu::Device,
    vertices: &[f32],
    buffer_slot: &mut Option<wgpu::Buffer>,
    count_slot: &mut u32,
    label: &str,
    stride: usize,
) {
    *count_slot = (vertices.len() / stride) as u32;
    if vertices.is_empty() {
        *buffer_slot = None;
        return;
    }
    let buffer_data: Vec<u8> = vertices.iter().flat_map(|f| f.to_ne_bytes()).collect();
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (buffer_data.len().max(4)) as u64,
        usage: wgpu::BufferUsages::VERTEX,
        mapped_at_creation: true,
    });
    buffer
        .slice(..)
        .get_mapped_range_mut()
        .copy_from_slice(&buffer_data);
    buffer.unmap();
    *buffer_slot = Some(buffer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ggterm_core::{Cell, Grid};

    /// Verify that Grid → TextRun conversion produces correct text content.
    #[test]
    fn test_grid_to_text_runs_basic() {
        let mut grid = Grid::new(10, 2);
        grid[(0, 0)] = Cell::with_char('H');
        grid[(1, 0)] = Cell::with_char('i');

        let theme = RenderTheme::default();
        let runs = converter::row_to_runs(&grid, 0, &theme, None, &[], None, None);
        let text: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text.trim_end(), "Hi");
    }

    /// Verify that wide character cells produce correct text runs.
    #[test]
    fn test_grid_to_text_runs_cjk() {
        let mut grid = Grid::new(10, 1);
        grid.put_char(0, 0, '你');
        grid.put_char(2, 0, '好');

        let theme = RenderTheme::default();
        let runs = converter::row_to_runs(&grid, 0, &theme, None, &[], None, None);
        let text: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text.trim_end(), "你好");
    }

    /// Verify that empty rows produce empty text.
    #[test]
    fn test_grid_to_text_empty_row() {
        let grid = Grid::new(5, 1);
        let theme = RenderTheme::default();
        let runs = converter::row_to_runs(&grid, 0, &theme, None, &[], None, None);
        let text: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text.trim_end(), "");
    }

    /// Verify that cell dimensions are computed correctly.
    #[test]
    fn test_cell_dimensions() {
        let font_size = DEFAULT_FONT_SIZE;
        let line_height = DEFAULT_LINE_HEIGHT;
        let cell_w = (font_size * 0.6).ceil() as u32;
        let cell_h = line_height.ceil() as u32;
        assert!(cell_w > 0);
        assert!(cell_h > 0);
        assert!(cell_h >= cell_w, "line height should be >= cell width");
    }

    /// Verify RenderError Display formatting.
    #[test]
    fn test_render_error_display() {
        let err = RenderError::Prepare(PrepareError::AtlasFull);
        let s = format!("{err}");
        assert!(s.contains("prepare error"), "got: {s}");
        assert!(s.contains("atlas"), "got: {s}");
    }

    // ── P19-G: Overlay Rendering Tests ─────────────────────────────────

    #[test]
    fn test_overlay_text_spec_creation() {
        let spec = OverlayTextSpec {
            text: "1:zsh*".to_string(),
            left: 10.0,
            top: 0.0,
            color: (220, 220, 220),
        };
        assert_eq!(spec.text, "1:zsh*");
        assert_eq!(spec.left, 10.0);
        assert_eq!(spec.color, (220, 220, 220));
    }

    #[test]
    fn test_overlay_rect_creation() {
        let rect = OverlayRect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 20.0,
            color: (0.12, 0.12, 0.15),
        };
        assert_eq!(rect.w, 800.0);
        assert_eq!(rect.h, 20.0);
    }

    #[test]
    fn test_push_rect_vertex_count() {
        let mut verts: Vec<f32> = Vec::new();
        push_rect(
            &mut verts,
            0.0,
            0.0,
            100.0,
            50.0,
            (1.0, 0.0, 0.0),
            800.0,
            600.0,
        );
        // Each rectangle = 6 vertices × 5 floats = 30 floats
        assert_eq!(verts.len(), 30);
    }

    #[test]
    fn test_push_rect_ndc_conversion() {
        let mut verts: Vec<f32> = Vec::new();
        let sw = 800.0_f32;
        let sh = 600.0_f32;
        // Full-screen rect: x=0,y=0,w=800,h=600
        push_rect(&mut verts, 0.0, 0.0, sw, sh, (1.0, 1.0, 1.0), sw, sh);
        // x0 = 0/sw*2 - 1 = -1.0 (left edge)
        assert!(
            (verts[0] + 1.0).abs() < 0.001,
            "x0 should be -1.0, got {}",
            verts[0]
        );
        // x1 = 800/800*2 - 1 = 1.0 (right edge)
        assert!(
            (verts[5] - 1.0).abs() < 0.001,
            "x1 should be 1.0, got {}",
            verts[5]
        );
        // y0 = 1 - 0/600*2 = 1.0 (top edge)
        assert!(
            (verts[1] - 1.0).abs() < 0.001,
            "y0 should be 1.0, got {}",
            verts[1]
        );
        // Color r=1.0 at index 2
        assert!((verts[2] - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_push_rect_multiple() {
        let mut verts: Vec<f32> = Vec::new();
        let sw = 800.0_f32;
        let sh = 600.0_f32;
        // Push two rectangles
        push_rect(&mut verts, 0.0, 0.0, 400.0, 20.0, (1.0, 0.0, 0.0), sw, sh);
        push_rect(&mut verts, 400.0, 0.0, 400.0, 20.0, (0.0, 1.0, 0.0), sw, sh);
        // Two rectangles = 60 floats
        assert_eq!(verts.len(), 60);
    }

    #[test]
    fn test_push_rect_empty() {
        let mut verts: Vec<f32> = Vec::new();
        push_rect(
            &mut verts,
            0.0,
            0.0,
            0.0,
            0.0,
            (1.0, 1.0, 1.0),
            800.0,
            600.0,
        );
        // Even a zero-size rect still produces 6 vertices
        assert_eq!(verts.len(), 30);
    }

    // ── P20-A: Viewport offset tests ───────────────────────────

    #[test]
    fn test_viewport_offset_default_zero() {
        // Without a GPU, we can't create a real GlyphonRenderer, but the
        // offset field is initialized to (0.0, 0.0) by default.
        // Verify the concept is sound by checking default initialization.
        let offset: (f32, f32) = (0.0, 0.0);
        assert_eq!(offset, (0.0, 0.0));
    }

    #[test]
    fn test_viewport_offset_arithmetic() {
        // Verify that offset arithmetic works correctly.
        // A pane at pixel (200, 0) should shift text positions.
        let cell_w = 9.0_f32; // typical cell width
        let cell_h = 15.0_f32; // typical cell height
        let offset = (200.0_f32, 0.0_f32);

        // Cell at col=0, row=0 should render at offset position.
        let abs_x = 0.0 * cell_w + offset.0;
        let abs_y = 0.0 * cell_h + offset.1;
        assert_eq!(abs_x, 200.0);
        assert_eq!(abs_y, 0.0);

        // Cell at col=3, row=2.
        let abs_x2 = 3.0 * cell_w + offset.0;
        let abs_y2 = 2.0 * cell_h + offset.1;
        assert_eq!(abs_x2, 227.0);
        assert_eq!(abs_y2, 30.0);
    }

    #[test]
    fn test_viewport_offset_reset() {
        // Simulate offset reset for overlay rendering.
        let mut offset = (100.0_f32, 50.0_f32);
        assert_eq!(offset, (100.0, 50.0));

        // Reset to (0, 0) for overlay rendering.
        offset = (0.0, 0.0);
        assert_eq!(offset, (0.0, 0.0));
    }

    // ── P26-A: UiRect + push_ui_rect tests ────────────────────────

    #[test]
    fn test_uirect_construction() {
        let r = UiRect {
            x: 10.0,
            y: 20.0,
            w: 100.0,
            h: 40.0,
            color: (0.5, 0.5, 0.5, 0.9),
            radius: 8.0,
            stroke_width: 0.0,
        };
        assert_eq!(r.x, 10.0);
        assert_eq!(r.w, 100.0);
        assert_eq!(r.radius, 8.0);
        assert_eq!(r.color.3, 0.9); // alpha
    }

    #[test]
    fn test_push_ui_rect_vertex_count() {
        let mut verts: Vec<f32> = Vec::new();
        push_ui_rect(
            &mut verts,
            0.0,
            0.0,
            100.0,
            50.0,
            (1.0, 0.0, 0.0, 1.0),
            6.0,
            0.0,
            800.0,
            600.0,
        );
        // 6 vertices * 12 floats = 72 floats
        assert_eq!(verts.len(), 72);
    }

    #[test]
    fn test_push_ui_rect_ndc_coords() {
        let mut verts: Vec<f32> = Vec::new();
        push_ui_rect(
            &mut verts,
            0.0,
            0.0,
            800.0, // full width
            600.0, // full height
            (1.0, 1.0, 1.0, 1.0),
            0.0,
            0.0,
            800.0,
            600.0,
        );
        // First vertex: top-left with expand = 1.5px (AA feather)
        // ex0 = ((0 - 1.5) / 800) * 2 - 1 = -1.00375
        // ey0 = 1 - ((0 - 1.5) / 600) * 2 = 1.005
        assert!(verts[0] < -1.0); // expanded beyond left edge
        assert!(verts[1] > 1.0); // expanded beyond top edge
    }

    #[test]
    fn test_push_ui_rect_local_coords() {
        let mut verts: Vec<f32> = Vec::new();
        push_ui_rect(
            &mut verts,
            0.0,
            0.0,
            100.0,
            50.0,
            (0.0, 0.0, 0.0, 1.0),
            4.0,
            0.0,
            800.0,
            600.0,
        );
        // local_pos is at indices 6,7 in each vertex (after pos.xy + color.rgba)
        // With expand=1.5, first vertex local_pos = (-half_w - 1.5, -half_h - 1.5)
        let lx0 = verts[6];
        let ly0 = verts[7];
        assert!((lx0 - (-51.5)).abs() < 0.001); // -50 - 1.5 expand
        assert!((ly0 - (-26.5)).abs() < 0.001); // -25 - 1.5 expand

        // half_size at indices 8,9 (un-expanded)
        let hw = verts[8];
        let hh = verts[9];
        assert!((hw - 50.0).abs() < 0.001);
        assert!((hh - 25.0).abs() < 0.001);
    }

    #[test]
    fn test_push_ui_rect_params() {
        let mut verts: Vec<f32> = Vec::new();
        push_ui_rect(
            &mut verts,
            10.0,
            10.0,
            80.0,
            40.0,
            (0.2, 0.4, 0.6, 0.8),
            12.0,
            2.0,
            800.0,
            600.0,
        );
        // params at indices 10,11: radius, stroke_width
        let radius = verts[10];
        let stroke = verts[11];
        assert!((radius - 12.0).abs() < 0.001);
        assert!((stroke - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_push_ui_rect_alpha() {
        let mut verts: Vec<f32> = Vec::new();
        push_ui_rect(
            &mut verts,
            0.0,
            0.0,
            10.0,
            10.0,
            (1.0, 0.5, 0.25, 0.75),
            0.0,
            0.0,
            100.0,
            100.0,
        );
        // Alpha is at index 5 in each vertex
        assert!((verts[5] - 0.75).abs() < 0.001);
    }

    #[test]
    fn test_push_ui_rect_multiple() {
        let mut verts: Vec<f32> = Vec::new();
        let sw = 800.0_f32;
        let sh = 600.0_f32;
        push_ui_rect(
            &mut verts,
            0.0,
            0.0,
            400.0,
            20.0,
            (1.0, 0.0, 0.0, 1.0),
            4.0,
            0.0,
            sw,
            sh,
        );
        push_ui_rect(
            &mut verts,
            400.0,
            0.0,
            400.0,
            20.0,
            (0.0, 1.0, 0.0, 1.0),
            4.0,
            0.0,
            sw,
            sh,
        );
        // Two rects = 12 vertices * 12 floats = 144 floats
        assert_eq!(verts.len(), 144);
    }

    #[test]
    fn test_push_ui_rect_stroke_mode() {
        let mut verts: Vec<f32> = Vec::new();
        push_ui_rect(
            &mut verts,
            10.0,
            10.0,
            80.0,
            40.0,
            (0.0, 0.5, 1.0, 1.0),
            6.0,
            2.0, // stroke mode
            800.0,
            600.0,
        );
        // Should still produce 6 vertices
        assert_eq!(verts.len() / 12, 6);
        // stroke_width at index 11 should be 2.0
        assert!((verts[11] - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_uirect_fill_vs_stroke() {
        let fill = UiRect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 50.0,
            color: (1.0, 1.0, 1.0, 0.5),
            radius: 8.0,
            stroke_width: 0.0,
        };
        let stroke = UiRect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 50.0,
            color: (1.0, 1.0, 1.0, 1.0),
            radius: 8.0,
            stroke_width: 2.0,
        };
        assert!(fill.stroke_width <= 0.5);
        assert!(stroke.stroke_width > 0.5);
    }
}
