//! Desktop window: winit event loop + wgpu surface + GlyphonRenderer.
//!
//! This module ties together the full rendering stack:
//!
//! 1. **winit** creates the OS window and delivers keyboard/mouse/resize events.
//! 2. **wgpu** creates a GPU device + swap-chain surface backed by that window.
//! 3. **GlyphonRenderer** renders the terminal `Grid` into the surface texture.
//! 4. **PtySession** spawns the child shell; a reader thread pumps bytes into
//!    the main loop via an `mpsc` channel.
//!
//! ## Event flow
//!
//! ```text
//! PTY reader thread ──bytes──▶ mpsc channel ──▶ about_to_wait()
//!                                                    │
//!                                                    ▼
//!                                           app.pump()
//!                                           window.request_redraw()
//!                                                    │
//!                                                    ▼
//!                                           RedrawRequested
//!                                           gpu.render_frame()
//! ```

use std::sync::Arc;

use ggterm_core::pty::PtySession;
use ggterm_render_wgpu::GlyphonRenderer;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::Key;
use winit::window::{Window, WindowId};

use crate::app::{spawn_pty_reader, App};
use crate::event::{AppEvent, EventSender};
use crate::gpu::{cursor_state, init_wgpu, GpuContext};
use crate::input::InputEncoder;
use crate::keymap::map_winit_key;

// ══════════════════════════════════════════════════════════════════
//  DesktopConfig
// ══════════════════════════════════════════════════════════════════

/// Configuration for the desktop terminal window.
#[derive(Debug, Clone)]
pub struct DesktopConfig {
    /// Window title.
    pub title: String,
    /// Initial column count.
    pub cols: u16,
    /// Initial row count.
    pub rows: u16,
    /// Cell width in pixels.
    pub cell_width: f32,
    /// Cell height in pixels.
    pub cell_height: f32,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            title: "GGTerm".to_string(),
            cols: 80,
            rows: 24,
            cell_width: 8.0,
            cell_height: 16.0,
        }
    }
}

impl DesktopConfig {
    /// Set the window title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Set initial terminal dimensions.
    pub fn with_size(mut self, cols: u16, rows: u16) -> Self {
        self.cols = cols;
        self.rows = rows;
        self
    }

    /// Set cell dimensions in pixels.
    pub fn with_cell_size(mut self, w: f32, h: f32) -> Self {
        self.cell_width = w;
        self.cell_height = h;
        self
    }

    /// Window pixel width = cols * cell_width.
    pub fn window_width(&self) -> u32 {
        (self.cols as f32 * self.cell_width).round() as u32
    }

    /// Window pixel height = rows * cell_height.
    pub fn window_height(&self) -> u32 {
        (self.rows as f32 * self.cell_height).round() as u32
    }
}

// ══════════════════════════════════════════════════════════════════
//  DesktopApp — implements winit ApplicationHandler
// ══════════════════════════════════════════════════════════════════

/// Current key modifiers state (updated by ModifiersChanged events).
#[derive(Debug, Clone, Copy, Default)]
pub struct ModsState {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl From<ModsState> for crate::input::KeyModifiers {
    fn from(m: ModsState) -> Self {
        crate::input::KeyModifiers {
            shift: m.shift,
            ctrl: m.ctrl,
            alt: m.alt,
        }
    }
}

/// Desktop terminal application.
///
/// Implements winit's `ApplicationHandler` trait to receive OS events.
/// GPU resources (surface, device, renderer) are lazily initialized in
/// `resumed()`.
pub struct DesktopApp {
    /// Terminal state (Parser + Terminal + Grid).
    app: App,
    /// PTY session (owned, kept alive for the lifetime of the app).
    pty: Option<PtySession>,
    /// Configuration.
    config: DesktopConfig,
    /// Current key modifiers state.
    mods: ModsState,

    // ── GPU resources (initialized in resumed()) ──
    /// The winit window. Wrapped in Arc so we can pass a clone to wgpu
    /// while keeping a reference for redraw requests.
    window: Option<Arc<Window>>,
    /// wgpu surface (raw window handle, 'static lifetime).
    surface: Option<wgpu::Surface<'static>>,
    /// GPU device/queue/surface config.
    gpu: Option<GpuContext>,
    /// Glyphon text renderer.
    renderer: Option<GlyphonRenderer>,

