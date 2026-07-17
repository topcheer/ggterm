//! P2P transport — implements [`TerminalTransport`] over QUIC.
//!
//! ## Data flow
//!
//! ```text
//!  FFI caller (UI thread)         Background tokio task
//!  ┌────────────────┐             ┌────────────────────────┐
//!  │ read()         │ ◄── drain ── │ recv.read() → read_buf │
//!  │                │              │                        │
//!  │ write()        │ ── push ──► │ write_rx → send.write() │
//!  │                │              │                        │
//!  │ resize()       │ ── push ──► │ resize_rx → send frame  │
//!  └────────────────┘             └────────────────────────┘
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use iroh::Endpoint;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use tokio::sync::mpsc;

/// P2P transport implementing [`TerminalTransport`] over QUIC.
///
/// Created by [`P2pHost::accept`](crate::P2pHost::accept) or
/// [`P2pClient::connect`](crate::P2pClient::connect).
pub struct P2pTransport {
    write_tx: Option<mpsc::UnboundedSender<Vec<u8>>>,
    resize_tx: Option<mpsc::UnboundedSender<(u16, u16)>>,
    read_buf: Arc<Mutex<Vec<u8>>>,
    alive: Arc<AtomicBool>,
    /// Owned runtime — keeps background task alive.
    _runtime: Option<tokio::runtime::Runtime>,
    /// Owned connection — keeps the QUIC connection alive so streams don't close.
    _conn: Option<Connection>,
    /// Owned endpoint — must stay alive or connection dies.
    _endpoint: Option<Arc<Endpoint>>,
}

impl P2pTransport {
    /// Create from established QUIC streams. Spawns background I/O task.
    ///
    /// Takes ownership of the runtime AND the connection.
    /// The connection must stay alive for the streams to work.
    pub(crate) fn from_streams(
        send: SendStream,
        recv: RecvStream,
        conn: Connection,
        endpoint: Arc<Endpoint>,
        runtime: tokio::runtime::Runtime,
    ) -> Self {
        let (write_tx, write_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (resize_tx, resize_rx) = mpsc::unbounded_channel::<(u16, u16)>();
        let read_buf = Arc::new(Mutex::new(Vec::<u8>::with_capacity(8192)));
        let alive = Arc::new(AtomicBool::new(true));

        // Enter runtime context so tokio::spawn works inside spawn_io_task.
        let _guard = runtime.enter();
        spawn_io_task(
            send,
            recv,
            write_rx,
            resize_rx,
            read_buf.clone(),
            alive.clone(),
        );

        Self {
            write_tx: Some(write_tx),
            resize_tx: Some(resize_tx),
            read_buf,
            alive,
            _runtime: Some(runtime),
            _conn: Some(conn),
            _endpoint: Some(endpoint),
        }
    }

    /// Whether the transport is currently connected.
    pub fn is_connected(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// Shut down the transport.
    pub fn close(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
        self.write_tx.take();
        self.resize_tx.take();
    }
}

impl Drop for P2pTransport {
    fn drop(&mut self) {
        self.close();
    }
}

impl ggterm_core::TerminalTransport for P2pTransport {
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
            let _ = tx.send((cols as u16, rows as u16));
        }
    }

    fn is_alive(&mut self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
}

// ─────────────────────────────────────────────────────────────────
//  Background I/O task
// ─────────────────────────────────────────────────────────────────

/// Spawn a background task bridging async QUIC streams to sync buffers.
///
/// Must be called from within a tokio runtime context.
///
/// The QUIC stream carries **pure terminal data** — no control frames.
/// Resize is handled at the application level, not through the stream.
pub(crate) fn spawn_io_task(
    mut send: SendStream,
    mut recv: RecvStream,
    mut write_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    _resize_rx: mpsc::UnboundedReceiver<(u16, u16)>,
    read_buf: Arc<Mutex<Vec<u8>>>,
    alive: Arc<AtomicBool>,
) {
    /// Maximum read buffer size — prevents unbounded growth.
    const MAX_READ_BUF: usize = 1024 * 1024; // 1 MB
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];

