//! SSH remote transport for ggterm.
//!
//! Provides [`SshSession`] — an SSH client implementing
//! [`TerminalTransport`](ggterm_core::TerminalTransport) for remote
//! terminal sessions.
//!
//! ## Architecture
//!
//! All SSH I/O runs on a background tokio task. The synchronous
//! [`TerminalTransport`] methods are **instant and non-blocking** — they
//! just read from / write to `std::sync::Mutex<Vec<u8>>` buffers.  This is
//! critical because the FFI is called from Flutter's UI thread; any
//! `block_on()` in `read()`/`write()` would freeze the entire app.
//!
//! ```text
//!  Flutter UI thread          Background tokio task
//!  ┌────────────┐             ┌─────────────────────┐
//!  │ read()     │ ◄── drain ── │ channel.wait()      │
//!  │            │              │  → push to read_buf │
//!  │ write()    │ ── push ──► │  → pop write_buf    │
//!  │            │              │  → channel.data()   │
//!  │ resize()   │ ── push ──► │  → window_change()  │
//!  └────────────┘             └─────────────────────┘
//! ```

pub mod error;

pub use error::SshError;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use russh::client::{self};
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg};
use russh::{ChannelMsg, Disconnect};
use tokio::sync::mpsc;

// ─────────────────────────────────────────────────────────────────
//  Client handler
// ─────────────────────────────────────────────────────────────────

/// russh client handler — stores the server's public key for verification.
struct ClientHandler {
    /// Filled in by check_server_key with the SHA-256 fingerprint.
    server_fingerprint: Arc<Mutex<Option<String>>>,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        // Compute SHA-256 fingerprint of the server key for logging/display.
        let fingerprint = sha256_fingerprint(server_public_key);
        log::info!("SSH server key fingerprint: {}", fingerprint);
        *self.server_fingerprint.lock().unwrap() = Some(fingerprint);

        // Accept all server keys. For production use, this should verify
        // against ~/.ssh/known_hosts, but that requires a host key database
        // and user confirmation UI which is beyond the current scope.
        // The fingerprint is available via SshSession::server_fingerprint().
        Ok(true)
    }
}

/// Compute the SHA-256 fingerprint of a public key in OpenSSH format.
/// Returns "SHA256:base64..." for display and logging.
fn sha256_fingerprint(key: &russh::keys::PublicKey) -> String {
    // Use the SSH wire format public key blob, then SHA-256 + base64.
    let blob = key.fingerprint(russh::keys::HashAlg::Sha256);
    let encoded = base64_encoded(blob.as_bytes());
    let mut s = String::with_capacity(7 + encoded.len());
    s.push_str("SHA256:");
    s.push_str(&encoded);
    s
}

