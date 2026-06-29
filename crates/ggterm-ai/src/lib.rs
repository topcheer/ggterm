//! # GGTerm AI Engine
//!
//! AI-powered features for the terminal:
//! - **Explain**: Natural-language explanation of command output
//! - **Suggest**: AI-suggested next commands based on context
//! - **Error Help**: Diagnose failed commands and suggest fixes
//! - **NL→Command**: Natural language to shell command translation
//!
//! ## Architecture
//!
//! ```text
//! Terminal + CommandNavigator
//!     ↓
//! AIContext (context.rs)     ← extracts command blocks, exit codes, output
//!     ↓
//! Prompt Templates (prompt.rs) ← builds system + user messages
//!     ↓
//! LLM Client (client.rs)     ← OpenAI-compatible HTTP + SSE streaming
//!     ↓
//! AIEngine (engine.rs)       ← high-level API
//! ```

pub mod context;
pub mod prompt;

#[cfg(feature = "http")]
pub mod client;

pub mod engine;

// Re-export key types
pub use context::AIContext;
pub use engine::{AIEngine, AIError, AIResult, LLMProvider};
pub use prompt::{Action, ChatMessage, Role, build_messages};

#[cfg(feature = "http")]
pub use client::{AIConfig, LLMClient};
