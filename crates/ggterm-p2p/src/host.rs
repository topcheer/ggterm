//! Host-side P2P endpoint — desktop creates this, generates QR code, accepts mobile connections.

use std::sync::Arc;

use iroh::Endpoint;
use iroh::endpoint::presets;

use crate::error::P2pError;
use crate::transport::P2pTransport;
use crate::{ALPN, serialize_ticket};

/// Desktop-side P2P host.
///
/// Listens for incoming connections from mobile clients. The connection
/// ticket can be encoded as a QR code for the mobile app to scan.
///
/// ## Usage
///
/// ```no_run
/// # use ggterm_p2p::P2pHost;
/// let mut host = P2pHost::start().unwrap();
/// println!("Ticket: {}", host.ticket());
/// let transport = host.accept().unwrap(); // blocks until mobile connects
/// ```
pub struct P2pHost {
    /// Dedicated tokio runtime — owns the iroh endpoint.
    runtime: Option<tokio::runtime::Runtime>,
    /// The iroh endpoint.
    endpoint: Option<Arc<Endpoint>>,
    /// Connection ticket string (generated at start).
    ticket: String,
}

impl P2pHost {
    /// Start listening for incoming P2P connections.
    ///
    /// Creates an iroh endpoint with default N0 relay servers and
    /// generates a connection ticket.
    pub fn start() -> Result<Self, P2pError> {
        let runtime =
            tokio::runtime::Runtime::new().map_err(|e| P2pError::Runtime(e.to_string()))?;

        let endpoint = runtime.block_on(async {
            Endpoint::builder(presets::N0)
                .alpns(vec![ALPN.to_vec()])
                .bind()
                .await
                .map_err(|e| P2pError::EndpointCreate(e.to_string()))
        })?;

        let endpoint = Arc::new(endpoint);

        // Generate the connection ticket.
        let addr = endpoint.addr();
        let ticket = serialize_ticket(&addr);

        Ok(Self {
            runtime: Some(runtime),
            endpoint: Some(endpoint),
            ticket,
        })
    }

    /// Get the connection ticket string for QR code generation.
    ///
    /// The ticket contains:
    /// - EndpointId (Ed25519 public key)
    /// - Relay URL (for NAT traversal coordination)
    /// - Direct addresses (for local network connections)
    pub fn ticket(&self) -> &str {
        &self.ticket
    }

    /// Wait for an incoming connection and return a transport.
    ///
    /// Blocks the calling thread until a mobile client connects.
    /// Returns a [`P2pTransport`] that implements [`TerminalTransport`](ggterm_core::TerminalTransport).
    pub fn accept(&mut self) -> Result<P2pTransport, P2pError> {
        let runtime = self.runtime.as_ref().ok_or(P2pError::ConnectionClosed)?;
        let endpoint = self.endpoint.as_ref().ok_or(P2pError::ConnectionClosed)?;
        let endpoint = endpoint.clone();

        // Accept incoming connection.
        let conn = runtime.block_on(async move {
            let incoming = endpoint
                .accept()
                .await
                .ok_or_else(|| P2pError::Connect("no incoming connection".into()))?;
            incoming.await.map_err(|e| P2pError::Connect(e.to_string()))
        })?;

        // Accept a bidirectional stream from the client.
        let (send, recv) = runtime
            .block_on(conn.accept_bi())
            .map_err(|e| P2pError::Stream(e.to_string()))?;

        // Take ownership of the runtime so the background task stays alive.
        let owned_runtime = self.runtime.take().ok_or(P2pError::ConnectionClosed)?;

        Ok(P2pTransport::from_streams(send, recv, owned_runtime))
    }

    /// Shut down the host and close the endpoint.
    pub fn close(&mut self) {
        if let Some(runtime) = self.runtime.take()
            && let Some(endpoint) = self.endpoint.take()
        {
            runtime.block_on(async {
                endpoint.close().await;
            });
        }
    }
}

impl Drop for P2pHost {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_p2p_error_closed_display() {
        let e = P2pError::ConnectionClosed;
        assert!(format!("{e}").contains("closed"));
    }
}
