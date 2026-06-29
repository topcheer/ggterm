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

/// Get the default shell path as a String.
fn default_shell_string() -> String {
    ggterm_core::pty::default_shell()
}
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::app::{App, spawn_pty_reader};
use crate::config::ConfigManager;
use crate::event::{AppEvent, EventSender};
use crate::gpu::{GpuContext, cursor_state, init_wgpu};
use crate::input::InputEncoder;
use crate::keymap::map_winit_key;

// ══════════════════════════════════════════════════════════════════
//  P9-H: Resize constants + computation helpers
// ══════════════════════════════════════════════════════════════════

/// Minimum terminal dimensions in cells.
/// Prevents the window from shrinking to an unusable size.
pub const MIN_COLS: u16 = 10;
pub const MIN_ROWS: u16 = 3;

/// Resize debounce interval (milliseconds).
/// During a window drag-resize, winit fires many `Resized` events.
/// We defer the actual Terminal/PTY resize until 100ms after the last event.
pub const RESIZE_DEBOUNCE_MS: u64 = 100;

/// Compute terminal cell dimensions (cols, rows) from pixel dimensions.
///
/// `width`/`height` are the window inner size in physical pixels.
/// `cell_width`/`cell_height` are the pixel dimensions of a single cell.
/// The result is clamped to at least `MIN_COLS` x `MIN_ROWS`.
pub fn compute_cell_dimensions(
    width: u32,
    height: u32,
    cell_width: f32,
    cell_height: f32,
) -> (u16, u16) {
    let cols = ((width as f32 / cell_width) as u16).max(MIN_COLS);
    let rows = ((height as f32 / cell_height) as u16).max(MIN_ROWS);
    (cols, rows)
}

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
    /// Shell program path. `None` = auto-detect.
    pub shell: Option<String>,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            title: "GGTerm".to_string(),
            cols: 80,
            rows: 24,
            cell_width: 8.0,
            cell_height: 16.0,
            shell: None,
        }
    }
}

