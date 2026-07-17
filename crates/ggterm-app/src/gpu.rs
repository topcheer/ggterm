//! wgpu GPU context: surface, device, queue initialization.
//!
//! This module encapsulates the wgpu boilerplate needed to create a
//! GPU surface from a winit window, request an adapter + device, and
//! configure the swap chain.

use ggterm_render_wgpu::GlyphonRenderer;

/// Wraps the wgpu device/queue and surface configuration needed for rendering.
///
/// The `Surface` itself is managed by the caller (typically `DesktopApp`)
/// because wgpu 29 surfaces have a lifetime tied to the window.
pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub surface_format: wgpu::TextureFormat,
}

impl GpuContext {
    /// Initialize device + queue + surface configuration from an existing
    /// surface and adapter.
    ///
    /// Uses `pollster::block_on` for the async device request.
    pub fn from_surface(
        surface: &wgpu::Surface,
        adapter: &wgpu::Adapter,
        width: u32,
        height: u32,
    ) -> Result<Self, GpuError> {
        use pollster::FutureExt as _;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("ggterm device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .block_on()
            .map_err(GpuError::RequestDevice)?;

        let caps = surface.get_capabilities(adapter);

        // Clamp surface dimensions to the device's maximum texture size.
        // Must use device limits (not adapter) since the device was created
        // with specific required_limits. wgpu panics if we exceed device limits.
        let max_dim = device.limits().max_texture_dimension_2d;
        let width = width.min(max_dim).max(1);
        let height = height.min(max_dim).max(1);

        // Prefer an sRGB format for correct color rendering.
        let surface_format = caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(wgpu::TextureFormat::Bgra8Unorm);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps
                .alpha_modes
                .iter()
                .find(|&&mode| mode == wgpu::CompositeAlphaMode::PostMultiplied)
                .copied()
                .unwrap_or_else(|| {
                    caps.alpha_modes
                        .first()
                        .copied()
                        .unwrap_or(wgpu::CompositeAlphaMode::Auto)
                }),
            view_formats: vec![],
        };

        surface.configure(&device, &config);

        Ok(Self {
            device,
            queue,
            config,
            surface_format,
        })
    }

    /// Reconfigure the surface after a window resize.
    pub fn resize(&mut self, surface: &wgpu::Surface, width: u32, height: u32) {
        // Clamp to GPU max texture size to prevent wgpu validation panic.
        let max_dim = self.device.limits().max_texture_dimension_2d;
        self.config.width = width.min(max_dim).max(1);
        self.config.height = height.min(max_dim).max(1);
        surface.configure(&self.device, &self.config);
    }

    /// Create a GlyphonRenderer configured for this surface's format.
    pub fn create_renderer(
        &self,
        surface_w: u32,
        surface_h: u32,
        scale_factor: f64,
    ) -> GlyphonRenderer {
        GlyphonRenderer::new(
            &self.device,
            &self.queue,
            self.surface_format,
            surface_w,
            surface_h,
            scale_factor,
        )
    }

