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

/// Measure the advance width of a CJK character (e.g. '已').
/// For perfect grid alignment: cell_w should be cjk_advance / 2.
/// This ensures 2 cells = 1 CJK glyph with no cumulative drift.
fn measure_cjk_width(font_system: &mut FontSystem, font_size: f32) -> f32 {
    let metrics = Metrics::new(font_size, font_size);
    let attrs = Attrs::new().family(Family::Name(TERMINAL_FONT));
    let attrs_list = AttrsList::new(&attrs);

    let mut buffer = Buffer::new(font_system, metrics);
    buffer.lines = vec![BufferLine::new(
        "已已已已已".to_string(), // 5 CJK chars
        LineEnding::None,
        attrs_list,
        Shaping::Advanced,
    )];
    buffer.shape_until_scroll(font_system, false);

    let runs: Vec<LayoutRun> = buffer.layout_runs().collect();
    if let Some(run) = runs.first() {
        if run.glyphs.len() >= 2 {
            let advance = run.glyphs[1].x - run.glyphs[0].x;
            if advance > 0.0 {
                return advance;
            }
        }
        if run.glyphs.len() >= 5 {
            let total = run.glyphs.last().unwrap().x + run.glyphs.last().unwrap().w;
            return total / 5.0;
        }
    }

    // Fallback: CJK chars are typically square (same as font height).
    font_size
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

        // P18-B: Measure real font advance widths.
        // CRITICAL: CJK chars take 2 cells in the terminal model.
        // If cell_w != cjk_advance/2, CJK rows drift from ASCII rows.
        // Fix: derive cell_w from CJK advance / 2 so they're always aligned.
        let cjk_advance = measure_cjk_width(&mut font_system, scaled_font);
        let cell_w_from_cjk = (cjk_advance / 2.0).round() as u32;
        let cell_w_from_ascii = measure_cell_width(&mut font_system, scaled_font).round() as u32;
        // Use the larger of the two to avoid clipping characters.
        let cell_w = cell_w_from_cjk.max(cell_w_from_ascii);
        let cell_h = scaled_lh.round() as u32;
        // letter_spacing adjusts each char to exactly cell_w pixels.
        // If natural advance is 9.3 and cell_w is 9, add 0 to keep natural.
        // This avoids cumulative drift from sub-pixel differences.

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
            cell_width_f32: cell_w as f32,
            measured_cell_width: cell_w,
            scale_factor,
            theme: RenderTheme::default(),
            underline_pipeline: None,
            underline_vertex_buffer: None,
            underline_vertex_count: 0,
            strike_vertex_buffer: None,
            strike_vertex_count: 0,
            highlights: Vec::new(),
        }
    }

    /// Get the cell width in pixels (measured from actual font).
    pub fn cell_width(&self) -> u32 {
        self.measured_cell_width
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

    /// Set the font size and recompute cell metrics (P11-A).
    ///
    /// Line height is derived from font size with a 1.3x multiplier.
    /// Cell width is derived from font size with a 0.6x multiplier.
    pub fn set_font_size(&mut self, size: f32) {
        // P18-A: size is logical points; multiply by scale_factor for physical pixels.
        let physical = size * self.scale_factor as f32;
        self.font_size = physical.clamp(6.0, 144.0);
        // P18-B: line_height = font_size for seamless box-drawing character tiling.
        self.line_height = self.font_size;
        // P18-B: Re-measure cell width for the new font size.
        let measured = measure_cell_width(&mut self.font_system, self.font_size);
        self.cell_width_f32 = measured;
        self.measured_cell_width = measured.round() as u32;
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

            let runs = converter::row_to_runs(grid, row_idx, theme, Some(cursor), &row_highlights);

            for run in &runs {
                if run.text.is_empty() {
                    continue;
                }

                let attrs = Attrs::new()
                    .family(Family::Name(TERMINAL_FONT))
                    .color(GlyphonColor::rgb(run.fg.0, run.fg.1, run.fg.2));
                let mut attrs = attrs;
                if run.bold {
                    attrs = attrs.weight(glyphon::Weight::BOLD);
                }
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

                let abs_x = run.start_col as f32 * cell_w;
                let abs_y = row_idx as f32 * cell_h_f32;
                text_area_specs.push((buf_idx, abs_x, abs_y, run.fg));
            }
        }

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
        // P12-B/P13-A: Draw underlines and strikethroughs after text.
        self.draw_decorations(render_pass);
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
                if let Some(cell) = grid.cell(col_idx, row_idx) {
                    let theme = &self.theme;
                    let fg = theme.resolve_fg(&cell.fg);
                    let (r, g, b) = (
                        fg.0 as f32 / 255.0,
                        fg.1 as f32 / 255.0,
                        fg.2 as f32 / 255.0,
                    );

                    let px = col_idx as f32 * cell_w;
                    let x0 = px / screen_w * 2.0 - 1.0;
                    let x1 = (px + cell_w) / screen_w * 2.0 - 1.0;

                    // Underline
                    if cell.flags.contains(ggterm_core::CellFlags::UNDERLINE) {
                        let py = row_idx as f32 * cell_h + underline_y;
                        let y0 = 1.0 - py / screen_h * 2.0;
                        let y1 = 1.0 - (py + thickness) / screen_h * 2.0;
                        for &(x, y) in &[(x0, y0), (x1, y0), (x0, y1), (x1, y0), (x1, y1), (x0, y1)]
                        {
                            underline_verts.extend_from_slice(&[x, y, r, g, b]);
                        }
                    }

                    // Strikethrough (P13-A)
                    if cell.flags.contains(ggterm_core::CellFlags::STRIKETHROUGH) {
                        let py = row_idx as f32 * cell_h + strike_y;
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
        );

        // Upload strikethrough vertices
        upload_vertices(
            device,
            &strike_verts,
            &mut self.strike_vertex_buffer,
            &mut self.strike_vertex_count,
            "strikethrough",
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
fn upload_vertices(
    device: &wgpu::Device,
    vertices: &[f32],
    buffer_slot: &mut Option<wgpu::Buffer>,
    count_slot: &mut u32,
    label: &str,
) {
    *count_slot = (vertices.len() / 5) as u32;
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
        let runs = converter::row_to_runs(&grid, 0, &theme, None, &[]);
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
        let runs = converter::row_to_runs(&grid, 0, &theme, None, &[]);
        let text: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text.trim_end(), "你好");
    }

    /// Verify that empty rows produce empty text.
    #[test]
    fn test_grid_to_text_empty_row() {
        let grid = Grid::new(5, 1);
        let theme = RenderTheme::default();
        let runs = converter::row_to_runs(&grid, 0, &theme, None, &[]);
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
}
