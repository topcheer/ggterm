//! P28-E: File preview overlay for drag & drop.
//!
//! When a file is dragged over the terminal window, shows a preview
//! card with the file icon, name, size, and type.

use std::path::Path;

/// Maximum file size to show full preview (1 MB).
const MAX_PREVIEW_SIZE: u64 = 1_048_576;

/// File category for icon selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileCategory {
    /// Source code (.rs, .go, .py, .js, .ts, etc.)
    Code,
    /// Text document (.txt, .md, .log)
    Text,
    /// Image (.png, .jpg, .gif, .webp, .svg)
    Image,
    /// Video (.mp4, .mov, .avi)
    Video,
    /// Audio (.mp3, .wav, .flac)
    Audio,
    /// Archive (.zip, .tar.gz, .7z)
    Archive,
    /// Directory
    Directory,
    /// Executable / binary
    Executable,
    /// Configuration (.toml, .yaml, .json, .ini)
    Config,
    /// PDF / document
    Document,
    /// Unknown / other
    Other,
}

impl FileCategory {
    /// Get an emoji/icon character for this category.
    pub fn icon_char(self) -> &'static str {
        match self {
            FileCategory::Code => "</>",
            FileCategory::Text => "TXT",
            FileCategory::Image => "IMG",
            FileCategory::Video => "VID",
            FileCategory::Audio => "AUD",
            FileCategory::Archive => "ZIP",
            FileCategory::Directory => "DIR",
            FileCategory::Executable => "EXE",
            FileCategory::Config => "CFG",
            FileCategory::Document => "PDF",
            FileCategory::Other => "FILE",
        }
    }

    /// Get a color (R, G, B) for this category.
    pub fn color(self) -> (u8, u8, u8) {
        match self {
            FileCategory::Code => (120, 200, 255),       // light blue
            FileCategory::Text => (200, 200, 200),       // gray
            FileCategory::Image => (255, 150, 200),      // pink
            FileCategory::Video => (255, 100, 100),      // red
            FileCategory::Audio => (200, 150, 255),      // purple
            FileCategory::Archive => (255, 180, 80),     // orange
            FileCategory::Directory => (100, 255, 150),  // green
            FileCategory::Executable => (100, 255, 100), // bright green
            FileCategory::Config => (255, 220, 100),     // yellow
            FileCategory::Document => (255, 100, 50),    // dark orange
            FileCategory::Other => (160, 160, 160),      // dim gray
        }
    }

    /// Categorize a file by its extension.
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            // Code
            "rs" | "go" | "py" | "js" | "ts" | "tsx" | "jsx" | "c" | "cpp" | "h" | "hpp"
            | "java" | "kt" | "swift" | "rb" | "lua" | "dart" | "scala" | "sh" | "bash" | "zsh"
            | "fish" | "vim" | "el" | "clj" | "ex" | "exs" | "erl" | "hs" | "ml" | "fs" | "nim"
            | "zig" | "v" => FileCategory::Code,

            // Config
            "toml" | "yaml" | "yml" | "json" | "ini" | "conf" | "cfg" | "env" | "xml" => {
                FileCategory::Config
            }

            // Text
            "txt" | "md" | "markdown" | "rst" | "log" | "csv" | "tsv" => FileCategory::Text,

            // Image
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico" | "tiff" | "avif" => {
                FileCategory::Image
            }

            // Video
            "mp4" | "mov" | "avi" | "mkv" | "webm" | "flv" | "wmv" | "m4v" => FileCategory::Video,

            // Audio
            "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" | "wma" | "opus" => FileCategory::Audio,

            // Archive
            "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "tgz" | "lz" => {
                FileCategory::Archive
            }

            // Document
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp"
            | "epub" => FileCategory::Document,

            _ => FileCategory::Other,
        }
    }
}

/// File preview info card.
#[derive(Debug, Clone)]
pub struct FilePreview {
    /// Full path to the file.
    pub path: String,
    /// File name only.
    pub name: String,
    /// File extension (lowercase, no dot).
    pub extension: String,
    /// File size in bytes (None if directory or unknown).
    pub size: Option<u64>,
    /// Category.
    pub category: FileCategory,
    /// Whether it's a directory.
    pub is_dir: bool,
}

/// State for the file preview overlay.
#[derive(Debug, Default)]
pub struct FilePreviewState {
    /// Currently previewed file (if any).
    pub current: Option<FilePreview>,
    /// Whether the preview is visible.
    pub visible: bool,
    /// Preview card position (x, y in pixels).
    pub x: f32,
    pub y: f32,
}

impl FilePreviewState {
    /// Create new file preview state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Show a preview for the given path.
    pub fn show(&mut self, path: &str, x: f32, y: f32) {
        if let Some(preview) = build_preview(path) {
            self.current = Some(preview);
            self.visible = true;
            self.x = x;
            self.y = y;
        }
    }

    /// Hide the preview.
    pub fn hide(&mut self) {
        self.visible = false;
    }

    /// Whether the preview is active.
    pub fn is_active(&self) -> bool {
        self.visible && self.current.is_some()
    }
}

/// Build a file preview from a path string.
pub fn build_preview(path_str: &str) -> Option<FilePreview> {
    let path = Path::new(path_str);
    let name = path.file_name()?.to_string_lossy().to_string();
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let is_dir = path.is_dir();
    let category = if is_dir {
        FileCategory::Directory
    } else if !extension.is_empty() {
        FileCategory::from_extension(&extension)
    } else {
        FileCategory::Other
    };
    let size = path.metadata().ok().map(|m| m.len());

    Some(FilePreview {
        path: path_str.to_string(),
        name,
        extension,
        size,
        category,
        is_dir,
    })
}

