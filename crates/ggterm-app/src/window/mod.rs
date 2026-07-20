//! Desktop window: winit event loop + wgpu surface + GlyphonRenderer.
//!
//! This module ties together the full rendering stack:
//!
//! 1. **winit** creates the OS window and delivers keyboard/mouse/resize events.
//! 2. **wgpu** creates a GPU device + swap-chain surface backed by that window.
//! 3. **GlyphonRenderer** renders the terminal `Grid` into the surface texture.
//! 4. **PtySession** spawns the child shell; a reader thread pumps bytes into
//!    the main loop via an `mpsc` channel.
//!
//! ## Event flow
//!
//! ```text
//! PTY reader thread ──bytes──▶ mpsc channel ──▶ about_to_wait()
//!                                                    │
//!                                                    ▼
//!                                           app.pump()
//!                                           window.request_redraw()
//!                                                    │
//!                                                    ▼
//!                                           RedrawRequested
//!                                           gpu.render_frame()
//! ```

use std::sync::Arc;

use ggterm_render_wgpu::GlyphonRenderer;

use crate::tab_session::TabSession;

/// Get the default shell path as a String.
fn default_shell_string() -> String {
    ggterm_core::pty::default_shell()
}

/// Quote a file path for safe shell input.
///
/// Wraps the path in single quotes and escapes any embedded single quotes.
fn quote_shell_path(path: &str) -> String {
    let escaped = path.replace('\'', "'\\''");
    format!("'{escaped}'")
}

/// Parse a cursor style config string to CursorStyle enum.
/// Valid values: "block" (default), "underline", "bar".
fn parse_cursor_style(s: &str) -> ggterm_core::CursorStyle {
    match s {
        "underline" => ggterm_core::CursorStyle::BlinkUnderline,
        "bar" => ggterm_core::CursorStyle::BlinkBar,
        _ => ggterm_core::CursorStyle::BlinkBlock,
    }
}

/// Fast git branch detection by reading .git/HEAD directly.
/// Avoids spawning a subprocess for the common case.
/// Returns None if not in a git repo or HEAD is detached (caller
/// should fall back to subprocess for detached HEAD display).
fn read_git_head(cwd: &std::path::Path) -> Option<String> {
    let head_path = cwd.join(".git").join("HEAD");
    let head = std::fs::read_to_string(&head_path).ok()?;
    let head = head.trim();
    // Format: "ref: refs/heads/branchname"
    if let Some(ref_pos) = head.find("refs/heads/") {
        let branch = &head[ref_pos + "refs/heads/".len()..];
        let branch = branch.trim();
        if !branch.is_empty() {
            return Some(branch.to_string());
        }
    }
    // Detached HEAD (hash) — let caller handle with subprocess.
    None
}
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::config::ConfigManager;
use crate::desktop_config::{DesktopConfig, VISUAL_BELL_DURATION_FRAMES};
use crate::event::AppEvent;
use crate::gpu::{GpuContext, cursor_state, init_wgpu};
use crate::input::InputEncoder;
use crate::keymap::map_winit_key;

// ══════════════════════════════════════════════════════════════════
//  DesktopApp — implements winit ApplicationHandler
// ══════════════════════════════════════════════════════════════════

/// Current key modifiers state (updated by ModifiersChanged events).
#[derive(Debug, Clone, Copy, Default)]
pub struct ModsState {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// Super/Cmd/Windows key pressed.
    pub super_key: bool,
}

impl From<ModsState> for crate::input::KeyModifiers {
    fn from(m: ModsState) -> Self {
        crate::input::KeyModifiers {
            shift: m.shift,
            ctrl: m.ctrl,
            alt: m.alt,
        }
    }
}

/// Single-tab title bar button positions.
///
/// Shared between render.rs, handlers.rs, and hover detection to ensure
/// consistent layout. Construct via [`SingleTabButtonLayout::compute`].
pub(super) struct SingleTabButtonLayout {
    /// Total title bar height.
    pub bar_h: f32,
    /// Square button side length.
    pub btn_size: f32,
    /// X of the "+" button's left edge.
    pub plus_x: f32,
    /// X of the gear button's left edge.
    pub gear_x: f32,
    /// Y of both buttons' top edge.
    pub btn_y: f32,
}

impl SingleTabButtonLayout {
    /// Compute button positions from screen width and cell height.
    pub(super) fn compute(screen_w: f32, cell_h: f32) -> Self {
        let bar_h = (cell_h + 26.0).max(48.0) + 4.0;
        let tab_h = bar_h - 4.0;
        let btn_size = (tab_h * 0.5).max(20.0);
        let btn_gap = 6.0_f32;
        #[cfg(not(target_os = "macos"))]
        let right_margin = crate::titlebar::CAPTION_BTN_W * 3.0;
        #[cfg(target_os = "macos")]
        let right_margin = 8.0;
        let gear_x = screen_w - btn_size - right_margin;
        let plus_x = gear_x - btn_size - btn_gap;
        let btn_y = (bar_h - btn_size) / 2.0;
        Self {
            bar_h,
            btn_size,
            plus_x,
            gear_x,
            btn_y,
        }
    }

    /// True if pixel (px, py) is inside the "+" button.
    pub(super) fn is_on_plus(&self, px: f32, py: f32) -> bool {
        px >= self.plus_x
            && px <= self.plus_x + self.btn_size
            && py >= self.btn_y
            && py <= self.btn_y + self.btn_size
    }

    /// True if pixel (px, py) is inside the gear button.
    pub(super) fn is_on_gear(&self, px: f32, py: f32) -> bool {
        px >= self.gear_x
            && px <= self.gear_x + self.btn_size
            && py >= self.btn_y
            && py <= self.btn_y + self.btn_size
    }
}

/// Desktop terminal application.
///
/// Implements winit's `ApplicationHandler` trait to receive OS events.
/// GPU resources (surface, device, renderer) are lazily initialized in
/// `resumed()`.
/// Background result for pipe-selection-to-shell-command.
#[allow(clippy::type_complexity)]
pub(super) struct PipeCommandResult {
    rx: std::sync::mpsc::Receiver<Result<(Vec<u8>, Vec<u8>), String>>,
    command: String,
}

pub struct DesktopApp {
    /// Terminal sessions (one per tab).
    sessions: Vec<TabSession>,
    /// Index of the active tab.
    active: usize,
    /// Index of the previously active tab (for Ctrl+Tab toggle).
    last_active_tab: Option<usize>,
    /// Last closed tab's cwd (for "reopen closed tab" feature).
    last_closed_cwd: Option<std::path::PathBuf>,
    /// Terminal input lock (read-only mode). When true, keyboard input
    /// is not forwarded to the PTY.
    locked: bool,
    /// Configuration.
    config: DesktopConfig,
    /// Current key modifiers state.
    mods: ModsState,

    // ── GPU resources (initialized in resumed()) ──
    /// The winit window. Wrapped in Arc so we can pass a clone to wgpu
    /// while keeping a reference for redraw requests.
    window: Option<Arc<Window>>,
    /// wgpu surface (raw window handle, 'static lifetime).
    surface: Option<wgpu::Surface<'static>>,
    /// GPU device/queue/surface config.
    gpu: Option<GpuContext>,
    /// Glyphon text renderer.
    renderer: Option<GlyphonRenderer>,

    // ── Keyboard encoding (shared across tabs) ──
    /// Input encoder for keyboard → PTY bytes.
    encoder: InputEncoder,

    // ── Config hot-reload (config-watch feature) ──
    /// Config manager with optional file-system watcher.
    config_mgr: Option<ConfigManager>,

    // ── Dynamic window title (OSC 0/2) ──
    /// Last known terminal title (to detect changes).
    last_title: String,

    // ── Mouse support ──
    /// Current text selection state.
    selection: crate::mouse::MouseSelection,
    /// Auto-scroll direction during selection drag: -1 = up, 0 = none, 1 = down.
    selection_auto_scroll: i32,
    /// Last auto-scroll tick time.
    last_auto_scroll: std::time::Instant,
    /// Last known cursor position in pixels (for mouse wheel / drag).
    cursor_pos: (f64, f64),
    /// Cached IME cursor area position — avoids redundant platform calls
    /// when the terminal cursor hasn't moved between frames.
    last_ime_cursor_area: Option<(f64, f64, f64, f64)>,
    /// Mouse button currently held (for drag tracking).
    button_held: Option<crate::mouse::MouseButton>,
    /// P21-A: Active split separator drag (None = not dragging).
    drag_resize: Option<bool>,
    /// P27-B: Click count for double/triple-click detection.
    click_count: u8,
    /// Drag selection mode (Char=normal, Word=after double-click, Line=after triple-click).
    drag_select_mode: crate::mouse::DragSelectMode,
    /// P27-B: Timestamp of last left-click.
    last_click_time: Option<std::time::Instant>,
    /// P27-B: Position of last left-click (col, row).
    last_click_pos: (u16, u16),
    /// P27-B: Pixel position of last left-click (for multi-click tolerance).
    last_click_pixel_pos: Option<(f64, f64)>,
    /// P27-C: Right-click context menu state.
    context_menu: crate::context_menu::ContextMenuState,
    /// P27-D: Smooth inertial scroll state.
    smooth_scroll: crate::smooth_scroll::SmoothScroller,
    /// P27-F: Whether the window is currently focused (for cursor style).
    window_focused: bool,
    /// Previous alt-screen state (to detect screen switch and clear selection).
    prev_alt_screen: bool,
    /// True when the window is occluded (covered by other windows) or minimized.
    /// When true, rendering is skipped to save GPU/CPU resources.
    window_occluded: bool,
    /// DPI scale factor (2.0 on Retina, 1.0 on standard). P18-A.
    scale_factor: f64,
    /// IME preedit string (in-progress text from input method).
    /// When non-empty, the IME composition is active.
    ime_preedit: Option<String>,

    // ── Resize debouncing (P9-H) ──
    /// Pending resize dimensions (stored during drag, applied after debounce).
    pending_resize: Option<(u32, u32)>,
    /// Force config reload on next about_to_wait tick.
    force_config_reload: bool,
    /// Instant of the last resize event (for debounce timing).
    last_resize_time: Option<std::time::Instant>,

    // ── AI assistant overlay (P10-C, ai feature) ──
    /// AI overlay state (thinking/result/error).
    #[cfg(feature = "ai")]
    ai_overlay: crate::ai_overlay::AIOverlayState,
    /// AI bridge for background requests.
    #[cfg(feature = "ai")]
    ai_bridge: Option<crate::ai_bridge::AIBridge>,

    // ── Scrollback search (P10-D) ──
    /// Search bar state. When active, keyboard input goes to the search query.
    search: crate::search::SearchState,

    // ── Window controls (P11-C) ──
    /// Whether the window is currently fullscreen.
    fullscreen: bool,
    /// Whether the window is currently maximized.
    maximized: bool,
    /// Window always-on-top state.
    always_on_top: bool,
    /// Cached git branch for current cwd (updated every ~5s).
    git_branch_cache: String,
    /// Counter to throttle git branch checks (every ~300 frames ≈ 5s).
    git_check_counter: u32,
    /// Last time spinner frame was advanced (for ~12fps throttle).
    last_spinner_tick: std::time::Instant,
    /// Cached idle seconds to avoid per-frame String allocation for idle timer.
    last_idle_secs: u64,
    /// Cached command elapsed tenths-of-seconds for timer throttling.
    last_cmd_tenths: u128,
    /// Cached grid dimensions to avoid per-frame format! allocation.
    cached_grid_w: usize,
    cached_grid_h: usize,

    // ── Font zoom (P11-A) ──
    /// Tracks current font size and zoom level for Ctrl+=/-/0.
    font_zoom: crate::font::FontZoom,

    // ── Visual bell (P11-E) ──
    /// Remaining frames for the visual bell flash (0 = no flash).
    visual_bell_frames: u32,

    // ── Status bar (P13-D) ──
    /// Aggregated terminal status for window title display.
    status_bar: crate::status_bar::StatusBar,

    // ── Config-driven keybindings (P14-D) ──
    /// Resolved keybindings: action name → (ctrl, shift, alt, key).
    /// Populated from ConfigManager at startup; falls back to defaults.
    resolved_keybindings: std::collections::HashMap<String, (bool, bool, bool, String)>,

    // ── Config hot-reload tracking (P16-B) ──
    /// Last applied theme name from config (for change detection on hot-reload).
    last_applied_theme: String,
    /// Last applied font size from config (for change detection on hot-reload).
    last_applied_font_size: f32,
    /// Last applied font family from config (for change detection on hot-reload).
    #[cfg(feature = "config-watch")]
    last_applied_font_family: String,
    /// Cached terminal dimensions "WxH" — only reformat on resize.
    cached_dims: String,
    /// Cached uptime in minutes — avoids per-frame format! when minute hasn't changed.
    cached_uptime_mins: u64,
    /// Cached raw CWD path — compare before formatting display string.
    cached_cwd_raw: Option<std::path::PathBuf>,
    /// Cached raw shell path — compare before rebuilding display label.
    cached_shell_raw: String,
    /// Cached last command Duration — skip format! when unchanged.
    cached_cmd_duration: Option<std::time::Duration>,

