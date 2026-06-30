//! P25-E: Session Recording (asciinema v2 format)
//!
//! Records terminal output with timestamps for later playback.
//! Output format is [asciinema v2 cast](https://docs.asciinema.org/manual/cli/usage/).
//!
//! ## File Format
//!
//! ```json
//! {"version": 2, "width": 80, "height": 24, "timestamp": 1234567890, "env": {"SHELL": "/bin/zsh", "TERM": "xterm-256color"}}
//! [0.001234, "o", "Hello, World!\r\n"]
//! [0.567890, "o", "$ "]
//! ```
//!
//! Line 1 is the header (JSON object), each subsequent line is a 3-element array:
//! `[timestamp_seconds, event_type, data]`.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Recording header metadata (asciinema v2).
#[derive(Debug, Clone)]
pub struct RecordingHeader {
    /// Terminal width in columns.
    pub width: u32,
    /// Terminal height in rows.
    pub height: u32,
    /// Unix timestamp of recording start.
    pub timestamp: u64,
    /// Environment variables recorded in the header.
    pub env: Vec<(String, String)>,
}

impl RecordingHeader {
    /// Create a new recording header with the given terminal dimensions.
    pub fn new(width: u32, height: u32) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Self {
            width,
            height,
            timestamp,
            env: vec![
                (
                    "SHELL".to_string(),
                    std::env::var("SHELL").unwrap_or_default(),
                ),
                ("TERM".to_string(), "xterm-256color".to_string()),
            ],
        }
    }

    /// Set the shell environment variable.
    pub fn with_shell(mut self, shell: &str) -> Self {
        self.env.retain(|(k, _)| k != "SHELL");
        self.env.push(("SHELL".to_string(), shell.to_string()));
        self
    }

    /// Set the TERM environment variable.
    pub fn with_term(mut self, term: &str) -> Self {
        self.env.retain(|(k, _)| k != "TERM");
        self.env.push(("TERM".to_string(), term.to_string()));
        self
    }

    /// Serialize the header to a JSON string (single line, no trailing newline).
    pub fn to_json(&self) -> String {
        let env_str: Vec<String> = self
            .env
            .iter()
            .map(|(k, v)| {
                format!(
                    "\"{}\": \"{}\"",
                    k,
                    v.replace('\\', "\\\\").replace('"', "\\\"")
                )
            })
            .collect();

        format!(
            r#"{{"version": 2, "width": {}, "height": {}, "timestamp": {}, "env": {{{}}}}}"#,
            self.width,
            self.height,
            self.timestamp,
            env_str.join(", ")
        )
    }
}

/// Session recorder that writes terminal output in asciinema v2 format.
///
/// Call `start()` to begin recording, `feed()` for each chunk of PTY data,
/// and `stop()` to finalize the file.
pub struct SessionRecorder {
    /// The output file writer (None when not recording).
    writer: Option<BufWriter<File>>,
    /// Recording start instant (for relative timestamps).
    start: Option<Instant>,
    /// Total bytes recorded.
    bytes_written: u64,
    /// Total events (lines) recorded.
    events_written: u64,
    /// Whether recording is active.
    active: bool,
}

impl Default for SessionRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionRecorder {
    /// Create a new inactive recorder.
    pub fn new() -> Self {
        Self {
            writer: None,
            start: None,
            bytes_written: 0,
            events_written: 0,
            active: false,
        }
    }

    /// Check if recording is active.
    pub fn is_recording(&self) -> bool {
        self.active
    }

    /// Total bytes recorded so far.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Total events (PTY data chunks) recorded so far.
    pub fn events_written(&self) -> u64 {
        self.events_written
    }

    /// Start recording to a file.
    ///
    /// Writes the asciinema v2 header immediately.
    /// If a recording is already active, it will be stopped first.
    pub fn start(&mut self, path: &Path, header: &RecordingHeader) -> std::io::Result<()> {
        // Stop any existing recording
        if self.active {
            self.stop()?;
        }

        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);