    /// Render a single frame: clears, renders terminal grid, presents.
    ///
    /// In wgpu 29, `get_current_texture()` returns a `CurrentSurfaceTexture`
    /// enum rather than `Result`. We match on it to extract the texture.
    pub fn render_frame(
        &mut self,
        surface: &wgpu::Surface,
        renderer: &mut GlyphonRenderer,
        grid: &ggterm_core::Grid,
        cursor: &ggterm_render::CursorState,
        bg_color: [f64; 4],
    ) -> Result<(), RenderFrameError> {
        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Timeout => {
                // Surface needs reconfiguration; skip this frame.
                surface.configure(&self.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                // Surface was lost — reconfigure and skip this frame.
                // Next frame should succeed with the fresh surface.
                surface.configure(&self.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Occluded => {
                // Window is not visible (minimized/covered) — skip frame silently.
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(RenderFrameError::Surface("surface validation error".into()));
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ggterm encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ggterm render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: bg_color[0],
                            g: bg_color[1],
                            b: bg_color[2],
                            a: bg_color[3],
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            renderer
                .render_to_pass(&self.device, &self.queue, grid, cursor, &mut pass)
                .map_err(RenderFrameError::Render)?;
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();

        Ok(())
    }

    /// Render multiple panes into a single frame (P20-A).
    ///
    /// Each pane's grid is rendered at its pixel offset within the surface.
    /// After all panes, overlays are drawn at full-screen scope.
    pub fn render_multi_pane_frame(
        &mut self,
        surface: &wgpu::Surface,
        renderer: &mut GlyphonRenderer,
        panes: &[PaneRenderSpec],
        bg_color: [f64; 4],
    ) -> Result<(), RenderFrameError> {
        if panes.is_empty() {
            return Ok(());
        }

        // No fast-path fallback to render_frame() — always use the
        // multi-pane path for consistent overlay text rendering.
        // render_frame() uses render_to_pass() which merges overlay text
        // with grid text in a single text_renderer.prepare() call, while
        // the multi-pane path renders them separately. This subtle
        // difference causes visible font inconsistencies between
        // single-pane and multi-pane modes.

        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Timeout => {
                surface.configure(&self.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                surface.configure(&self.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(RenderFrameError::Surface("surface validation error".into()));
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Surface extent — scissor rects must not exceed this.
        let surf_w = self.config.width;
        let surf_h = self.config.height;

        // CRITICAL: Each pane must be rendered in a separate command encoder +
        // queue.submit(). All panes share the same glyphon TextRenderer vertex
        // buffers. If we prepare pane B's text (which calls queue.write_buffer)
        // in the same submit as pane A's draw, the GPU coalesces all
        // write_buffer calls before any draw commands — so pane B's text data
        // overwrites pane A's before A is even drawn.
        //
        // Per-pane submit() creates synchronization points: the GPU finishes
        // pane A's draw before pane B's write_buffer executes.
        for (i, spec) in panes.iter().enumerate() {
            // Clamp to surface extent.
            let x = spec.offset_x.min(surf_w);
            let y = spec.offset_y.min(surf_h);
            let w = spec.width.max(1).min(surf_w.saturating_sub(x));
            let h = spec.height.max(1).min(surf_h.saturating_sub(y));

            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some(&format!("ggterm pane {i}")),
                });

            // First pane clears the surface; subsequent panes load existing content.
            let load = if i == 0 {
                wgpu::LoadOp::Clear(wgpu::Color {
                    r: bg_color[0],
                    g: bg_color[1],
                    b: bg_color[2],
                    a: bg_color[3],
                })
            } else {
                wgpu::LoadOp::Load
            };

            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(&format!("ggterm pane {i} pass")),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });

                pass.set_scissor_rect(x, y, w, h);
                renderer.set_viewport_offset(x as f32, y as f32);
                // Per-pane terminal state: each pane has its own reverse_video,
                // dynamic_fg, dynamic_bg from its terminal.
                renderer.set_reverse_video(spec.reverse_video);
                renderer.set_dynamic_fg(spec.dynamic_fg);
                renderer.set_dynamic_bg(spec.dynamic_bg);
                renderer.set_underline_color(spec.underline_color);
                renderer
                    .render_pane_to_pass(
                        &self.device,
                        &self.queue,
                        spec.grid,
                        spec.cursor,
                        // Always re-prepare: panes share text buffers, so
                        // each pane must rebuild its text before drawing.
                        true,
                        &mut pass,
                    )
                    .map_err(RenderFrameError::Render)?;
            }

            // Submit per pane — synchronization point ensures GPU ordering.
            self.queue.submit(std::iter::once(encoder.finish()));
        }

        // Overlays: final submit with LoadOp::Load to preserve pane content.
        {
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("ggterm overlays"),
                });

            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("ggterm overlay pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });

                // Full-screen scissor for overlays.
                pass.set_scissor_rect(0, 0, surf_w, surf_h);
                renderer
                    .render_overlays_to_pass(&self.device, &self.queue, &mut pass)
                    .map_err(RenderFrameError::Render)?;
            }

            self.queue.submit(std::iter::once(encoder.finish()));
        }

        frame.present();
        Ok(())
    }
}

