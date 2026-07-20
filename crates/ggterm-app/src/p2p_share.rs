//! P2P terminal sharing — desktop-side host state and QR code generation.
//!
//! When `visible` is true, a share overlay is rendered showing:
//! - A QR code encoding the iroh NodeTicket (~130 chars)
//! - Connection status (Generating / Waiting / Connected / Error)
//! - Instructions for the mobile app
//!
//! The host runs `accept()` in a background thread. Once a mobile device
//! connects, the P2P transport is available for teeing PTY output.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use ggterm_p2p::{P2pHost, P2pTransport};

/// QR code module matrix (black = true, white = false).
pub type QrMatrix = Vec<Vec<bool>>;

/// Connection status of the P2P share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum P2pShareStatus {
    /// Generating ticket / QR code.
    Generating,
    /// Waiting for mobile device to scan and connect.
    Waiting,
    /// Mobile device connected.
    Connected,
    /// Error occurred.
    Error,
}

/// P2P share overlay state.
///
/// Manages the iroh host lifecycle, QR code generation, and connection
/// acceptance. The host runs `accept()` on a background thread.
pub struct P2pShareState {
    /// Whether the share overlay is visible.
    pub visible: bool,
    /// Current connection status.
    pub status: P2pShareStatus,
    /// Error message (if status == Error).
    pub error: Option<String>,
    /// Connection ticket string (~130 chars, encoded for QR).
    ticket: Option<String>,
    /// QR code module matrix.
    qr: Option<QrMatrix>,
    /// The iroh host (keeps the endpoint alive).
    host: Option<P2pHost>,
    /// Background thread handle for accept().
    accept_thread: Option<std::thread::JoinHandle<()>>,
    /// Channel: background thread sends the connected transport here.
    /// Wrapped in Arc<Mutex> so the thread can set it.
    connected_tx: Arc<Mutex<Option<mpsc::Receiver<P2pTransport>>>>,
    /// The connected P2P transport (set when mobile connects).
    transport: Option<P2pTransport>,
    /// Buffer of PTY output to forward to the mobile device.
    tee_buffer: Vec<u8>,
}

impl Default for P2pShareState {
    fn default() -> Self {
        Self::new()
    }
}

impl P2pShareState {
    /// Create a new idle P2P share state.
    pub fn new() -> Self {
        Self {
            visible: false,
            status: P2pShareStatus::Generating,
            error: None,
            ticket: None,
            qr: None,
            host: None,
            accept_thread: None,
            connected_tx: Arc::new(Mutex::new(None)),
            transport: None,
            tee_buffer: Vec::new(),
        }
    }