        // Write header line
        let header_json = header.to_json();
        writeln!(writer, "{}", header_json)?;

        self.writer = Some(writer);
        self.start = Some(Instant::now());
        self.bytes_written = header_json.len() as u64 + 1; // +1 for newline
        self.events_written = 0;
        self.active = true;

        log::info!("Recording started: {:?}", path);
        Ok(())
    }

    /// Feed a chunk of PTY output data into the recording.
    ///
    /// Each call writes one event line: `[timestamp, "o", "data"]`.
    /// Data is escaped as a JSON string.
    ///
    /// No-op if recording is not active.
    pub fn feed(&mut self, data: &[u8]) -> std::io::Result<()> {
        if !self.active {
            return Ok(());
        }

        let writer = self.writer.as_mut().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotConnected, "no active recording")
        })?;

        let elapsed = self.start.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);

        // Escape data as JSON string value
        let escaped = escape_json_string(data);

        // Write: [timestamp, "o", "escaped_data"]
        writeln!(writer, "[{:.6}, \"o\", \"{}\"]", elapsed, escaped)?;

        self.bytes_written += data.len() as u64;
        self.events_written += 1;

        Ok(())
    }

    /// Stop recording and flush the file.
    ///
    /// No-op if not recording.
    pub fn stop(&mut self) -> std::io::Result<()> {
        if let Some(mut writer) = self.writer.take() {
            writer.flush()?;
            log::info!(
                "Recording stopped: {} events, {} bytes",
                self.events_written,
                self.bytes_written
            );
        }
        self.active = false;
        self.start = None;
        Ok(())
    }
}