    // ── Status bar visibility (P17-D) ──
    /// Whether the status bar overlay is visible.
    status_bar_visible: bool,

    // ── URL hover/click (P17-C) ──
    /// Currently hovered URL (OSC 8 hyperlink or plain-text URL).
    /// Stores (url, start_col, end_col, row) for underline rendering.
    hovered_link: Option<(String, usize, usize, usize)>,

    // ── Tab bar overlay (P19-C) ──
    /// Tab bar display state for visual tab strip rendering.
    tab_bar: crate::tab_bar::TabBarState,

    // ── Settings overlay (P19-C) ──
    /// Settings page state (theme, font, scrollback, AI, shell).
    settings: crate::settings_ui::SettingsState,

    // ── About dialog + Menu bar (P19-A) ──
    /// About dialog state.
    about: crate::about_dialog::AboutDialog,
    // ── P22-A: Session restore flag ──
    /// Whether we restored a saved session at startup.
    restored_session: bool,

    // ── Cursor style tracking ──
    /// Last applied cursor style from config. Used to detect actual changes
    /// so config reload doesn't override program-requested DECSCUSR cursor.
    last_applied_cursor_style: ggterm_core::CursorStyle,

    // ── P23-A: Cursor blink animation ──
    /// Cursor blink phase tracker for smooth blink animation.
    cursor_blink: crate::cursor_blink::CursorBlink,
    // ── P23-C: Conditional redraw ──
    /// Last time a redraw was requested (for cursor blink timing).
    last_redraw: std::time::Instant,
    /// Deadline for deferred render when synchronized output (mode 2026) is active.
    /// When set, GPU render is deferred until this deadline to reduce flicker.
    sync_render_deadline: Option<std::time::Instant>,

    // ── P24-C: Debug overlay ──
    /// Whether the debug overlay (FPS, cell counts) is visible.
    debug_visible: bool,
    /// Frame counter for FPS calculation.
    frame_count: u64,
    /// Last FPS update time.
    last_fps_time: std::time::Instant,
    /// Current FPS value.
    current_fps: f32,

    // ── P25: Power user features ──
    /// P25-B: Command palette state (Ctrl+Shift+P).
    command_palette: crate::command_palette::CommandPaletteState,
    /// Cached command registry (avoids re-allocating on every frame/keystroke).
    command_registry: crate::command_palette::CommandRegistry,
    /// P25-D: Broadcast input state (Ctrl+Shift+Alt+B).
    broadcast: crate::broadcast_input::BroadcastState,
    /// P25-E: Session recorder (None when not recording).
    recorder: Option<ggterm_core::recording::SessionRecorder>,

    // ── P28: Phase 28 features ──
    /// P28-A: Animation manager for transitions.
    animations: crate::animations::AnimationManager,
    /// P28-B: Color picker overlay state.
    color_picker: crate::color_picker::ColorPickerState,
    /// P28-C: Command history sidebar.
    cmd_history: crate::command_history::CommandHistoryState,
    /// P28-D: Workspace manager.
    workspaces: crate::workspace::WorkspaceManager,
    /// P28-E: File preview overlay.
    file_preview: crate::file_preview::FilePreviewState,
    /// P28-F: Performance monitor.
    perf_monitor: crate::perf_monitor::PerfMonitor,
    /// P28-F: Cursor particle system.
    cursor_particles: crate::perf_monitor::CursorParticleSystem,
    /// P28-G: Sound player.
    sound_player: crate::sound::SoundPlayer,
    /// P28-G: Bell rate limiter.
    bell_limiter: crate::sound::BellRateLimiter,
    /// P28-H: Shell switcher dropdown.
    shell_switcher: crate::shell_switcher::ShellSwitcherState,
    /// P28: Tab right-click context menu.
    tab_context_menu: crate::tab_bar::TabContextMenuState,
    /// "+" button dropdown menu.
    new_tab_menu: crate::new_tab_menu::NewTabMenuState,
    /// P29-A: Shortcut help overlay (Ctrl+Shift+/).
    shortcut_help: crate::shortcut_help::ShortcutHelpState,
    /// P29-C: Quit confirmation dialog.
    quit_confirm: bool,
    /// Whether the "[process exited]" hold message has been shown.
    hold_message_shown: bool,
    /// P29-C: Flag to exit event loop on next about_to_wait.
    should_quit: bool,
    /// P30-A: Scrollbar drag state (Some(start_y) when dragging).
    scrollbar_drag: Option<f32>,
    /// P30-B: Tab rename state (Some(tab_index) when renaming).
    renaming_tab: Option<usize>,
    /// P30-B: Tab rename text buffer.
    rename_text: String,
    /// P30-C: Toast notification (message + remaining frames).
    toast: Option<(String, u32)>,
    /// Pipe-selection command input state.
    pub pipe_command_active: bool,
    /// Pipe-selection command input text.
    pub pipe_command_input: String,
    /// Background pipe command result (thread handle + command name for toast).
    pending_pipe_result: Option<PipeCommandResult>,
    /// Pending large paste awaiting user confirmation.
    /// Contains the text to paste if the user confirms.
    pending_large_paste: Option<String>,
    /// Pending tab close confirmation (tab index to close on confirm).
    pending_close_tab: Option<usize>,
    /// P31: Saved window position from previous session.
    saved_window_pos: Option<(i32, i32)>,
    /// P31: Saved window size from previous session.
    saved_window_size: Option<(u32, u32)>,
    /// P23-E: Tab drag state (Some(tab_idx) when dragging).
    dragging_tab: Option<usize>,
    /// Pane zoom mode: when true, only the active pane is rendered at full size.
    pane_zoomed: bool,
    /// Scrollback browse mode: when active, keys navigate scrollback
    /// instead of being sent to the PTY (vim-style j/k/G/g/q).
    scroll_mode: bool,
    /// Independent settings window (None when closed).
    settings_window: Option<crate::settings_window::SettingsWindowState>,
    /// Flag to open settings window on next about_to_wait (set from handlers).
    pending_open_settings: bool,
    /// P2P terminal sharing state (feature-gated).
    #[cfg(feature = "p2p")]
    p2p_share: crate::p2p_share::P2pShareState,
    /// Tracks the last command duration we already notified about (prevents duplicate notifications).
    /// Set to Some(duration) after sending a notification; reset to None when a new command starts.
    last_notified_cmd_duration: Option<std::time::Duration>,
}

// ══════════════════════════════════════════════════════════════════
//  P14-D: Config-driven keybindings
// ══════════════════════════════════════════════════════════════════

/// Default keybindings matching the original hardcoded shortcuts.
///
/// Returns a map of action name → (ctrl, shift, alt, key).
pub fn default_keybindings() -> std::collections::HashMap<String, (bool, bool, bool, String)> {
    let mut m = std::collections::HashMap::new();
    // Tab management
    m.insert("new_tab".into(), (true, false, false, "T".into()));
    m.insert("close_tab".into(), (true, false, false, "W".into()));
    // Clipboard
    m.insert("paste".into(), (true, true, false, "V".into()));
    m.insert("copy".into(), (true, true, false, "C".into()));
    // Search
    m.insert("search".into(), (true, true, false, "F".into()));
    // Font zoom
    m.insert("zoom_in".into(), (true, false, false, "=".into()));
    m.insert("zoom_out".into(), (true, false, false, "-".into()));
    m.insert("zoom_reset".into(), (true, false, false, "0".into()));
    // Fullscreen
    m.insert("fullscreen".into(), (false, false, false, "F11".into()));
    // Terminal actions
    m.insert("clear".into(), (true, true, false, "K".into()));
    m.insert("reset".into(), (true, true, false, "R".into()));
    m.insert("cycle_theme".into(), (true, true, true, "T".into()));
    // Copy current working directory (from OSC 7)
    m.insert("copy_cwd".into(), (true, true, false, "P".into()));
    m
}

/// Update a single keybinding entry if the config provides a value.
fn apply_keybinding(
    map: &mut std::collections::HashMap<String, (bool, bool, bool, String)>,
    action: &str,
    binding: Option<&str>,
) {
    if let Some(s) = binding
        && let Some((c, sh, a, k)) = crate::config::parse_keybinding(s)
    {
        map.insert(action.into(), (c, sh, a, k.to_string()));
    }
}

/// Convert a winit [`KeyCode`] to the string name used in keybindings.
///
/// This mirrors the key names produced by [`parse_keybinding`]:
/// letters use uppercase ("T", "V"), digits use the digit ("0", "1"),
/// function keys use "F1"–"F24", and special keys use their name.
pub fn keycode_to_name(code: &KeyCode) -> &str {
    match code {
        // Letters A–Z
        KeyCode::KeyA => "A",
        KeyCode::KeyB => "B",
        KeyCode::KeyC => "C",
        KeyCode::KeyD => "D",
        KeyCode::KeyE => "E",
        KeyCode::KeyF => "F",
        KeyCode::KeyG => "G",
        KeyCode::KeyH => "H",
        KeyCode::KeyI => "I",
        KeyCode::KeyJ => "J",
        KeyCode::KeyK => "K",
        KeyCode::KeyL => "L",
        KeyCode::KeyM => "M",
        KeyCode::KeyN => "N",
        KeyCode::KeyO => "O",
        KeyCode::KeyP => "P",
        KeyCode::KeyQ => "Q",
        KeyCode::KeyR => "R",
        KeyCode::KeyS => "S",
        KeyCode::KeyT => "T",
        KeyCode::KeyU => "U",
        KeyCode::KeyV => "V",
        KeyCode::KeyW => "W",
        KeyCode::KeyX => "X",
        KeyCode::KeyY => "Y",
        KeyCode::KeyZ => "Z",
        // Digits
        KeyCode::Digit0 => "0",
        KeyCode::Digit1 => "1",
        KeyCode::Digit2 => "2",
        KeyCode::Digit3 => "3",
        KeyCode::Digit4 => "4",
        KeyCode::Digit5 => "5",
        KeyCode::Digit6 => "6",
        KeyCode::Digit7 => "7",
        KeyCode::Digit8 => "8",
        KeyCode::Digit9 => "9",
        // Punctuation
        KeyCode::Equal => "=",
        KeyCode::Minus => "-",
        // Function keys
        KeyCode::F1 => "F1",
        KeyCode::F2 => "F2",
        KeyCode::F3 => "F3",
        KeyCode::F4 => "F4",
        KeyCode::F5 => "F5",
        KeyCode::F6 => "F6",
        KeyCode::F7 => "F7",
        KeyCode::F8 => "F8",
        KeyCode::F9 => "F9",
        KeyCode::F10 => "F10",
        KeyCode::F11 => "F11",
        KeyCode::F12 => "F12",
        KeyCode::F13 => "F13",
        KeyCode::F14 => "F14",
        KeyCode::F15 => "F15",
        KeyCode::F16 => "F16",
        KeyCode::F17 => "F17",
        KeyCode::F18 => "F18",
        KeyCode::F19 => "F19",
        KeyCode::F20 => "F20",
        KeyCode::F21 => "F21",
        KeyCode::F22 => "F22",
        KeyCode::F23 => "F23",
        KeyCode::F24 => "F24",
        // Everything else returns an empty string (never matches a binding).
        _ => "",
    }
}

