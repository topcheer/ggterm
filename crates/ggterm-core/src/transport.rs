//! Terminal transport abstraction.
//!
//! Provides a unified interface for feeding data into the terminal engine,
//! regardless of whether the underlying transport is a local PTY (desktop)
//! or a remote SSH channel (mobile).
//!
//! Both [`crate::pty::PtySession`] and `ggterm_ssh::SshSession` implement
//! this trait, allowing the application layer to work with either transport
//! transparently.

/// A bidirectional byte transport for terminal I/O.
///
/// Implementations must be `Send` because the transport is typically read
/// from a background thread while the main thread writes to it.
pub trait TerminalTransport: Send {
    /// Read available output bytes from the transport (stdout/stderr).
    ///
    /// Returns an empty `Vec` if no data is immediately available.
    /// This is a non-blocking or short-timeout read.
    fn read(&mut self) -> Vec<u8>;

    /// Send input bytes (keystrokes) to the transport (stdin).
    fn write(&mut self, data: &[u8]);

    /// Notify the transport that the terminal size has changed.
    ///
    /// For PTY: sends `TIOCSWINSZ` ioctl.
    /// For SSH: sends `window-change` channel request.
    fn resize(&mut self, cols: usize, rows: usize);

    /// Whether the transport is still alive (process running / connection open).
    ///
    /// Takes `&mut self` because checking process status on some platforms
    /// (e.g. portable-pty's `Child::try_wait`) requires mutation.
    fn is_alive(&mut self) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A simple in-memory transport for testing.
    struct MockTransport {
        output: Vec<u8>,
        input: Vec<u8>,
        alive: bool,
        resize_calls: Vec<(usize, usize)>,
    }

    impl MockTransport {
        fn new(output: Vec<u8>) -> Self {
            Self {
                output,
                input: Vec::new(),
                alive: true,
                resize_calls: Vec::new(),
            }
        }
    }

    impl TerminalTransport for MockTransport {
        fn read(&mut self) -> Vec<u8> {
            std::mem::take(&mut self.output)
        }

        fn write(&mut self, data: &[u8]) {
            self.input.extend_from_slice(data);
        }

        fn resize(&mut self, cols: usize, rows: usize) {
            self.resize_calls.push((cols, rows));
        }

        fn is_alive(&mut self) -> bool {
            self.alive
        }
    }

    #[test]
    fn t_mock_read_returns_output() {
        let mut t = MockTransport::new(b"hello".to_vec());
        assert_eq!(t.read(), b"hello");
        // Second read returns empty (data consumed).
        assert!(t.read().is_empty());
    }

    #[test]
    fn t_mock_write_stores_input() {
        let mut t = MockTransport::new(vec![]);
        t.write(b"ls\n");
        assert_eq!(&t.input, b"ls\n");
    }

    #[test]
    fn t_mock_resize_records_calls() {
        let mut t = MockTransport::new(vec![]);
        t.resize(80, 24);
        t.resize(120, 40);
        assert_eq!(t.resize_calls, vec![(80, 24), (120, 40)]);
    }

    #[test]
    fn t_mock_is_alive_default() {
        let mut t = MockTransport::new(vec![]);
        assert!(t.is_alive());
    }

    #[test]
    fn t_mock_alive_toggle() {
        let mut t = MockTransport::new(vec![]);
        assert!(t.is_alive());
        t.alive = false;
        assert!(!t.is_alive());
    }

    #[test]
    fn t_mock_send_safe() {
        // Ensure MockTransport is Send (required by trait bound).
        fn assert_send<T: Send>(_: &T) {}
        let t = MockTransport::new(vec![]);
        assert_send(&t);
    }
}
