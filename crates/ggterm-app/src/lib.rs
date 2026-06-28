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
//! PTY Thread                    Main Thread (Event Loop)
//! ──────────                    ────────────────────────
//! PtySession::try_read()        winit events (keyboard, resize, close)
//!         │                             │
//!         ▼                             ▼
//!   mpsc::Sender ──────────► AppEvent channel ──► App::handle_event()
//! (PtyBytes)                                               │
//!                                                          ▼
//!                                              Parser::feed(bytes, &mut Terminal)
//!                                                          │
//!                                                          ▼
//!                                              Renderer::render(grid, cursor, dirty)
//!                                                          │
//!                                                          ▼
//!                                              wgpu surface / stdout
//! ```
//!
//! ## Thread Safety
//!
//! - `Terminal` and `Parser` live on the main thread (no synchronization needed).
//! - `PtySession` is moved to a reader thread; bytes are sent via channel.
//! - Input (keyboard) is encoded on the main thread and written to PTY via
//!   a shared writer (behind a mutex or channel).

pub mod app;
pub mod event;
pub mod input;

pub use app::App;
pub use event::AppEvent;
pub use input::InputEncoder;
