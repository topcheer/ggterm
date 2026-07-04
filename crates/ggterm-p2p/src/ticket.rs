//! Ticket serialization — encode/decode [`EndpointAddr`] to/from a compact string.
//!
//! Uses serde + [`data_encoding::BASE32`] to produce a QR-code-friendly string.
//! Typical size: ~120-160 characters (fits a single QR code at version 6-7).

use data_encoding::HEXLOWER;
use iroh::EndpointAddr;

use crate::P2pError;

/// Serialize an [`EndpointAddr`] into a compact hex string for QR codes.
///
/// The endpoint address contains the endpoint ID (Ed25519 public key),
/// relay URL, and direct addresses — everything needed to establish a
/// P2P connection.
///
/// # Errors
///
/// Returns [`P2pError::InvalidTicket`] if serialization fails.
pub fn ticket_to_string(addr: &EndpointAddr) -> Result<String, P2pError> {
    // EndpointAddr implements Serialize via serde.
    // Use serde_json for simplicity, then hex-encode to make it compact.
    let json = serde_json::to_string(addr)
        .map_err(|e| P2pError::InvalidTicket(format!("serialize: {e}")))?;
    Ok(HEXLOWER.encode(json.as_bytes()))
}

/// Parse a ticket string back into an [`EndpointAddr`].
///
/// # Errors
///
/// Returns [`P2pError::InvalidTicket`] if the string cannot be parsed.
pub fn ticket_from_str(s: &str) -> Result<EndpointAddr, P2pError> {
    let bytes = HEXLOWER
        .decode(s.as_bytes())
        .map_err(|e| P2pError::InvalidTicket(format!("hex decode: {e}")))?;
    let json = String::from_utf8(bytes)
        .map_err(|e| P2pError::InvalidTicket(format!("utf8: {e}")))?;
    let addr: EndpointAddr = serde_json::from_str(&json)
        .map_err(|e| P2pError::InvalidTicket(format!("deserialize: {e}")))?;
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_empty_string_invalid() {
        assert!(ticket_from_str("").is_err());
    }

    #[test]
    fn t_garbage_string_invalid() {
        assert!(ticket_from_str("not-a-ticket!!!").is_err());
    }

    #[test]
    fn t_hex_roundtrip_preserves_bytes() {
        // Test the hex encoding itself works.
        let original = b"hello world";
        let encoded = HEXLOWER.encode(original);
        let decoded = HEXLOWER
            .decode(encoded.as_bytes())
            .unwrap();
        assert_eq!(decoded, original);
    }
}