    /// Start P2P sharing: create host, generate ticket + QR code.
    pub fn start(&mut self) {
        self.stop();
        self.visible = true;
        self.status = P2pShareStatus::Generating;
        self.error = None;
        self.transport = None;
        self.tee_buffer.clear();

        log::debug!("start() called");
        match P2pHost::start() {
            Ok(host) => {
                let ticket = host.ticket().to_string();
                log::debug!("host started, ticket len={}", ticket.len());
                // Write ticket to file for automation/testing.
                // Use PID-suffixed path with restrictive permissions to prevent
                // other users from reading the connection ticket.
                let ticket_path = format!("/tmp/ggterm_p2p_ticket_{}", std::process::id());
                let _ = std::fs::write(&ticket_path, &ticket);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(
                        &ticket_path,
                        std::fs::Permissions::from_mode(0o600),
                    );
                }

                // Generate QR code from the ticket.
                let qr = generate_qr(&ticket);
                self.qr = qr;
                self.ticket = Some(ticket);

                // Set up accept channel.
                let (tx, rx) = mpsc::channel::<P2pTransport>();
                *self.connected_tx.lock().unwrap() = Some(rx);

                // Spawn background thread to accept connections.
                let accept_tx = self.connected_tx.clone();
                self.accept_thread = Some(
                    std::thread::Builder::new()
                        .name("p2p-accept".into())
                        .spawn(move || {
                            let mut host = host;
                            log::debug!("accept thread: waiting...");
                            match host.accept() {
                                Ok(transport) => {
                                    log::debug!("accept thread: connection received!");
                                    // host.accept() takes runtime + endpoint out of host.
                                    // host is now empty, safe to drop.
                                    if let Ok(guard) = accept_tx.lock()
                                        && let Some(_rx) = guard.as_ref()
                                    {
                                        let _ = tx.send(transport);
                                        log::debug!("accept thread: transport sent");
                                    } else {
                                        log::debug!("accept thread: rx gone!");
                                    }
                                    // Drop host normally — it has no runtime/endpoint left.
                                    drop(host);
                                }
                                Err(e) => {
                                    log::debug!("accept thread: FAILED: {e}");
                                }
                            }
                        })
                        .expect("p2p-accept thread spawn"),
                );

                self.status = P2pShareStatus::Waiting;
                log::info!(
                    "P2P sharing started, ticket length: {}",
                    self.ticket.as_ref().map(|t| t.len()).unwrap_or(0)
                );
            }
            Err(e) => {
                self.status = P2pShareStatus::Error;
                self.error = Some(format!("{e}"));
                log::error!("P2P host start failed: {e}");
            }
        }
    }

    /// Stop P2P sharing: close host, clear state.
    pub fn stop(&mut self) {
        self.visible = false;
        self.status = P2pShareStatus::Generating;
        self.ticket = None;
        self.qr = None;
        self.transport = None;
        self.error = None;
        self.tee_buffer.clear();

        // The host was moved into the accept thread, so we can't close it here.
        // Dropping the thread handle is sufficient — when accept() returns or
        // errors, the host is cleaned up.
        self.host = None;
        self.accept_thread = None;
        *self.connected_tx.lock().unwrap() = None;
    }

    /// Toggle sharing on/off.
    pub fn toggle(&mut self) {
        if self.visible {
            self.stop();
        } else {
            self.start();
        }
    }

    /// Poll for completed connections from the background thread.
    /// Call this from `about_to_wait()`.
    ///
    /// Returns `true` if a new connection was established.
    pub fn poll_connection(&mut self) -> bool {
        if self.status != P2pShareStatus::Waiting {
            return false;
        }

        let rx = {
            let guard = self.connected_tx.lock().unwrap();
            guard.as_ref().map(|rx| rx.try_recv().ok())
        };

        match rx.flatten() {
            Some(transport) => {
                log::debug!("poll_connection: got transport!");
                self.transport = Some(transport);
                self.status = P2pShareStatus::Connected;
                log::debug!("poll_connection: status=Connected");
                true
            }
            None => false,
        }
    }

    /// Tee PTY output to the P2P stream.
    /// Called whenever new bytes arrive from the local PTY.
    pub fn tee_output(&mut self, bytes: &[u8]) {
        if self.status != P2pShareStatus::Connected {
            return;
        }
        if let Some(ref mut transport) = self.transport {
            use ggterm_core::TerminalTransport;
            transport.write(bytes);
        }
    }

    /// Read input from the mobile device (if connected).
    /// Returns bytes that should be forwarded to the PTY.
    pub fn read_input(&mut self) -> Vec<u8> {
        if self.status != P2pShareStatus::Connected {
            return Vec::new();
        }
        if let Some(ref mut transport) = self.transport {
            use ggterm_core::TerminalTransport;
            return transport.read();
        }
        Vec::new()
    }

    /// Forward terminal resize to the mobile device.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        if self.status != P2pShareStatus::Connected {
            return;
        }
        if let Some(ref mut transport) = self.transport {
            use ggterm_core::TerminalTransport;
            transport.resize(cols as usize, rows as usize);
        }
    }

    /// Get the ticket string (for display/copy).
    pub fn ticket(&self) -> &str {
        self.ticket.as_deref().unwrap_or("")
    }

    /// Get the QR code matrix (if generated).
    pub fn qr(&self) -> Option<&QrMatrix> {
        self.qr.as_ref()
    }

    /// Check if sharing is active (host is running).
    pub fn is_active(&self) -> bool {
        self.visible || self.status == P2pShareStatus::Connected
    }

    /// Check if the P2P transport is still connected.
    pub fn is_connected(&self) -> bool {
        self.transport.as_ref().is_some_and(|t| t.is_connected())
    }

    /// Mark the connection as lost, clearing the transport.
    pub fn mark_connection_lost(&mut self) {
        self.status = P2pShareStatus::Error;
        self.error = Some("Connection lost".into());
        self.transport = None;
    }
}