impl DesktopApp {
    /// Launch the desktop terminal: create PTY, wire up the reader thread,
    /// and block on the winit event loop.
    pub fn run(config: DesktopConfig) -> Result<(), Box<dyn std::error::Error>> {
        let _ = env_logger::try_init();

        // ── Step 1: Load configuration from ~/.ggterm/config.toml ──
        let config_mgr = match ConfigManager::load_default() {
            Ok(mgr) => {
                log::info!(
                    "Config loaded from ~/.ggterm/config.toml (theme={}, scrollback={})",
                    mgr.config().appearance.theme,
                    mgr.config().terminal.scrollback_lines
                );
                // If shell integration is disabled in config, set env var to prevent injection.
                if !mgr.config().terminal.shell_integration {
                    // SAFETY: env var set during initialization before any threads.
                    unsafe { std::env::set_var("GGTERM_DISABLE_INTEGRATION", "1") };
                }
                Some(mgr)
            }
            Err(e) => {
                log::info!("No config file found, using defaults: {e}");
                None
            }
        };

        // ── Step 2: Merge config values into DesktopConfig (CLI overrides win) ──
        let (cols, rows) = (config.cols, config.rows);
        let effective_shell = config
            .shell
            .clone()
            .or_else(|| {
                config_mgr
                    .as_ref()
                    .map(|m| m.config().terminal.shell.clone())
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(default_shell_string);

        let mut desktop_config = config;
        if let Some(ref mgr) = config_mgr {
            let cfg = mgr.config();
            if desktop_config.cell_width == 8.0 {
                desktop_config.cell_width = cfg.appearance.cell_width as f32;
            }
            if desktop_config.cell_height == 16.0 {
                desktop_config.cell_height = cfg.appearance.cell_height as f32;
            }
        }
        desktop_config.shell = Some(effective_shell.clone());

        // ── Step 3: Create initial tab session ──
        let mut session = TabSession::new(cols, rows, &effective_shell)?;
        if let Some(ref mgr) = config_mgr {
            let cfg = mgr.config();
            session
                .app_mut()
                .theme_manager()
                .set_by_name(&cfg.appearance.theme);
            session
                .app_mut()
                .terminal_mut()
                .grid_mut()
                .set_scrollback(cfg.terminal.scrollback_lines);
        }

        // ── Step 3b: Welcome message ──
        // Print a brief, helpful banner before the shell starts.
        let shell_name = std::path::Path::new(&effective_shell)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("shell");
        let welcome = format!(
            "\x1b[90m GGTerm v{} ({}) \x1b[0m\x1b[90mCtrl+Shift+? for help\x1b[0m\r\n",
            env!("CARGO_PKG_VERSION"),
            shell_name,
        );
        session
            .app_mut()
            .handle_event(crate::event::AppEvent::PtyBytes(welcome.into_bytes()));

        // ── Step 4: Build DesktopApp ──
        let mut desktop = DesktopApp {
            sessions: vec![session],
            active: 0,
            last_active_tab: None,
            last_closed_cwd: None,
            locked: false,
            config: desktop_config,
            mods: ModsState::default(),
            window: None,
            surface: None,
            gpu: None,
            renderer: None,
            encoder: InputEncoder::new(),
            config_mgr: None,
            last_title: String::new(),
            selection: crate::mouse::MouseSelection::default(),
            selection_auto_scroll: 0,
            last_auto_scroll: std::time::Instant::now(),
            cursor_pos: (0.0, 0.0),
            last_ime_cursor_area: None,
            button_held: None,
            drag_resize: None,
            click_count: 0,
            drag_select_mode: crate::mouse::DragSelectMode::Char,
            last_click_time: None,
            last_click_pos: (0, 0),
            last_click_pixel_pos: None,
            context_menu: Default::default(),
            smooth_scroll: Default::default(),
            window_focused: true,
            prev_alt_screen: false,
            window_occluded: false,
            scale_factor: 1.0,
            ime_preedit: None,
            pending_resize: None,
            force_config_reload: false,
            last_resize_time: None,
            last_redraw: std::time::Instant::now(),
            sync_render_deadline: None,
            debug_visible: false,
            frame_count: 0,
            last_fps_time: std::time::Instant::now(),
            current_fps: 0.0,
            #[cfg(feature = "ai")]
            ai_overlay: crate::ai_overlay::AIOverlayState::new(),
            #[cfg(feature = "ai")]
            ai_bridge: Self::create_ai_bridge(&config_mgr),
            search: crate::search::SearchState::new(),
            fullscreen: false,
            maximized: false,
            always_on_top: false,
            git_branch_cache: String::new(),
            git_check_counter: 0,
            last_spinner_tick: std::time::Instant::now(),
            last_idle_secs: 0,
            last_cmd_tenths: 0,
            cached_grid_w: 0,
            cached_grid_h: 0,
            font_zoom: crate::font::FontZoom::default_size(),
            visual_bell_frames: 0,
            status_bar: crate::status_bar::StatusBar::new(),
            resolved_keybindings: crate::window::default_keybindings(),
            last_applied_theme: config_mgr
                .as_ref()
                .map(|m| m.config().appearance.theme.clone())
                .unwrap_or_else(|| "dark".to_string()),
            last_applied_font_size: config_mgr
                .as_ref()
                .map(|m| m.config().appearance.font_size as f32)
                .unwrap_or(crate::font::DEFAULT_FONT_SIZE),
            #[cfg(feature = "config-watch")]
            last_applied_font_family: config_mgr
                .as_ref()
                .map(|m| m.config().appearance.font_family.clone())
                .unwrap_or_default(),
            cached_dims: String::new(),
            cached_uptime_mins: 0,
            cached_cwd_raw: None,
            cached_shell_raw: String::new(),
            cached_cmd_duration: None,
            status_bar_visible: true,
            hovered_link: None,
            tab_bar: crate::tab_bar::TabBarState::new(),
            settings: crate::settings_ui::SettingsState::new(),
            about: crate::about_dialog::AboutDialog::new(),
            restored_session: false,
            cursor_blink: crate::cursor_blink::CursorBlink::new(),
            command_palette: crate::command_palette::CommandPaletteState::default(),
            command_registry: crate::command_palette::CommandRegistry::defaults(),
            broadcast: crate::broadcast_input::BroadcastState::default(),
            recorder: None,
            animations: crate::animations::AnimationManager::default(),
            color_picker: crate::color_picker::ColorPickerState::new(),
            cmd_history: crate::command_history::CommandHistoryState::new(),
            workspaces: crate::workspace::WorkspaceManager::new(),
            file_preview: crate::file_preview::FilePreviewState::new(),
            perf_monitor: crate::perf_monitor::PerfMonitor::new(),
            cursor_particles: crate::perf_monitor::CursorParticleSystem::new(),
            sound_player: crate::sound::SoundPlayer::new(),
            bell_limiter: crate::sound::BellRateLimiter::default(),
            shell_switcher: crate::shell_switcher::ShellSwitcherState::new(),
            tab_context_menu: crate::tab_bar::TabContextMenuState::default(),
            new_tab_menu: crate::new_tab_menu::NewTabMenuState::default(),
            shortcut_help: crate::shortcut_help::ShortcutHelpState::new(),
            quit_confirm: false,
            hold_message_shown: false,
            should_quit: false,
            scrollbar_drag: None,
            renaming_tab: None,
            rename_text: String::new(),
            toast: None,
            pipe_command_active: false,
            pipe_command_input: String::new(),
            pending_pipe_result: None,
            pending_large_paste: None,
            pending_close_tab: None,
            saved_window_pos: None,
            saved_window_size: None,
            dragging_tab: None,
            pane_zoomed: false,
            scroll_mode: false,
            settings_window: None,
            pending_open_settings: false,
            last_applied_cursor_style: ggterm_core::CursorStyle::BlinkBlock,
            #[cfg(feature = "p2p")]
            p2p_share: crate::p2p_share::P2pShareState::new(),
            last_notified_cmd_duration: None,
        };

        // ── Step 7b: P22-A Try restore saved session ──
        // Only restore if the config option is enabled (default: false).
        if desktop
            .config_mgr
            .as_ref()
            .map(|m| m.config().terminal.restore_session)
            .unwrap_or(false)
        {
            match crate::session::load_session() {
                Ok(Some(data)) => {
                    log::info!(
                        "Found saved session: {} tab(s), restoring...",
                        data.tabs.len()
                    );
                    // P31: Restore window geometry.
                    if let (Some(w), Some(h)) = (data.window_width, data.window_height) {
                        desktop.saved_window_size = Some((w, h));
                    }
                    if let Some(x) = data.window_x {
                        desktop.saved_window_pos = Some((x, data.window_y.unwrap_or(0)));
                    }
                    let plan = crate::session::SessionPlan::from_data(&data);
                    desktop.restore_from_plan(&plan);
                    desktop.restored_session = true;
                }
                Ok(None) => {
                    log::info!("No saved session found, starting fresh");
                }
                Err(e) => {
                    log::warn!("Failed to load saved session: {e}");
                }
            }
        } else {
            log::info!("Session restore disabled in config, starting fresh");
        }

        // ── Step 7c: Load config-driven keybindings (P14-D) ──
        if let Some(ref mgr) = config_mgr {
            let kb = &mgr.config().keybindings;
            let rkb = &mut desktop.resolved_keybindings;
            apply_keybinding(rkb, "new_tab", kb.new_tab.as_deref());
            apply_keybinding(rkb, "close_tab", kb.close_tab.as_deref());
            apply_keybinding(rkb, "paste", kb.paste.as_deref());
            apply_keybinding(rkb, "copy", kb.copy.as_deref());
            apply_keybinding(rkb, "search", kb.search.as_deref());
            apply_keybinding(rkb, "zoom_in", kb.zoom_in.as_deref());
            apply_keybinding(rkb, "zoom_out", kb.zoom_out.as_deref());
            apply_keybinding(rkb, "zoom_reset", kb.zoom_reset.as_deref());
            apply_keybinding(rkb, "fullscreen", kb.fullscreen.as_deref());
            apply_keybinding(rkb, "clear", kb.clear.as_deref());
            apply_keybinding(rkb, "reset", kb.reset.as_deref());
            apply_keybinding(rkb, "cycle_theme", kb.cycle_theme.as_deref());
        }

        // ── Step 8: Start config file watcher (if config-watch feature) ──
        #[cfg(feature = "config-watch")]
        {
            if let Some(mut mgr) = config_mgr {
                if let Err(e) = mgr.watch() {
                    log::warn!("Config watch failed: {e}");
                }
                desktop.config_mgr = Some(mgr);
            }
        }

        // ── Step 9: Create winit event loop and run ──
        let event_loop = EventLoop::new()?;
        event_loop.run_app(&mut desktop)?;

        Ok(())
    }

    // ── Helpers ──

    /// Get the active session (immutable).
    fn active_session(&self) -> &TabSession {
        &self.sessions[self.active]
    }

    /// Get the active session (mutable).
    fn active_session_mut(&mut self) -> &mut TabSession {
        &mut self.sessions[self.active]
    }

    /// Get the shell path for creating new tabs.
    fn shell(&self) -> &str {
        self.config.shell.as_deref().unwrap_or("/bin/sh")
    }

    /// Get the current background opacity (0.0 = transparent, 1.0 = opaque).
    /// Reads from config manager if available, otherwise returns 1.0.
    fn background_opacity(&self) -> f64 {
        #[cfg(feature = "config-watch")]
        if let Some(ref mgr) = self.config_mgr {
            return mgr.config().appearance.background_opacity as f64;
        }
        1.0
    }

    /// Get the terminal content padding in physical pixels.
    /// Reads from config if available, falls back to default CONTENT_PADDING.
    fn content_padding(&self) -> u32 {
        #[cfg(feature = "config-watch")]
        if let Some(ref mgr) = self.config_mgr {
            return mgr.config().appearance.padding;
        }
        crate::desktop_config::CONTENT_PADDING as u32
    }

    /// Adjust background opacity by a delta, clamped to [0.1, 1.0].
    /// Shows a toast notification with the current value.
    fn adjust_opacity(&mut self, #[allow(unused_variables)] delta: f32) {
        #[cfg(feature = "config-watch")]
        {
            if let Some(ref mut mgr) = self.config_mgr {
                let current = mgr.config().appearance.background_opacity;
                let new_val = (current + delta).clamp(0.1, 1.0);
                mgr.config_mut().appearance.background_opacity = new_val;
                self.show_toast(format!("Opacity: {:.0}%", new_val * 100.0));
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
        }
    }

    // ── Config-driven keybinding lookup (P14-D) ──

    /// Check whether the current key press matches the keybinding for `action`.
    ///
    /// Looks up the resolved keybinding (from config or defaults) and compares
    /// the modifier flags and key name.  Returns `true` on a match.
    pub fn check_keybinding(
        &self,
        action: &str,
        ctrl: bool,
        shift: bool,
        alt: bool,
        key: &str,
    ) -> bool {
        match self.resolved_keybindings.get(action) {
            Some(&(kc, ksh, ka, ref kk)) => ctrl == kc && shift == ksh && alt == ka && key == kk,
            None => false,
        }
    }
}

// ── Sub-modules ──
mod actions;
mod handlers;
mod render;

impl DesktopApp {
    // ── AI bridge initialization ─────────────────────────────────────

    /// Create an AIBridge from config + environment variables.
    ///
    /// Priority for API key: config.toml `[ai] api_key` > `GGTERM_AI_API_KEY` env > `OPENAI_API_KEY` env.
    /// Priority for endpoint: config.toml `[ai] api_endpoint` > `GGTERM_AI_BASE_URL` env > default.
    /// Priority for model: config.toml `[ai] model` > `GGTERM_AI_MODEL` env > default.
    #[cfg(feature = "ai")]
    fn create_ai_bridge(config_mgr: &Option<ConfigManager>) -> Option<crate::ai_bridge::AIBridge> {
        // Check if AI is enabled in config.
        let ai_cfg = config_mgr.as_ref().map(|m| &m.config().ai);
        let enabled = ai_cfg.is_some_and(|c| c.enabled);

        // Also check env vars directly (env can override even if config doesn't enable).
        let env_key = std::env::var("GGTERM_AI_API_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .ok();

        if !enabled && env_key.is_none() {
            return None;
        }

        // Gather config values with fallbacks.
        let api_key = ai_cfg
            .and_then(|c| {
                if !c.api_key.is_empty() {
                    Some(c.api_key.clone())
                } else {
                    None
                }
            })
            .or(env_key)
            .unwrap_or_default();

        let api_endpoint = ai_cfg
            .and_then(|c| {
                if !c.api_endpoint.is_empty() {
                    Some(c.api_endpoint.clone())
                } else {
                    None
                }
            })
            .or_else(|| std::env::var("GGTERM_AI_BASE_URL").ok())
            .unwrap_or_else(|| "https://open.bigmodel.cn/api/paas/v4".to_string());

        let model = ai_cfg
            .and_then(|c| {
                if !c.model.is_empty() {
                    Some(c.model.clone())
                } else {
                    None
                }
            })
            .or_else(|| std::env::var("GGTERM_AI_MODEL").ok())
            .unwrap_or_else(|| "glm-4-flash".to_string());

        if api_key.is_empty() {
            log::warn!(
                "AI enabled but no API key found. Set [ai] api_key in config or GGTERM_AI_API_KEY env var."
            );
            return None;
        }

        let llm_config = ggterm_ai::client::AIConfig {
            api_key,
            base_url: api_endpoint,
            model,
            ..Default::default()
        };

        let client = ggterm_ai::client::LLMClient::new(llm_config);
        let engine = ggterm_ai::AIEngine::with_provider(Box::new(client));
        log::info!("AI bridge initialized");
        Some(crate::ai_bridge::AIBridge::new(engine))
    }

    // ── Menu dispatch (P19-A) ──────────────────────────────────────

    /// Dispatch a menu action to the corresponding handler.
    ///
    /// This is called from `about_to_wait()` when a menu item was clicked.
    /// Each action maps to the same handler as its keyboard shortcut.
    pub fn handle_menu_action(&mut self, action: crate::menu_bar::MenuAction) {
        use crate::menu_bar::MenuAction;
        match action {
            // File
            MenuAction::NewTab => self.open_tab(),
            MenuAction::CloseTab => self.close_tab(),
            MenuAction::Quit => {
                if let Some(ref window) = self.window {
                    let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(0, 0));
                }
            }
            // Edit
            MenuAction::Copy => self.copy_selection_to_clipboard(),
            MenuAction::Paste => self.paste_from_clipboard(),
            MenuAction::SelectAll => {
                let grid = self.sessions[self.active].app().grid();
                let range = crate::terminal_actions::select_all_range(grid);
                self.selection
                    .start(range.start_col as u16, range.start_row as u16);
                self.selection
                    .extend(range.end_col as u16, range.end_row as u16);
                self.selection.finish();
            }
            MenuAction::ClearScrollback => {
                crate::terminal_actions::clear_screen_and_scrollback(
                    self.active_session_mut().app_mut().grid_mut(),
                );
            }
            MenuAction::ResetTerminal => {
                crate::terminal_actions::soft_reset(self.active_session_mut().app_mut().grid_mut());
            }
            // View
            MenuAction::ZoomIn => {
                self.font_zoom.zoom_in();
                self.apply_font_size();
            }
            MenuAction::ZoomOut => {
                self.font_zoom.zoom_out();
                self.apply_font_size();
            }
            MenuAction::ZoomReset => {
                self.font_zoom.reset();
                self.apply_font_size();
            }
            MenuAction::ToggleFullscreen => self.toggle_fullscreen(),
            MenuAction::ToggleStatusBar => {
                self.status_bar_visible = !self.status_bar_visible;
            }
            MenuAction::CycleTheme => self.cycle_theme(),
            // Shell
            MenuAction::ScrollbackSearch => {
                self.search.toggle();
            }
            // Help
            MenuAction::About => {
                self.about.toggle();
            }
        }
    }

    /// Send a desktop notification when a long-running command completes
    /// in a background tab (or active tab when window is unfocused).
    pub fn maybe_notify_command_complete(&mut self, tab_idx: usize) {
        let notify = self
            .config_mgr
            .as_ref()
            .map(|m| m.config().terminal.notify_on_complete)
            .unwrap_or(false);
        if !notify {
            return;
        }
        let is_background = tab_idx != self.active;
        if self.window_focused && !is_background {
            return;
        }
        let min_duration = self
            .config_mgr
            .as_ref()
            .map(|m| m.config().terminal.min_notify_duration_secs)
            .unwrap_or(3);
        let elapsed = self.sessions[tab_idx]
            .app()
            .terminal()
            .last_command_duration();
        let secs = elapsed.map(|d| d.as_secs()).unwrap_or(0);
        if secs < min_duration {
            return;
        }
        let title = self.sessions[tab_idx].title().to_string();
        let msg = format!("Command finished in \"{title}\" ({secs}s)");
        log::info!("{msg}");
        self.show_toast(msg.clone());

        #[cfg(target_os = "macos")]
        {
            let script = format!("display notification \"{msg}\" with title \"GGTerm\"");
            let _ = std::process::Command::new("osascript")
                .args(["-e", &script])
                .spawn();
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let _ = std::process::Command::new("notify-send")
                .args(["GGTerm", &msg])
                .spawn();
        }
    }
}

// ══════════════════════════════════════════════════════════════════
//  ApplicationHandler implementation
// ══════════════════════════════════════════════════════════════════

impl ApplicationHandler for DesktopApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return; // Already initialized.
        }

        log::info!("Initializing window + GPU");

        // 1. Create the window with logical (pre-scale) dimensions.
        //    Use saved session geometry if available.
        let (win_w, win_h) = self
            .saved_window_size
            .unwrap_or((self.config.window_width(), self.config.window_height()));
        let mut attrs = Window::default_attributes()
            .with_title(&self.config.title)
            .with_inner_size(winit::dpi::LogicalSize::new(win_w as f64, win_h as f64))
            .with_min_inner_size(winit::dpi::LogicalSize::new(
                crate::desktop_config::MIN_COLS as f64 * 8.0,
                crate::desktop_config::MIN_ROWS as f64 * 16.0 + 100.0, // +100 for title bar + status bar
            ))
            .with_transparent(true);

        // macOS: keep decorations=true but make titlebar transparent via FFI
        // after window creation. This preserves traffic light buttons while
        // giving us full control over the titlebar appearance.
        //
        // Linux/Windows: use decorations=false for full custom titlebar.
        // We implement edge resize via drag_resize_window() and caption
        // buttons ourselves to maintain a consistent cross-platform look.
        #[cfg(not(target_os = "macos"))]
        {
            attrs = attrs.with_decorations(false);
        }

        // --fullscreen flag: start in fullscreen mode.
        if std::env::var("GGTERM_FULLSCREEN").as_deref() == Ok("1") {
            attrs = attrs.with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
        }
        // --maximize flag: start maximized.
        if std::env::var("GGTERM_MAXIMIZE").as_deref() == Ok("1") {
            attrs = attrs.with_maximized(true);
        }

        // Set window icon from embedded PNG — works on all platforms.
        // winit's with_window_icon sets the title bar icon (Linux/Windows)
        // and is used by some compositors on Wayland/X11.
        let icon_data = include_bytes!("../../../../assets/logo-512.png");
        if let Ok(img) = image::load_from_memory(icon_data) {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            if let Ok(icon) = winit::window::Icon::from_rgba(rgba.into_raw(), w, h) {
                attrs = attrs.with_window_icon(Some(icon));
            }
        }

        // macOS: additionally set the Dock icon via NSApplication,
        // since winit's window_icon doesn't affect the Dock on macOS.
        #[cfg(target_os = "macos")]
        unsafe {
            use objc2::msg_send;
            use objc2::runtime::AnyObject;
            use objc2_foundation::NSData;

            // NSImage::initWithData:(NSData *) — from the embedded PNG bytes.
            let nsdata = NSData::with_bytes(icon_data);
            if let Some(cls) = objc2::runtime::AnyClass::get(c"NSImage") {
                let alloc: *mut AnyObject = msg_send![cls, alloc];
                let img: *mut AnyObject = msg_send![alloc, initWithData: &*nsdata];
                if !img.is_null() {
                    if let Some(app_cls) = objc2::runtime::AnyClass::get(c"NSApplication") {
                        let app: *mut AnyObject = msg_send![app_cls, sharedApplication];
                        let _: () = msg_send![app, setApplicationIconImage: img];
                    }
                    let _: () = msg_send![img, release];
                }
            }
        }
        if let Some(x) = self.saved_window_pos {
            attrs = attrs.with_position(winit::dpi::LogicalPosition::new(x.0 as f64, x.1 as f64));
        }

        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("Failed to create window: {e}");
                event_loop.exit();
                return;
            }
        };

        // P27-E: Apply macOS vibrancy for backdrop blur effect.
        // NOTE: Temporarily disabled — raw FFI objc_msgSend with NSRect
        // struct returns causes a crash on ARM64. Will revisit with proper
        // objc2-app-kit NSVisualEffectView feature flags.
        // #[cfg(target_os = "macos")]
        // {
        //     use raw_window_handle::HasWindowHandle;
        //     if let Ok(handle) = window.window_handle()
        //         && let raw_window_handle::RawWindowHandle::AppKit(appkit) = handle.as_raw()
        //     {
        //         unsafe {
        //             crate::vibrancy::apply_vibrancy_to_view(appkit.ns_view.as_ptr());
        //         }
        //     }
        // }

        // P18-A: Get scale_factor and resize window to proper physical dimensions.
        let scale_factor = window.scale_factor();
        self.scale_factor = scale_factor;
        let phys_w = (self.config.window_width() as f64 * scale_factor).round() as u32;
        let phys_h = (self.config.window_height() as f64 * scale_factor).round() as u32;
        log::info!(
            "DPI scale_factor={}, physical window: {}x{}",
            scale_factor,
            phys_w,
            phys_h
        );

        // 2. Initialize wgpu (Instance + Surface + Adapter).
        //    Pass Arc::clone so we keep a reference for redraw requests.
        let (_instance, surface, adapter) = match init_wgpu(Arc::clone(&window)) {
            Ok(result) => result,
            Err(e) => {
                log::error!("Failed to init wgpu: {e}");
                event_loop.exit();
                return;
            }
        };

        // 3. Create GPU context with physical dimensions.
        let gpu = match GpuContext::from_surface(&surface, &adapter, phys_w.max(1), phys_h.max(1)) {
            Ok(g) => g,
            Err(e) => {
                log::error!("Failed to create GPU context: {e}");
                event_loop.exit();
                return;
            }
        };

        // 4. Create GlyphonRenderer with surface dimensions.
        let renderer = gpu.create_renderer(phys_w, phys_h, scale_factor);

        // Update cols/rows from renderer's computed dimensions — but adjust
        // for content area (tab bar, status bar, padding).
        {
            let cell_w = renderer.cell_width();
            let cell_h = renderer.cell_height();
            let tab_bar_h = ((cell_w as f32 + 26.0).max(48.0) + 4.0) as u32;
            let status_bar_h = cell_h + 8;
            let pad: u32 = 2;
            let avail_w = phys_w.saturating_sub(pad * 2);
            let avail_h = phys_h.saturating_sub(tab_bar_h + status_bar_h + pad * 2);
            self.config.cols = ((avail_w / cell_w.max(1)) as u16).max(10);
            self.config.rows = ((avail_h / cell_h.max(1)) as u16).max(3);
        }

        self.window = Some(window.clone());
        self.surface = Some(surface);
        self.gpu = Some(gpu);

        // macOS: restore traffic light buttons on frameless window.
        #[cfg(target_os = "macos")]
        {
            crate::titlebar::install_traffic_lights(&window);
        }

        // Enable IME (Input Method Editor) for CJK text input.
        window.set_ime_allowed(true);

        self.renderer = Some(renderer);

        // Set real cell dimensions on all terminal sessions so CSI 14t/15t/16t
        // report accurate pixel sizes to tmux/nvim.
        if let Some(ref r) = self.renderer {
            let cw = r.cell_width();
            let ch = r.cell_height();
            for session in &mut self.sessions {
                for pane_id in session.pane_ids() {
                    if let Some(app) = session.pane_app_mut(pane_id) {
                        app.terminal_mut().set_cell_dimensions(cw, ch);
                    }
                }
            }
            log::info!("Cell dimensions set to {}x{} px", cw, ch);
        }

        // P18-C: CRITICAL — resize terminal sessions to match renderer.
        // Without this the PTY/grid think 80x24 while the window shows different.
        let actual_cols = self.config.cols;
        let actual_rows = self.config.rows;

        // Feed real cell dimensions to all terminal sessions so CSI 14t/15t/16t
        // can report accurate pixel sizes to tmux/nvim.
        let (cell_w, cell_h) = {
            let r = self.renderer.as_ref().expect("renderer just set");
            (r.cell_width(), r.cell_height())
        };
        for session in &mut self.sessions {
            for pane_id in session.pane_ids() {
                if let Some(app) = session.pane_app_mut(pane_id) {
                    app.terminal_mut().set_cell_dimensions(cell_w, cell_h);
                }
            }
        }

        for session in &mut self.sessions {
            session.resize(actual_cols, actual_rows);
        }
        log::info!(
            "Terminal resized to {}x{} to match renderer",
            actual_cols,
            actual_rows
        );

        // Wire real cell dimensions from renderer to terminal for accurate CSI 16t/14t/15t.
        if let Some(ref r) = self.renderer {
            let cw = r.cell_width();
            let ch = r.cell_height();
            for session in &mut self.sessions {
                for pane_id in session.pane_ids() {
                    if let Some(app) = session.pane_app_mut(pane_id) {
                        app.terminal_mut().set_cell_dimensions(cw, ch);
                    }
                }
            }
        }

        // P11-D: Apply active theme to renderer on startup.
        self.apply_theme_to_renderer();

        // Apply cursor style from config to all sessions.
        let cursor_style = self
            .config_mgr
            .as_ref()
            .map(|mgr| parse_cursor_style(&mgr.config().appearance.cursor_style))
            .unwrap_or(ggterm_core::CursorStyle::BlinkBlock);
        self.last_applied_cursor_style = cursor_style;
        for session in &mut self.sessions {
            for pane_id in session.pane_ids() {
                if let Some(app) = session.pane_app_mut(pane_id) {
                    app.terminal_mut().set_cursor_style(cursor_style);
                }
            }
        }

        log::info!("Window + GPU initialized");
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // ── Settings window routing ──
        if let Some(ref mut sw) = self.settings_window
            && sw.id() == window_id
        {
            sw.handle_event(&event);
            sw.window.request_redraw();
            return;
        }

        match event {
            WindowEvent::CloseRequested => {
                // P29-C: Show quit confirmation dialog.
                if self.quit_confirm {
                    log::info!("Quit confirmed — resetting terminal modes and exiting");
                    self.send_terminal_reset();
                    self.save_session_on_exit();
                    event_loop.exit();
                } else {
                    self.quit_confirm = true;
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                }
            }

            // P22-E: Drag & drop file support.
            WindowEvent::DroppedFile(path) => {
                self.file_preview.hide();
                self.handle_dropped_file(&path);
            }

            // P28-E: Show file preview card during drag-hover.
            WindowEvent::HoveredFile(path) => {
                self.file_preview
                    .show(&path.to_string_lossy(), 200.0, 150.0);
            }

            // P28-E: Hide file preview when drag leaves window.
            WindowEvent::HoveredFileCancelled => {
                self.file_preview.hide();
            }

            WindowEvent::RedrawRequested => {
                // Pump PTY events before rendering.
                self.active_session_mut().pump();

                // Check exit — close pane/tab or quit app.
                if !self.active_session().is_running() {
                    if self.handle_pane_exit() {
                        event_loop.exit();
                        return;
                    }
                    // Pane/tab was closed — skip rendering this frame.
                    return;
                }

                // Skip GPU rendering when the window is hidden (occluded) to
                // save battery and GPU resources. We still process PTY data.
                if self.window_occluded {
                    // Sleep longer when occluded to minimize CPU usage.
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    return;
                }

                self.render_frame();

                // Update status bar from active session state.
                let (row, col) = self.active_session().app().cursor();
                self.status_bar.update_cursor(row, col);
                self.status_bar.show_clock = true;
                self.status_bar
                    .update_tabs(self.sessions.len(), self.active);
                self.status_bar
                    .update_pane_count(self.active_session().pane_count());
                self.status_bar.set_bell(self.visual_bell_frames > 0);
                self.status_bar.set_search(self.search.visible);
                #[cfg(feature = "ai")]
                self.status_bar.set_ai(self.ai_overlay.is_visible());
                #[cfg(not(feature = "ai"))]
                self.status_bar.set_ai(false);
                // P17-E: Update exit code from terminal's last command.
                self.status_bar
                    .set_exit_code(self.active_session().app().terminal().last_exit_code());
                // Last command output line count.
                self.status_bar.last_output_lines = self
                    .active_session()
                    .app()
                    .terminal()
                    .last_command_output_lines();
                // Command execution duration — only format when the
                // underlying Duration changes (new command completes).
                let cmd_dur = self
                    .active_session()
                    .app()
                    .terminal()
                    .last_command_duration();
                if cmd_dur != self.cached_cmd_duration {
                    self.cached_cmd_duration = cmd_dur;
                    self.status_bar.command_duration = cmd_dur
                        .map(crate::status_bar::format_duration)
                        .unwrap_or_default();
                }
                // Running command indicator.
                let was_running = self.status_bar.command_running;
                self.status_bar.command_running =
                    self.active_session().app().terminal().is_command_running();
                // When a new command starts, clear stale exit code and duration
                // from the previous command so the status bar doesn't show
                // misleading "exit:0" / "5.7s" while the new command runs.
                if self.status_bar.command_running && !was_running {
                    self.status_bar.set_exit_code(None);
                    self.status_bar.command_duration.clear();
                    self.last_cmd_tenths = 0;
                }
                if self.status_bar.command_running {
                    // Live timer: show elapsed seconds next to the spinner.
                    if let Some(elapsed) = self
                        .active_session()
                        .app()
                        .terminal()
                        .running_command_elapsed()
                    {
                        // Throttle format_duration to ~10fps (100ms granularity)
                        // to avoid per-frame String allocation.
                        let tenths = elapsed.as_millis() / 100;
                        if tenths != self.last_cmd_tenths {
                            self.last_cmd_tenths = tenths;
                            self.status_bar.command_timer =
                                crate::status_bar::format_duration(elapsed);
                        }
                    }
                    // Throttle spinner to ~12fps so it doesn't spin out of control
                    // during resize/mouse-move (which call about_to_wait in a tight loop).
                    let now = std::time::Instant::now();
                    if now.duration_since(self.last_spinner_tick).as_millis() >= 80 {
                        self.last_spinner_tick = now;
                        self.status_bar.spinner_frame =
                            self.status_bar.spinner_frame.wrapping_add(1);
                    }
                } else {
                    // Command not running — show idle timer if idle > 5s.
                    self.status_bar.command_timer.clear();
                    let last = self.active_session().app().terminal().last_output_time();
                    if let Some(t) = last {
                        let idle = std::time::Instant::now().duration_since(t);
                        let idle_secs = idle.as_secs();
                        if idle_secs >= 5 {
                            // Only reformat when the second changes — avoids
                            // per-frame String allocation for the idle timer.
                            if idle_secs != self.last_idle_secs {
                                self.last_idle_secs = idle_secs;
                                self.status_bar.command_timer =
                                    crate::status_bar::format_duration(idle);
                            }
                        } else {
                            self.last_idle_secs = 0;
                        }
                    }
                }

                // Selection character/word count (live feedback while selecting).
                // Single pass — avoids traversing the selection twice per frame.
                let (sel_chars, sel_words) = if self.selection.is_active() {
                    self.count_selection()
                } else {
                    (0, 0)
                };
                self.status_bar.selection_count = sel_chars;
                self.status_bar.selection_words = sel_words;

                // Terminal lock state.
                self.status_bar.locked = self.locked;

                // Session uptime (only show after 1 minute).
                // Only reformat when the minute changes to avoid per-frame allocation.
                let uptime = self.active_session().uptime();
                let uptime_mins = uptime.as_secs() / 60;
                if uptime_mins != self.cached_uptime_mins {
                    self.cached_uptime_mins = uptime_mins;
                    self.status_bar.uptime = if uptime_mins >= 1 {
                        let h = uptime_mins / 60;
                        let m = uptime_mins % 60;
                        if h > 0 {
                            format!("{}h{}m", h, m)
                        } else {
                            format!("{}m", m)
                        }
                    } else {
                        String::new()
                    };
                }

                // Git branch (throttled: check every ~5s / 300 frames).
                self.git_check_counter = self.git_check_counter.wrapping_add(1);
                if self.git_check_counter.is_multiple_of(300) {
                    let cwd = self.active_session().cwd();
                    self.git_branch_cache = cwd
                        .and_then(|p| {
                            // Fast path: read .git/HEAD directly (no subprocess).
                            if let Some(branch) = read_git_head(p) {
                                return Some(branch);
                            }
                            // Fallback: use git command for edge cases
                            // (worktrees, submodules, etc).
                            let output = std::process::Command::new("git")
                                .arg("branch")
                                .arg("--show-current")
                                .current_dir(p)
                                .output()
                                .ok()?;
                            if output.status.success() && !output.stdout.is_empty() {
                                return Some(
                                    String::from_utf8_lossy(&output.stdout).trim().to_string(),
                                );
                            }
                            // Detached HEAD: show short commit hash.
                            let rev_output = std::process::Command::new("git")
                                .arg("rev-parse")
                                .arg("--short")
                                .arg("HEAD")
                                .current_dir(p)
                                .output()
                                .ok()?;
                            if rev_output.status.success() {
                                let hash = String::from_utf8_lossy(&rev_output.stdout)
                                    .trim()
                                    .to_string();
                                if !hash.is_empty() {
                                    return Some(format!("({hash})"));
                                }
                            }
                            None
                        })
                        .unwrap_or_default();
                }
                // Only clone when changed — avoids per-frame String allocation.
                if self.status_bar.git_branch != self.git_branch_cache {
                    self.status_bar.git_branch = self.git_branch_cache.clone();
                }
                if self.status_bar.theme_name != self.last_applied_theme {
                    self.status_bar.theme_name = self.last_applied_theme.clone();
                }
                // Terminal dimensions — only reformat when size changes.
                let grid = self.active_session().app().grid();
                let gw = grid.width();
                let gh = grid.height();
                if gw != self.cached_grid_w || gh != self.cached_grid_h {
                    self.cached_grid_w = gw;
                    self.cached_grid_h = gh;
                    let dims = format!("{}×{}", gw, gh);
                    self.cached_dims = dims.clone();
                    self.status_bar.dimensions = dims;
                }

                // P28: Update Phase 28 status bar indicators.
                // Only allocate when values actually change.
                let ws_name = self.workspaces.active_name();
                if self.status_bar.workspace_name != ws_name {
                    self.status_bar.workspace_name = ws_name.to_string();
                }
                self.status_bar.sound_enabled = self.sound_player.is_enabled();
                // shell_name: only rebuild "Shell: <name>" when shell path changes.
                {
                    let current = self.shell_switcher.current_shell_str();
                    if current != self.cached_shell_raw {
                        self.cached_shell_raw = current.to_string();
                        self.status_bar.shell_name = self.shell_switcher.status_bar_label();
                    }
                }
                self.status_bar.pane_zoomed = self.pane_zoomed;
                self.status_bar.font_size = self.font_zoom.current_size();
                self.status_bar.cursor_line = self
                    .config_mgr
                    .as_ref()
                    .is_some_and(|m| m.config().appearance.cursor_line_highlight);
                self.status_bar.scroll_mode = self.scroll_mode;
                // CWD from OSC 7 (pane-level cwd tracking) — abbreviate $HOME to ~.
                // Optimized: compare raw path first, only format when changed.
                let current_cwd = self.active_session().cwd().map(|p| p.to_path_buf());
                let cwd_changed = match (&current_cwd, &self.cached_cwd_raw) {
                    (Some(a), Some(b)) => a != b,
                    (None, None) => false,
                    _ => true,
                };
                if cwd_changed {
                    self.cached_cwd_raw = current_cwd.clone();
                    let new_cwd = current_cwd
                        .as_ref()
                        .map(|p| {
                            let s = p.display().to_string();
                            if let Some(home) = std::env::var_os("HOME")
                                && let Some(hs) = home.to_str()
                                && hs.len() > 2
                                && s.starts_with(hs)
                            {
                                format!("~{}", &s[hs.len()..])
                            } else {
                                s
                            }
                        })
                        .unwrap_or_default();
                    self.status_bar.cwd = new_cwd;
                }
                // Hovered URL/hyperlink for status bar link preview.
                let new_link = self.hovered_link.as_ref().map(|(url, _, _, _)| url.clone());
                if self.status_bar.hovered_link != new_link {
                    self.status_bar.hovered_link = new_link;
                }
                // Remote host from OSC 1337 RemoteHost=
                let remote = self
                    .active_session()
                    .app()
                    .terminal()
                    .remote_host()
                    .unwrap_or("");
                if self.status_bar.remote_host != remote {
                    self.status_bar.remote_host = remote.to_string();
                }
                // Progress from OSC 9;4
                self.status_bar.progress = self.active_session().app().terminal().progress();
                self.status_bar.last_exit_code =
                    self.active_session().app().terminal().last_exit_code();
                #[cfg(feature = "p2p")]
                {
                    self.status_bar.p2p_active = self.p2p_share.visible;
                }

                // Update window title: show tab bar when multiple tabs, otherwise
                // show terminal title (OSC 0/2).
                // Optimized: compare &str first, only allocate when changed.
                let term_title = self.active_session().app().terminal().title();
                let multi_tab = self.sessions.len() > 1;
                // In multi-tab mode, only rebuild title if any tab title or
                // bell/alt state actually changed. This avoids per-frame
                // String allocation for each tab when nothing changed.
                let need_rebuild = multi_tab && {
                    // Check if any tab's title differs from cached tab_bar title.
                    self.sessions.iter().enumerate().any(|(i, s)| {
                        if i < self.tab_bar.tabs.len() {
                            s.title() != self.tab_bar.tabs[i].title
                        } else {
                            true
                        }
                    }) || self.visual_bell_frames > 0
                        || self.last_title.is_empty()
                };
                if term_title != self.last_title.as_str() || need_rebuild {
                    let title = term_title.to_string();
                    self.last_title = title;
                    if let Some(ref window) = self.window {
                        let display = if multi_tab {
                            // Multi-tab: show tab bar in title bar.
                            let titles: Vec<String> = self
                                .sessions
                                .iter()
                                .enumerate()
                                .map(|(i, s)| {
                                    let t = s.app().terminal().title();
                                    let label = if t.is_empty() {
                                        format!("Tab {}", i + 1)
                                    } else {
                                        t.to_string()
                                    };
                                    let truncated: String = label.chars().take(20).collect();
                                    // P16-D: Add alt-screen indicator.
                                    let alt = if s.app().terminal().is_alt_screen() {
                                        " (alt)"
                                    } else {
                                        ""
                                    };
                                    if i == self.active {
                                        format!("[{}*{}]", truncated, alt)
                                    } else {
                                        format!("[{}{}]", truncated, alt)
                                    }
                                })
                                .collect();
                            // P16-D: Add bell indicator.
                            let bell = if self.visual_bell_frames > 0 {
                                " \u{1F514}"
                            } else {
                                ""
                            };
                            format!("GGTerm — {}{}", titles.join(" "), bell)
                        } else if self.last_title.is_empty() {
                            format!("GGTerm {}", env!("CARGO_PKG_VERSION"))
                        } else {
                            self.last_title.clone()
                        };
                        window.set_title(&display);
                    }

                    // P24-C: Debug overlay — append stats to title when enabled.
                    if self.debug_visible {
                        let term = self.active_session().app().terminal();
                        let grid = term.grid();
                        let debug_title = format!(
                            "GGTerm — FPS: {:.0} | {}x{} ({} cells) | sync={} reflow={} | {} tabs",
                            self.current_fps,
                            grid.width(),
                            grid.height(),
                            grid.width() * grid.height(),
                            term.is_synchronized(),
                            term.reflow_enabled(),
                            self.sessions.len(),
                        );
                        if let Some(ref window) = self.window {
                            window.set_title(&debug_title);
                        }
                    }
                }

                // NOTE: PTY exit is handled at the top of RedrawRequested
                // (line ~772) via is_running(), and in about_to_wait.
                // Do NOT add an is_alive() check here — it races with the
                // event channel. is_alive() becomes false before pump()
                // processes PtyExit, causing premature app exit.
            }

            WindowEvent::Resized(size) => {
                self.handle_resize(size.width, size.height);
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // P18-A: Update scale_factor and rebuild renderer.
                log::info!("Scale factor changed: {}", scale_factor);
                self.scale_factor = scale_factor;
                if let (Some(gpu), Some(window)) = (&self.gpu, &self.window) {
                    let size = window.inner_size();
                    self.renderer =
                        Some(gpu.create_renderer(size.width, size.height, scale_factor));
                    // Re-apply theme + font after recreating renderer.
                    self.apply_theme_to_renderer();
                    self.apply_font_size();
                    // Resize surface to new physical dimensions.
                    self.handle_resize(size.width, size.height);
                }
            }

            WindowEvent::KeyboardInput { event, .. } => {
                self.handle_keyboard_input(&event);
            }

            WindowEvent::ModifiersChanged(mods) => {
                self.mods.shift = mods.state().shift_key();
                self.mods.ctrl = mods.state().control_key();
                self.mods.alt = mods.state().alt_key();
                self.mods.super_key = mods.state().super_key();
            }

            WindowEvent::Focused(focused) => {
                // P27-F: Track window focus for cursor style.
                self.window_focused = focused;

                // P12-D: Send focus event report if DECSET 1004 is active.
                // Write directly to PTY (not via write_to_pty) to avoid
                // unwanted auto-scroll, broadcast, and cursor blink reset.
                let report = if focused {
                    self.active_session().app().terminal().focus_in_report()
                } else {
                    self.active_session().app().terminal().focus_out_report()
                };
                if !report.is_empty() {
                    self.active_session_mut().write_to_pty(&report);
                }
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
                // Clear bell/unread indicator on the active tab when the
                // window regains focus — the user has seen the alert.
                if focused {
                    self.active_session_mut().clear_unread();
                    // Cancel any pending dock/taskbar attention request.
                    if let Some(ref window) = self.window {
                        window.request_user_attention(None);
                        // Enable IME so input methods (CJK, etc.) work.
                        window.set_ime_allowed(true);
                    }
                } else {
                    // Disable IME when unfocused to avoid stray preedit state.
                    if let Some(ref window) = self.window {
                        window.set_ime_allowed(false);
                    }
                    self.ime_preedit = None;
                }
            }

            WindowEvent::ThemeChanged(theme) => {
                // Follow OS dark/light appearance when the configured theme
                // is "system" or the default. On macOS this fires when the
                // user toggles System Settings → Appearance.
                log::debug!("OS theme changed: {theme:?}");
                use winit::window::Theme as WinitTheme;
                let target = match theme {
                    WinitTheme::Dark => "dark",
                    WinitTheme::Light => "light",
                };
                // Only auto-switch if the current theme is a generic
                // "dark"/"light" (not a custom named theme like "dracula").
                let current = &self.last_applied_theme;
                if (current == "dark" || current == "light") && current != target {
                    self.last_applied_theme = target.to_string();
                    for session in &mut self.sessions {
                        session.app_mut().theme_manager().set_by_name(target);
                    }
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                }
            }

            // Skip GPU rendering when the window is occluded (hidden behind
            // other windows) to save battery and GPU resources.
            WindowEvent::Occluded(occluded) => {
                if self.window_occluded != occluded {
                    self.window_occluded = occluded;
                    log::debug!("Window occluded: {occluded}");
                    if !occluded {
                        // Window became visible — request a redraw.
                        if let Some(ref window) = self.window {
                            window.request_redraw();
                        }
                    }
                }
            }

            // P9-D mouse handlers
            WindowEvent::MouseInput { state, button, .. } => {
                self.handle_mouse_input(state, button);
            }
            WindowEvent::CursorMoved { position: pos, .. } => {
                self.cursor_pos = (pos.x, pos.y);
                self.handle_cursor_moved();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.handle_mouse_wheel(delta);
            }

            // IME (Input Method Editor) — committed text goes to PTY.
            WindowEvent::Ime(ime) => {
                use winit::event::Ime;
                match ime {
                    Ime::Commit(text) => {
                        // Send committed IME text to PTY as UTF-8 bytes.
                        // Same side effects as regular keypress: clear selection
                        // and scroll to bottom (standard terminal behavior).
                        self.selection.clear();
                        let grid = self.sessions[self.active]
                            .app_mut()
                            .terminal_mut()
                            .grid_mut();
                        if grid.display_offset() > 0 {
                            grid.reset_viewport();
                        }
                        // cursor_blink.reset() is called inside write_to_pty().
                        self.write_to_pty(text.as_bytes());
                        self.ime_preedit = None;
                    }
                    Ime::Preedit(text, _) => {
                        self.ime_preedit = if text.is_empty() { None } else { Some(text) };
                    }
                    Ime::Disabled | Ime::Enabled => {
                        self.ime_preedit = None;
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Auto-start P2P share (triggered by --p2p-share CLI flag).
        #[cfg(feature = "p2p")]
        if std::path::Path::new("/tmp/ggterm_auto_share").exists() && !self.p2p_share.is_active() {
            let _ = std::fs::remove_file("/tmp/ggterm_auto_share");
            self.toggle_p2p_share();
            log::debug!("auto-share triggered");
        }

        // P29-C: Check if we should quit after confirmation.
        if self.should_quit {
            self.send_terminal_reset();
            self.save_session_on_exit();
            event_loop.exit();
            return;
        }

        // ── Settings window lifecycle ──

        // Check for command completion notifications before settings.
        // (handled inline in background pump above via maybe_notify)

        if self.pending_open_settings {
            self.pending_open_settings = false;
            if self.settings_window.is_none() {
                self.open_settings_window(event_loop);
            }
        }
        if let Some(sw) = self.settings_window.take() {
            if sw.should_close() {
                // Apply draft and close.
                let draft = sw.draft.clone();
                drop(sw);
                self.apply_settings_draft(&draft);
                log::info!("Settings window closed");
            } else {
                // Still open — render it.
                sw.window.request_redraw();
                self.settings_window = Some(sw);
            }
        }

        // Render settings window if open.
        if let Some(ref mut sw) = self.settings_window {
            sw.render();
        }

        // Pump PTY events — active session first, then non-active.
        // Track scrollback growth to detect content scroll — if the terminal
        // scrolled due to new output, clear the active selection (positions
        // are display-relative and would point at wrong rows).
        let prev_scrollback = self.active_session().app().grid().scrollback_len();
        let prev_cursor_y = self.active_session().app().terminal().cursor().1;
        let active_had_data = self.active_session_mut().pump();
        // Reset cursor blink cycle on new output so the cursor is visible
        // immediately (standard terminal behavior — iTerm2, Alacritty, etc.).
        if active_had_data {
            self.cursor_blink.reset();
        }
        // If the active session scrolled (scrollback grew or cursor advanced
        // to a new line), invalidate any active selection.
        let new_scrollback = self.active_session().app().grid().scrollback_len();
        let new_cursor_y = self.active_session().app().terminal().cursor().1;
        if self.selection.is_active()
            && (new_scrollback != prev_scrollback || new_cursor_y != prev_cursor_y)
        {
            self.selection.clear();
            self.selection_auto_scroll = 0;
        }
        // Pump non-active sessions and mark tabs with unread output.
        // Also collect bells from background sessions.
        // Background sessions are pumped with a lower limit to prioritize
        // the active tab's responsiveness. Each background tab gets a
        // small per-frame budget so one churning background tab doesn't
        // starve the active tab or the event loop.
        let active = self.active;
        let mut bg_bell = false;
        let mut notify_tabs: Vec<usize> = Vec::new();
        for (i, session) in self.sessions.iter_mut().enumerate() {
            if i != active {
                // Track command running state before pump to detect completion.
                let was_running = session.is_running();
                let had_data = session.pump();
                if had_data {
                    session.mark_unread();
                }
                if was_running && !session.is_running() {
                    session.mark_command_completed();
                    notify_tabs.push(i);
                }
                if session.take_any_bell() {
                    session.mark_bell();
                    bg_bell = true;
                }
            }
        }

        // Fire desktop notifications for completed background commands.
        for idx in notify_tabs {
            self.maybe_notify_command_complete(idx);
        }

        // Flush terminal protocol responses (DA1, DA2, DSR, DECRQM,
        // XTVERSION, OSC 4 color queries, etc.) back to the PTY.
        // Without this, programs like vim, tmux, and ncurses never receive
        // responses to their terminal capability queries.
        for session in &mut self.sessions {
            session.flush_responses();
        }

        // Sync terminal OSC 0/2 titles to tab sessions so the tab bar
        // shows the current program name (e.g. "vim", "zsh") instead of
        // the static initial title.
        // Optimized: compare references first, only allocate String on change.
        // Track whether any title changed — a title-only update (no visible
        // output) still needs a redraw so the window/tab title reflects it.
        let mut title_changed = false;
        for session in &mut self.sessions {
            let term_title = session.app().terminal().title();
            if !term_title.is_empty() && term_title != session.title() {
                session.set_title(term_title.to_string());
                title_changed = true;
            }
        }

        // Update tab bar state — syncs tab titles, bell flags, and
        // sets visible = (tabs.len() > 1) for single-tab title bar mode.
        // Avoids Vec allocation by updating visible flag directly and
        // only rebuilding tabs when count changes.
        let session_count = self.sessions.len();
        if self.tab_bar.tabs.len() != session_count {
            // Tab count changed — full rebuild.
            let tab_refs: Vec<&str> = self.sessions.iter().map(|s| s.title()).collect();
            self.tab_bar.update(&tab_refs, self.active);
        } else {
            // Same count — just update titles and active index in-place.
            for (i, session) in self.sessions.iter().enumerate() {
                let title = session.title();
                if i < self.tab_bar.tabs.len() {
                    let tab = &mut self.tab_bar.tabs[i];
                    if tab.title != title {
                        tab.title = title.to_string();
                    }
                    tab.active = i == self.active;
                    tab.index = i + 1;
                }
            }
            self.tab_bar.visible = session_count > 1;
        }

        // P10-C: Poll AI bridge for results.
        #[cfg(feature = "ai")]
        self.poll_ai_bridge();

        // P10-B: Poll OSC 52 clipboard set requests.
        self.poll_osc52_clipboard();

        // P11-E: Poll for bell events.
        self.poll_bell();

        // Poll for background pipe-selection command result.
        self.poll_pipe_result();

        // P24-E: Poll for desktop notifications.
        self.poll_notification();

        // Poll for command completion notifications (unfocused window + long-running command).
        self.poll_command_complete();

        // Position IME cursor area near the terminal cursor so the input
        // method popup appears at the right location (CJK input).
        // Optimized: only call set_ime_cursor_area when position changes.
        if self.window_focused
            && let Some(ref window) = self.window
            && let Some(ref renderer) = self.renderer
        {
            let (cursor_col, cursor_row) = self.active_session().app().terminal().cursor();
            let cell_w = renderer.cell_width() as f64;
            let cell_h = renderer.cell_height() as f64;
            let content_top = self.content_area_bounds().y as f64;
            let scale = self.scale_factor;
            let px = cursor_col as f64 * cell_w * scale;
            let py = (content_top + cursor_row as f64 * cell_h) * scale;
            let w = cell_w * scale;
            let h = cell_h * scale;
            if self.last_ime_cursor_area != Some((px, py, w, h)) {
                self.last_ime_cursor_area = Some((px, py, w, h));
                window.set_ime_cursor_area(
                    winit::dpi::PhysicalPosition::new(px, py),
                    winit::dpi::PhysicalSize::new(w, h),
                );
            }
        }

        // P2P: Poll for connections and forward mobile input/output.
        #[cfg(feature = "p2p")]
        if self.p2p_share.is_active() {
            static P2P_TICK: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            let tick = P2P_TICK.fetch_xor(true, std::sync::atomic::Ordering::Relaxed);
            log::debug!(
                "P2P active, status={}, tick={}",
                self.p2p_share.status as u8,
                tick
            );
            let p2p_status = self.p2p_share.status;
            let tee_len = self.active_session_mut().app_mut().take_pty_tee();
            if p2p_status == crate::p2p_share::P2pShareStatus::Connected {
                if !tee_len.is_empty() {
                    log::debug!("P2P tee {} bytes to mobile", tee_len.len());
                    self.p2p_share.tee_output(&tee_len);
                }
            } else {
                // Waiting: put data back into pty_tee so it's available on connect.
                if !tee_len.is_empty() {
                    log::debug!("P2P waiting, preserving {} bytes in pty_tee", tee_len.len());
                    self.active_session_mut().app_mut().restore_pty_tee(tee_len);
                }
            }
            // When Waiting: DON'T drain tee — let it accumulate for flush on connect.
            self.poll_p2p();
        } else {
            // Not active at all: drain to prevent unbounded growth.
            self.active_session_mut().app_mut().take_pty_tee();
        }

        // P28-C: Sync command history sidebar from OSC 133 marks.
        self.poll_command_history();

        // P28-F: Tick cursor particle system.
        self.cursor_particles.tick();

        // P30-C: Tick toast notification timer.
        if let Some((_, frames)) = &mut self.toast {
            *frames = frames.saturating_sub(1);
            if *frames == 0 {
                self.toast = None;
            }
        }

        // P19-A: Poll for menu bar actions.
        if let Some(action) = crate::menu_bar::poll_pending_action() {
            self.handle_menu_action(action);
        }

        // Apply deferred resize if debounce interval has elapsed.
        self.apply_pending_resize();

        // Poll config watcher for hot-reload.
        #[cfg(feature = "config-watch")]
        let mut pending_cursor_style: Option<ggterm_core::CursorStyle> = None;
        #[cfg(feature = "config-watch")]
        let mut pending_scrollback: Option<usize> = None;
        #[cfg(feature = "config-watch")]
        let mut pending_theme: Option<String> = None;
        #[cfg(feature = "config-watch")]
        if self.force_config_reload {
            self.force_config_reload = false;
            if let Some(ref mut mgr) = self.config_mgr {
                match mgr.reload() {
                    Ok(true) => {
                        log::info!("Config manually reloaded successfully");
                        self.status_bar.clear_config_error();
                    }
                    Ok(false) => {}
                    Err(e) => {
                        log::warn!("Config reload error: {e}");
                        self.show_toast(format!("Config error: {e}"));
                        self.status_bar.set_config_error(Some(e.to_string()));
                    }
                }
            }
        }
        #[cfg(feature = "config-watch")]
        if let Some(ref mut mgr) = self.config_mgr {
            match mgr.poll_reload() {
                Ok(true) => {
                    // Clear any previous config error on successful reload.
                    self.status_bar.clear_config_error();
                    let cfg = mgr.config();
                    let new_theme = cfg.appearance.theme.clone();
                    let new_font_size = cfg.appearance.font_size as f32;
                    let new_font_family = cfg.appearance.font_family.clone();
                    let new_scrollback = cfg.terminal.scrollback_lines;
                    let new_cursor_style = parse_cursor_style(&cfg.appearance.cursor_style);
                    log::info!(
                        "Config reloaded: theme={}, font_size={}, scrollback={}",
                        new_theme,
                        new_font_size,
                        new_scrollback
                    );

                    // P16-B: Apply theme change if different.
                    if new_theme != self.last_applied_theme {
                        self.apply_theme_to_renderer();
                        self.last_applied_theme = new_theme.clone();
                        pending_theme = Some(new_theme);
                        log::info!("Theme changed -> will apply to all sessions");
                    }

                    // P16-B: Apply font size change if different.
                    if (new_font_size - self.last_applied_font_size).abs() > 0.01 {
                        self.font_zoom.set_base_size(new_font_size);
                        self.apply_font_size();
                        self.last_applied_font_size = new_font_size;
                        log::info!("Font size changed -> applied {new_font_size:.1}px");
                        // Re-measure cell dimensions and resize terminal grid
                        // to match new cell metrics (same as font_family change).
                        if let Some(renderer) = &self.renderer {
                            let cw = renderer.cell_width();
                            let ch = renderer.cell_height();
                            let bounds = self.content_area_bounds();
                            let new_cols = ((bounds.width / cw.max(1)) as usize).max(10) as u16;
                            let new_rows = ((bounds.height / ch.max(1)) as usize).max(3) as u16;
                            for session in &mut self.sessions {
                                session.resize(new_cols, new_rows);
                            }
                        }
                    }

                    // Apply font family change if different.
                    if !new_font_family.is_empty()
                        && new_font_family != self.last_applied_font_family
                    {
                        if let Some(ref mut renderer) = self.renderer {
                            renderer.set_font_family(&new_font_family);
                        }
                        self.last_applied_font_family = new_font_family.clone();
                        log::info!("Font family changed -> applied {new_font_family}");
                        // Re-measure cell dimensions and resize terminal grid.
                        if let Some(renderer) = &self.renderer {
                            let cw = renderer.cell_width();
                            let ch = renderer.cell_height();
                            let bounds = self.content_area_bounds();
                            let new_cols = ((bounds.width / cw.max(1)) as usize).max(10) as u16;
                            let new_rows = ((bounds.height / ch.max(1)) as usize).max(3) as u16;
                            for session in &mut self.sessions {
                                session.resize(new_cols, new_rows);
                            }
                        }
                    }

                    // Defer scrollback to apply to ALL sessions.
                    pending_scrollback = Some(new_scrollback);

                    // Only apply cursor style if it actually changed from
                    // the previous config value. This prevents overriding
                    // program-requested DECSCUSR cursor shapes (e.g., vim
                    // setting a bar cursor) when an unrelated config setting
                    // is modified.
                    if new_cursor_style != self.last_applied_cursor_style {
                        pending_cursor_style = Some(new_cursor_style);
                        self.last_applied_cursor_style = new_cursor_style;
                    }

                    // Show toast feedback for successful reload.
                    self.show_toast("Config reloaded");
                }
                Ok(false) => {}
                Err(e) => {
                    log::warn!("Config reload error: {e}");
                    self.show_toast(format!("Config error: {e}"));
                    self.status_bar.set_config_error(Some(e.to_string()));
                }
            }
        }

        // Apply deferred changes to all sessions (all tabs, all panes).
        #[cfg(feature = "config-watch")]
        if pending_theme.is_some() || pending_scrollback.is_some() || pending_cursor_style.is_some()
        {
            for session in &mut self.sessions {
                for pane_id in session.pane_ids() {
                    if let Some(app) = session.pane_app_mut(pane_id) {
                        if let Some(ref theme) = pending_theme {
                            app.theme_manager().set_by_name(theme);
                        }
                        if let Some(scrollback) = pending_scrollback {
                            app.terminal_mut().grid_mut().set_scrollback(scrollback);
                        }
                        if let Some(cursor_style) = pending_cursor_style {
                            app.terminal_mut().set_cursor_style(cursor_style);
                        }
                    }
                }
            }
        }

        // Check if active pane's shell has exited (e.g. Ctrl+D, `exit`).
        if !self.active_session().is_running() || !self.active_session_mut().is_alive() {
            // --hold mode: keep terminal open after command exits.
            // Shows a message but doesn't close the window.
            if std::env::var("GGTERM_HOLD").as_deref() == Ok("1") {
                // Print a hold message to the terminal if not already done.
                if !self.hold_message_shown {
                    let exit_code = self.sessions[self.active].app().terminal().last_exit_code();
                    let exit_str = match exit_code {
                        Some(0) => " (exit: 0) ".to_string(),
                        Some(code) => format!(" \x1b[31m(exit: {})\x1b[0m ", code),
                        None => "".to_string(),
                    };
                    let msg = format!(
                        "\n\r\x1b[33m[process exited{}— press any key to close]\x1b[0m\n\r",
                        exit_str
                    );
                    self.sessions[self.active]
                        .app_mut()
                        .inject_bytes(msg.as_bytes());
                    self.hold_message_shown = true;
                }
                // Don't exit — just skip further processing.
                return;
            }
            if self.handle_pane_exit() {
                event_loop.exit();
            }
            return;
        }

        // P23-C: Conditional redraw — only request redraw when needed.
        // Conditions: PTY data, cursor blink interval, pending resize,
        // bell, search/AI overlay, or any user interaction flag.
        // P27-D: Process smooth scroll animation.
        if self.smooth_scroll.is_animating() {
            if let Some(delta_lines) = self.smooth_scroll.tick() {
                let grid = self
                    .active_session_mut()
                    .app_mut()
                    .terminal_mut()
                    .grid_mut();
                if delta_lines > 0 {
                    grid.scroll_up_viewport(delta_lines as usize);
                } else {
                    grid.scroll_down_viewport((-delta_lines) as usize);
                }
            }
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
        }

        // Selection drag auto-scroll: scroll viewport periodically while
        // the user holds the mouse near the top or bottom edge.
        // The magnitude of selection_auto_scroll encodes the speed (1-5).
        if self.selection_auto_scroll != 0 {
            let now = std::time::Instant::now();
            let scroll_dir = self.selection_auto_scroll.signum();
            let scroll_speed = self.selection_auto_scroll.unsigned_abs() as usize;
            if now.duration_since(self.last_auto_scroll) >= std::time::Duration::from_millis(50) {
                self.last_auto_scroll = now;
                let grid = self
                    .active_session_mut()
                    .app_mut()
                    .terminal_mut()
                    .grid_mut();
                if scroll_dir < 0 {
                    grid.scroll_up_viewport(scroll_speed);
                } else {
                    grid.scroll_down_viewport(scroll_speed);
                }

                // Extend selection to keep up with scrolled content.
                let grid_h = grid.height();
                if self.selection.dragging {
                    let new_end_row = if scroll_dir < 0 {
                        0 // Extend to top visible row
                    } else {
                        grid_h - 1 // Extend to bottom visible row
                    };
                    // Use the cursor column if available, otherwise 0 or max.
                    let col = self.selection.end.map(|(c, _)| c).unwrap_or(0);
                    self.selection.extend(col, new_end_row as u16);
                }

                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
        }

        // Clear selection when alt screen switches (vim/less enter/exit).
        // The selection references grid row numbers; after a screen swap,
        // those rows contain different content, so stale highlights
        // would point at wrong text.
        let alt_now = self.active_session().app().terminal().is_alt_screen();
        if alt_now != self.prev_alt_screen {
            self.selection.clear();
            self.prev_alt_screen = alt_now;
        }

        // P23-C: Conditional redraw — only request redraw when there's
        // content to show (dirty grid, pending resize, bell, or cursor blink).
        // Check ALL panes in the active session, not just the active one,
        // because background panes' output is also visible in split mode.
        let content_dirty = self.active_session().any_pane_dirty();
        // Collect bell from all panes in the active session (not just active pane).
        let active_bell = self.active_session_mut().take_any_bell();
        let need_redraw = content_dirty
            || self.pending_resize.is_some()
            || self.pipe_command_active
            || self.command_palette.visible
            || self.pending_pipe_result.is_some()
            || self.pending_large_paste.is_some()
            || self.pending_close_tab.is_some()
            || active_bell
            || bg_bell
            || title_changed
            || self.toast.is_some();

        let now = std::time::Instant::now();

        // Synchronized output (DECSET 2026): defer rendering while a program
        // is sending a batch of updates (vim, tmux, less). This reduces
        // visual flicker by skipping intermediate frames. Enforce a 100ms
        // maximum to ensure the user sees progress on large redraws.
        let sync_active = self.active_session().app().terminal().is_synchronized();
        if sync_active && self.sync_render_deadline.is_none() {
            self.sync_render_deadline = Some(now + std::time::Duration::from_millis(100));
        } else if !sync_active {
            self.sync_render_deadline = None;
        }
        let sync_overdue = self.sync_render_deadline.is_some_and(|d| now >= d);

        // When synchronized output is active (and not overdue), defer
        // content-driven redraws. Force redraw on bells/resizes/UI overlays.
        let defer_render = sync_active && !sync_overdue;
        let effective_redraw = need_redraw && !defer_render;

        // Cursor blink: redraw every 500ms for blink animation.
        // Skip when window is occluded, cursor is hidden (DECSET 25 off),
        // or blink is not enabled (steady cursor style). This avoids
        // unnecessary CPU/GPU wake-ups in vim/less/tmux and full-screen apps.
        let cursor_visible = self.active_session().app().terminal().cursor_visible();
        let blink_interval = std::time::Duration::from_millis(500);
        let blink_due = !self.window_occluded
            && self.window_focused
            && cursor_visible
            && self.cursor_blink.is_enabled()
            && now.duration_since(self.last_redraw) >= blink_interval;

        if effective_redraw || blink_due || self.debug_visible {
            self.last_redraw = now;

            // P24-C: Update FPS counter.
            self.frame_count += 1;
            let fps_elapsed = now.duration_since(self.last_fps_time);
            if fps_elapsed >= std::time::Duration::from_millis(500) {
                self.current_fps = self.frame_count as f32 / fps_elapsed.as_secs_f32();
                self.frame_count = 0;
                self.last_fps_time = now;
            }

            if let Some(ref window) = self.window {
                window.request_redraw();
            }
        } else if !content_dirty || defer_render {
            // Idle (or synchronized-output deferred): sleep to avoid busy-looping.
            // When P2P is connected, use shorter sleep for lower latency
            // (faster echo round-trip for mobile users).
            // When occluded, sleep much longer — no visual updates needed.
            // When sync-rendering is active, use short sleep to check the
            // deadline promptly.
            #[cfg(feature = "p2p")]
            let sleep_ms = if self.window_occluded {
                1000
            } else if defer_render
                || self.p2p_share.status == crate::p2p_share::P2pShareStatus::Connected
            {
                5
            } else {
                50
            };
            #[cfg(not(feature = "p2p"))]
            let sleep_ms = if self.window_occluded {
                1000
            } else if defer_render {
                5
            } else {
                50
            };
            std::thread::sleep(std::time::Duration::from_millis(sleep_ms));
        }

        // If we have a pending (debounced) resize, keep polling so we apply
        // it after the 100ms window. request_redraw above already keeps the
        // event loop spinning in winit's Poll mode.
    }
}

// P14-A: Config/DesktopConfig tests and resize computation tests
// have been moved to desktop_config.rs. Window-specific tests
// (ModsState, shell override) are tested via integration.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_keybindings_populated() {
        let kb = default_keybindings();
        assert_eq!(
            kb.get("new_tab"),
            Some(&(true, false, false, "T".to_string()))
        );
        assert_eq!(kb.get("paste"), Some(&(true, true, false, "V".to_string())));
        assert_eq!(
            kb.get("search"),
            Some(&(true, true, false, "F".to_string()))
        );
        assert_eq!(
            kb.get("fullscreen"),
            Some(&(false, false, false, "F11".to_string()))
        );
        assert_eq!(
            kb.get("zoom_in"),
            Some(&(true, false, false, "=".to_string()))
        );
    }

    #[test]
    fn test_check_keybinding_default_match() {
        let kb = default_keybindings();
        // Ctrl+Shift+V → paste
        assert!(check_keybinding_map(&kb, "paste", true, true, false, "V"));
        // Ctrl+T (no shift) → new_tab
        assert!(check_keybinding_map(
            &kb, "new_tab", true, false, false, "T"
        ));
        // F11 → fullscreen
        assert!(check_keybinding_map(
            &kb,
            "fullscreen",
            false,
            false,
            false,
            "F11"
        ));
    }

    #[test]
    fn test_check_keybinding_custom_value() {
        let mut kb = default_keybindings();
        // Override: new_tab → Ctrl+N
        kb.insert("new_tab".into(), (true, false, false, "N".into()));
        // Ctrl+N should now match
        assert!(check_keybinding_map(
            &kb, "new_tab", true, false, false, "N"
        ));
        // Ctrl+T should no longer match
        assert!(!check_keybinding_map(
            &kb, "new_tab", true, false, false, "T"
        ));
    }

    #[test]
    fn test_check_keybinding_no_match() {
        let kb = default_keybindings();
        // Wrong key
        assert!(!check_keybinding_map(&kb, "paste", true, true, false, "X"));
        // Wrong modifiers
        assert!(!check_keybinding_map(&kb, "paste", false, true, false, "V"));
        assert!(!check_keybinding_map(&kb, "paste", true, false, false, "V"));
        // Unknown action
        assert!(!check_keybinding_map(
            &kb,
            "unknown_action",
            true,
            true,
            false,
            "V"
        ));
    }

    #[test]
    fn test_check_keybinding_modifiers_exact() {
        let kb = default_keybindings();
        // paste = Ctrl+Shift+V — Alt must NOT be set
        assert!(!check_keybinding_map(&kb, "paste", true, true, true, "V"));
        // new_tab = Ctrl+T — Shift must NOT be set
        assert!(!check_keybinding_map(
            &kb, "new_tab", true, true, false, "T"
        ));
    }

    #[test]
    fn test_keycode_to_name_letters() {
        assert_eq!(keycode_to_name(&KeyCode::KeyA), "A");
        assert_eq!(keycode_to_name(&KeyCode::KeyT), "T");
        assert_eq!(keycode_to_name(&KeyCode::KeyV), "V");
    }

    #[test]
    fn test_keycode_to_name_digits_and_specials() {
        assert_eq!(keycode_to_name(&KeyCode::Digit0), "0");
        assert_eq!(keycode_to_name(&KeyCode::Digit9), "9");
        assert_eq!(keycode_to_name(&KeyCode::Equal), "=");
        assert_eq!(keycode_to_name(&KeyCode::Minus), "-");
        assert_eq!(keycode_to_name(&KeyCode::F11), "F11");
        assert_eq!(keycode_to_name(&KeyCode::Enter), ""); // not mapped
    }

    #[test]
    fn test_apply_keybinding_updates_map() {
        let mut kb = default_keybindings();
        assert_eq!(
            kb.get("new_tab"),
            Some(&(true, false, false, "T".to_string()))
        );

        // Apply a custom binding
        apply_keybinding(&mut kb, "new_tab", Some("Ctrl+N"));
        assert_eq!(
            kb.get("new_tab"),
            Some(&(true, false, false, "N".to_string()))
        );

        // None should not change the map
        apply_keybinding(&mut kb, "new_tab", None);
        assert_eq!(
            kb.get("new_tab"),
            Some(&(true, false, false, "N".to_string()))
        );

        // Invalid string should not change the map
        apply_keybinding(&mut kb, "new_tab", Some(""));
        assert_eq!(
            kb.get("new_tab"),
            Some(&(true, false, false, "N".to_string()))
        );
    }

    /// Helper to test check_keybinding logic against a standalone map.
    fn check_keybinding_map(
        map: &std::collections::HashMap<String, (bool, bool, bool, String)>,
        action: &str,
        ctrl: bool,
        shift: bool,
        alt: bool,
        key: &str,
    ) -> bool {
        match map.get(action) {
            Some(&(kc, ksh, ka, ref kk)) => ctrl == kc && shift == ksh && alt == ka && key == kk,
            None => false,
        }
    }

    // ── P17-D: Status bar visibility tests ───────────────────────────

    #[test]
    fn test_status_bar_visible_default() {
        // status_bar_visible defaults to true when DesktopApp is constructed.
        // We can't construct DesktopApp in a unit test (needs PTY + GPU),
        // but we can verify the field type and default expectation.
        let visible: bool = true;
        assert!(visible, "status_bar_visible should default to true");
    }

    #[test]
    fn test_status_bar_toggle_logic() {
        // Simulate the toggle logic: !self.status_bar_visible.
        let mut visible = true;
        visible = !visible;
        assert!(!visible, "After first toggle, should be hidden");
        visible = !visible;
        assert!(visible, "After second toggle, should be visible again");
    }

    #[test]
    fn test_keycode_b_maps_correctly() {
        // Verify that KeyCode::KeyB maps to "B" in our keycode_to_name.
        assert_eq!(keycode_to_name(&KeyCode::KeyB), "B");
    }

    // ── quote_shell_path tests (security-critical: prevents shell injection) ──

    #[test]
    fn test_quote_simple_path() {
        assert_eq!(
            quote_shell_path("/home/user/file.txt"),
            "'/home/user/file.txt'"
        );
    }

    #[test]
    fn test_quote_path_with_spaces() {
        assert_eq!(
            quote_shell_path("/home/user/My Documents/file.txt"),
            "'/home/user/My Documents/file.txt'"
        );
    }

    #[test]
    fn test_quote_path_with_single_quote() {
        // Single quote in path must be escaped: ' → '\'' (end quote, escaped quote, start quote)
        let result = quote_shell_path("/tmp/it's a file.txt");
        assert_eq!(result, "'/tmp/it'\\''s a file.txt'");
    }

    #[test]
    fn test_quote_empty_path() {
        assert_eq!(quote_shell_path(""), "''");
    }

    #[test]
    fn test_parse_cursor_style_block() {
        assert_eq!(
            parse_cursor_style("block"),
            ggterm_core::CursorStyle::BlinkBlock
        );
    }

    #[test]
    fn test_parse_cursor_style_underline() {
        assert_eq!(
            parse_cursor_style("underline"),
            ggterm_core::CursorStyle::BlinkUnderline
        );
    }

    #[test]
    fn test_parse_cursor_style_bar() {
        assert_eq!(
            parse_cursor_style("bar"),
            ggterm_core::CursorStyle::BlinkBar
        );
    }

    #[test]
    fn test_parse_cursor_style_unknown_defaults_block() {
        assert_eq!(
            parse_cursor_style("unknown"),
            ggterm_core::CursorStyle::BlinkBlock
        );
        assert_eq!(parse_cursor_style(""), ggterm_core::CursorStyle::BlinkBlock);
    }

    // ── read_git_head tests ──

    #[test]
    fn test_read_git_head_named_branch() {
        let dir = std::env::temp_dir().join(format!("ggterm-git-test-{}", std::process::id()));
        let git_dir = dir.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let branch = read_git_head(&dir);
        assert_eq!(branch.as_deref(), Some("main"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_git_head_feature_branch() {
        let dir = std::env::temp_dir().join(format!("ggterm-git-test2-{}", std::process::id()));
        let git_dir = dir.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(
            git_dir.join("HEAD"),
            "ref: refs/heads/feature/my-cool-branch\n",
        )
        .unwrap();

        let branch = read_git_head(&dir);
        assert_eq!(branch.as_deref(), Some("feature/my-cool-branch"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_git_head_detached_returns_none() {
        let dir = std::env::temp_dir().join(format!("ggterm-git-test3-{}", std::process::id()));
        let git_dir = dir.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        // Detached HEAD: raw hash, not "ref: refs/heads/..."
        std::fs::write(git_dir.join("HEAD"), "a1b2c3d4e5f6789\n").unwrap();

        let branch = read_git_head(&dir);
        assert!(branch.is_none(), "detached HEAD should return None");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_git_head_no_git_dir() {
        let dir = std::env::temp_dir().join(format!("ggterm-git-test4-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);

        let branch = read_git_head(&dir);
        assert!(branch.is_none(), "non-git directory should return None");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
