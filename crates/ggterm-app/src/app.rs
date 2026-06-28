//! The main application: connects PTY, Terminal, Parser, and Renderer.
//!
//! This is the core integration point. It owns the Terminal + Parser on the
//! main thread and processes events from the PTY reader thread.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use ggterm_core::{Parser, Terminal};
use ggterm_render::{ConsoleRenderer, CursorState, Renderer};

use crate::event::{AppEvent, EventReceiver, EventSender};
use crate::input::InputEncoder;

/// The terminal application.
///
/// Owns the terminal state, parser, renderer, and PTY communication.
/// In a full desktop build, this also manages the winit window and
/// wgpu surface (feature-gated behind `desktop`).
pub struct App {
    terminal: Terminal,
    parser: Parser,
    renderer: ConsoleRenderer,
    input_encoder: InputEncoder,
    event_rx: EventReceiver,
    pty_writer: Option<Box<dyn std::io::Write + Send>>,
    running: bool,
}

impl App {
    /// Create a new application with the given terminal size.
    pub fn new(cols: usize, rows: usize) -> (Self, EventSender) {
        let (tx, rx) = mpsc::channel::<AppEvent>();

        let app = Self {
            terminal: Terminal::new(cols, rows),
            parser: Parser::new(),
            renderer: ConsoleRenderer::new(cols, rows),
            input_encoder: InputEncoder::new(),
            event_rx: rx,
            pty_writer: None,
            running: false,
        };

        (app, tx)
    }

    /// Attach a PTY writer for sending keyboard input to the child process.
    pub fn set_pty_writer(&mut self, writer: Box<dyn std::io::Write + Send>) {
        self.pty_writer = Some(writer);
    }

    /// Process a single event. Returns `true` if the app should continue running.
    pub fn handle_event(&mut self, event: AppEvent) -> bool {
        match event {
            AppEvent::PtyBytes(bytes) => {
                self.parser.feed(&bytes, &mut self.terminal);
                self.render();
            }

            AppEvent::Resize { cols, rows } => {
                self.terminal.resize(cols as usize, rows as usize);
                self.renderer.resize(cols as usize, rows as usize);
                self.render();
            }

            AppEvent::Keyboard(bytes) => {
                if let Some(ref mut writer) = self.pty_writer {
                    let _ = writer.write_all(&bytes);
                    let _ = writer.flush();
                }
            }

            AppEvent::PtyExit => {
                self.running = false;
            }

            AppEvent::Quit => {
                self.running = false;
            }
        }

        self.running
    }

    /// Render the current terminal state.
    fn render(&mut self) {
        let (cx, cy) = self.terminal.cursor();
        let cursor = CursorState::new(cx, cy);
        self.renderer.render(self.terminal.grid(), &cursor, None);
    }

    /// Run the event loop (blocking).
    ///
    /// This is the headless/test mode — no winit window. Events arrive
    /// via the channel from the PTY reader thread or test code.
    pub fn run(&mut self) {
        self.running = true;

        while self.running {
            match self.event_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(event) => {
                    self.handle_event(event);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Idle — in desktop mode, this is where wgpu vsync would block.
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    // All senders dropped — quit.
                    self.running = false;
                }
            }
        }
    }

    /// Process all pending events without blocking.
    ///
    /// Useful for tests: feed events and check state without blocking.
    pub fn pump(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            self.handle_event(event);
        }
    }

    /// Get the current terminal grid (read-only).
    pub fn grid(&self) -> &ggterm_core::Grid {
        self.terminal.grid()
    }

    /// Get the current rendered output (ConsoleRenderer).
    pub fn output(&self) -> &str {
        self.renderer.output()
    }

    /// Check if the app is still running.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Send typed text as keyboard input (convenience for testing).
    pub fn send_text(&mut self, text: &str) {
        for ch in text.chars() {
            let key = crate::input::InputKey {
                key: ch,
                modifiers: crate::input::KeyModifiers::default(),
            };
            let bytes = self.input_encoder.encode(&key);
            if let Some(ref mut writer) = self.pty_writer {
                let _ = writer.write_all(&bytes);
                let _ = writer.flush();
            }
        }
    }
}

