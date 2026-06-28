//! # GGTerm Render — wgpu + glyphon
//!
//! GPU-accelerated terminal renderer using [wgpu] and [glyphon].
//!
//! Implements the [`Renderer`] trait from `ggterm-render`.

pub mod colors;
pub mod converter;

pub use colors::{map_bg, map_color, map_fg, indexed_to_rgb, ANSI_16, DEFAULT_FG, DEFAULT_BG};
pub use converter::{row_to_runs, row_to_text, TextRun};

use ggterm_core::{DirtyRect, Grid};
use ggterm_render::theme::RenderTheme;
use ggterm_render::{CursorState, Renderer};
use glyphon::{
   Attrs, AttrsList, Buffer, BufferLine, Cache as GlyphonCache, Color as GlyphonColor, Family,
    FontSystem, Metrics, Resolution, Shaping, SwashCache, TextAtlas, TextBounds, TextArea,
    TextRenderer, Viewport,
};
use glyphon::cosmic_text::LineEnding;

const DEFAULT_FONT_SIZE: f32 = 15.0;
const DEFAULT_LINE_HEIGHT: f32 = 20.0;

/// GPU-accelerated terminal renderer using wgpu + glyphon.
#[allow(dead_code)]
pub struct GlyphonRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    #[allow(dead_code)]
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
}

impl GlyphonRenderer {
    /// Create a new GlyphonRenderer.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        cols: usize,
        rows: usize,
    ) -> Self {
        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = GlyphonCache::new(device);
        let mut atlas = TextAtlas::new(device, queue, &cache, surface_format);

        let cell_w = (DEFAULT_FONT_SIZE * 0.6).ceil() as u32;
        let cell_h = DEFAULT_LINE_HEIGHT.ceil() as u32;

        let text_renderer = TextRenderer::new(
            &mut atlas,
            device,
            wgpu::MultisampleState::default(),
            None,
        );

        let mut viewport = Viewport::new(device, &cache);
        viewport.update(queue, Resolution {
            width: cols.max(1) as u32 * cell_w,
            height: rows.max(1) as u32 * cell_h,
        });

        Self {
            font_system,
            swash_cache,
            cache,
            atlas,
            text_renderer,
            viewport,
            resolution: Resolution {
                width: cols.max(1) as u32 * cell_w,
                height: rows.max(1) as u32 * cell_h,
            },
            cols,
            rows,
            font_size: DEFAULT_FONT_SIZE,
            line_height: DEFAULT_LINE_HEIGHT,
        }
    }

    /// Get the estimated cell width in pixels.
    pub fn cell_width(&self) -> u32 {
        (self.font_size * 0.6).ceil() as u32
    }

    /// Get the cell height in pixels.
    pub fn cell_height(&self) -> u32 {
        self.line_height.ceil() as u32
    }

    /// Prepare and render the grid into a wgpu render pass.
    pub fn render_to_pass(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        grid: &Grid,
        cursor: &CursorState,
        render_pass: &mut wgpu::RenderPass<'_>,
    ) -> Result<(), glyphon::RenderError> {
        // Update viewport
        self.viewport.update(queue, self.resolution);

        // Build text buffers from grid rows
        let theme = RenderTheme::default();
        let metrics = Metrics::new(self.font_size, self.line_height);
        let cell_h = self.cell_height() as f32;

        let mut buffers: Vec<Buffer> = Vec::with_capacity(grid.height());

        for row_idx in 0..grid.height() {
            let runs = converter::row_to_runs(grid, row_idx, &theme, Some(cursor));

            let mut text = String::new();
            let default_attrs = Attrs::new()
                .family(Family::Monospace)
                .color(GlyphonColor::rgb(0xE0, 0xE0, 0xE0));
            let mut attrs_list = AttrsList::new(&default_attrs);

            for run in &runs {
                let start = text.len();
                text.push_str(&run.text);
                let end = text.len();

                let mut attrs = Attrs::new()
                    .family(Family::Monospace)
                    .color(GlyphonColor::rgb(run.fg.0, run.fg.1, run.fg.2));

                if run.bold {
                    attrs = attrs.weight(glyphon::Weight::BOLD);
                }
                if run.italic {
                    attrs = attrs.style(glyphon::Style::Italic);
                }

                attrs_list.add_span(start..end, &attrs);
            }

            let mut buffer = Buffer::new(&mut self.font_system, metrics);
            buffer.lines = vec![BufferLine::new(text, LineEnding::None, attrs_list, Shaping::Advanced)];
            buffers.push(buffer);
        }

        // Build TextArea references into the buffers
        let text_areas: Vec<TextArea> = buffers
            .iter()
            .enumerate()
            .map(|(row_idx, buf)| TextArea {
                buffer: buf,
                left: 0.0,
                top: row_idx as f32 * cell_h,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: 0,
                    right: self.resolution.width as i32,
                    bottom: self.resolution.height as i32,
                },
                default_color: GlyphonColor::rgb(0xE0, 0xE0, 0xE0),
                custom_glyphs: &[],
            })
            .collect();

        // Prepare text renderer
        self.text_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                text_areas,
                &mut self.swash_cache,
            )
            .map_err(|_| glyphon::RenderError::RemovedFromAtlas)?;

        // Render text into the pass
        self.text_renderer
            .render(&self.atlas, &self.viewport, render_pass)?;

        Ok(())
    }
}

impl Renderer for GlyphonRenderer {
    fn render(
        &mut self,
        _grid: &Grid,
        _cursor: &CursorState,
        _dirty: Option<&DirtyRect>,
    ) {
        // GPU rendering requires a wgpu render pass from the surface.
        // The app layer (P1-F3: winit) calls render_to_pass() in its render loop.
    }

    fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.rows = rows;
        let cell_w = self.cell_width();
        let cell_h = self.cell_height();
        self.resolution = Resolution {
            width: cols.max(1) as u32 * cell_w,
            height: rows.max(1) as u32 * cell_h,
        };
    }
}
