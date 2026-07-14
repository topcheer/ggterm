//! # GGTerm P2P — Terminal Sharing via QUIC + NAT Traversal
//!
//! Provides P2P terminal connections between desktop (host) and mobile (client)
//! using [iroh] for QUIC connectivity with automatic NAT hole punching and
//! relay fallback.
//!
//! ## Architecture
//!
//! ```text
//! Desktop (Host)                    Mobile (Client)
//! ┌──────────────┐                  ┌──────────────┐
//! │ P2pHost      │ ◄── QUIC ──►    │ P2pClient     │
//! │  Endpoint    │   (P2P or relay) │  connect()    │
//! │  PTY ↔ Stream│                  │  Stream ↔ Term│
//! └──────────────┘                  └──────────────┘
//! ```

pub mod client;
pub mod error;
pub mod host;
pub mod transport;

pub use client::P2pClient;
pub use error::P2pError;
pub use host::P2pHost;
pub use transport::P2pTransport;

use data_encoding::BASE32;
use iroh::EndpointAddr;

/// ALPN protocol identifier for GGTerm P2P terminal sessions.
///
/// Both host and client must use this ALPN for the QUIC connection to be accepted.
pub const ALPN: &[u8] = b"/ggterm/term/1";

/// Serialize an `EndpointAddr` to a compact ticket string for QR codes.
///
/// Uses base32 encoding of postcard-serialized bytes (matching iroh's own
/// ticket format). The result is ~130 characters — fits in a single QR code.
pub fn serialize_ticket(addr: &EndpointAddr) -> String {
    let bytes = postcard::to_allocvec(addr).unwrap_or_default();
    BASE32.encode(&bytes)
}

/// Deserialize a ticket string back into an `EndpointAddr`.
pub fn deserialize_ticket(s: &str) -> Result<EndpointAddr, P2pError> {
    let bytes = BASE32
        .decode(s.to_uppercase().as_bytes())
        .map_err(|e| P2pError::InvalidTicket(e.to_string()))?;
    postcard::from_bytes::<EndpointAddr>(&bytes).map_err(|e| P2pError::InvalidTicket(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_alpn_constant() {
        assert_eq!(ALPN, b"/ggterm/term/1");
    }

    #[test]
    fn t_error_display_all_variants() {
        let _ = format!("{}", P2pError::EndpointCreate("x".into()));
        let _ = format!("{}", P2pError::Connect("x".into()));
        let _ = format!("{}", P2pError::Stream("x".into()));
        let _ = format!("{}", P2pError::InvalidTicket("x".into()));
        let _ = format!("{}", P2pError::ConnectionClosed);
        let _ = format!("{}", P2pError::Runtime("x".into()));
    }

    #[test]
    fn t_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "test");
        let p2p_err: P2pError = io_err.into();
        assert!(matches!(p2p_err, P2pError::Io(_)));
    }

    #[test]
    fn t_deserialize_invalid_ticket() {
        let result = deserialize_ticket("not-a-valid-ticket!!!");
        assert!(matches!(result, Err(P2pError::InvalidTicket(_))));
    }

    #[test]
    fn t_deserialize_empty_ticket() {
        let result = deserialize_ticket("");
        assert!(matches!(result, Err(P2pError::InvalidTicket(_))));
    }
}
