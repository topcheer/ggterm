//! VTE (Virtual Terminal Emulator) parser.
//!
//! Implements the Paul Williams ANSI parser state machine for parsing
//! VT100/xterm escape sequences.

mod parser;
mod perform;
#[cfg(test)]
mod vte_tests;

pub use parser::Parser;
pub use perform::Perform;
