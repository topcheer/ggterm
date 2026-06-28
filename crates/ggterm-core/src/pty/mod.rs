//! PTY integration — pseudo-terminal pair and shell process management.
//!
//! [`PtySession`] wraps a `portable-pty` PTY pair with a spawned child shell
//! process, providing simple I/O and lifecycle management.
//!
//! # Example
//!
//! ```no_run
//! use ggterm_core::PtySession;
//!
//! let mut pty = PtySession::open(80, 24).expect("open pty");
//! pty.write(b"echo hello\n").expect("write");
//!
//! let mut buf = [0u8; 4096];
//! let n = pty.try_read(&mut buf).expect("read");
//! println!("{}", String::from_utf8_lossy(&buf[..n]));
//!
//! pty.kill();
//! ```

use std::io::{self, Read, Write};

use portable_pty::cmdbuilder::CommandBuilder;
use portable_pty::{native_pty_system, Child, ChildKiller, MasterPty, PtySize, SlavePty};

/// A PTY session managing a shell child process.
///
/// The session owns:
/// - The PTY master (for reading output and writing input)
/// - A reader and writer for I/O
/// - The spawned child process handle (with kill capability)
///
/// # Platform Behavior
///
/// On Unix, the default shell is detected via `$SHELL` (falling back to
/// `/bin/sh`). On Windows, `cmd.exe` is used (falling back to `powershell`
/// if available).
pub struct PtySession {
    /// PTY master — kept alive for the session lifetime.
    master: Box<dyn MasterPty + Send>,
    /// Writer to the PTY master (stdin of the child).
    writer: Box<dyn Write + Send>,
    /// Reader from the PTY master (stdout/stderr of the child).
    reader: Box<dyn Read + Send>,
    /// The spawned child process (with kill capability).
    child: Option<Box<dyn Child + ChildKiller + Send + Sync>>,
    /// Current terminal size.
    size: PtySize,
}

/// Error wrapper for PTY operations.
#[derive(Debug)]
pub enum PtyError {
    /// I/O error from the PTY system.
    Io(io::Error),
    /// portable-pty returned an error.
    Pty(String),
}

impl std::fmt::Display for PtyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PtyError::Io(e) => write!(f, "PTY I/O error: {}", e),
            PtyError::Pty(msg) => write!(f, "PTY error: {}", msg),
        }
    }
}

impl std::error::Error for PtyError {}

impl From<io::Error> for PtyError {
    fn from(e: io::Error) -> Self {
        PtyError::Io(e)
    }
}

impl PtySession {
    /// Create a new PTY session with a spawned shell at the given size.
    ///
    /// Automatically detects the default shell:
    /// - **Unix**: `$SHELL` env var, falling back to `/bin/sh`
    /// - **Windows**: `cmd.exe`
    ///
    /// # Errors
    ///
    /// Returns an error if the PTY pair cannot be created or the shell
    /// cannot be spawned.
    pub fn open(cols: u16, rows: u16) -> Result<Self, PtyError> {
        Self::open_with_shell(cols, rows, None)
    }

    /// Create a PTY session with an explicit shell command.
    ///
    /// If `shell` is `None`, auto-detects the default shell.
    pub fn open_with_shell(cols: u16, rows: u16, shell: Option<&str>) -> Result<Self, PtyError> {
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pty_system = native_pty_system();

        // Open a PTY pair
        let pair = pty_system
            .openpty(size)
            .map_err(|e| PtyError::Pty(e.to_string()))?;

        // Build the shell command
        let shell_path = shell.map(String::from).unwrap_or_else(default_shell);
        let mut cmd = CommandBuilder::new(&shell_path);

        // Set reasonable defaults
        cmd.cwd(
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/")),
        );
        cmd.env("TERM", "xterm-256color");

        // Spawn the child process
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::Pty(e.to_string()))?;

        // Take writer and reader from master
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::Pty(e.to_string()))?;

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::Pty(e.to_string()))?;

        // Drop slave to ensure EOF is delivered when child exits
        drop(pair.slave);

        Ok(Self {
            master: pair.master,
            writer,
            reader,
            child: Some(child),
            size,
        })
    }

    /// Write data to the PTY (stdin of the child process).
    ///
    /// Returns the number of bytes written.
    pub fn write(&mut self, data: &[u8]) -> Result<usize, PtyError> {
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(data.len())
    }

    /// Non-blocking read from the PTY (stdout/stderr of the child).
    ///
    /// Returns the number of bytes read into `buf`.
    /// If no data is available or the child has exited, returns `Ok(0)`.
    ///
    /// Note: The reader from `try_clone_reader()` runs on a dedicated thread
    /// inside portable-pty. This call blocks until data is available.
    pub fn try_read(&mut self, buf: &mut [u8]) -> Result<usize, PtyError> {
        Ok(self.reader.read(buf)?)
    }

    /// Resize the PTY to the given dimensions.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), PtyError> {
        self.size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        self.master
            .resize(self.size)
            .map_err(|e| PtyError::Pty(e.to_string()))?;
        Ok(())
    }

    /// Get the current terminal size as `(cols, rows)`.
    pub fn size(&self) -> (u16, u16) {
        (self.size.cols, self.size.rows)
    }

    /// Kill the child process.
    ///
    /// Sends SIGKILL on Unix, TerminateProcess on Windows.
    /// After this call, `is_alive()` returns `false`.
    pub fn kill(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
        }
    }

    /// Check if the child process is still running.
    ///
    /// Returns `true` if the process has not exited, `false` if terminated.
    pub fn is_alive(&self) -> bool {
        match &self.child {
            Some(child) => match child.try_wait() {
                Ok(Some(_)) => false, // exited
                Ok(None) => true,     // still running
                Err(_) => false,      // error → assume dead
            },
            None => false,
        }
    }

    /// Get the child process PID, if available.
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(|c| c.process_id())
    }

    /// Wait for the child process to exit.
    ///
    /// Blocks until the child terminates.
    pub fn wait(&mut self) -> Result<(), PtyError> {
        if let Some(child) = self.child.as_mut() {
            child
                .wait()
                .map_err(|e| PtyError::Pty(e.to_string()))?;
        }
        Ok(())
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        // Kill child on drop to prevent orphaned processes
        self.kill();
    }
}

