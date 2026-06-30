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

pub mod about_dialog;
pub mod app;
pub mod clipboard;
pub mod command_nav;
pub mod config;
pub mod event;
pub mod font;
pub mod input;
pub mod mouse;
pub mod search;
pub mod session;
pub mod settings_ui;
pub mod shell_integration;
pub mod splits;
pub mod status_bar;
pub mod tab_bar;
pub mod tab_session;
pub mod tabs;
pub mod terminal_actions;
pub mod theme;
pub mod version_info;

#[cfg(feature = "ai")]
pub mod ai_bridge;
#[cfg(feature = "ai")]
pub mod ai_overlay;

#[cfg(feature = "desktop")]
pub mod desktop_config;
#[cfg(feature = "desktop")]
pub mod gpu;
#[cfg(feature = "desktop")]
pub mod keymap;
#[cfg(feature = "desktop")]
pub mod menu_bar;
/// macOS native menu bar (desktop feature, macOS-only).
#[cfg(all(feature = "desktop", target_os = "macos"))]
#[cfg(target_os = "macos")]
pub mod native_menu;
#[cfg(feature = "desktop")]
pub mod resize;
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
pub use search::SearchState;
pub use shell_integration::{ShellIntegrationConfig, ShellKind};

#[cfg(feature = "desktop")]
pub use desktop_config::DesktopConfig;
#[cfg(feature = "desktop")]
pub use gpu::{GpuContext, GpuError};
#[cfg(feature = "desktop")]
pub use keymap::map_winit_key;
#[cfg(feature = "desktop")]
pub use window::DesktopApp;
