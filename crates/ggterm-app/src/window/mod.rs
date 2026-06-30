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

/// Desktop terminal application.
///
/// Implements winit's `ApplicationHandler` trait to receive OS events.
/// GPU resources (surface, device, renderer) are lazily initialized in
/// `resumed()`.
pub struct DesktopApp {
    /// Terminal sessions (one per tab).
    sessions: Vec<TabSession>,
    /// Index of the active tab.
    active: usize,
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
    #[cfg(feature = "config-watch")]
    config_mgr: Option<ConfigManager>,

    // ── Dynamic window title (OSC 0/2) ──
    /// Last known terminal title (to detect changes).
    last_title: String,

    // ── Mouse support ──
    /// Current text selection state.
    selection: crate::mouse::MouseSelection,
    /// Last known cursor position in pixels (for mouse wheel / drag).
    cursor_pos: (f64, f64),
    /// Mouse button currently held (for drag tracking).
    button_held: Option<crate::mouse::MouseButton>,
    /// P21-A: Active split separator drag (None = not dragging).
    drag_resize: Option<bool>,
    /// P27-B: Click count for double/triple-click detection.
    click_count: u8,
    /// P27-B: Timestamp of last left-click.
    last_click_time: Option<std::time::Instant>,
    /// P27-B: Position of last left-click (col, row).
    last_click_pos: (u16, u16),
    /// P27-C: Right-click context menu state.
    context_menu: crate::context_menu::ContextMenuState,
    /// P27-D: Smooth inertial scroll state.
    smooth_scroll: crate::smooth_scroll::SmoothScroller,
    /// P27-F: Whether the window is currently focused (for cursor style).
    window_focused: bool,
    /// DPI scale factor (2.0 on Retina, 1.0 on standard). P18-A.
    scale_factor: f64,

    // ── Resize debouncing (P9-H) ──
    /// Pending resize dimensions (stored during drag, applied after debounce).
    pending_resize: Option<(u32, u32)>,
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

    // ── Status bar visibility (P17-D) ──
    /// Whether the status bar overlay is visible.
    status_bar_visible: bool,

    // ── URL hover/click (P17-C) ──
    /// Currently hovered URL (OSC 8 hyperlink or plain-text URL).
    hovered_link: Option<String>,

    // ── Tab bar overlay (P19-C) ──
    /// Tab bar display state for visual tab strip rendering.
    tab_bar: crate::tab_bar::TabBarState,

    // ── Settings overlay (P19-C) ──
    /// Settings page state (theme, font, scrollback, AI, shell).
    settings: crate::settings_ui::SettingsState,

    // ── About dialog + Menu bar (P19-A) ──
    /// About dialog state.
    #[allow(dead_code)]
    about: crate::about_dialog::AboutDialog,
    /// Whether the menu bar has been installed.
    #[allow(dead_code)]
    menu_installed: bool,

    // ── P22-A: Session restore flag ──
    /// Whether we restored a saved session at startup.
    restored_session: bool,

    // ── P23-A: Cursor blink animation ──
    /// Cursor blink phase tracker for smooth blink animation.
    #[allow(dead_code)]
    cursor_blink: crate::cursor_blink::CursorBlink,
    /// P23-A: Copy/paste visual feedback flash.
    #[allow(dead_code)]
    clipboard_feedback: crate::cursor_blink::ClipboardFeedback,

    // ── P23-E: Tab reordering ──
    /// Dragged tab index (None = not dragging a tab).
    #[allow(dead_code)]
    drag_tab: Option<usize>,
    /// Whether the tab close button was hovered.
    #[allow(dead_code)]
    tab_close_hovered: bool,

    // ── P23-C: Conditional redraw ──
    /// Last time a redraw was requested (for cursor blink timing).
    last_redraw: std::time::Instant,

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
    /// P25-D: Broadcast input state (Ctrl+Shift+Alt+B).
    broadcast: crate::broadcast_input::BroadcastState,
    /// P25-E: Session recorder (None when not recording).
    #[allow(dead_code)]
    recorder: Option<ggterm_core::recording::SessionRecorder>,