impl Drop for SessionRecorder {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Escape raw bytes as a JSON string value (without surrounding quotes).
///
/// Handles: `"`, `\`, control chars (\n, \r, \t, etc.), and non-UTF8.
fn escape_json_string(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len());
    for &b in data {
        match b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\r\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0x00..=0x1F => {
                out.push_str(&format!("\\u{:04x}", b));
            }
            0x20..=0x7E => out.push(b as char),
            _ => {
                // Non-ASCII byte — encode as UTF-8 if valid, else \u00XX
                out.push_str(&format!("\\u{:04x}", b));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_recording_header_json() {
        let header = RecordingHeader::new(80, 24);
        let json = header.to_json();
        assert!(json.contains("\"version\": 2"));
        assert!(json.contains("\"width\": 80"));
        assert!(json.contains("\"height\": 24"));
        assert!(json.contains("\"timestamp\""));
        assert!(json.contains("\"env\""));
    }

    #[test]
    fn t_recording_header_custom_env() {
        let header = RecordingHeader::new(120, 40)
            .with_shell("/bin/fish")
            .with_term("screen-256color");
        let json = header.to_json();
        assert!(json.contains("/bin/fish"));
        assert!(json.contains("screen-256color"));
    }

    #[test]
    fn t_recorder_new_not_active() {
        let r = SessionRecorder::new();
        assert!(!r.is_recording());
        assert_eq!(r.bytes_written(), 0);
        assert_eq!(r.events_written(), 0);
    }

    #[test]
    fn t_recorder_start_stop() {
        let path = std::env::temp_dir().join("ggterm_test_recording.cast");
        let mut recorder = SessionRecorder::new();
        let header = RecordingHeader::new(80, 24);

        recorder.start(&path, &header).unwrap();
        assert!(recorder.is_recording());
        assert!(recorder.bytes_written() > 0);

        recorder.stop().unwrap();
        assert!(!recorder.is_recording());

        // Verify file exists and has header
        let content = std::fs::read_to_string(&path).unwrap();
        let first_line = content.lines().next().unwrap();
        assert!(first_line.contains("\"version\": 2"));

        // Clean up
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn t_recorder_feed_writes_events() {
        let path = std::env::temp_dir().join("ggterm_test_feed.cast");
        let mut recorder = SessionRecorder::new();
        let header = RecordingHeader::new(80, 24);

        recorder.start(&path, &header).unwrap();
        recorder.feed(b"Hello, World!\r\n").unwrap();
        recorder.feed(b"$ ").unwrap();
        recorder.stop().unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        // Header + 2 events
        assert_eq!(lines.len(), 3);

        // Check event format
        assert!(lines[1].starts_with("["));
        assert!(lines[1].contains("\"o\""));
        assert!(lines[1].contains("Hello, World!"));

        // Clean up
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn t_recorder_feed_when_inactive() {
        let mut recorder = SessionRecorder::new();
        // Should be a no-op
        recorder.feed(b"data").unwrap();
        assert_eq!(recorder.events_written(), 0);
    }

    #[test]
    fn t_recorder_stop_when_inactive() {
        let mut recorder = SessionRecorder::new();
        // Should be a no-op
        recorder.stop().unwrap();
        assert!(!recorder.is_recording());
    }

    #[test]
    fn t_escape_json_string_basic() {
        let escaped = escape_json_string(b"Hello");
        assert_eq!(escaped, "Hello");
    }

    #[test]
    fn t_escape_json_string_quotes() {
        let escaped = escape_json_string(b"say \"hi\"");
        assert_eq!(escaped, "say \\\"hi\\\"");
    }

    #[test]
    fn t_escape_json_string_backslash() {
        let escaped = escape_json_string(b"path\\to\\file");
        assert_eq!(escaped, "path\\\\to\\\\file");
    }

    #[test]
    fn t_escape_json_string_newline() {
        let escaped = escape_json_string(b"line1\nline2");
        assert_eq!(escaped, "line1\\r\\nline2");
    }

    #[test]
    fn t_escape_json_string_control_chars() {
        let escaped = escape_json_string(&[0x01, 0x02]);
        assert!(escaped.contains("\\u0001"));
        assert!(escaped.contains("\\u0002"));
    }

    #[test]
    fn t_recorder_restart() {
        let path = std::env::temp_dir().join("ggterm_test_restart.cast");
        let mut recorder = SessionRecorder::new();
        let header = RecordingHeader::new(80, 24);

        // First recording
        recorder.start(&path, &header).unwrap();
        recorder.feed(b"first").unwrap();
        recorder.stop().unwrap();

        // Second recording (overwrites file)
        recorder.start(&path, &header).unwrap();
        recorder.feed(b"second").unwrap();
        recorder.stop().unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2); // header + 1 event
        assert!(lines[1].contains("second"));
        assert!(!lines[1].contains("first"));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn t_recorder_counts() {
        let path = std::env::temp_dir().join("ggterm_test_counts.cast");
        let mut recorder = SessionRecorder::new();
        let header = RecordingHeader::new(80, 24);

        recorder.start(&path, &header).unwrap();
        recorder.feed(b"AAAA").unwrap();
        recorder.feed(b"BB").unwrap();
        recorder.feed(b"CCCCCC").unwrap();

        assert_eq!(recorder.events_written(), 3);
        assert!(recorder.bytes_written() > 12); // header + 12 bytes of data

        recorder.stop().unwrap();
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn t_recorder_timestamps_increment() {
        let path = std::env::temp_dir().join("ggterm_test_ts.cast");
        let mut recorder = SessionRecorder::new();
        let header = RecordingHeader::new(80, 24);

        recorder.start(&path, &header).unwrap();
        recorder.feed(b"event1").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        recorder.feed(b"event2").unwrap();
        recorder.stop().unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();

        // Parse timestamps
        let ts1: f64 = lines[1]
            .split(',')
            .next()
            .unwrap()
            .trim_start_matches('[')
            .parse()
            .unwrap();
        let ts2: f64 = lines[2]
            .split(',')
            .next()
            .unwrap()
            .trim_start_matches('[')
            .parse()
            .unwrap();

        assert!(ts2 > ts1, "second event should have later timestamp");
        std::fs::remove_file(&path).ok();
    }
}
