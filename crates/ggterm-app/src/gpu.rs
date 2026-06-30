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
                required_limits: wgpu::Limits::downlevel_defaults(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .block_on()
            .map_err(GpuError::RequestDevice)?;

        let caps = surface.get_capabilities(adapter);

        // Clamp surface dimensions to the GPU's maximum texture size.
        // wgpu will panic if we exceed this (e.g. on a Retina display with scaling).
        let max_dim = adapter.limits().max_texture_dimension_2d;
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
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto),
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
        bg_color: [f64; 3],
    ) -> Result<(), RenderFrameError> {
        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Timeout => {
                // Surface needs reconfiguration; skip this frame.
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => {
                return Err(RenderFrameError::Surface("surface lost or invalid".into()));
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
                            a: 1.0,
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
        bg_color: [f64; 3],
    ) -> Result<(), RenderFrameError> {
        if panes.is_empty() {
            return Ok(());
        }

        // Single-pane fast path: delegate to the simpler method.
        if panes.len() == 1 {
            renderer.set_viewport_offset(0.0, 0.0);
            return self.render_frame(surface, renderer, panes[0].grid, panes[0].cursor, bg_color);
        }

        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Timeout => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => {
                return Err(RenderFrameError::Surface("surface lost or invalid".into()));
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ggterm multi-pane encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ggterm multi-pane render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: bg_color[0],
                            g: bg_color[1],
                            b: bg_color[2],
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // Render each pane's grid at its offset.
            for spec in panes {
                let x = spec.offset_x;
                let y = spec.offset_y;
                let w = spec.width.max(1);
                let h = spec.height.max(1);

                // Set scissor to clip this pane's rendering area.
                pass.set_scissor_rect(x, y, w, h);
                renderer.set_viewport_offset(x as f32, y as f32);
                renderer
                    .render_pane_to_pass(
                        &self.device,
                        &self.queue,
                        spec.grid,
                        spec.cursor,
                        spec.needs_prepare,
                        &mut pass,
                    )
                    .map_err(RenderFrameError::Render)?;
            }

            // Reset scissor to full screen for overlay rendering.
            let full_w = renderer.resolution_width();
            let full_h = renderer.resolution_height();
            pass.set_scissor_rect(0, 0, full_w, full_h);

            // Draw overlays (tab bar, borders, settings, about) once.
            renderer
                .render_overlays_to_pass(&self.device, &mut pass)
                .map_err(RenderFrameError::Render)?;
        }

        self.queue.submit(std::iter::once(encoder.finish()));
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
    /// P21-D: Whether to re-prepare glyphon buffers (true when grid changed).
    pub needs_prepare: bool,
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
    ggterm_render::CursorState {
        x,
        y,
        visible: app.cursor_visible(),
        shape,
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