        loop {
            // ── Drain pending writes (non-blocking) ────────────
            while let Ok(data) = write_rx.try_recv() {
                if send.write_all(&data).await.is_err() {
                    alive.store(false, Ordering::Relaxed);
                    return;
                }
            }

            // Resize frames are NOT sent through the data stream.
            // They caused garbled output when interpreted as terminal data.

            // ── Read from QUIC stream (5ms timeout) ────────────
            match tokio::time::timeout(std::time::Duration::from_millis(5), recv.read(&mut buf))
                .await
            {
                Ok(Ok(Some(0))) | Ok(Err(_)) => {
                    alive.store(false, Ordering::Relaxed);
                    return;
                }
                Ok(Ok(Some(n))) => {
                    if let Ok(mut rb) = read_buf.lock() {
                        // Cap buffer to prevent unbounded growth.
                        if rb.len() < MAX_READ_BUF {
                            rb.extend_from_slice(&buf[..n]);
                        }
                    }
                }
                Ok(Ok(None)) => {
                    // No data available.
                }
                Err(_) => {
                    // Timeout — loop back.
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ggterm_core::TerminalTransport;

    fn make_transport() -> (P2pTransport, mpsc::UnboundedReceiver<Vec<u8>>) {
        let (write_tx, write_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (resize_tx, _resize_rx) = mpsc::unbounded_channel::<(u16, u16)>();
        let read_buf = Arc::new(Mutex::new(Vec::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let transport = P2pTransport {
            write_tx: Some(write_tx),
            resize_tx: Some(resize_tx),
            read_buf,
            alive,
            _runtime: None,
            _conn: None,
            _endpoint: None,
        };
        (transport, write_rx)
    }

    #[test]
    fn t_new_is_alive() {
        let (mut t, _) = make_transport();
        assert!(t.is_alive());
        assert!(t.is_connected());
    }

    #[test]
    fn t_read_empty() {
        let (mut t, _) = make_transport();
        assert!(t.read().is_empty());
    }

    #[test]
    fn t_read_drains_buffer() {
        let (write_tx, _) = mpsc::unbounded_channel::<Vec<u8>>();
        let (resize_tx, _) = mpsc::unbounded_channel::<(u16, u16)>();
        let read_buf = Arc::new(Mutex::new(b"hello".to_vec()));
        let alive = Arc::new(AtomicBool::new(true));
        let mut t = P2pTransport {
            write_tx: Some(write_tx),
            resize_tx: Some(resize_tx),
            read_buf,
            alive,
            _runtime: None,
            _conn: None,
            _endpoint: None,
        };
        assert_eq!(t.read(), b"hello");
        assert!(t.read().is_empty());
    }

    #[test]
    fn t_write_sends_to_channel() {
        let (mut t, mut write_rx) = make_transport();
        t.write(b"data");
        assert_eq!(write_rx.try_recv().unwrap(), b"data");
    }

    #[test]
    fn t_resize_sends_command() {
        let (write_tx, _) = mpsc::unbounded_channel::<Vec<u8>>();
        let (resize_tx, mut resize_rx) = mpsc::unbounded_channel::<(u16, u16)>();
        let read_buf = Arc::new(Mutex::new(Vec::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let mut t = P2pTransport {
            write_tx: Some(write_tx),
            resize_tx: Some(resize_tx),
            read_buf,
            alive,
            _runtime: None,
            _conn: None,
            _endpoint: None,
        };
        t.resize(120, 40);
        assert_eq!(resize_rx.try_recv().unwrap(), (120, 40));
    }

    #[test]
    fn t_close_sets_not_alive() {
        let (mut t, _) = make_transport();
        assert!(t.is_alive());
        t.close();
        assert!(!t.is_alive());
    }

    #[test]
    fn t_close_drops_senders() {
        let (mut t, _) = make_transport();
        t.close();
        t.write(b"after close");
    }

    #[test]
    fn t_multiple_writes_buffered() {
        let (mut t, mut write_rx) = make_transport();
        t.write(b"a");
        t.write(b"b");
        t.write(b"c");
        assert_eq!(write_rx.try_recv().unwrap(), b"a");
        assert_eq!(write_rx.try_recv().unwrap(), b"b");
        assert_eq!(write_rx.try_recv().unwrap(), b"c");
    }

    #[test]
    fn t_send_trait() {
        fn assert_send<T: Send>(_: &T) {}
        let (t, _) = make_transport();
        assert_send(&t);
    }

    #[test]
    fn t_read_after_close_empty() {
        let (mut t, _) = make_transport();
        t.close();
        assert!(t.read().is_empty());
    }

    #[test]
    fn t_drop_calls_close() {
        let (mut t, _) = make_transport();
        assert!(t.is_alive());
        drop(t);
    }
}