/// Specification for rendering one pane (P20-A).
///
/// Each pane has its own grid + cursor and is rendered at a pixel offset
/// within the surface, clipped to the given width/height.
pub struct PaneRenderSpec<'a> {
    /// The terminal grid to render.
    pub grid: &'a ggterm_core::Grid,
    /// The cursor state for this pane.
    pub cursor: &'a ggterm_render::CursorState,
    /// X pixel offset within the surface.
    pub offset_x: u32,
    /// Y pixel offset within the surface.
    pub offset_y: u32,
    /// Width in pixels for clipping.
    pub width: u32,
    /// Height in pixels for clipping.
    pub height: u32,
    /// DECSCNM reverse video mode for this pane.
    pub reverse_video: bool,
    /// Dynamic foreground color override (OSC 10) for this pane.
    pub dynamic_fg: Option<(u8, u8, u8)>,
    /// Dynamic background color override (OSC 11) for this pane.
    pub dynamic_bg: Option<(u8, u8, u8)>,
    /// SGR 58 underline color override for this pane.
    pub underline_color: Option<(u8, u8, u8)>,
}

/// Create a wgpu Instance + Adapter + Surface from a window.
///
/// Returns `(instance, surface, adapter)` ready for `GpuContext::from_surface()`.
pub fn init_wgpu<W>(
    window: W,
) -> Result<(wgpu::Instance, wgpu::Surface<'static>, wgpu::Adapter), GpuError>
where
    W: raw_window_handle::HasWindowHandle
        + raw_window_handle::HasDisplayHandle
        + Send
        + Sync
        + 'static,
{
    use pollster::FutureExt as _;

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        flags: wgpu::InstanceFlags::default(),
        memory_budget_thresholds: Default::default(),
        backend_options: wgpu::BackendOptions::default(),
        display: None,
    });

    let surface = instance
        .create_surface(window)
        .map_err(GpuError::CreateSurface)?;

    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        })
        .block_on()
        .map_err(|_| GpuError::NoAdapter)?;

    Ok((instance, surface, adapter))
}

/// Build a [`CursorState`] from the app's cursor position and visibility.
pub fn cursor_state(app: &crate::App) -> ggterm_render::CursorState {
    let (x, y) = app.cursor();
    // Map terminal CursorStyle to renderer CursorShape.
    let shape = match app.terminal().cursor_style() {
        ggterm_core::CursorStyle::BlinkUnderline | ggterm_core::CursorStyle::SteadyUnderline => {
            ggterm_render::CursorShape::Underline
        }
        ggterm_core::CursorStyle::BlinkBar | ggterm_core::CursorStyle::SteadyBar => {
            ggterm_render::CursorShape::Bar
        }
        _ => ggterm_render::CursorShape::Block,
    };
    let cursor_color = app.terminal().dynamic_cursor().map(|c| match c {
        ggterm_core::Color::Rgb(r, g, b) => (*r, *g, *b),
        ggterm_core::Color::Indexed(idx) => {
            // Resolve 16-color palette index to RGB.
            match idx {
                0 => (0x00, 0x00, 0x00),
                1 => (0xcc, 0x00, 0x00),
                2 => (0x4e, 0x9a, 0x06),
                3 => (0xc4, 0xa0, 0x00),
                4 => (0x34, 0x65, 0xa4),
                5 => (0x75, 0x50, 0x7b),
                6 => (0x06, 0x98, 0x9a),
                7 => (0xd3, 0xd7, 0xcf),
                8 => (0x55, 0x57, 0x53),
                9 => (0xef, 0x29, 0x29),
                10 => (0x8a, 0xe2, 0x34),
                11 => (0xfc, 0xe9, 0x4f),
                12 => (0x73, 0x9f, 0xcf),
                13 => (0xad, 0x7f, 0xa8),
                14 => (0x34, 0xe2, 0xe2),
                _ => (0xee, 0xee, 0xec),
            }
        }
        ggterm_core::Color::Default => (200, 200, 200),
    });
    ggterm_render::CursorState {
        x,
        y,
        visible: app.cursor_visible(),
        shape,
        blink_alpha: 1.0, // P23-A: set by DesktopApp before render
        color: cursor_color,
        focused: true, // Set by DesktopApp render_frame from window_focused
    }
}

/// Errors during GPU context initialization.
#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    #[error("failed to create wgpu surface: {0}")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    #[error("no suitable GPU adapter found")]
    NoAdapter,
    #[error("failed to request GPU device: {0}")]
    RequestDevice(#[from] wgpu::RequestDeviceError),
}

/// Errors during frame rendering.
#[derive(Debug, thiserror::Error)]
pub enum RenderFrameError {
    #[error("surface error: {0}")]
    Surface(String),
    #[error("render error: {0}")]
    Render(#[from] ggterm_render_wgpu::RenderError),
}