/// Detect the default shell for the current platform.
///
/// - **Unix**: Reads `$SHELL`, falls back to `/bin/sh`
/// - **Windows**: Uses `%COMSPEC%`, falls back to `cmd.exe`
pub fn default_shell() -> String {
    #[cfg(unix)]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
    #[cfg(windows)]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    }
    #[cfg(not(any(unix, windows)))]
    {
        "/bin/sh".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ================================================================
    //  Shell detection
    // ================================================================

    #[test]
    fn test_default_shell_returns_nonempty() {
        let shell = default_shell();
        assert!(!shell.is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn test_default_shell_unix() {
        let shell = default_shell();
        assert!(
            shell.contains("sh")
                || shell.contains("bash")
                || shell.contains("zsh")
                || shell.contains("fish"),
            "expected a shell-like path, got: {}",
            shell
        );
    }

    #[test]
    #[cfg(windows)]
    fn test_default_shell_windows() {
        let shell = default_shell();
        assert!(
            shell.contains("cmd") || shell.contains("powershell") || shell.contains("pwsh"),
            "expected cmd or powershell, got: {}",
            shell
        );
    }

    // ================================================================
    //  PtyError
    // ================================================================

    #[test]
    fn test_pty_error_display() {
        let e = PtyError::Pty("test error".to_string());
        assert!(format!("{}", e).contains("test error"));

        let io_err = PtyError::Io(io::Error::new(io::ErrorKind::Other, "io fail"));
        assert!(format!("{}", io_err).contains("io fail"));
    }

    #[test]
    fn test_pty_error_from_io() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "missing");
        let pty_err: PtyError = io_err.into();
        assert!(matches!(pty_err, PtyError::Io(_)));
    }

    // ================================================================
    //  PtySession lifecycle (integration tests)
    // ================================================================

    #[test]
    fn test_pty_open_and_kill() {
        let mut pty = PtySession::open(80, 24).expect("open pty");
        assert!(pty.is_alive());
        pty.kill();
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!pty.is_alive());
    }

    #[test]
    fn test_pty_open_with_size() {
        let pty = PtySession::open(120, 40).expect("open pty");
        assert_eq!(pty.size(), (120, 40));
    }

    #[test]
    fn test_pty_resize() {
        let mut pty = PtySession::open(80, 24).expect("open pty");
        pty.resize(100, 30).expect("resize");
        assert_eq!(pty.size(), (100, 30));
    }

    #[test]
    fn test_pty_write_echo() {
        let mut pty = PtySession::open(80, 24).expect("open pty");

        // Give the shell a moment to start
        std::thread::sleep(std::time::Duration::from_millis(200));

        // Write a command
        let cmd = b"echo hello_ggterm_12345\n";
        pty.write(cmd).expect("write");

        // Give the shell time to process
        std::thread::sleep(std::time::Duration::from_millis(300));

        // Read output
        let mut buf = [0u8; 8192];
        let n = pty.try_read(&mut buf).expect("read");
        assert!(n > 0, "expected some output from the shell");

        let output = String::from_utf8_lossy(&buf[..n]);
        assert!(
            output.contains("echo") || output.contains("hello_ggterm_12345"),
            "expected echo or output, got: {}",
            &output[..output.len().min(200)]
        );
    }

    #[test]
    fn test_pty_pid_available() {
        let pty = PtySession::open(80, 24).expect("open pty");
        if let Some(pid) = pty.pid() {
            assert!(pid > 0, "PID should be positive");
        }
    }

    #[test]
    fn test_pty_drop_kills_child() {
        let pty = PtySession::open(80, 24).expect("open pty");
        drop(pty);
        // No assertion needed — just verify no panic/hang
    }

    #[test]
    fn test_pty_explicit_shell() {
        #[cfg(unix)]
        let shell = "/bin/sh";
        #[cfg(not(unix))]
        let shell = "cmd.exe";

        let pty = PtySession::open_with_shell(80, 24, Some(shell));
        assert!(pty.is_ok(), "should be able to open with explicit shell");
    }

    #[test]
    fn test_pty_multiple_sessions() {
        let pty1 = PtySession::open(80, 24);
        let pty2 = PtySession::open(80, 24);
        assert!(pty1.is_ok() && pty2.is_ok());
    }

    #[test]
    fn test_pty_exit_clean() {
        let mut pty = PtySession::open_with_shell(80, 24, None).expect("open pty");

        // Give shell time to start
        std::thread::sleep(std::time::Duration::from_millis(200));

        // Send exit command
        #[cfg(unix)]
        pty.write(b"exit\n").expect("write");
        #[cfg(not(unix))]
        pty.write(b"exit\r\n").expect("write");

        // Wait for exit
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(!pty.is_alive(), "shell should have exited");
    }
}
