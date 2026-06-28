//! Events flowing through the application event loop.

use std::sync::mpsc;

/// Events that can arrive in the main event loop.
#[derive(Debug)]
pub enum AppEvent {
    /// Raw bytes read from the PTY (sent from the PTY reader thread).
    PtyBytes(Vec<u8>),

    /// Terminal was resized (cols, rows).
    Resize { cols: u16, rows: u16 },

    /// Keyboard input (encoded as ANSI bytes, ready to write to PTY).
    Keyboard(Vec<u8>),

    /// The PTY child process has exited.
    PtyExit,

    /// Application should quit.
    Quit,
}

/// Channel type for sending events to the main loop.
pub type EventSender = mpsc::Sender<AppEvent>;

/// Channel type for receiving events in the main loop.
pub type EventReceiver = mpsc::Receiver<AppEvent>;