/// Format file size for human-readable display.
pub fn format_size(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    const TB: u64 = 1024 * GB;

    if size >= TB {
        format!("{:.1} TB", size as f64 / TB as f64)
    } else if size >= GB {
        format!("{:.1} GB", size as f64 / GB as f64)
    } else if size >= MB {
        format!("{:.1} MB", size as f64 / MB as f64)
    } else if size >= KB {
        format!("{:.1} KB", size as f64 / KB as f64)
    } else {
        format!("{} B", size)
    }
}

/// Whether a file can be previewed as text.
pub fn is_text_previewable(preview: &FilePreview) -> bool {
    if preview.is_dir {
        return false;
    }
    if let Some(size) = preview.size
        && size > MAX_PREVIEW_SIZE
    {
        return false;
    }
    matches!(
        preview.category,
        FileCategory::Code | FileCategory::Text | FileCategory::Config
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_category_from_extension_code() {
        assert_eq!(FileCategory::from_extension("rs"), FileCategory::Code);
        assert_eq!(FileCategory::from_extension("PY"), FileCategory::Code);
        assert_eq!(FileCategory::from_extension("go"), FileCategory::Code);
    }

    #[test]
    fn t_category_from_extension_config() {
        assert_eq!(FileCategory::from_extension("toml"), FileCategory::Config);
        assert_eq!(FileCategory::from_extension("json"), FileCategory::Config);
    }

    #[test]
    fn t_category_from_extension_image() {
        assert_eq!(FileCategory::from_extension("png"), FileCategory::Image);
        assert_eq!(FileCategory::from_extension("JPG"), FileCategory::Image);
    }

    #[test]
    fn t_category_from_extension_unknown() {
        assert_eq!(FileCategory::from_extension("xyz"), FileCategory::Other);
    }

    #[test]
    fn t_category_icon_char() {
        assert_eq!(FileCategory::Code.icon_char(), "</>");
        assert_eq!(FileCategory::Directory.icon_char(), "DIR");
        assert_eq!(FileCategory::Image.icon_char(), "IMG");
    }

    #[test]
    fn t_category_color() {
        let c = FileCategory::Code.color();
        // Color should not be all-zero (black) — each category has a distinct color.
        assert!(
            !(c.0 == 0 && c.1 == 0 && c.2 == 0),
            "color should be non-black"
        );
    }

    #[test]
    fn t_format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(500), "500 B");
    }

    #[test]
    fn t_format_size_kb() {
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
    }

    #[test]
    fn t_format_size_mb() {
        assert_eq!(format_size(1048576), "1.0 MB");
        assert_eq!(format_size(5_242_880), "5.0 MB");
    }

    #[test]
    fn t_format_size_gb() {
        assert_eq!(format_size(1_073_741_824), "1.0 GB");
    }

    #[test]
    fn t_format_size_tb() {
        assert_eq!(format_size(1_099_511_627_776), "1.0 TB");
    }

    #[test]
    fn t_build_preview_directory() {
        let preview = build_preview("/tmp").unwrap();
        assert!(preview.is_dir);
        assert_eq!(preview.category, FileCategory::Directory);
    }

    #[test]
    fn t_build_preview_rust_file() {
        // Use this source file
        let preview = build_preview(file!()).unwrap();
        assert!(!preview.is_dir);
        assert_eq!(preview.extension, "rs");
        assert_eq!(preview.category, FileCategory::Code);
    }

    #[test]
    fn t_build_preview_nonexistent() {
        // Nonexistent file still builds a preview (metadata is optional)
        let preview = build_preview("/nonexistent/file.txt").unwrap();
        assert_eq!(preview.name, "file.txt");
        assert_eq!(preview.extension, "txt");
        assert_eq!(preview.category, FileCategory::Text);
        // size is None because file doesn't exist
        assert_eq!(preview.size, None);
    }

    #[test]
    fn t_preview_state_default() {
        let state = FilePreviewState::new();
        assert!(!state.is_active());
        assert!(!state.visible);
    }

    #[test]
    fn t_preview_state_show_hide() {
        let mut state = FilePreviewState::new();
        state.show("/tmp", 100.0, 200.0);
        // /tmp exists, so it should show
        assert!(state.visible);
        state.hide();
        assert!(!state.visible);
    }

    #[test]
    fn t_is_text_previewable_code() {
        let preview = FilePreview {
            path: "test.rs".into(),
            name: "test.rs".into(),
            extension: "rs".into(),
            size: Some(100),
            category: FileCategory::Code,
            is_dir: false,
        };
        assert!(is_text_previewable(&preview));
    }

    #[test]
    fn t_is_text_previewable_large_file() {
        let preview = FilePreview {
            path: "big.log".into(),
            name: "big.log".into(),
            extension: "log".into(),
            size: Some(10_000_000), // 10 MB
            category: FileCategory::Text,
            is_dir: false,
        };
        assert!(!is_text_previewable(&preview));
    }

    #[test]
    fn t_is_text_previewable_directory() {
        let preview = FilePreview {
            path: "/tmp".into(),
            name: "tmp".into(),
            extension: "".into(),
            size: None,
            category: FileCategory::Directory,
            is_dir: true,
        };
        assert!(!is_text_previewable(&preview));
    }

    #[test]
    fn t_is_text_previewable_image() {
        let preview = FilePreview {
            path: "test.png".into(),
            name: "test.png".into(),
            extension: "png".into(),
            size: Some(5000),
            category: FileCategory::Image,
            is_dir: false,
        };
        assert!(!is_text_previewable(&preview));
    }
}
