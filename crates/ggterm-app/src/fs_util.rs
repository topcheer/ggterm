/// Atomic file write utilities.
///
/// Writes to a temporary file first, then renames atomically.
/// POSIX `rename(2)` is atomic on the same filesystem, so the
/// destination file is never in a partially-written state.
use std::path::Path;

/// Write `content` to `path` atomically.
///
/// Creates parent directories if needed. Writes to `path.with_extension("tmp")`
/// first, then renames to `path`. Cleans up the temp file on rename failure.
pub fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

/// Write bytes to `path` atomically (for non-UTF-8 data).
pub fn write_bytes_atomic(path: &Path, content: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_atomic_roundtrip() {
        let dir = std::env::temp_dir();
        let path = dir.join("ggterm_test_atomic_write.toml");
        write_atomic(&path, "hello = world\n").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "hello = world\n");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_write_atomic_creates_parent() {
        let dir = std::env::temp_dir().join("ggterm_test_atomic_subdir");
        let path = dir.join("config.toml");
        write_atomic(&path, "key = value\n").unwrap();
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_bytes_atomic_roundtrip() {
        let dir = std::env::temp_dir();
        let path = dir.join("ggterm_test_atomic_bytes.bin");
        write_bytes_atomic(&path, &[0x00, 0xFF, 0x42]).unwrap();
        let content = std::fs::read(&path).unwrap();
        assert_eq!(content, vec![0x00, 0xFF, 0x42]);
        let _ = std::fs::remove_file(&path);
    }
}