    // ── PTY communication ──
    /// Event sender (cloned for the PTY reader thread).
    _event_tx: EventSender,
    /// Input encoder for keyboard → PTY bytes.
    encoder: InputEncoder,
}

impl DesktopApp {
    /// Launch the desktop terminal: create PTY, wire up the reader thread,
    /// and block on the winit event loop.
    pub fn run(config: DesktopConfig) -> Result<(), Box<dyn std::error::Error>> {
        let _ = env_logger::try_init();

        let (cols, rows) = (config.cols, config.rows);

        // 1. Create PTY session.
        let pty = PtySession::open_with_shell(cols, rows, None)?;

        // 2. Create the headless App (Terminal + Parser + InputEncoder).
        let (mut app, event_tx) = App::new(cols as usize, rows as usize);

        // 3. Spawn PTY reader thread → pump bytes into event channel.
        let reader = pty.try_clone_reader()?;
        spawn_pty_reader(reader, event_tx.clone());

        // 4. Mark app as running.
        app.start();

        // 5. Build DesktopApp.
        let mut desktop = DesktopApp {
            app,
            pty: Some(pty),
            config,
            mods: ModsState::default(),
            window: None,
            surface: None,
            gpu: None,
            renderer: None,
            _event_tx: event_tx,
            encoder: InputEncoder::new(),
        };

        // 6. Create winit event loop and run.
        let event_loop = EventLoop::new()?;
        event_loop.run_app(&mut desktop)?;

        Ok(())
    }

    // ── Helpers ──

    /// Write encoded keyboard bytes to the PTY.
    fn write_to_pty(&mut self, bytes: &[u8]) {
        if let Some(ref mut pty) = self.pty {
            if let Err(e) = pty.write(bytes) {
                log::warn!("PTY write error: {e}");
            }
        }
    }

    /// Handle window resize: recalculate cols/rows, resize Terminal + PTY + GPU.
    fn handle_resize(&mut self, width: u32, height: u32) {
        let new_cols = ((width as f32 / self.config.cell_width) as u16).max(1);
        let new_rows = ((height as f32 / self.config.cell_height) as u16).max(1);

        log::debug!("Resize: {}x{}px → {}x{} cells", width, height, new_cols, new_rows);

        self.app
            .handle_event(AppEvent::Resize { cols: new_cols, rows: new_rows });

        if let Some(ref mut pty) = self.pty {
            if let Err(e) = pty.resize(new_cols, new_rows) {
                log::warn!("PTY resize failed: {e}");
            }
        }

        if let (Some(gpu), Some(surface)) = (&mut self.gpu, &self.surface) {
            gpu.resize(surface, width.max(1), height.max(1));
        }

        // Recreate renderer with new dimensions.
        if let Some(gpu) = &self.gpu {
            self.renderer = Some(gpu.create_renderer(new_cols as usize, new_rows as usize));
        }
    }

    /// Render one frame.
    fn render_frame(&mut self) {
        let (gpu, surface, renderer) = match (&mut self.gpu, &self.surface, &mut self.renderer) {
            (Some(g), Some(s), Some(r)) => (g, s, r),
            _ => return,
        };

        let grid = self.app.grid();
        let cursor = cursor_state(&self.app);

        if let Err(e) = gpu.render_frame(surface, renderer, grid, &cursor) {
            log::error!("Render error: {e}");
        }
    }

    /// Handle a winit key event using the existing keymap module.
    fn handle_keyboard_input(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }

        // Extract logical text for printable character support.
        let logical_text: Option<&str> = match &event.logical_key {
            Key::Character(s) => Some(s.as_ref()),
            _ => None,
        };

        // Use the shared keymap module for mapping.
        let mods: crate::input::KeyModifiers = self.mods.into();
        if let Some(input_key) = map_winit_key(&event.physical_key, logical_text, &mods) {
            let bytes = self.encoder.encode(&input_key);
            if !bytes.is_empty() {
                self.write_to_pty(&bytes);
            }
        }
    }
}

// ══════════════════════════════════════════════════════════════════
//  ApplicationHandler implementation
// ══════════════════════════════════════════════════════════════════

