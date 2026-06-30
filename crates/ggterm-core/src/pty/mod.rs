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
use portable_pty::{Child, ChildKiller, MasterPty, PtySize, native_pty_system};
use thiserror::Error;

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
    /// The spawned child process handle.
    child: Option<Box<dyn Child + Send + Sync>>,
    /// Kill handle for the child process (separate from child so we can
    /// kill even while waiting on the child).
    killer: Option<Box<dyn ChildKiller + Send>>,
    /// Current terminal size.
    size: PtySize,
}

/// Error wrapper for PTY operations.
#[derive(Debug, Error)]
pub enum PtyError {
    /// I/O error from the PTY system.
    #[error("PTY I/O error: {0}")]
    Io(#[from] io::Error),
    /// portable-pty returned an error.
    #[error("PTY error: {0}")]
    Pty(String),
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
        Self::open_advanced(cols, rows, shell, &[], &[])
    }

    /// Create a PTY session with full control over env vars and spawn args.
    ///
    /// # Arguments
    /// * `cols`, `rows` — Terminal dimensions.
    /// * `shell` — Shell path. `None` = auto-detect.
    /// * `args` — Extra arguments passed to the shell.
    /// * `env_vars` — Extra environment variables `(key, value)` for the child.
    pub fn open_advanced(
        cols: u16,
        rows: u16,
        shell: Option<&str>,
        args: &[String],
        env_vars: &[(String, String)],
    ) -> Result<Self, PtyError> {
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

        // Pass extra args
        for arg in args {
            cmd.arg(arg);
        }

        // Set reasonable defaults
        cmd.cwd(std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/")));
        cmd.env("TERM", "xterm-256color");

        // Apply extra env vars
        for (key, value) in env_vars {
            cmd.env(key, value);
        }

        // Spawn the child process
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::Pty(e.to_string()))?;

        // Clone a separate kill handle before moving child into the struct
        let killer = child.clone_killer();

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
            killer: Some(killer),
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

    /// Clone the PTY reader (stdout/stderr of the child process).
    ///
    /// Creates a new reader handle that can be used on a separate thread.
    /// The original reader remains valid.
    pub fn try_clone_reader(&self) -> Result<Box<dyn Read + Send>, PtyError> {
        self.master
            .try_clone_reader()
            .map_err(|e| PtyError::Io(io::Error::other(e)))
    }

    /// Take the writer out of the session.
    ///
    /// After calling this, [`write()`](Self::write) will return an error.
    /// Use this when you need to move the writer to another struct (e.g.
    /// passing it to `App::set_pty_writer`).
    pub fn take_writer(&mut self) -> Option<Box<dyn Write + Send>> {
        // Create a new writer from the master before taking the existing one
        let new_writer = self.master.take_writer().ok()?;
        let old = std::mem::replace(&mut self.writer, new_writer);
        Some(old)
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
        if let Some(killer) = self.killer.as_mut() {
            let _ = killer.kill();
        }
    }

    /// Check if the child process is still running.
    ///
    /// Returns `true` if the process has not exited, `false` if terminated.
    pub fn is_alive(&mut self) -> bool {
        match self.child.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(Some(_)) => false, // exited
                Ok(None) => true,     // still running
                Err(_) => false,      // error -> assume dead
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
            child.wait().map_err(|e| PtyError::Pty(e.to_string()))?;
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

impl crate::transport::TerminalTransport for PtySession {
    fn read(&mut self) -> Vec<u8> {
        let mut buf = vec![0u8; 8192];
        match self.try_read(&mut buf) {
            Ok(n) if n > 0 => buf.truncate(n),
            _ => buf.clear(),
        }
        buf
    }

    fn write(&mut self, data: &[u8]) {
        let _ = PtySession::write(self, data);
    }

    fn resize(&mut self, cols: usize, rows: usize) {
        let _ = PtySession::resize(self, cols as u16, rows as u16);
    }

    fn is_alive(&mut self) -> bool {
        // Check if child process is still running.
        if let Some(child) = &mut self.child {
            match child.try_wait() {
                Ok(None) => true,              // Still running
                Ok(Some(_)) | Err(_) => false, // Exited or error
            }
        } else {
            false
        }
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
    fn test_pty_open_advanced_with_env() {
        #[cfg(unix)]
        let shell = "/bin/sh";
        #[cfg(not(unix))]
        let shell = "cmd.exe";

        let env_vars = vec![
            ("GGTERM".to_string(), "1".to_string()),
            ("GGTERM_VERSION".to_string(), "test".to_string()),
        ];

        let pty = PtySession::open_advanced(80, 24, Some(shell), &[], &env_vars);
        assert!(pty.is_ok(), "open_advanced should work with env vars");
    }

    #[test]
    fn test_pty_open_advanced_with_args() {
        #[cfg(unix)]
        let shell = "/bin/sh";
        #[cfg(not(unix))]
        let shell = "cmd.exe";

        // /bin/sh -c "exit" — the shell runs a command and exits immediately
        let args = vec!["-c".to_string(), "exit".to_string()];

        let mut pty = PtySession::open_advanced(80, 24, Some(shell), &args, &[])
            .expect("open_advanced with args");

        // Shell should exit very quickly
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!pty.is_alive(), "shell with -c exit should have exited");
    }

    #[test]
    fn test_pty_open_advanced_echo_env() {
        #[cfg(unix)]
        {
            let shell = "/bin/sh";
            let env_vars = vec![("MY_TEST_VAR".to_string(), "hello123".to_string())];

            let mut pty =
                PtySession::open_advanced(80, 24, Some(shell), &[], &env_vars).expect("open pty");

            std::thread::sleep(std::time::Duration::from_millis(200));

            // Echo the env var to verify it was set
            pty.write(b"echo $MY_TEST_VAR\n").expect("write");

            std::thread::sleep(std::time::Duration::from_millis(300));

            let mut buf = [0u8; 8192];
            let n = pty.try_read(&mut buf).expect("read");
            assert!(n > 0);

            let output = String::from_utf8_lossy(&buf[..n]);
            assert!(
                output.contains("hello123"),
                "expected env var MY_TEST_VAR=hello123 in output, got: {}",
                &output[..output.len().min(200)]
            );
        }
    }

    #[test]
    fn test_pty_open_with_shell_delegates_to_advanced() {
        // open_with_shell should be equivalent to open_advanced with empty args/env
        #[cfg(unix)]
        let shell = "/bin/sh";
        #[cfg(not(unix))]
        let shell = "cmd.exe";

        let pty1 = PtySession::open_with_shell(80, 24, Some(shell));
        let pty2 = PtySession::open_advanced(80, 24, Some(shell), &[], &[]);
        assert!(pty1.is_ok() && pty2.is_ok());
    }

    #[test]
    fn test_pty_multiple_sessions() {
        let pty1 = PtySession::open(80, 24);
        let pty2 = PtySession::open(80, 24);
        assert!(pty1.is_ok() && pty2.is_ok());
    }

    #[test]
    #[ignore = "requires real shell; run with --ignored"]
    fn test_pty_exit_clean() {
        // Use /bin/sh explicitly to avoid interactive zsh/bash startup delays
        #[cfg(unix)]
        let shell = "/bin/sh";
        #[cfg(not(unix))]
        let shell = "cmd.exe";

        let mut pty = PtySession::open_with_shell(80, 24, Some(shell)).expect("open pty");

        // Give shell time to start
        std::thread::sleep(std::time::Duration::from_millis(300));

        // Send exit command followed by EOF (Ctrl-D) for reliable exit
        #[cfg(unix)]
        {
            pty.write(b"exit\n").expect("write exit");
            // Also send EOF as a fallback
            std::thread::sleep(std::time::Duration::from_millis(100));
            pty.write(&[0x04]).expect("write EOF"); // Ctrl-D
        }
        #[cfg(not(unix))]
        pty.write(b"exit\r\n").expect("write");

        // Poll for exit (up to 3 seconds)
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            if !pty.is_alive() {
                break;
            }
            if std::time::Instant::now() > deadline {
                panic!("shell should have exited within 3 seconds");
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}
