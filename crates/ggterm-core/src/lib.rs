//! # GGTerm Core
//!
//! Terminal emulation core: VTE parser, grid model, and terminal state machine.
//!
//! This crate is 100% pure Rust with zero rendering dependencies. It handles:
//! - VT/ANSI escape sequence parsing (Paul Williams state machine)
//! - Grid-based cell storage with scrollback
//! - Terminal state management (cursor, modes, colors)
//! - PTY I/O abstraction

pub mod grid;
pub mod pty;
pub mod term;
pub mod vte;

// Re-export key types for convenience
pub use grid::{Cell, CellFlags, Color, DamageTracker, DirtyRect, Grid, Row};
pub use pty::{default_shell, PtyError, PtySession};
pub use term::{Charset, CommandBlock, CommandMark, CommandMarkKind, CursorStyle, Terminal};
pub use vte::{Parser, Perform};