impl DesktopConfig {
    /// Set the window title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Set the shell program path.
    pub fn with_shell(mut self, shell: impl Into<String>) -> Self {
        self.shell = Some(shell.into());
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
#[allow(dead_code)] // P9-D mouse fields/methods pending integration
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

    // ── Config hot-reload (config-watch feature) ──
    /// Config manager with optional file-system watcher.
    #[cfg(feature = "config-watch")]
    config_mgr: Option<ConfigManager>,

    // ── Dynamic window title (OSC 0/2) ──
    /// Last known terminal title (to detect changes).
    last_title: String,

    // ── Mouse support ──
    /// Current text selection state.
    selection: crate::mouse::MouseSelection,
    /// Last known cursor position in pixels (for mouse wheel / drag).
    cursor_pos: (f64, f64),
    /// Mouse button currently held (for drag tracking).
    button_held: Option<crate::mouse::MouseButton>,

    // ── Resize debouncing (P9-H) ──
    /// Pending resize dimensions (stored during drag, applied after debounce).
    pending_resize: Option<(u32, u32)>,
    /// Instant of the last resize event (for debounce timing).
    last_resize_time: Option<std::time::Instant>,
}

impl DesktopApp {
    /// Launch the desktop terminal: create PTY, wire up the reader thread,
    /// and block on the winit event loop.
    pub fn run(config: DesktopConfig) -> Result<(), Box<dyn std::error::Error>> {
        let _ = env_logger::try_init();

        // ── Step 1: Load configuration from ~/.ggterm/config.toml ──
        let config_mgr = match ConfigManager::load_default() {
            Ok(mgr) => {
                log::info!(
                    "Config loaded from ~/.ggterm/config.toml (theme={}, scrollback={})",
                    mgr.config().appearance.theme,
                    mgr.config().terminal.scrollback_lines
                );
                Some(mgr)
            }
            Err(e) => {
                log::info!("No config file found, using defaults: {e}");
                None
            }
        };

        // ── Step 2: Merge config values into DesktopConfig (CLI overrides win) ──
        let (cols, rows) = (config.cols, config.rows);
        let effective_shell = config
            .shell
            .clone()
            .or_else(|| {
                config_mgr
                    .as_ref()
                    .map(|m| m.config().terminal.shell.clone())
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(default_shell_string);

        let mut desktop_config = config;
        if let Some(ref mgr) = config_mgr {
            let cfg = mgr.config();
            if desktop_config.cell_width == 8.0 {
                desktop_config.cell_width = cfg.appearance.cell_width as f32;
            }
            if desktop_config.cell_height == 16.0 {
                desktop_config.cell_height = cfg.appearance.cell_height as f32;
            }
        }
        desktop_config.shell = Some(effective_shell.clone());

        // ── Step 3: Prepare shell integration (OSC 133 auto-injection) ──
        let shell_integration =
            crate::shell_integration::ShellIntegrationConfig::prepare(&effective_shell);
        let (program, spawn_args) = shell_integration.spawn_args();
        let env_vars = shell_integration.env_vars();

        // ── Step 4: Create PTY session with shell integration ──
        let pty = PtySession::open_advanced(cols, rows, Some(&program), &spawn_args, &env_vars)?;

        // ── Step 5: Create App and apply config (theme + scrollback) ──
        let (mut app, event_tx) = App::new(cols as usize, rows as usize);
        if let Some(ref mgr) = config_mgr {
            let cfg = mgr.config();
            app.theme_manager().set_by_name(&cfg.appearance.theme);
            app.terminal_mut()
                .grid_mut()
                .set_scrollback(cfg.terminal.scrollback_lines);
        }

        // ── Step 6: Spawn PTY reader thread ──
        let reader = pty.try_clone_reader()?;
        spawn_pty_reader(reader, event_tx.clone());
        app.start();

        // ── Step 7: Build DesktopApp ──
        let mut desktop = DesktopApp {
            app,
            pty: Some(pty),
            config: desktop_config,
            mods: ModsState::default(),
            window: None,
            surface: None,
            gpu: None,
            renderer: None,
            _event_tx: event_tx,
            encoder: InputEncoder::new(),
            #[cfg(feature = "config-watch")]
            config_mgr: None,
            last_title: String::new(),
            selection: crate::mouse::MouseSelection::default(),
            cursor_pos: (0.0, 0.0),
            button_held: None,
            pending_resize: None,
            last_resize_time: None,
        };

        // ── Step 8: Start config file watcher (if config-watch feature) ──
        #[cfg(feature = "config-watch")]
        {
            if let Some(mut mgr) = config_mgr {
                if let Err(e) = mgr.watch() {
                    log::warn!("Config watch failed: {e}");
                }
                desktop.config_mgr = Some(mgr);
            }
        }

        // ── Step 9: Create winit event loop and run ──
        let event_loop = EventLoop::new()?;
        event_loop.run_app(&mut desktop)?;

        Ok(())
    }

    // ── Helpers ──

    /// Write encoded keyboard bytes to the PTY.
    fn write_to_pty(&mut self, bytes: &[u8]) {
        if let Some(ref mut pty) = self.pty
            && let Err(e) = pty.write(bytes)
        {
            log::warn!("PTY write error: {e}");
        }
    }

    /// Handle window resize: store pending dimensions for debounced apply.
    ///
    /// During a drag-resize, winit fires many `Resized` events. We store the
    /// latest dimensions and defer the actual Terminal/PTY resize until the
    /// user stops dragging (100ms debounce). See `apply_pending_resize()`.
    fn handle_resize(&mut self, width: u32, height: u32) {
        self.pending_resize = Some((width.max(1), height.max(1)));
        self.last_resize_time = Some(std::time::Instant::now());
    }

    /// Apply a pending resize if the debounce interval (100ms) has elapsed.
    ///
    /// Called from `about_to_wait()`. Returns `true` if a resize was applied.
    fn apply_pending_resize(&mut self) -> bool {
        let Some((width, height)) = self.pending_resize else {
            return false;
        };
        let Some(last) = self.last_resize_time else {
            return false;
        };

        // Check if enough time has passed since the last resize event.
        if std::time::Instant::now().duration_since(last) < std::time::Duration::from_millis(100) {
            return false; // Not enough time elapsed — wait.
        }

        // Clear pending state.
        self.pending_resize = None;
        self.last_resize_time = None;

        // Compute new cell dimensions with minimum size (10x3).
        let new_cols = ((width as f32 / self.config.cell_width) as u16).max(10);
        let new_rows = ((height as f32 / self.config.cell_height) as u16).max(3);

        log::debug!(
            "Resize: {}x{}px → {}x{} cells",
            width,
            height,
            new_cols,
            new_rows
        );

        self.app.handle_event(AppEvent::Resize {
            cols: new_cols,
            rows: new_rows,
        });

        if let Some(ref mut pty) = self.pty
            && let Err(e) = pty.resize(new_cols, new_rows)
        {
            log::warn!("PTY resize failed: {e}");
        }

        if let (Some(gpu), Some(surface)) = (&mut self.gpu, &self.surface) {
            gpu.resize(surface, width.max(1), height.max(1));
        }

        // Recreate renderer with new dimensions.
        if let Some(gpu) = &self.gpu {
            self.renderer = Some(gpu.create_renderer(new_cols as usize, new_rows as usize));
        }

        true
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

        // Phase 8-D: Ctrl+Shift+Up/Down for command block navigation
        if self.mods.ctrl
            && self.mods.shift
            && let PhysicalKey::Code(code) = &event.physical_key
        {
            match code {
                KeyCode::ArrowUp => {
                    self.app.handle_event(AppEvent::PrevCommandBlock);
                    return;
                }
                KeyCode::ArrowDown => {
                    self.app.handle_event(AppEvent::NextCommandBlock);
                    return;
                }
                _ => {}
            }
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

    // ── Mouse handling (P9-D, pending integration) ────────────────

    /// Convert pixel position to terminal cell coordinates.
    #[allow(dead_code)]
    fn pixel_to_cell_pos(&self) -> (u16, u16) {
        crate::mouse::pixel_to_cell(
            self.cursor_pos.0,
            self.cursor_pos.1,
            self.config.cell_width as f64,
            self.config.cell_height as f64,
        )
    }

    /// Handle winit MouseInput events (button press/release).
    #[allow(dead_code)]
    fn handle_mouse_input(&mut self, state: ElementState, button: winit::event::MouseButton) {
        let mouse_button = match button {
            winit::event::MouseButton::Left => crate::mouse::MouseButton::Left,
            winit::event::MouseButton::Right => crate::mouse::MouseButton::Right,
            winit::event::MouseButton::Middle => crate::mouse::MouseButton::Middle,
            winit::event::MouseButton::Back => crate::mouse::MouseButton::Other(8),
            winit::event::MouseButton::Forward => crate::mouse::MouseButton::Other(16),
            winit::event::MouseButton::Other(n) => crate::mouse::MouseButton::Other(n as u8),
        };

        let (col, row) = self.pixel_to_cell_pos();
        let mods = crate::mouse::MouseModifiers {
            shift: self.mods.shift,
            ctrl: self.mods.ctrl,
            alt: self.mods.alt,
        };

        let term = self.app.terminal();

        // Check if mouse tracking is active.
        if term.mouse_tracking_enabled() {
            let mouse_ev = crate::mouse::MouseEvent {
                button: mouse_button,
                x: col,
                y: row,
                mods,
            };

            let sgr = term.mouse_sgr_enabled();
            let urxvt = term.mouse_urxvt_enabled();

            match state {
                ElementState::Pressed => {
                    self.button_held = Some(mouse_button);
                    if let Some(bytes) =
                        crate::mouse::encode_mouse_event(&mouse_ev, sgr, urxvt, true)
                    {
                        self.write_to_pty(&bytes);
                    }
                }
                ElementState::Released => {
                    self.button_held = None;
                    if let Some(bytes) =
                        crate::mouse::encode_mouse_event(&mouse_ev, sgr, urxvt, false)
                    {
                        self.write_to_pty(&bytes);
                    }
                }
            }
            return;
        }

        // Mouse tracking is OFF — handle selection locally.
        match (mouse_button, state) {
            (crate::mouse::MouseButton::Left, ElementState::Pressed) => {
                self.button_held = Some(mouse_button);
                self.selection.start(col, row);
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            (crate::mouse::MouseButton::Left, ElementState::Released) => {
                self.button_held = None;
                self.selection.finish();
                // Copy selection to clipboard if active.
                if self.selection.is_active() {
                    self.copy_selection_to_clipboard();
                }
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    /// Handle cursor motion — extend selection or report mouse motion.
    fn handle_cursor_moved(&mut self) {
        let (col, row) = self.pixel_to_cell_pos();

        let term = self.app.terminal();
        let any_event = term.mouse_any_event_enabled();
        let button_event = term.mouse_button_event_enabled();

        // If mouse motion tracking is on, report motion.
        if any_event || button_event {
            let held = self.button_held.is_some();
            if crate::mouse::should_report_motion(any_event, button_event, held) {
                let button = self.button_held.unwrap_or(crate::mouse::MouseButton::Left);
                let mods = crate::mouse::MouseModifiers {
                    shift: self.mods.shift,
                    ctrl: self.mods.ctrl,
                    alt: self.mods.alt,
                };
                let mouse_ev = crate::mouse::MouseEvent {
                    button,
                    x: col,
                    y: row,
                    mods,
                };

                if term.mouse_sgr_enabled() {
                    let bytes = crate::mouse::encode_sgr_motion(&mouse_ev, held);
                    self.write_to_pty(bytes.as_bytes());
                }
                return;
            }
        }

        // Extend selection while dragging.
        if self.selection.dragging {
            self.selection.extend(col, row);
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
        }
    }

    /// Handle mouse wheel events — scroll scrollback or report to PTY.
    fn handle_mouse_wheel(&mut self, delta: winit::event::MouseScrollDelta) {
        let (col, row) = self.pixel_to_cell_pos();
        let mods = crate::mouse::MouseModifiers {
            shift: self.mods.shift,
            ctrl: self.mods.ctrl,
            alt: self.mods.alt,
        };

        let term = self.app.terminal();

        // When mouse tracking is on, send wheel as button events.
        if term.mouse_tracking_enabled() {
            let sgr = term.mouse_sgr_enabled();
            let urxvt = term.mouse_urxvt_enabled();

            let (dx, dy) = match delta {
                winit::event::MouseScrollDelta::LineDelta(x, y) => (x as i32, -(y as i32)),
                winit::event::MouseScrollDelta::PixelDelta(pos) => {
                    let x = (pos.x as f32 / 8.0).round() as i32;
                    let y = (pos.y as f32 / 16.0).round() as i32;
                    (x, -y)
                }
            };

            // Each scroll line = one wheel event.
            for _ in 0..dy.abs() {
                let button = if dy > 0 {
                    crate::mouse::MouseButton::WheelUp
                } else {
                    crate::mouse::MouseButton::WheelDown
                };
                let ev = crate::mouse::MouseEvent {
                    button,
                    x: col,
                    y: row,
                    mods,
                };
                if let Some(bytes) = crate::mouse::encode_mouse_event(&ev, sgr, urxvt, true) {
                    self.write_to_pty(&bytes);
                }
            }

            // Horizontal scroll.
            for _ in 0..dx.abs() {
                let button = if dx > 0 {
                    crate::mouse::MouseButton::WheelRight
                } else {
                    crate::mouse::MouseButton::WheelLeft
                };
                let ev = crate::mouse::MouseEvent {
                    button,
                    x: col,
                    y: row,
                    mods,
                };
                if let Some(bytes) = crate::mouse::encode_mouse_event(&ev, sgr, urxvt, true) {
                    self.write_to_pty(&bytes);
                }
            }
            return;
        }

        // Mouse tracking OFF — scroll the scrollback buffer.
        let (lines, direction) = match delta {
            winit::event::MouseScrollDelta::LineDelta(_x, y) => (y.abs() as usize, y > 0.0),
            winit::event::MouseScrollDelta::PixelDelta(pos) => {
                let lines = (pos.y.abs() as f32 / 16.0).round() as usize;
                (lines.max(1), pos.y < 0.0) // Natural scroll: pixel up = scroll up
            }
        };

        let grid = self.app.terminal_mut().grid_mut();
        if direction {
            grid.scroll_up_viewport(lines);
        } else {
            grid.scroll_down_viewport(lines);
        }

        if let Some(ref window) = self.window {
            window.request_redraw();
        }
    }

    /// Copy the current text selection to the clipboard.
    ///
    /// Extracts text from the grid between selection start and end.
    fn copy_selection_to_clipboard(&self) {
        let Some(((sx, sy), (ex, ey))) = self.selection.normalized() else {
            return;
        };

        let grid = self.app.grid();
        let mut text = String::new();

        if sy == ey {
            // Single-line selection.
            for x in sx..=ex {
                if let Some(cell) = grid.cell(x as usize, sy as usize) {
                    text.push(cell.ch);
                }
            }
        } else {
            // Multi-line selection.
            // First line: from sx to end of row.
            let width = grid.width();
            for x in sx..width as u16 {
                if let Some(cell) = grid.cell(x as usize, sy as usize) {
                    text.push(cell.ch);
                }
            }
            text.push('\n');
            // Middle lines: full rows.
            for y in (sy + 1)..ey {
                for x in 0..width as u16 {
                    if let Some(cell) = grid.cell(x as usize, y as usize) {
                        text.push(cell.ch);
                    }
                }
                text.push('\n');
            }
            // Last line: from start of row to ex.
            for x in 0..=ex {
                if let Some(cell) = grid.cell(x as usize, ey as usize) {
                    text.push(cell.ch);
                }
            }
        }

        // Trim trailing whitespace per line.
        let text = text
            .lines()
            .map(|l| l.trim_end())
            .collect::<Vec<_>>()
            .join("\n");

        if !text.is_empty() {
            log::debug!("Clipboard copy: {} chars", text.len());
            #[cfg(target_os = "macos")]
            {
                use std::process::Command;
                let _ = Command::new("pbcopy")
                    .stdin(std::process::Stdio::piped())
                    .spawn()
                    .and_then(|mut child| {
                        use std::io::Write;
                        child.stdin.take().unwrap().write_all(text.as_bytes())?;
                        child.wait()
                    });
            }
            #[cfg(not(target_os = "macos"))]
            {
                log::debug!("Clipboard copy not implemented on this platform");
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
        let renderer = gpu.create_renderer(self.config.cols as usize, self.config.rows as usize);

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

                // Update window title if the terminal title changed (OSC 0/2).
                let title = self.app.terminal().title().to_string();
                if title != self.last_title
                    && let Some(ref window) = self.window
                {
                    let display = if title.is_empty() {
                        format!("GGTerm {}", env!("CARGO_PKG_VERSION"))
                    } else {
                        title.clone()
                    };
                    window.set_title(&display);
                    self.last_title = title;
                }

                // Check PTY exit.,
                if let Some(ref mut pty) = self.pty
                    && !pty.is_alive()
                {
                    log::info!("PTY exited");
                    event_loop.exit();
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
                if focused && let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }

            // P9-D mouse handlers
            WindowEvent::MouseInput { state, button, .. } => {
                self.handle_mouse_input(state, button);
            }
            WindowEvent::CursorMoved { position: pos, .. } => {
                self.cursor_pos = (pos.x, pos.y);
                self.handle_cursor_moved();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.handle_mouse_wheel(delta);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Pump PTY events.
        self.app.pump();

        // Apply deferred resize if debounce interval has elapsed.
        self.apply_pending_resize();

        // Poll config watcher for hot-reload.
        #[cfg(feature = "config-watch")]
        if let Some(ref mut mgr) = self.config_mgr {
            match mgr.poll_reload() {
                Ok(true) => {
                    let cfg = mgr.config();
                    log::info!(
                        "Config reloaded: theme={}, scrollback={}",
                        cfg.appearance.theme,
                        cfg.terminal.scrollback_lines
                    );
                }
                Ok(false) => {}
                Err(e) => log::warn!("Config reload error: {e}"),
            }
        }

        // Check exit.
        if !self.app.is_running() {
            event_loop.exit();
            return;
        }

        if let Some(ref mut pty) = self.pty
            && !pty.is_alive()
        {
            event_loop.exit();
        }

        // Always request redraw to keep the render loop alive.
        // The GPU only draws when there's new content (PTY output is event-driven).
        // But for a terminal we want continuous rendering to show the blinking cursor.
        if let Some(ref window) = self.window {
            window.request_redraw();
        }

        // If we have a pending (debounced) resize, keep polling so we apply
        // it after the 100ms window. request_redraw above already keeps the
        // event loop spinning in winit's Poll mode.
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

    #[test]
    fn test_desktop_config_with_shell() {
        let cfg = DesktopConfig::default().with_shell("/bin/bash");
        assert_eq!(cfg.shell.as_deref(), Some("/bin/bash"));
    }

    #[test]
    fn test_desktop_config_shell_default_none() {
        let cfg = DesktopConfig::default();
        assert!(cfg.shell.is_none(), "shell should default to None");
    }

    #[test]
    fn test_config_manager_load_default_fails_gracefully() {
        // ConfigManager::load_default() should return Err when no config file exists,
        // not panic. This verifies the graceful degradation behavior.
        // (On a dev machine it might find a real config, which is also fine.)
        let _ = ConfigManager::load_default();
    }

    #[test]
    fn test_cli_shell_overrides_config() {
        // Simulate the merge logic: CLI shell wins over config shell
        let cli_shell: Option<String> = Some("/bin/zsh".to_string());
        let config_shell: Option<String> = Some("/bin/bash".to_string());

        let effective = cli_shell
            .clone()
            .or(config_shell)
            .unwrap_or_else(default_shell_string);

        assert_eq!(effective, "/bin/zsh", "CLI shell should take precedence");
    }

    #[test]
    fn test_config_shell_used_when_no_cli() {
        // Simulate: CLI is None, config provides the shell
        let cli_shell: Option<String> = None;
        let config_shell: Option<String> = Some("/bin/fish".to_string());

        let effective = cli_shell
            .clone()
            .or(config_shell)
            .unwrap_or_else(default_shell_string);

        assert_eq!(effective, "/bin/fish");
    }

    #[test]
    fn test_default_shell_when_no_config_no_cli() {
        let cli_shell: Option<String> = None;
        let config_shell: Option<String> = None;

        let effective = cli_shell
            .clone()
            .or(config_shell)
            .unwrap_or_else(default_shell_string);

        // Should not be empty — should resolve to some system shell
        assert!(!effective.is_empty());
    }

    #[test]
    fn test_cell_size_from_config_applied() {
        // Simulate the config merge logic for cell dimensions
        let mut desktop_config = DesktopConfig::default();
        assert_eq!(desktop_config.cell_width, 8.0); // default

        // Config says cell_width = 10
        let config_cell_width: u32 = 10;
        if desktop_config.cell_width == 8.0 {
            desktop_config.cell_width = config_cell_width as f32;
        }
        assert_eq!(desktop_config.cell_width, 10.0);
    }

    #[test]
    fn test_cell_size_cli_overrides_config() {
        // If CLI set a non-default cell_width, config should NOT override it
        let mut desktop_config = DesktopConfig::default().with_cell_size(9.5, 19.0);
        let config_cell_width: u32 = 10;

        if desktop_config.cell_width == 8.0 {
            // This branch should NOT execute since CLI set 9.5
            desktop_config.cell_width = config_cell_width as f32;
        }
        assert_eq!(
            desktop_config.cell_width, 9.5,
            "CLI cell_width should be preserved"
        );
    }

    // ── P9-H: Resize computation tests ────────────────────────────────

    #[test]
    fn test_compute_cell_dimensions_basic() {
        // 640px / 8px = 80 cols, 384px / 16px = 24 rows
        let (cols, rows) = compute_cell_dimensions(640, 384, 8.0, 16.0);
        assert_eq!(cols, 80);
        assert_eq!(rows, 24);
    }

    #[test]
    fn test_compute_cell_dimensions_minimum() {
        // 0px → clamped to MIN_COLS x MIN_ROWS
        let (cols, rows) = compute_cell_dimensions(0, 0, 8.0, 16.0);
        assert_eq!(cols, MIN_COLS);
        assert_eq!(rows, MIN_ROWS);
    }

    #[test]
    fn test_compute_cell_dimensions_small_window() {
        // 40px / 8 = 5 cols → clamped to 10
        let (cols, rows) = compute_cell_dimensions(40, 32, 8.0, 16.0);
        assert_eq!(cols, MIN_COLS); // 5 → 10
        assert_eq!(rows, MIN_ROWS); // 2 → 3
    }

    #[test]
    fn test_compute_cell_dimensions_just_at_minimum() {
        // 80px / 8 = 10 cols (exactly MIN_COLS)
        // 48px / 16 = 3 rows (exactly MIN_ROWS)
        let (cols, rows) = compute_cell_dimensions(80, 48, 8.0, 16.0);
        assert_eq!(cols, 10);
        assert_eq!(rows, 3);
    }

    #[test]
    fn test_compute_cell_dimensions_large_window() {
        // 3840px / 8 = 480, 2160px / 16 = 135
        let (cols, rows) = compute_cell_dimensions(3840, 2160, 8.0, 16.0);
        assert_eq!(cols, 480);
        assert_eq!(rows, 135);
    }

    #[test]
    fn test_compute_cell_dimensions_custom_cell_size() {
        // cell_width=12, cell_height=24
        let (cols, rows) = compute_cell_dimensions(1200, 720, 12.0, 24.0);
        assert_eq!(cols, 100);
        assert_eq!(rows, 30);
    }

    #[test]
    fn test_compute_cell_dimensions_subpixel_floor() {
        // 644px / 8.0 = 80.5 → floor → 80
        let (cols, _) = compute_cell_dimensions(644, 384, 8.0, 16.0);
        assert_eq!(cols, 80);
    }

    #[test]
    fn test_min_cols_constant() {
        assert_eq!(MIN_COLS, 10);
    }

    #[test]
    fn test_min_rows_constant() {
        assert_eq!(MIN_ROWS, 3);
    }

    #[test]
    fn test_debounce_ms_constant() {
        assert_eq!(RESIZE_DEBOUNCE_MS, 100);
    }
}
