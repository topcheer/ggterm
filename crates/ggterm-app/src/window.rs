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
//! ## Thread model
//!
//! ```text
//! ┌─────────────────┐     mpsc::channel     ┌──────────────────┐
//! │  PTY Reader     │ ────────────────────▶ │  Main Loop       │
//! │  (std::thread)  │   AppEvent::PtyBytes  │  (winit event    │
//! └─────────────────┘                       │   poll)          │
//!                                           │                  │
//!                                           │  Parser.feed()   │
//!                                           │  Terminal        │
//!                                           │  GlyphonRenderer │
//!                                           │  Surface present │
//!                                           └──────────────────┘
//! ```
//!
//! All Terminal/Parser/Grid mutations happen on the **main thread**.
//! The reader thread only reads raw bytes from the PTY and sends them
//! as `AppEvent::PtyBytes(Vec<u8>)`.

use std::sync::mpsc;
use std::sync::Arc;
use std::sync::Mutex;

use ggterm_core::pty::PtySession;
use ggterm_core::term::Terminal;
use ggterm_core::vte::Parser;
use ggterm_render::theme::RenderTheme;
use ggterm_render::CursorState;

use crate::app::App;
use crate::event::{spawn_pty_reader, AppEvent, EventSender};
use crate::input::{InputEncoder, KeyModifiers};

/// Configuration for the desktop terminal window.
#[derive(Debug, Clone)]
pub struct DesktopConfig {
    /// Initial window title.
    pub title: String,
    /// Initial column count (default 80).
    pub cols: u16,
    /// Initial row count (default 24).
    pub rows: u16,
    /// Cell width in pixels (for DPI calculation).
    pub cell_width: f32,
    /// Cell height in pixels.
    pub cell_height: f32,
    /// Shell binary path (None = auto-detect via $SHELL).
    pub shell: Option<String>,
    /// Render theme for colors.
    pub theme: RenderTheme,
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
            theme: RenderTheme::default(),
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