/// Generate a QR code matrix from a string.
///
/// Returns `None` if QR encoding fails (e.g., data too large).
fn generate_qr(data: &str) -> Option<QrMatrix> {
    let code = qrcode::QrCode::new(data.as_bytes()).ok()?;
    let width = code.width();
    let mut matrix = Vec::with_capacity(width);
    for y in 0..width {
        let mut row = Vec::with_capacity(width);
        for x in 0..width {
            row.push(code[(x, y)] == qrcode::Color::Dark);
        }
        matrix.push(row);
    }
    Some(matrix)
}

// ═══════════════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_new_is_idle() {
        let state = P2pShareState::new();
        assert!(!state.visible);
        assert_eq!(state.status, P2pShareStatus::Generating);
        assert!(state.ticket.is_none());
        assert!(state.qr.is_none());
    }

    #[test]
    fn t_qr_generation_simple() {
        let qr = generate_qr("hello world");
        assert!(qr.is_some());
        let matrix = qr.unwrap();
        assert!(!matrix.is_empty());
        assert!(matrix[0].len() == matrix.len()); // square
    }

    #[test]
    fn t_qr_generation_empty_data() {
        let qr = generate_qr("");
        assert!(qr.is_some()); // QR codes can encode empty strings
    }

    #[test]
    fn t_qr_matrix_is_square() {
        let qr = generate_qr("test data for qr").unwrap();
        let w = qr.len();
        for row in &qr {
            assert_eq!(row.len(), w);
        }
    }

    #[test]
    fn t_qr_matrix_has_dark_modules() {
        let qr = generate_qr("some test data").unwrap();
        let has_dark = qr.iter().any(|row| row.iter().any(|&cell| cell));
        assert!(has_dark, "QR matrix should have at least some dark modules");
    }

    #[test]
    fn t_qr_matrix_has_light_modules() {
        let qr = generate_qr("some test data").unwrap();
        let has_light = qr.iter().any(|row| row.iter().any(|&cell| !cell));
        assert!(
            has_light,
            "QR matrix should have at least some light modules"
        );
    }

    #[test]
    fn t_status_variants() {
        assert_ne!(P2pShareStatus::Generating, P2pShareStatus::Waiting);
        assert_ne!(P2pShareStatus::Waiting, P2pShareStatus::Connected);
        assert_ne!(P2pShareStatus::Connected, P2pShareStatus::Error);
        assert_ne!(P2pShareStatus::Generating, P2pShareStatus::Error);
    }

    #[test]
    fn t_poll_connection_when_not_waiting() {
        let mut state = P2pShareState::new();
        assert!(!state.poll_connection());
    }

    #[test]
    fn t_tee_output_when_not_connected() {
        let mut state = P2pShareState::new();
        state.tee_output(b"test");
        // Should not crash, should not send anything
    }

    #[test]
    fn t_read_input_when_not_connected() {
        let mut state = P2pShareState::new();
        let input = state.read_input();
        assert!(input.is_empty());
    }

    #[test]
    fn t_ticket_empty_by_default() {
        let state = P2pShareState::new();
        assert_eq!(state.ticket(), "");
    }

    #[test]
    fn t_is_active_false_by_default() {
        let state = P2pShareState::new();
        assert!(!state.is_active());
    }
}