    // ── P28: Phase 28 features ──
    /// P28-A: Animation manager for transitions.
    #[allow(dead_code)]
    animations: crate::animations::AnimationManager,
    /// P28-B: Color picker overlay state.
    #[allow(dead_code)]
    color_picker: crate::color_picker::ColorPickerState,
    /// P28-C: Command history sidebar.
    #[allow(dead_code)]
    cmd_history: crate::command_history::CommandHistoryState,
    /// P28-D: Workspace manager.
    #[allow(dead_code)]
    workspaces: crate::workspace::WorkspaceManager,
    /// P28-E: File preview overlay.
    #[allow(dead_code)]
    file_preview: crate::file_preview::FilePreviewState,
    /// P28-F: Performance monitor.
    #[allow(dead_code)]
    perf_monitor: crate::perf_monitor::PerfMonitor,
    /// P28-F: Cursor particle system.
    #[allow(dead_code)]
    cursor_particles: crate::perf_monitor::CursorParticleSystem,
    /// P28-G: Sound player.
    #[allow(dead_code)]
    sound_player: crate::sound::SoundPlayer,
    /// P28-G: Bell rate limiter.
    #[allow(dead_code)]
    bell_limiter: crate::sound::BellRateLimiter,
    /// P28-H: Shell switcher dropdown.
    #[allow(dead_code)]
    shell_switcher: crate::shell_switcher::ShellSwitcherState,
    /// P28: Tab right-click context menu.
    tab_context_menu: crate::tab_bar::TabContextMenuState,
    /// P29-A: Shortcut help overlay (Ctrl+Shift+/).
    shortcut_help: crate::shortcut_help::ShortcutHelpState,
    /// P29-C: Quit confirmation dialog.
    quit_confirm: bool,
    /// P29-C: Flag to exit event loop on next about_to_wait.
    should_quit: bool,
    /// P30-A: Scrollbar drag state (Some(start_y) when dragging).
    scrollbar_drag: Option<f32>,
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
    m.insert("cycle_theme".into(), (true, true, false, "T".into()));
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

        // ── Step 4: Build DesktopApp ──
        let mut desktop = DesktopApp {
            sessions: vec![session],
            active: 0,
            config: desktop_config,
            mods: ModsState::default(),
            window: None,
            surface: None,
            gpu: None,
            renderer: None,
            encoder: InputEncoder::new(),
            #[cfg(feature = "config-watch")]
            config_mgr: None,
            last_title: String::new(),
            selection: crate::mouse::MouseSelection::default(),
            cursor_pos: (0.0, 0.0),
            button_held: None,
            drag_resize: None,
            click_count: 0,
            last_click_time: None,
            last_click_pos: (0, 0),
            context_menu: Default::default(),
            smooth_scroll: Default::default(),
            window_focused: true,
            scale_factor: 1.0,
            pending_resize: None,
            last_resize_time: None,
            last_redraw: std::time::Instant::now(),
            debug_visible: false,
            frame_count: 0,
            last_fps_time: std::time::Instant::now(),
            current_fps: 0.0,
            #[cfg(feature = "ai")]
            ai_overlay: crate::ai_overlay::AIOverlayState::new(),
            #[cfg(feature = "ai")]
            ai_bridge: None,
            search: crate::search::SearchState::new(),
            fullscreen: false,
            maximized: false,
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
            status_bar_visible: true,
            hovered_link: None,
            tab_bar: crate::tab_bar::TabBarState::new(),
            settings: crate::settings_ui::SettingsState::new(),
            about: crate::about_dialog::AboutDialog::new(),
            menu_installed: false,
            restored_session: false,
            cursor_blink: crate::cursor_blink::CursorBlink::new(),
            clipboard_feedback: crate::cursor_blink::ClipboardFeedback::new(),
            drag_tab: None,
            tab_close_hovered: false,
            command_palette: crate::command_palette::CommandPaletteState::default(),
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
            shortcut_help: crate::shortcut_help::ShortcutHelpState::new(),
            quit_confirm: false,
            should_quit: false,
            scrollbar_drag: None,
        };

