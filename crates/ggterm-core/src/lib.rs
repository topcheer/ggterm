//! # GGTerm Core
//!
//! Terminal emulation core: VTE parser, grid model, and terminal state machine.
//!
//! This crate is 100% pure Rust with zero rendering dependencies. It handles:
//! - VT/ANSI escape sequence parsing (Paul Williams state machine)
//! - Grid-based cell storage with scrollback
//! - Terminal state management (cursor, modes, colors)
//! - PTY I/O abstraction
//! - Session recording (asciinema v2 format)

pub mod grid;
pub mod pty;
pub mod recording;
pub mod term;
pub mod transport;
pub mod vte;

// Re-export key types for convenience
pub use grid::{Cell, CellFlags, Color, DamageTracker, DirtyRect, Grid, Row};
pub use pty::{PtyError, PtySession, default_shell};
pub use recording::{RecordingHeader, SessionRecorder};
pub use term::{Charset, CommandBlock, CommandMark, CommandMarkKind, CursorStyle, Terminal};
pub use transport::TerminalTransport;
pub use vte::{Parser, Perform};
