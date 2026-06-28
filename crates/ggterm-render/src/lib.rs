//! # GGTerm Render
//!
//! Rendering abstraction layer for GGTerm.
//!
//! This crate provides a [`Renderer`] trait that abstracts terminal rendering.
//! The core crate (`ggterm-core`) produces a [`Grid`] of cells; renderers consume
//! that grid and display it.
//!
//! ## Implementations
//! - [`ConsoleRenderer`] — Renders to an ANSI-colored string (for testing/headless).
//! - (Future) `WgpuRenderer` — GPU-accelerated rendering via wgpu + glyphon.

pub mod console;
pub mod theme;

pub use console::ConsoleRenderer;
pub use theme::{CursorStyle, RenderTheme};

use ggterm_core::{DirtyRect, Grid};

/// Cursor shape for rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorShape {
    /// Solid block cursor (default).
    #[default]
    Block,
    /// Underline cursor.
    Underline,
    /// Vertical bar cursor.
    Bar,
}

/// Cursor state passed to the renderer.
#[derive(Debug, Clone, Copy, Default)]
pub struct CursorState {
    /// Column (0-based).
    pub x: usize,
    /// Row (0-based).
    pub y: usize,
    /// Whether the cursor is visible.
    pub visible: bool,
    /// Cursor shape.
    pub shape: CursorShape,
}

impl CursorState {
    /// Create a visible cursor at the given position.
    pub fn new(x: usize, y: usize) -> Self {
        Self {
            x,
            y,
            visible: true,
            shape: CursorShape::Block,
        }
    }

    /// Create a hidden cursor.
    pub fn hidden() -> Self {
        Self {
            x: 0,
            y: 0,
            visible: false,
            shape: CursorShape::Block,
        }
    }
}

/// The core rendering trait.
///
/// Implementations consume a [`Grid`] and produce visual output.
/// The `dirty` parameter enables incremental rendering:
/// - `None` — Full redraw (first frame, resize, or theme change).
/// - `Some(rect)` — Only update cells within the dirty rectangle.
pub trait Renderer {
    /// Render the grid with cursor state.
    ///
    /// `dirty`: `None` = full redraw, `Some(rect)` = partial update.
    fn render(
        &mut self,
        grid: &Grid,
        cursor: &CursorState,
        dirty: Option<&DirtyRect>,
    );

    /// Resize the renderer viewport (in cell columns/rows).
    fn resize(&mut self, cols: usize, rows: usize);
}
