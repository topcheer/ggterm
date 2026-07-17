//! Local shell transport for Android.
//!
//! Uses `forkpty()` to spawn `/system/bin/sh` inside a pseudo-terminal.
//! This provides a fully interactive local shell on Android — no root,
//! no proot, no external app dependencies.
//!
//! On Android, the shell binary is at `/system/bin/sh` (MKSH, a Korn shell
//! variant). Environment variables like `HOME`, `PATH`, and `TERM` are set
//! to give a usable interactive session within the app's sandbox.

use ggterm_core::TerminalTransport;
use std::ffi::CString;
use std::io;
use std::os::unix::io::RawFd;

/// Errors from local shell operations.
#[derive(Debug)]
pub enum LocalShellError {
    Io(io::Error),
    Forkpty(i32),
    Exec(io::Error),
}

impl std::fmt::Display for LocalShellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LocalShellError::Io(e) => write!(f, "I/O error: {e}"),
            LocalShellError::Forkpty(errno) => write!(f, "forkpty() failed (errno={errno})"),
            LocalShellError::Exec(e) => write!(f, "exec failed: {e}"),
        }
    }
}

impl std::error::Error for LocalShellError {}

/// A local shell session backed by a PTY.
///
/// Created via [`LocalShellTransport::connect()`]. Uses `forkpty()` on
/// Android to run `/system/bin/sh` inside a PTY pair. The master fd is
/// used for non-blocking I/O.
pub struct LocalShellTransport {
    /// Master PTY file descriptor (read + write).
    master_fd: RawFd,
    /// Child PID (0 = already exited or unknown).
    child_pid: i32,
    /// Read buffer.
    read_buf: Vec<u8>,
    /// Whether the session is alive.
    alive: bool,
}

impl LocalShellTransport {
    /// Spawn a local shell using `forkpty()`.
    ///
    /// On Android, this runs `/system/bin/sh` with basic environment setup.
    pub fn connect(cols: usize, rows: usize) -> Result<Self, LocalShellError> {
        // Ensure cols/rows are valid.
        let cols = cols.max(1).min(500) as u16;
        let rows = rows.max(1).min(200) as u16;

        let (master_fd, child_pid) = unsafe { forkpty_and_exec(cols, rows)? };

        Ok(Self {
            master_fd,
            child_pid,
            read_buf: vec![0u8; 8192],
            alive: true,
        })
    }

    /// Try to read output from the shell (non-blocking).
    fn try_read(&mut self) -> io::Result<Vec<u8>> {
        // Set master fd to non-blocking.
        set_nonblocking(self.master_fd)?;

        let mut total = Vec::new();
        loop {
            let buf = &mut self.read_buf;
            match unsafe {
                libc::read(
                    self.master_fd,
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                )
            } {
                n if n > 0 => {
                    total.extend_from_slice(&buf[..n as usize]);
                }
                0 => break, // EOF
                _ => {
                    // n < 0: retry on EINTR (signal interrupted), break on EAGAIN/EWOULDBLOCK.
                    let err = io::Error::last_os_error();
                    if err.raw_os_error() == Some(libc::EINTR) {
                        continue;
                    }
                    break;
                }
            }
        }
        // Restore blocking.
        set_blocking(self.master_fd)?;
        Ok(total)
    }

    /// Write input to the shell's stdin.
    fn try_write(&mut self, data: &[u8]) -> io::Result<()> {
        let mut written = 0;
        while written < data.len() {
            let n = unsafe {
                libc::write(
                    self.master_fd,
                    data[written..].as_ptr() as *const libc::c_void,
                    data.len() - written,
                )
            };
            if n <= 0 {
                let err = io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue; // Retry on signal interruption.
                }
                return Err(err);
            }
            written += n as usize;
        }
        Ok(())
    }

    /// Send window resize signal (TIOCSWINSZ).
    fn do_resize(&self, cols: usize, rows: usize) {
        let ws = libc::winsize {
            ws_row: rows as u16,
            ws_col: cols as u16,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe {
            libc::ioctl(self.master_fd, libc::TIOCSWINSZ, &ws);
        }
    }

    /// Check if child process is still alive.
    fn check_alive(&mut self) -> bool {
        if !self.alive {
            return false;
        }
        // Try non-blocking waitpid to check if child exited.
        let mut status: libc::c_int = 0;
        let ret = unsafe { libc::waitpid(self.child_pid, &mut status, libc::WNOHANG) };
        if ret == self.child_pid {
            // Child exited.
            self.alive = false;
            false
        } else if ret == -1 {
            // waitpid error (ECHILD = already reaped, EINVAL, etc.).
            // Child is gone — treat as not alive.
            self.alive = false;
            false
        } else {
            // ret == 0: child still running.
            true
        }
    }
}

