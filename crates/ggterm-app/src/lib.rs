//! # GGTerm App
//!
//! Desktop terminal application crate.
//!
//! Connects the PTY session, VTE parser, terminal state machine,
//! and renderer into a running application.
//!
//! ## Architecture
//!
//! ```text
//! PTY Reader Thread → mpsc::channel → Main Thread
//!                                     ↓
//!                              AppEvent::PtyBytes
//!                                     ↓
//!                             Parser.feed() → Terminal
//!                                     ↓
//!                                 Renderer
//!                                     ↓
//!                             Console (headless) or
//!                             wgpu Surface (desktop)
//! ```
//!
//! ## Features
//!
//! - **default**: Headless mode using `ConsoleRenderer` (ANSI output). Suitable
//!   for testing and non-GPU environments.
//! - **desktop**: Enables winit window + wgpu GPU rendering via
//!   `GlyphonRenderer`. Adds winit, wgpu, pollster dependencies.

pub mod app;
pub mod event;
pub mod input;

#[cfg(feature = "desktop")]
pub mod keymap;
#[cfg(feature = "desktop")]
pub mod window;

pub use app::App;
pub use event::AppEvent;
pub use input::InputEncoder;

#[cfg(feature = "desktop")]
pub use keymap::map_winit_key;
#[cfg(feature = "desktop")]
pub use window::{DesktopApp, DesktopConfig};
