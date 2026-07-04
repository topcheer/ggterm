//! Client-side P2P — mobile scans QR code, connects to desktop host.

use iroh::Endpoint;
use iroh::endpoint::presets;

use crate::{ALPN, P2pError, P2pTransport, deserialize_ticket};

/// Mobile-side P2P client.
///
/// Connects to a desktop host using a ticket string obtained by
/// scanning the QR code.
///
/// ## Usage
///
/// ```no_run
/// # use ggterm_p2p::P2pClient;
/// let transport = P2pClient::connect("ticket-string-here").unwrap();
/// // transport implements TerminalTransport
/// ```
pub struct P2pClient;

impl P2pClient {
    /// Connect to a desktop host using a ticket string.
    ///
    /// The ticket is obtained by scanning the QR code displayed on
    /// the desktop side. This function blocks until the connection
    /// is established (or fails).
    pub fn connect(ticket: &str) -> Result<P2pTransport, P2pError> {
        let addr = deserialize_ticket(ticket)?;

        let runtime =
            tokio::runtime::Runtime::new().map_err(|e| P2pError::Runtime(e.to_string()))?;

        let conn = runtime.block_on(async {
            let endpoint = Endpoint::builder(presets::N0)
                .alpns(vec![ALPN.to_vec()])
                .bind()
                .await
                .map_err(|e| P2pError::EndpointCreate(e.to_string()))?;

            let conn = endpoint
                .connect(addr, ALPN)
                .await
                .map_err(|e| P2pError::Connect(e.to_string()))?;

            Ok::<_, P2pError>(conn)
        })?;

        // Open a bidirectional stream.
        let (mut send, recv) = runtime
            .block_on(conn.open_bi())
            .map_err(|e| P2pError::Stream(e.to_string()))?;

        // Send an initial byte so the host's accept_bi() returns immediately.
        // QUIC streams are lazy — accept_bi won't complete until data is sent.
        let init_result: Result<(), P2pError> = runtime.block_on(async {
            use tokio::io::AsyncWriteExt;
            send.write_all(b"\x00")
                .await
                .map_err(|e| P2pError::Stream(e.to_string()))?;
            let _ = send.flush().await;
            Ok(())
        });
        init_result?;

        // Create transport — takes ownership of the runtime.
        Ok(P2pTransport::from_streams(send, recv, runtime))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_invalid_ticket_short() {
        let result = P2pClient::connect("invalid");
        assert!(matches!(result, Err(P2pError::InvalidTicket(_))));
    }

    #[test]
    fn t_invalid_ticket_empty() {
        let result = P2pClient::connect("");
        assert!(result.is_err());
    }
}
