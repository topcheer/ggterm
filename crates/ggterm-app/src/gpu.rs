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
            alpha_mode: caps.alpha_modes[0],
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
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        surface.configure(&self.device, &self.config);
    }

    /// Create a GlyphonRenderer configured for this surface's format.
    pub fn create_renderer(&self, cols: usize, rows: usize) -> GlyphonRenderer {
        GlyphonRenderer::new(&self.device, &self.queue, self.surface_format, cols, rows)
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
                            r: 0.03,
                            g: 0.03,
                            b: 0.03,
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
    ggterm_render::CursorState {
        x,
        y,
        visible: app.cursor_visible(),
        shape: ggterm_render::CursorShape::Block,
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