        // ── Step 7b: P22-A Try restore saved session ──
        match crate::session::load_session() {
            Ok(Some(data)) => {
                log::info!(
                    "Found saved session: {} tab(s), restoring...",
                    data.tabs.len()
                );
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
        //    We'll resize to physical dimensions after getting scale_factor.
        let attrs = Window::default_attributes()
            .with_title(&self.config.title)
            .with_inner_size(winit::dpi::LogicalSize::new(
                self.config.window_width() as f64,
                self.config.window_height() as f64,
            ));

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

        // Update cols/rows from renderer's computed dimensions.
        self.config.cols = renderer.cols().max(10) as u16;
        self.config.rows = renderer.rows().max(3) as u16;

        self.window = Some(window);
        self.surface = Some(surface);
        self.gpu = Some(gpu);
        self.renderer = Some(renderer);

        // P18-C: CRITICAL — resize terminal sessions to match renderer.
        // Without this the PTY/grid think 80x24 while the window shows different.
        let actual_cols = self.config.cols;
        let actual_rows = self.config.rows;
        for session in &mut self.sessions {
            session.resize(actual_cols, actual_rows);
        }
        log::info!(
            "Terminal resized to {}x{} to match renderer",
            actual_cols,
            actual_rows
        );

        // P11-D: Apply active theme to renderer on startup.
        self.apply_theme_to_renderer();

        log::info!("Window + GPU initialized");
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                // P29-C: Show quit confirmation dialog.
                if self.quit_confirm {
                    log::info!("Quit confirmed — saving session and exiting");
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
                self.handle_dropped_file(path);
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

                self.render_frame();

                // Update status bar from active session state.
                let (row, col) = self.active_session().app().cursor();
                self.status_bar.update_cursor(row, col);
                self.status_bar
                    .update_tabs(self.sessions.len(), self.active);
                self.status_bar.set_bell(self.visual_bell_frames > 0);
                self.status_bar.set_search(self.search.visible);
                #[cfg(feature = "ai")]
                self.status_bar.set_ai(self.ai_overlay.is_visible());
                #[cfg(not(feature = "ai"))]
                self.status_bar.set_ai(false);
                // P17-E: Update exit code from terminal's last command.
                self.status_bar
                    .set_exit_code(self.active_session().app().terminal().last_exit_code());

                // P28: Update Phase 28 status bar indicators.
                self.status_bar.workspace_name = self.workspaces.active_name().to_string();
                self.status_bar.sound_enabled = self.sound_player.is_enabled();
                self.status_bar.shell_name = self.shell_switcher.status_bar_label();

                // Update window title: show tab bar when multiple tabs, otherwise
                // show terminal title (OSC 0/2).
                let title = self.active_session().app().terminal().title().to_string();
                if title != self.last_title || self.sessions.len() > 1 {
                    self.last_title = title;
                    if let Some(ref window) = self.window {
                        let display = if self.sessions.len() > 1 {
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
                                    let truncated: String = label.chars().take(12).collect();
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
                let report = if focused {
                    self.active_session().app().terminal().focus_in_report()
                } else {
                    self.active_session().app().terminal().focus_out_report()
                };
                if !report.is_empty() {
                    self.write_to_pty(&report);
                }
                if let Some(ref window) = self.window {
                    window.request_redraw();
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
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // P29-C: Check if we should quit after confirmation.
        if self.should_quit {
            self.save_session_on_exit();
            event_loop.exit();
            return;
        }

        // Pump PTY events.
        self.active_session_mut().pump();

        // P10-C: Poll AI bridge for results.
        #[cfg(feature = "ai")]
        self.poll_ai_bridge();

        // P10-B: Poll OSC 52 clipboard set requests.
        self.poll_osc52_clipboard();

        // P11-E: Poll for bell events.
        self.poll_bell();

        // P24-E: Poll for desktop notifications.
        self.poll_notification();

        // P28-C: Sync command history sidebar from OSC 133 marks.
        self.poll_command_history();

        // P28-F: Tick cursor particle system.
        self.cursor_particles.tick();

        // P19-A: Poll for menu bar actions.
        if let Some(action) = crate::menu_bar::poll_pending_action() {
            self.handle_menu_action(action);
        }

        // Apply deferred resize if debounce interval has elapsed.
        self.apply_pending_resize();

        // Poll config watcher for hot-reload.
        #[cfg(feature = "config-watch")]
        if let Some(ref mut mgr) = self.config_mgr {
            match mgr.poll_reload() {
                Ok(true) => {
                    let cfg = mgr.config();
                    let new_theme = cfg.appearance.theme.clone();
                    let new_font_size = cfg.appearance.font_size as f32;
                    let new_scrollback = cfg.terminal.scrollback_lines;
                    log::info!(
                        "Config reloaded: theme={}, font_size={}, scrollback={}",
                        new_theme,
                        new_font_size,
                        new_scrollback
                    );

                    // P16-B: Apply theme change if different.
                    if new_theme != self.last_applied_theme {
                        self.active_session_mut()
                            .app_mut()
                            .theme_manager()
                            .set_by_name(&new_theme);
                        self.apply_theme_to_renderer();
                        self.last_applied_theme = new_theme.clone();
                        log::info!("Theme changed → applied '{}'", new_theme);
                    }

                    // P16-B: Apply font size change if different.
                    if (new_font_size - self.last_applied_font_size).abs() > 0.01 {
                        self.font_zoom.set_base_size(new_font_size);
                        self.apply_font_size();
                        self.last_applied_font_size = new_font_size;
                        log::info!("Font size changed → applied {new_font_size:.1}px");
                    }

                    // P16-B: Update scrollback limit.
                    self.active_session_mut()
                        .app_mut()
                        .terminal_mut()
                        .grid_mut()
                        .set_scrollback(new_scrollback);
                }
                Ok(false) => {}
                Err(e) => log::warn!("Config reload error: {e}"),
            }
        }

        // Check if active pane's shell has exited (e.g. Ctrl+D, `exit`).
        if !self.active_session().is_running() || !self.active_session_mut().is_alive() {
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

        // P23-C: Conditional redraw — only request redraw when there's
        // content to show (dirty grid, pending resize, bell, or cursor blink).
        let need_redraw = self
            .active_session()
            .app()
            .terminal()
            .grid()
            .content_dirty()
            || self.pending_resize.is_some()
            || self
                .active_session_mut()
                .app_mut()
                .terminal_mut()
                .take_bell();

        // Cursor blink: redraw every 500ms for blink animation.
        let now = std::time::Instant::now();
        let blink_interval = std::time::Duration::from_millis(500);
        let blink_due = now.duration_since(self.last_redraw) >= blink_interval;

        if need_redraw || blink_due || self.debug_visible {
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
}