/// Spawn a PTY reader thread.
///
/// Reads from the PTY in a background thread and sends `PtyBytes` events
/// to the main event loop.
pub fn spawn_pty_reader(
    mut reader: Box<dyn std::io::Read + Send>,
    sender: EventSender,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    // EOF — child process exited
                    let _ = sender.send(AppEvent::PtyExit);
                    break;
                }
                Ok(n) => {
                    let bytes = buf[..n].to_vec();
                    if sender.send(AppEvent::PtyBytes(bytes)).is_err() {
                        // Main loop dropped receiver — quit.
                        break;
                    }
                }
                Err(_e) => {
                    // Read error — treat as exit.
                    let _ = sender.send(AppEvent::PtyExit);
                    break;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::AppEvent;

    #[test]
    fn test_app_creation() {
        let (app, _tx) = App::new(80, 24);
        assert_eq!(app.grid().width(), 80);
        assert_eq!(app.grid().height(), 24);
        assert!(!app.is_running());
    }

    #[test]
    fn test_app_process_pty_bytes() {
        let (mut app, tx) = App::new(40, 5);

        // Feed some bytes via the event channel
        tx.send(AppEvent::PtyBytes(b"Hello World".to_vec())).unwrap();
        app.pump();

        // Check that the terminal processed the bytes
        let output = app.output().to_string();
        assert!(output.contains("Hello World"));
    }

    #[test]
    fn test_app_resize() {
        let (mut app, tx) = App::new(80, 24);

        tx.send(AppEvent::Resize { cols: 120, rows: 40 }).unwrap();
        app.pump();

        assert_eq!(app.grid().width(), 120);
        assert_eq!(app.grid().height(), 40);
    }

    #[test]
    fn test_app_quit_event() {
        let (mut app, tx) = App::new(80, 24);

        tx.send(AppEvent::Quit).unwrap();
        app.pump();

        assert!(!app.is_running());
    }

    #[test]
    fn test_app_pty_exit() {
        let (mut app, tx) = App::new(80, 24);

        tx.send(AppEvent::PtyExit).unwrap();
        app.pump();

        assert!(!app.is_running());
    }

    #[test]
    fn test_app_multiline_output() {
        let (mut app, tx) = App::new(20, 5);

        tx.send(AppEvent::PtyBytes(b"Line 1\r\nLine 2\r\nLine 3".to_vec())).unwrap();
        app.pump();

        let output = app.output().to_string();
        assert!(output.contains("Line 1"));
        assert!(output.contains("Line 2"));
        assert!(output.contains("Line 3"));
    }

    #[test]
    fn test_app_with_colors() {
        let (mut app, tx) = App::new(40, 3);

        tx.send(AppEvent::PtyBytes(b"\x1b[1;31mError\x1b[0m".to_vec())).unwrap();
        app.pump();

        let output = app.output();
        assert!(output.contains("1;31"));
    }

    #[test]
    fn test_app_multiple_events() {
        let (mut app, tx) = App::new(40, 5);

        tx.send(AppEvent::PtyBytes(b"First".to_vec())).unwrap();
        tx.send(AppEvent::PtyBytes(b"Second".to_vec())).unwrap();
        tx.send(AppEvent::PtyBytes(b"Third".to_vec())).unwrap();
        app.pump();

        let output = app.output();
        assert!(output.contains("FirstSecondThird"));
    }

    #[test]
    fn test_app_cjk_text() {
        let (mut app, tx) = App::new(40, 5);

        tx.send(AppEvent::PtyBytes("你好世界".as_bytes().to_vec())).unwrap();
        app.pump();

        let output = app.output();
        assert!(output.contains("你好世界"));
    }
}