impl ApplicationHandler for DesktopApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return; // Already initialized.
        }

        log::info!("Initializing window + GPU");

        // 1. Create the window.
        let attrs = Window::default_attributes()
            .with_title(&self.config.title)
            .with_inner_size(winit::dpi::PhysicalSize::new(
                self.config.window_width(),
                self.config.window_height(),
            ));

        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("Failed to create window: {e}");
                event_loop.exit();
                return;
            }
        };

        // 2. Initialize wgpu (Instance + Surface + Adapter).
        //    Pass Arc::clone so we keep a reference for redraw requests.
        let (_instance, surface, adapter) = match init_wgpu(Arc::clone(&window)) {
            Ok(result) => result,
            Err(e) => {
                log::error!("Failed to init wgpu: {e}");
                event_loop.exit();
                return;
            }
        };

        // 3. Create GPU context.
        let gpu = match GpuContext::from_surface(
            &surface,
            &adapter,
            self.config.window_width().max(1),
            self.config.window_height().max(1),
        ) {
            Ok(g) => g,
            Err(e) => {
                log::error!("Failed to create GPU context: {e}");
                event_loop.exit();
                return;
            }
        };

        // 4. Create GlyphonRenderer.
        let renderer =
            gpu.create_renderer(self.config.cols as usize, self.config.rows as usize);

        self.window = Some(window);
        self.surface = Some(surface);
        self.gpu = Some(gpu);
        self.renderer = Some(renderer);

        log::info!("Window + GPU initialized");
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                log::info!("CloseRequested, exiting");
                event_loop.exit();
            }

            WindowEvent::RedrawRequested => {
                // Pump PTY events before rendering.
                self.app.pump();

                // Check exit.
                if !self.app.is_running() {
                    event_loop.exit();
                    return;
                }

                self.render_frame();

                // Check PTY exit.
                if let Some(ref mut pty) = self.pty {
                    if !pty.is_alive() {
                        log::info!("PTY exited");
                        event_loop.exit();
                        return;
                    }
                }
            }

            WindowEvent::Resized(size) => {
                self.handle_resize(size.width, size.height);
            }

            WindowEvent::KeyboardInput { event, .. } => {
                self.handle_keyboard_input(&event);
            }

            WindowEvent::ModifiersChanged(mods) => {
                self.mods.shift = mods.state().shift_key();
                self.mods.ctrl = mods.state().control_key();
                self.mods.alt = mods.state().alt_key();
            }

            WindowEvent::Focused(focused) => {
                if focused {
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                }
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Pump PTY events.
        self.app.pump();

        // Check exit.
        if !self.app.is_running() {
            event_loop.exit();
            return;
        }

        if let Some(ref mut pty) = self.pty {
            if !pty.is_alive() {
                event_loop.exit();
                return;
            }
        }

        // Always request redraw to keep the render loop alive.
        // The GPU only draws when there's new content (PTY output is event-driven).
        // But for a terminal we want continuous rendering to show the blinking cursor.
        if let Some(ref window) = self.window {
            window.request_redraw();
        }
    }
}

// ══════════════════════════════════════════════════════════════════
//  Tests
// ══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = DesktopConfig::default();
        assert_eq!(cfg.title, "GGTerm");
        assert_eq!(cfg.cols, 80);
        assert_eq!(cfg.rows, 24);
        assert_eq!(cfg.cell_width, 8.0);
        assert_eq!(cfg.cell_height, 16.0);
    }

    #[test]
    fn test_config_builder() {
        let cfg = DesktopConfig::default()
            .with_title("My Terminal")
            .with_size(120, 40)
            .with_cell_size(7.5, 15.5);

        assert_eq!(cfg.title, "My Terminal");
        assert_eq!(cfg.cols, 120);
        assert_eq!(cfg.rows, 40);
        assert_eq!(cfg.cell_width, 7.5);
        assert_eq!(cfg.cell_height, 15.5);
    }

    #[test]
    fn test_window_dimensions_default() {
        let cfg = DesktopConfig::default();
        assert_eq!(cfg.window_width(), 640); // 80 * 8
        assert_eq!(cfg.window_height(), 384); // 24 * 16
    }

    #[test]
    fn test_window_dimensions_custom() {
        let cfg = DesktopConfig::default()
            .with_size(100, 30)
            .with_cell_size(7.5, 15.5);

        assert_eq!(cfg.window_width(), 750); // 100 * 7.5
        assert_eq!(cfg.window_height(), 465); // 30 * 15.5
    }

    #[test]
    fn test_mods_state_default() {
        let mods = ModsState::default();
        assert!(!mods.shift);
        assert!(!mods.ctrl);
        assert!(!mods.alt);
    }

    #[test]
    fn test_mods_state_to_key_modifiers() {
        let mods = ModsState {
            shift: true,
            ctrl: false,
            alt: true,
        };
        let km: crate::input::KeyModifiers = mods.into();
        assert!(km.shift);
        assert!(!km.ctrl);
        assert!(km.alt);
    }
}
