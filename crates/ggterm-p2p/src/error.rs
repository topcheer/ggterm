//! Error types for P2P operations.

/// Errors returned by P2P transport operations.
#[derive(Debug)]
pub enum P2pError {
    /// Failed to create or bind the iroh endpoint.
    EndpointCreate(String),
    /// Failed to connect to the remote endpoint.
    Connect(String),
    /// Failed to open or accept a QUIC stream.
    Stream(String),
    /// The ticket string is invalid or cannot be parsed.
    InvalidTicket(String),
    /// The connection was closed by the remote peer.
    ConnectionClosed,
    /// I/O error during data transfer.
    Io(String),
    /// Tokio runtime error.
    Runtime(String),
}

impl std::fmt::Display for P2pError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EndpointCreate(s) => write!(f, "failed to create endpoint: {s}"),
            Self::Connect(s) => write!(f, "connection failed: {s}"),
            Self::Stream(s) => write!(f, "stream error: {s}"),
            Self::InvalidTicket(s) => write!(f, "invalid ticket: {s}"),
            Self::ConnectionClosed => write!(f, "connection closed by peer"),
            Self::Io(s) => write!(f, "I/O error: {s}"),
            Self::Runtime(s) => write!(f, "runtime error: {s}"),
        }
    }
}

impl std::error::Error for P2pError {}

impl From<std::io::Error> for P2pError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}