impl Drop for LocalShellTransport {
    fn drop(&mut self) {
        if self.master_fd >= 0 {
            unsafe { libc::close(self.master_fd) };
        }
        // Reap child process to prevent zombie.
        if self.child_pid > 0 {
            unsafe {
                let mut status: libc::c_int = 0;
                libc::waitpid(self.child_pid, &mut status, libc::WNOHANG);
            }
        }
    }
}

impl TerminalTransport for LocalShellTransport {
    fn read(&mut self) -> Vec<u8> {
        match self.try_read() {
            Ok(data) => data,
            Err(_) => {
                self.alive = false;
                Vec::new()
            }
        }
    }

    fn write(&mut self, data: &[u8]) {
        let _ = self.try_write(data);
    }

    fn resize(&mut self, cols: usize, rows: usize) {
        self.do_resize(cols, rows);
    }

    fn is_alive(&mut self) -> bool {
        self.check_alive()
    }
}

// ── Helper functions ──────────────────────────────────────────────────

/// Set a file descriptor to non-blocking mode.
fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let ret = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Set a file descriptor back to blocking mode.
fn set_blocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let ret = unsafe { libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK) };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Fork a child process inside a PTY and exec the shell.
///
/// Returns the master fd and child PID on success.
unsafe fn forkpty_and_exec(cols: u16, rows: u16) -> Result<(RawFd, libc::pid_t), LocalShellError> {
    let mut master: libc::c_int = -1;
    let mut winsize = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let mut termios_ptr: *mut libc::termios = std::ptr::null_mut();

    // forkpty is a glibc/Bionic convenience function.
    let pid = libc::forkpty(&mut master, std::ptr::null_mut(), termios_ptr, &mut winsize);

    if pid < 0 {
        let errno = io::Error::last_os_error().raw_os_error().unwrap_or(-1);
        return Err(LocalShellError::Forkpty(errno));
    }

    if pid == 0 {
        // ── Child process ──
        // Set up environment for an interactive shell.

        let home = home_dir();
        let home_c = CString::new(home.to_string_lossy().to_string())
            .expect("HOME path should not contain null bytes");
        libc::setenv(b"HOME\0".as_ptr() as *const _, home_c.as_ptr(), 1);

        let path = CString::new("/system/bin:/system/xbin:/data/data/com.ggterm.ggterm/files/bin")
            .expect("PATH literal should not contain null bytes");
        libc::setenv(b"PATH\0".as_ptr() as *const _, path.as_ptr(), 1);

        let term = CString::new("xterm-256color").expect("TERM literal is valid");
        libc::setenv(b"TERM\0".as_ptr() as *const _, term.as_ptr(), 1);

        let shell_name = CString::new("sh").expect("SHELL literal is valid");
        libc::setenv(b"SHELL\0".as_ptr() as *const _, shell_name.as_ptr(), 1);

        // Change to HOME directory.
        let home_c2 = CString::new(home.to_string_lossy().to_string())
            .expect("HOME path should not contain null bytes");
        libc::chdir(home_c2.as_ptr());

        // Exec /system/bin/sh
        let sh_path = CString::new("/system/bin/sh").expect("path literal is valid");
        let argv = [sh_path.as_ptr(), std::ptr::null()];

        libc::execv(sh_path.as_ptr(), argv.as_ptr());

        // If execv returns, it failed.
        // Try /bin/sh as fallback (some Android environments).
        let sh2 = CString::new("/bin/sh").expect("path literal is valid");
        let argv2 = [sh2.as_ptr(), std::ptr::null()];
        libc::execv(sh2.as_ptr(), argv2.as_ptr());

        // Ultimate fallback — exit.
        libc::_exit(1);
    }

    // ── Parent process ──
    Ok((master, pid))
}

/// Get the app's private home directory on Android.
fn home_dir() -> std::path::PathBuf {
    // Android apps get their data at /data/data/<package>/files
    // The ANDROID_DATA or HOME env var may be set by the runtime.
    if let Some(home) = std::env::var_os("HOME") {
        return std::path::PathBuf::from(home);
    }
    // Fallback: try common Android paths.
    let candidates = [
        "/data/data/com.ggterm.ggterm/files",
        "/data/data/com.example.ggterm/files",
        "/sdcard",
        "/tmp",
    ];
    for c in &candidates {
        let p = std::path::Path::new(c);
        if p.exists() {
            return p.to_path_buf();
        }
    }
    std::path::PathBuf::from("/tmp")
}
