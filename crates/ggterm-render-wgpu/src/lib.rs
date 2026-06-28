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
use glyphon::RenderError;
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
    ///
    /// This is the main entry point for GPU rendering. The caller (typically
    /// the winit event loop in P1-F3) provides the wgpu device, queue, and
    /// an active render pass from the surface texture.
    pub fn render_to_pass(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        grid: &Grid,
        cursor: &CursorState,
        render_pass: &mut wgpu::RenderPass<'_>,
    ) -> Result<(), glyphon::RenderError> {
        self.render_to_pass_with_dirty(device, queue, grid, cursor, None, render_pass)
    }

    /// Prepare and render with dirty rect optimization.
    ///
    /// When `dirty` is `Some(rect)`, only the affected rows are rebuilt.
    /// When `None`, all rows are rebuilt (full repaint).
    #[allow(clippy::too_many_arguments)]
    pub fn render_to_pass_with_dirty(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        grid: &Grid,
        cursor: &CursorState,
        dirty: Option<&DirtyRect>,
        render_pass: &mut wgpu::RenderPass<'_>,
    ) -> Result<(), glyphon::RenderError> {
        // Update viewport resolution
        self.viewport.update(queue, self.resolution);

        let theme = RenderTheme::default();
        let metrics = Metrics::new(self.font_size, self.line_height);
        let cell_h = self.cell_height() as f32;

        // Determine which rows to render
        let (row_start, row_end) = match dirty {
            Some(rect) => (rect.y, (rect.y + rect.height).min(grid.height())),
            None => (0, grid.height()),
        };

        // Build glyphon buffers for each visible row
        let mut buffers: Vec<Buffer> = Vec::with_capacity(row_end - row_start);

        for row_idx in row_start..row_end {
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
            buffer.lines = vec![BufferLine::new(
                text,
                LineEnding::None,
                attrs_list,
                Shaping::Advanced,
            )];
            buffer.shape();
            buffers.push(buffer);
        }

        // Build TextArea references — each buffer is positioned at its absolute row
        let text_areas: Vec<TextArea> = buffers
            .iter()
            .enumerate()
            .map(|(i, buf)| {
                let abs_row = row_start + i;
                TextArea {
                    buffer: buf,
                    left: 0.0,
                    top: abs_row as f32 * cell_h,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: 0,
                        top: 0,
                        right: self.resolution.width as i32,
                        bottom: self.resolution.height as i32,
                    },
                    default_color: GlyphonColor::rgb(0xE0, 0xE0, 0xE0),
                    custom_glyphs: &[],
                }
            })
            .collect();

        // Prepare text renderer (shape + rasterize glyphs into atlas)
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
            .map_err(|_| glyphon::RenderError::RemovedFromAtlas)?;;

        // Render text quads into the wgpu render pass
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
        //
        // This trait method is used by the headless test harness and ConsoleRenderer.
        // For GPU rendering, use `render_to_pass()` directly.
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

#[cfg(test)]
mod tests {
    use super::*;
    use ggterm_core::{Grid, Cell};

    /// Verify that Grid → TextRun conversion produces correct text content.
    /// This test doesn't require a GPU — it validates the rendering pipeline's
    /// data transformation layer.
    #[test]
    fn test_grid_to_text_runs_basic() {
        let mut grid = Grid::new(10, 2);
        grid[(0, 0)] = Cell::with_char('H');
        grid[(1, 0)] = Cell::with_char('i');

        let theme = RenderTheme::default();
        let runs = converter::row_to_runs(&grid, 0, &theme, None);
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
        let runs = converter::row_to_runs(&grid, 0, &theme, None);
        let text: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text.trim_end(), "你好");
    }

    /// Verify that empty rows produce empty text.
    #[test]
    fn test_grid_to_text_empty_row() {
        let grid = Grid::new(5, 1);
        let theme = RenderTheme::default();
        let runs = converter::row_to_runs(&grid, 0, &theme, None);
        let text: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text.trim_end(), "");
    }

    /// Verify that cell dimensions are computed correctly.
    #[test]
    fn test_cell_dimensions() {
        // We can't create a GlyphonRenderer without a GPU device,
        // but we can verify the constant-based calculation.
        let font_size = DEFAULT_FONT_SIZE;
        let line_height = DEFAULT_LINE_HEIGHT;
        let cell_w = (font_size * 0.6).ceil() as u32;
        let cell_h = line_height.ceil() as u32;
        assert!(cell_w > 0);
        assert!(cell_h > 0);
        assert!(cell_h >= cell_w, "line height should be >= cell width");
    }
}
