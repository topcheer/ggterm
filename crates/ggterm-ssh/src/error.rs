//! SSH transport error types.

use thiserror::Error;

/// Errors that can occur during SSH session lifecycle.
#[derive(Debug, Error)]
pub enum SshError {
    /// Failed to establish TCP connection to the remote host.
    #[error("connection failed: {0}")]
    Connection(String),

    /// SSH handshake failed.
    #[error("handshake failed: {0}")]
    Handshake(String),

    /// Authentication failed (wrong password or key rejected).
    #[error("authentication failed: {0}")]
    Auth(String),

    /// Failed to open a channel or request PTY/shell.
    #[error("channel error: {0}")]
    Channel(String),

    /// Failed to load or parse a private key.
    #[error("key error: {0}")]
    Key(String),

    /// I/O error during read/write.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// The SSH session has been closed by the remote host.
    #[error("session closed")]
    SessionClosed,

    /// Failed to create the internal tokio runtime.
    #[error("runtime error: {0}")]
    Runtime(String),
}

impl From<russh::Error> for SshError {
    fn from(e: russh::Error) -> Self {
        use russh::Error as E;
        match e {
            E::IO(io) => SshError::Io(io),
            other => SshError::Handshake(other.to_string()),
        }
    }
}