/// Minimal base64 encoder (no external dependency needed for short blobs).
fn base64_encoded(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((n >> 18) & 63) as usize] as char);
        result.push(CHARS[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((n >> 6) & 63) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(n & 63) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

// ─────────────────────────────────────────────────────────────────
//  SshSession
// ─────────────────────────────────────────────────────────────────

/// An active SSH terminal session.
///
/// All async SSH I/O runs on a background tokio task communicating via
/// shared buffers, so the synchronous [`TerminalTransport`] methods are
/// non-blocking and safe to call from a UI thread.
///
/// On drop, the session disconnects automatically.
pub struct SshSession {
    /// Dedicated tokio runtime — owns the background I/O task.
    /// Dropping the runtime cancels the task.
    runtime: Option<tokio::runtime::Runtime>,
    /// SSH client handle for graceful disconnection.
    handle: Option<client::Handle<ClientHandler>>,
    /// Sender for writing data to the SSH channel (background task drains).
    write_tx: Option<mpsc::UnboundedSender<Vec<u8>>>,
    /// Sender for resize commands (background task applies).
    resize_tx: Option<mpsc::UnboundedSender<(u32, u32)>>,
    /// Read buffer filled by background task, drained by `read()`.
    read_buf: Arc<Mutex<Vec<u8>>>,
    /// Session alive flag (shared with background task).
    alive: Arc<AtomicBool>,
}

impl SshSession {
    /// Connect to a remote host using password authentication.
    ///
    /// Opens a PTY-backed shell channel at 80x24. Resize later via
    /// [`resize`](ggterm_core::TerminalTransport::resize).
    ///
    /// # Errors
    ///
    /// - [`SshError::Connection`] — TCP connection failed
    /// - [`SshError::Auth`] — authentication rejected
    /// - [`SshError::Channel`] — PTY or shell request failed
    pub fn connect(
        host: &str,
        port: u16,
        username: &str,
        password: &str,
    ) -> Result<Self, SshError> {
        Self::connect_impl(
            host,
            port,
            username,
            AuthMethod::Password(password.to_string()),
        )
    }

    /// Connect to a remote host using public key authentication.
    ///
    /// `key_path` should point to a private key file (e.g. `~/.ssh/id_ed25519`).
    ///
    /// # Errors
    ///
    /// - [`SshError::Key`] — key file cannot be loaded or parsed
    pub fn connect_with_key(
        host: &str,
        port: u16,
        username: &str,
        key_path: &Path,
    ) -> Result<Self, SshError> {
        let key = russh::keys::load_secret_key(key_path, None)
            .map_err(|e| SshError::Key(e.to_string()))?;
        Self::connect_impl(host, port, username, AuthMethod::PublicKey(Box::new(key)))
    }

    /// Internal: create runtime, connect, authenticate, open channel.
    fn connect_impl(
        host: &str,
        port: u16,
        username: &str,
        auth: AuthMethod,
    ) -> Result<Self, SshError> {
        let runtime =
            tokio::runtime::Runtime::new().map_err(|e| SshError::Runtime(e.to_string()))?;

        let server_fingerprint: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

        let mut handle = runtime
            .block_on(client::connect(
                Arc::new(client::Config::default()),
                (host, port),
                ClientHandler {
                    server_fingerprint: server_fingerprint.clone(),
                },
            ))
            .map_err(|e| SshError::Connection(e.to_string()))?;

        // Authenticate.
        let auth_ok = match auth {
            AuthMethod::Password(pw) => runtime
                .block_on(handle.authenticate_password(username, &pw))
                .map_err(|e| SshError::Auth(e.to_string()))?
                .success(),
            AuthMethod::PublicKey(key) => {
                let key_with_hash = PrivateKeyWithHashAlg::new(Arc::new(*key), None);
                runtime
                    .block_on(handle.authenticate_publickey(username, key_with_hash))
                    .map_err(|e| SshError::Auth(e.to_string()))?
                    .success()
            }
        };

        if !auth_ok {
            return Err(SshError::Auth("authentication rejected".into()));
        }

        // Open channel and request PTY + shell.
        let channel = runtime
            .block_on(handle.channel_open_session())
            .map_err(|e| SshError::Channel(e.to_string()))?;

        runtime
            .block_on(channel.request_pty(true, "xterm-256color", 80, 24, 0, 0, &[]))
            .map_err(|e| SshError::Channel(e.to_string()))?;

        runtime
            .block_on(channel.request_shell(true))
            .map_err(|e| SshError::Channel(e.to_string()))?;

        // ── Spawn background I/O task ───────────────────────────
        // All SSH reads/writes happen here, never blocking the FFI caller.
        let (write_tx, mut write_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (resize_tx, mut resize_rx) = mpsc::unbounded_channel::<(u32, u32)>();
        let read_buf = Arc::new(Mutex::new(Vec::<u8>::with_capacity(8192)));
        let alive = Arc::new(AtomicBool::new(true));

        let read_buf_task = read_buf.clone();
        let alive_task = alive.clone();

        runtime.spawn(async move {
            let mut channel = channel;
            // Keepalive: send SSH protocol keepalive every ~200 loop iterations
            // (~1 second at 5ms poll). This uses the SSH transport layer
            // (SSH_MSG_GLOBAL_REQUEST) rather than channel data, so it doesn't
            // interfere with terminal output.
            let mut tick: u32 = 0;
            const KEEPALIVE_INTERVAL: u32 = 6000; // ~30s at 5ms poll
            // Track current terminal dimensions for keepalive window_change.
            let mut current_cols: Option<u32> = None;
            let mut current_rows: Option<u32> = None;
            loop {
                // ── Drain pending writes (non-blocking) ────────────
                // &self borrow, released after each iteration
                while let Ok(data) = write_rx.try_recv() {
                    let _ = channel.data(data.as_slice()).await;
                }
                // ── Apply pending resize (non-blocking) ────────────
                while let Ok((cols, rows)) = resize_rx.try_recv() {
                    let _ = channel.window_change(cols, rows, 0, 0).await;
                    current_cols = Some(cols);
                    current_rows = Some(rows);
                }
                // ── Read from SSH channel (5ms timeout) ────────────
                // &mut self borrow — no other borrows active here
                match tokio::time::timeout(std::time::Duration::from_millis(5), channel.wait())
                    .await
                {
                    Ok(Some(ChannelMsg::Data { ref data })) => {
                        if let Ok(mut buf) = read_buf_task.lock() {
                            buf.extend_from_slice(data);
                        }
                    }
                    Ok(Some(ChannelMsg::ExtendedData { ref data, .. })) => {
                        if let Ok(mut buf) = read_buf_task.lock() {
                            buf.extend_from_slice(data);
                        }
                    }
                    Ok(Some(ChannelMsg::Eof)) | Ok(Some(ChannelMsg::Close)) | Ok(None) => {
                        alive_task.store(false, Ordering::Relaxed);
                        break;
                    }
                    Err(_) => {
                        // Timeout — no data. Send keepalive if interval elapsed.
                        // Use window_change (PTY resize) as a harmless keepalive:
                        // it sends a real SSH message that prevents NAT/firewall
                        // timeouts without injecting data into the terminal stream.
                        tick += 1;
                        if tick >= KEEPALIVE_INTERVAL {
                            tick = 0;
                            // Re-send the last known window size as keepalive.
                            // This is a no-op for the remote terminal but keeps
                            // the SSH connection alive.
                            let _ = channel
                                .window_change(
                                    current_cols.unwrap_or(80),
                                    current_rows.unwrap_or(24),
                                    0,
                                    0,
                                )
                                .await;
                        }
                    }
                    _ => {}
                }
            }
        });

        Ok(Self {
            runtime: Some(runtime),
            handle: Some(handle),
            write_tx: Some(write_tx),
            resize_tx: Some(resize_tx),
            read_buf,
            alive,
        })
    }

    /// Explicitly close the SSH session and disconnect.
    pub fn close(&mut self) {
        self.alive.store(false, Ordering::Relaxed);

        // Drop senders to signal background task to stop writing.
        self.write_tx.take();
        self.resize_tx.take();

        // Disconnect gracefully (best-effort).
        if let (Some(runtime), Some(handle)) = (self.runtime.take(), self.handle.take()) {
            let _ = runtime.block_on(handle.disconnect(
                Disconnect::ByApplication,
                "ggterm disconnect",
                "en",
            ));
            // Dropping runtime cancels the background task.
        }
    }
}

impl Drop for SshSession {
    fn drop(&mut self) {
        self.close();
    }
}

// ─────────────────────────────────────────────────────────────────
//  TerminalTransport implementation
// ─────────────────────────────────────────────────────────────────
//
// All methods are INSTANT and NON-BLOCKING.
// They only touch shared buffers / channels — no async, no block_on.

impl ggterm_core::TerminalTransport for SshSession {
    fn read(&mut self) -> Vec<u8> {
        if let Ok(mut buf) = self.read_buf.lock() {
            return std::mem::take(&mut *buf);
        }
        Vec::new()
    }

    fn write(&mut self, data: &[u8]) {
        if let Some(tx) = &self.write_tx {
            let _ = tx.send(data.to_vec());
        }
    }

    fn resize(&mut self, cols: usize, rows: usize) {
        if let Some(tx) = &self.resize_tx {
            let _ = tx.send((cols as u32, rows as u32));
        }
    }

    fn is_alive(&mut self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
}

// ─────────────────────────────────────────────────────────────────
//  Internal types
// ─────────────────────────────────────────────────────────────────

/// Authentication method.
enum AuthMethod {
    Password(String),
    PublicKey(Box<PrivateKey>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use ggterm_core::TerminalTransport;

    // ── Error tests ────────────────────────────────────────────

    #[test]
    fn test_ssh_error_display_connection() {
        let e = SshError::Connection("refused".into());
        assert!(format!("{e}").contains("refused"));
    }

    #[test]
    fn test_ssh_error_display_auth() {
        let e = SshError::Auth("bad password".into());
        let s = format!("{e}");
        assert!(s.contains("authentication"));
        assert!(s.contains("bad password"));
    }

    #[test]
    fn test_ssh_error_display_channel() {
        let e = SshError::Channel("timeout".into());
        assert!(format!("{e}").contains("channel"));
    }

    #[test]
    fn test_ssh_error_display_key() {
        let e = SshError::Key("file not found".into());
        assert!(format!("{e}").contains("key"));
    }

    #[test]
    fn test_ssh_error_display_handshake() {
        let e = SshError::Handshake("bad".into());
        assert!(format!("{e}").contains("handshake"));
    }

    #[test]
    fn test_ssh_error_display_session_closed() {
        let e = SshError::SessionClosed;
        assert!(format!("{e}").contains("closed"));
    }

    #[test]
    fn test_ssh_error_display_runtime() {
        let e = SshError::Runtime("create failed".into());
        assert!(format!("{e}").contains("runtime"));
    }

    #[test]
    fn test_ssh_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        let ssh_err: SshError = io_err.into();
        assert!(matches!(ssh_err, SshError::Io(_)));
    }

    #[test]
    fn test_ssh_error_debug() {
        let e = SshError::Auth("test".into());
        assert!(format!("{e:?}").contains("Auth"));
    }

    #[test]
    fn test_ssh_error_all_variants_display() {
        // Ensure every variant has a Display impl that doesn't panic.
        let _ = format!("{}", SshError::Connection("x".into()));
        let _ = format!("{}", SshError::Handshake("x".into()));
        let _ = format!("{}", SshError::Auth("x".into()));
        let _ = format!("{}", SshError::Channel("x".into()));
        let _ = format!("{}", SshError::Key("x".into()));
        let _ = format!("{}", SshError::Io(std::io::Error::other("x")));
        let _ = format!("{}", SshError::SessionClosed);
        let _ = format!("{}", SshError::Runtime("x".into()));
    }

    // ── Struct compilation / signature tests ───────────────────

    #[test]
    fn test_connect_fn_signature() {
        fn _assert(host: &str, port: u16, user: &str, pass: &str) -> Result<SshSession, SshError> {
            SshSession::connect(host, port, user, pass)
        }
    }

    #[test]
    fn test_connect_with_key_fn_signature() {
        fn _assert(host: &str, port: u16, user: &str, key: &Path) -> Result<SshSession, SshError> {
            SshSession::connect_with_key(host, port, user, key)
        }
    }

    #[test]
    fn test_terminal_transport_impl() {
        // Verify SshSession implements TerminalTransport at compile time.
        fn _assert_transport<T: ggterm_core::TerminalTransport>() {}
        _assert_transport::<SshSession>();
    }

    #[test]
    fn test_auth_method_variants() {
        let _pw = AuthMethod::Password("secret".into());
        // Key variant requires PrivateKey, tested via network tests.
    }

    #[test]
    fn test_ssh_session_is_send() {
        fn _assert_send<T: Send>() {}
        _assert_send::<SshSession>();
    }

    // ── Network tests (require real SSH server) ────────────────

    #[test]
    #[ignore = "requires real SSH server"]
    fn test_connect_password() {
        let mut session =
            SshSession::connect("localhost", 22, "testuser", "testpass").expect("connect");
        assert!(session.is_alive());
    }

    #[test]
    #[ignore = "requires real SSH server + key file"]
    fn test_connect_key() {
        let key_path = Path::new("/home/testuser/.ssh/id_ed25519");
        let mut session =
            SshSession::connect_with_key("localhost", 22, "testuser", key_path).expect("connect");
        assert!(session.is_alive());
    }

    #[test]
    #[ignore = "requires real SSH server"]
    fn test_write_and_read() {
        let mut session =
            SshSession::connect("localhost", 22, "testuser", "testpass").expect("connect");
        <SshSession as ggterm_core::TerminalTransport>::write(&mut session, b"echo hello\n");
        std::thread::sleep(std::time::Duration::from_millis(500));
        let data = <SshSession as ggterm_core::TerminalTransport>::read(&mut session);
        assert!(!data.is_empty());
    }

    #[test]
    #[ignore = "requires real SSH server"]
    fn test_resize() {
        let mut session =
            SshSession::connect("localhost", 22, "testuser", "testpass").expect("connect");
        <SshSession as ggterm_core::TerminalTransport>::resize(&mut session, 120, 40);
    }

    #[test]
    #[ignore = "requires real SSH server"]
    fn test_disconnect() {
        let mut session =
            SshSession::connect("localhost", 22, "testuser", "testpass").expect("connect");
        assert!(session.is_alive());
        session.close();
        assert!(!session.is_alive());
    }

    #[test]
    #[ignore = "requires real SSH server"]
    fn test_drop_disconnects() {
        {
            let _session =
                SshSession::connect("localhost", 22, "testuser", "testpass").expect("connect");
        }
        // Should not panic on drop.
    }
}
