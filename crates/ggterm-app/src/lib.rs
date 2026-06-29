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
pub mod command_nav;
pub mod config;
pub mod event;
pub mod input;
pub mod shell_integration;
pub mod tabs;
pub mod theme;

#[cfg(feature = "ai")]
pub mod ai_bridge;

#[cfg(feature = "desktop")]
pub mod gpu;
#[cfg(feature = "desktop")]
pub mod keymap;
#[cfg(feature = "desktop")]
pub mod window;

pub use app::App;
pub use config::Config;
pub use tabs::TabManager;
pub use theme::AppTheme;

/// Plugin integration (feature-gated behind `plugin`).
#[cfg(feature = "plugin")]
pub mod plugin_integration;

#[cfg(feature = "ai")]
pub use ai_bridge::{AIBridge, AIRequest, AIResponse};
pub use command_nav::{CommandNavigator, ExitStatusSummary};
pub use event::AppEvent;
pub use input::InputEncoder;
pub use shell_integration::{ShellIntegrationConfig, ShellKind};

#[cfg(feature = "desktop")]
pub use gpu::{GpuContext, GpuError};
#[cfg(feature = "desktop")]
pub use keymap::map_winit_key;
#[cfg(feature = "desktop")]
pub use window::{DesktopApp, DesktopConfig};