    /// Set the shell binary.
    pub fn with_shell(mut self, shell: impl Into<String>) -> Self {
        self.shell = Some(shell.into());
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

/// The desktop application: owns the winit window, wgpu device, and
/// the headless `App` (Terminal + Parser + Renderer).
///
/// Created via [`DesktopApp::run`], which blocks on the winit event loop.
pub struct DesktopApp {
    /// The headless application core (Terminal + Parser + InputEncoder).
    app: App,
    /// PTY session (owned, writer accessible via shared handle).
    pty: Arc<Mutex<PtySession>>,
    /// Event sender for the PTY reader thread.
    event_tx: EventSender,
    /// Current key modifiers state (updated by ModifiersChanged events).
    mods: KeyModifiers,
    /// Configuration.
    config: DesktopConfig,
    /// Whether the app should quit.
    quit: bool,
}

impl DesktopApp {
    /// Launch the desktop terminal: create window, GPU device, PTY, and
    /// block on the winit event loop.
    ///
    /// This function does not return until the window is closed or the
    /// user presses Ctrl+C / the shell exits.
    pub fn run(config: DesktopConfig) -> Result<(), Box<dyn std::error::Error>> {
        // 1. Create PTY session
        let (cols, rows) = (config.cols, config.rows);
        let pty = PtySession::open_with_shell(cols, rows, config.shell.as_deref())?;
        let pty = Arc::new(Mutex::new(pty));

        // 2. Create the headless App (Terminal + Parser + ConsoleRenderer)
        let (mut app, event_rx) = App::new(cols as usize, rows as usize);

        // 3. Spawn PTY reader thread → pump bytes into event channel
        {
            let pty_guard = pty.lock().unwrap();
            let reader = pty_guard.try_clone_reader();
            let writer = pty_guard.try_clone_writer();
            app.set_pty_writer(writer);

            if let Some(reader) = reader {
                spawn_pty_reader(reader, app.event_sender().clone());
            }
        }

        // 4. Poll events from the channel (non-blocking pump)
        //    The real winit event loop will be added in the next step.
        //    For now we drain the channel and process events.
        let mut desktop = DesktopApp {
            app,
            pty,
            event_tx: mpsc::channel::<AppEvent>().0, // placeholder
            mods: KeyModifiers::default(),
            config: config.clone(),
            quit: false,
        };

        desktop.event_loop(event_rx)?;
        Ok(())
    }

    /// The main event loop. Pumps events from the PTY reader channel,
    /// feeds them through the Terminal, and renders.
    ///
    /// In the full implementation this will be driven by winit's event loop.
    /// For now it's a simple blocking loop on the mpsc channel.
    fn event_loop(&mut self, rx: mpsc::Receiver<AppEvent>) -> Result<(), Box<dyn std::error::Error>> {
        use std::io::Write;

        while !self.quit {
            match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(AppEvent::PtyBytes(bytes)) => {
                    self.app.handle_event(AppEvent::PtyBytes(bytes));
                }
                Ok(AppEvent::Resize { cols, rows }) => {
                    self.app.handle_event(AppEvent::Resize { cols, rows });
                    // Resize the PTY
                    if let Ok(mut pty) = self.pty.lock() {
                        let _ = pty.resize(cols, rows);
                    }
                }
                Ok(AppEvent::Keyboard(key_bytes)) => {
                    // Write keyboard input to PTY
                    if let Ok(mut pty) = self.pty.lock() {
                        let _ = pty.write(&key_bytes);
                    }
                }
                Ok(AppEvent::PtyExit) => {
                    log::info!("PTY process exited");
                    self.quit = true;
                }
                Ok(AppEvent::Quit) => {
                    self.quit = true;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // No events — continue. In a real winit loop this is where
                    // we'd poll window events.
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    log::info!("Event channel disconnected — PTY reader exited");
                    self.quit = true;
                }
            }
        }

        Ok(())
    }

    /// Get a reference to the headless App.
    pub fn inner_app(&self) -> &App {
        &self.app
    }

    /// Get a mutable reference to the headless App.
    pub fn inner_app_mut(&mut self) -> &mut App {
        &mut self.app
    }

    /// Get the current key modifiers.
    pub fn modifiers(&self) -> &KeyModifiers {
        &self.mods
    }

    /// Check if the app should quit.
    pub fn should_quit(&self) -> bool {
        self.quit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_config_default() {
        let cfg = DesktopConfig::default();
        assert_eq!(cfg.title, "GGTerm");
        assert_eq!(cfg.cols, 80);
        assert_eq!(cfg.rows, 24);
        assert_eq!(cfg.cell_width, 8.0);
        assert_eq!(cfg.cell_height, 16.0);
    }

    #[test]
    fn desktop_config_builder() {
        let cfg = DesktopConfig::default()
            .with_title("My Terminal")
            .with_size(120, 40)
            .with_cell_size(7.0, 14.0)
            .with_shell("/bin/zsh");

        assert_eq!(cfg.title, "My Terminal");
        assert_eq!(cfg.cols, 120);
        assert_eq!(cfg.rows, 40);
        assert_eq!(cfg.cell_width, 7.0);
        assert_eq!(cfg.cell_height, 14.0);
        assert_eq!(cfg.shell.as_deref(), Some("/bin/zsh"));
    }

    #[test]
    fn desktop_config_window_dimensions() {
        let cfg = DesktopConfig::default(); // 80x24, 8x16 cells
        assert_eq!(cfg.window_width(), 640); // 80 * 8
        assert_eq!(cfg.window_height(), 384); // 24 * 16
    }

    #[test]
    fn desktop_config_window_dimensions_custom() {
        let cfg = DesktopConfig::default()
            .with_size(132, 50)
            .with_cell_size(7.0, 14.0);
        assert_eq!(cfg.window_width(), 924); // 132 * 7 = 924
        assert_eq!(cfg.window_height(), 700); // 50 * 14 = 700
    }

    #[test]
    fn desktop_config_window_dimensions_fractional() {
        // 100 cols * 7.5 px = 750 px
        let cfg = DesktopConfig::default()
            .with_size(100, 30)
            .with_cell_size(7.5, 15.5);
        assert_eq!(cfg.window_width(), 750); // 100 * 7.5 = 750.0 → 750
        assert_eq!(cfg.window_height(), 465); // 30 * 15.5 = 465.0 → 465
    }
}
