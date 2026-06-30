//! SSH remote transport for ggterm.
//!
//! Provides [`SshSession`] — an SSH client implementing
//! [`TerminalTransport`](ggterm_core::TerminalTransport) for remote
//! terminal sessions.
//!
//! # Example
//!
//! ```no_run
//! use ggterm_ssh::SshSession;
//! use ggterm_core::TerminalTransport;
//!
//! let mut session = SshSession::connect("example.com", 22, "user", "pass").unwrap();
//! session.write(b"ls -la\n");
//! let data = session.read();
//! ```

pub mod error;

pub use error::SshError;

use std::path::Path;
use std::sync::Arc;

use russh::client::{self};
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg};
use russh::{Channel, ChannelMsg, Disconnect};

// ─────────────────────────────────────────────────────────────────
//  Client handler
// ─────────────────────────────────────────────────────────────────

/// russh client handler — accepts all server keys.
/// TODO: implement known_hosts verification for production use.
struct ClientHandler;

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        // Accept all server keys for now.
        Ok(true)
    }
}

// ─────────────────────────────────────────────────────────────────
//  SshSession
// ─────────────────────────────────────────────────────────────────

/// An active SSH terminal session.
///
/// Wraps a russh connection + channel, bridging async SSH operations
/// to the synchronous [`TerminalTransport`] trait via an internal
/// tokio runtime.
///
/// On drop, the session disconnects automatically.
pub struct SshSession {
    /// Dedicated tokio runtime for all async operations.
    runtime: tokio::runtime::Runtime,
    /// SSH client handle for disconnection.
    handle: Option<client::Handle<ClientHandler>>,
    /// The SSH channel for PTY I/O.
    channel: Option<Channel<client::Msg>>,
    /// Whether the session is alive.
    alive: bool,
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

        let mut handle = runtime
            .block_on(client::connect(
                Arc::new(client::Config::default()),
                (host, port),
                ClientHandler,
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

        Ok(Self {
            runtime,
            handle: Some(handle),
            channel: Some(channel),
            alive: true,
        })
    }

    /// Explicitly close the SSH session and disconnect.
    pub fn close(&mut self) {
        if self.alive {
            self.alive = false;
            if let Some(ref channel) = self.channel {
                let _ = self.runtime.block_on(channel.close());
            }
            self.channel = None;
            if let Some(handle) = self.handle.take() {
                let _ = self.runtime.block_on(handle.disconnect(
                    Disconnect::ByApplication,
                    "ggterm disconnect",
                    "en",
                ));
            }
        }
    }
}

impl Drop for SshSession {
    fn drop(&mut self) {
        self.close();
    }
}

// ─────────────────────────────────────────────────────────────────
//  TerminalTransport implementation (P7-B)
// ─────────────────────────────────────────────────────────────────

impl ggterm_core::TerminalTransport for SshSession {
    fn read(&mut self) -> Vec<u8> {
        if !self.alive {
            return Vec::new();
        }
        let Some(ref mut channel) = self.channel else {
            return Vec::new();
        };
        // Read one message's worth of data (non-blocking-ish via block_on).
        self.runtime.block_on(async {
            let mut out = Vec::new();
            if let Some(msg) = channel.wait().await {
                match msg {
                    ChannelMsg::Data { ref data } => out.extend_from_slice(data),
                    ChannelMsg::ExtendedData { ref data, .. } => out.extend_from_slice(data),
                    ChannelMsg::Eof | ChannelMsg::Close => {}
                    _ => {}
                }
            }
            out
        })
    }

    fn write(&mut self, data: &[u8]) {
        if !self.alive {
            return;
        }
        let Some(ref channel) = self.channel else {
            return;
        };
        let _ = self.runtime.block_on(channel.data(data));
    }

    fn resize(&mut self, cols: usize, rows: usize) {
        if !self.alive {
            return;
        }
        let Some(ref channel) = self.channel else {
            return;
        };
        let _ = self
            .runtime
            .block_on(channel.window_change(cols as u32, rows as u32, 0, 0));
    }

    fn is_alive(&mut self) -> bool {
        self.alive
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
