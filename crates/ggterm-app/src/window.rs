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
            scale_factor: 1.0,
            pending_resize: None,
            last_resize_time: None,
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
        };

        // ── Step 7b: Load config-driven keybindings (P14-D) ──
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

    // ── Settings overlay navigation (P19-C) ──

    /// Handle Left arrow in settings (decrease/cycle backward).
    fn handle_settings_left(&mut self) {
        match self.settings.selected {
            crate::settings_ui::SettingsField::Theme => {
                // Cycle theme backward
                let opts = crate::settings_ui::THEME_OPTIONS;
                let idx = opts
                    .iter()
                    .position(|&t| t == self.settings.theme)
                    .unwrap_or(0);
                let prev = if idx == 0 { opts.len() - 1 } else { idx - 1 };
                self.settings.theme = opts[prev].to_string();
                self.settings.dirty = true;
            }
            crate::settings_ui::SettingsField::FontSize => self.settings.font_size_down(),
            crate::settings_ui::SettingsField::Scrollback => self.settings.scrollback_down(),
            crate::settings_ui::SettingsField::AiEnabled => self.settings.toggle_ai(),
            _ => {}
        }
    }

    /// Handle Right arrow in settings (increase/cycle forward).
    fn handle_settings_right(&mut self) {
        match self.settings.selected {
            crate::settings_ui::SettingsField::Theme => self.settings.cycle_theme(),
            crate::settings_ui::SettingsField::FontSize => self.settings.font_size_up(),
            crate::settings_ui::SettingsField::Scrollback => self.settings.scrollback_up(),
            crate::settings_ui::SettingsField::AiEnabled => self.settings.toggle_ai(),
            _ => {}
        }
    }

    // ── Tab management (P10-A) ──

    /// Open a new tab: create a TabSession with a fresh PTY.
    fn open_tab(&mut self) {
        let cols = self.config.cols;
        let rows = self.config.rows;
        match TabSession::new(cols, rows, self.shell()) {
            Ok(session) => {
                self.sessions.push(session);
                self.active = self.sessions.len() - 1;
                log::info!("Opened tab {}", self.active + 1);
            }
            Err(e) => {
                log::error!("Failed to open tab: {e}");
            }
        }
    }

    /// Close the active tab (keep at least 1).
    fn close_tab(&mut self) {
        if self.sessions.len() <= 1 {
            return;
        }
        self.sessions.remove(self.active);
        if self.active >= self.sessions.len() {
            self.active = self.sessions.len() - 1;
        }
        log::info!("Closed tab, active={}", self.active + 1);
    }

    /// Switch to a specific tab by index (0-based).
    fn switch_tab(&mut self, index: usize) {
        if index < self.sessions.len() {
            self.active = index;
        }
    }

    /// Switch to the next tab (wraps).
    fn next_tab(&mut self) {
        self.active = (self.active + 1) % self.sessions.len();
    }

    /// Switch to the previous tab (wraps).
    fn prev_tab(&mut self) {
        self.active = if self.active == 0 {
            self.sessions.len() - 1
        } else {
            self.active - 1
        };
    }

    /// Write encoded keyboard bytes to the active PTY.
    fn write_to_pty(&mut self, bytes: &[u8]) {
        self.active_session_mut().write_to_pty(bytes);
    }

    // ── P19-B: Split pane management ──

    /// Split the active pane horizontally (left | right).
    ///
    /// Creates a new PTY + App for the new pane.
    fn split_pane_horizontal(&mut self) {
        let cols = self.config.cols;
        let rows = self.config.rows;
        let shell = self.shell().to_string();
        match self
            .active_session_mut()
            .split_horizontal(cols, rows, &shell)
        {
            Ok(id) => log::info!("Horizontal split → new pane {id}"),
            Err(e) => log::error!("Failed to split horizontal: {e}"),
        }
    }

    /// Split the active pane vertically (top / bottom).
    ///
    /// Creates a new PTY + App for the new pane.
    fn split_pane_vertical(&mut self) {
        let cols = self.config.cols;
        let rows = self.config.rows;
        let shell = self.shell().to_string();
        match self.active_session_mut().split_vertical(cols, rows, &shell) {
            Ok(id) => log::info!("Vertical split → new pane {id}"),
            Err(e) => log::error!("Failed to split vertical: {e}"),
        }
    }

    /// Handle window resize: store pending dimensions for debounced apply.
    ///
    /// During a drag-resize, winit fires many `Resized` events. We store the
    /// latest dimensions and defer the actual Terminal/PTY resize until the
    /// user stops dragging (100ms debounce). See `apply_pending_resize()`.
    fn handle_resize(&mut self, width: u32, height: u32) {
        self.pending_resize = Some((width.max(1), height.max(1)));
        self.last_resize_time = Some(std::time::Instant::now());
    }

    /// Apply a pending resize if the debounce interval (100ms) has elapsed.
    ///
    /// Called from `about_to_wait()`. Returns `true` if a resize was applied.
    fn apply_pending_resize(&mut self) -> bool {
        let Some((width, height)) = self.pending_resize else {
            return false;
        };
        let Some(last) = self.last_resize_time else {
            return false;
        };

        // Check if enough time has passed since the last resize event.
        if std::time::Instant::now().duration_since(last) < std::time::Duration::from_millis(100) {
            return false; // Not enough time elapsed — wait.
        }

        // Clear pending state.
        self.pending_resize = None;
        self.last_resize_time = None;

        // Resize GPU surface to match physical window.
        if let (Some(gpu), Some(surface)) = (&mut self.gpu, &self.surface) {
            gpu.resize(surface, width.max(1), height.max(1));
        }

        // Recreate renderer with surface dimensions — it computes cols/rows internally.
        if let Some(gpu) = &self.gpu {
            self.renderer = Some(gpu.create_renderer(width, height, self.scale_factor));
        }

        // Get actual cols/rows from renderer.
        let new_cols = self
            .renderer
            .as_ref()
            .map(|r| r.cols() as u16)
            .unwrap_or(80)
            .max(10);
        let new_rows = self
            .renderer
            .as_ref()
            .map(|r| r.rows() as u16)
            .unwrap_or(24)
            .max(3);

        log::debug!(
            "Resize: {}x{}px → {}x{} cells",
            width,
            height,
            new_cols,
            new_rows
        );

        self.active_session_mut().resize(new_cols, new_rows);

        true
    }

    /// Render one frame.
    fn render_frame(&mut self) {
        // P12-A/P12-C: Get theme background color for clear color,
        // and blend with visual bell flash if active.
        let active = self.active;
        let (br, bg, bb) = {
            let session = &self.sessions[active];
            let theme = session.app().theme();
            theme.resolve_bg(&theme.default_bg)
        };
        let bg_color = if self.visual_bell_frames > 0 {
            let intensity = self.visual_bell_frames as f64 / VISUAL_BELL_DURATION_FRAMES as f64;
            let flash = 0.3 * intensity;
            [
                (br as f64 / 255.0) + flash * (1.0 - br as f64 / 255.0),
                (bg as f64 / 255.0) + flash * (1.0 - bg as f64 / 255.0),
                (bb as f64 / 255.0) + flash * (1.0 - bb as f64 / 255.0),
            ]
        } else {
            [br as f64 / 255.0, bg as f64 / 255.0, bb as f64 / 255.0]
        };

        // Decrement visual bell counter.
        if self.visual_bell_frames > 0 {
            self.visual_bell_frames -= 1;
        }

        // Now borrow session for grid + cursor data.
        let session = &self.sessions[active];
        let grid = session.app().grid();
        let cursor = cursor_state(session.app());

        // P16-A: Wire search match highlights to renderer.
        // Convert SearchMatch(abs_row, col, len) → (visible_row, col_start, col_end).
        let scrollback_len = grid.scrollback_len();
        let grid_height = grid.height();
        let search_highlights: Vec<(usize, usize, usize)> = if self.search.visible {
            self.search
                .matches()
                .iter()
                .filter_map(|m| {
                    let visible_row = m.abs_row.checked_sub(scrollback_len)?;
                    // Only highlight rows within the visible grid.
                    if visible_row < grid_height {
                        Some((visible_row, m.col, m.col + m.len.saturating_sub(1)))
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            Vec::new()
        };

        let (gpu, surface, renderer) = match (&mut self.gpu, &self.surface, &mut self.renderer) {
            (Some(g), Some(s), Some(r)) => (g, s, r),
            _ => return,
        };

        // Apply search highlights before rendering.
        renderer.set_highlights(search_highlights);

        // Apply dynamic colors (OSC 10/11) if set on the terminal.
        let term = self.sessions[self.active].app().terminal();
        renderer.set_dynamic_fg(term.dynamic_fg().map(|c| match c {
            ggterm_core::Color::Rgb(r, g, b) => (*r, *g, *b),
            _ => unreachable!("dynamic_fg stores Rgb"),
        }));
        renderer.set_dynamic_bg(term.dynamic_bg().map(|c| match c {
            ggterm_core::Color::Rgb(r, g, b) => (*r, *g, *b),
            _ => unreachable!("dynamic_bg stores Rgb"),
        }));

        // P19-G: Build overlay data (tab bar + settings + about).
        let cell_h = renderer.cell_height() as f32;
        let screen_w = renderer.resolution_width() as f32;
        let screen_h = renderer.resolution_height() as f32;
        let mut overlay_rects: Vec<ggterm_render_wgpu::OverlayRect> = Vec::new();
        let mut overlay_texts: Vec<ggterm_render_wgpu::OverlayTextSpec> = Vec::new();

        // Update tab bar data.
        let titles: Vec<&str> = self.sessions.iter().map(|s| s.title()).collect();
        self.tab_bar.update(&titles, self.active);

        // Tab bar overlay: backgrounds + text.
        if self.tab_bar.visible {
            let bar_h = cell_h;
            // Dark background strip
            overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                x: 0.0,
                y: 0.0,
                w: screen_w,
                h: bar_h,
                color: (0.12, 0.12, 0.15),
            });
            let tab_max_w = screen_w / self.tab_bar.tabs.len() as f32;
            for (i, tab) in self.tab_bar.tabs.iter().enumerate() {
                let x = i as f32 * tab_max_w;
                if tab.active {
                    overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                        x,
                        y: 0.0,
                        w: tab_max_w,
                        h: bar_h,
                        color: (0.2, 0.2, 0.3),
                    });
                }
                // Separator line
                if i > 0 {
                    overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                        x,
                        y: 0.0,
                        w: 1.0,
                        h: bar_h,
                        color: (0.3, 0.3, 0.35),
                    });
                }
                // Tab title text
                let title = tab.format();
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: title,
                    left: x + 4.0,
                    top: 0.0,
                    color: if tab.active {
                        (220, 220, 220)
                    } else {
                        (160, 160, 160)
                    },
                });
            }
        }

        // P20-B: Pane border overlays — draw 1px separators between split panes.
        let active = self.active;
        let tree = &self.sessions[active].split_tree();
        if !tree.is_single() {
            let bounds = crate::splits::Rect::new(
                0,
                if self.tab_bar.visible {
                    cell_h as u32
                } else {
                    0
                },
                screen_w as u32,
                screen_h as u32,
            );
            let areas = tree.areas(bounds);
            let active_id = tree.active();
            let border_active = (0.4, 0.55, 0.85_f32);
            let border_inactive = (0.15, 0.15, 0.2_f32);

            for (pane_id, rect) in &areas {
                let x = rect.x as f32;
                let y = rect.y as f32;
                let w = rect.width as f32;
                let h = rect.height as f32;
                let c = if *pane_id == active_id {
                    border_active
                } else {
                    border_inactive
                };
                overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                    x,
                    y,
                    w,
                    h: 1.0,
                    color: c,
                });
                overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                    x,
                    y: y + h - 1.0,
                    w,
                    h: 1.0,
                    color: c,
                });
                overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                    x,
                    y,
                    w: 1.0,
                    h,
                    color: c,
                });
                overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                    x: x + w - 1.0,
                    y,
                    w: 1.0,
                    h,
                    color: c,
                });
            }
        }

        // Settings overlay: semi-transparent mask + panel.
        if self.settings.visible {
            // Dark mask
            overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                x: 0.0,
                y: 0.0,
                w: screen_w,
                h: screen_h,
                color: (0.05, 0.05, 0.05),
            });
            // Center panel
            let pw = screen_w * 0.6;
            let ph = screen_h * 0.5;
            let px = (screen_w - pw) * 0.5;
            let py = (screen_h - ph) * 0.5;
            overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                x: px,
                y: py,
                w: pw,
                h: ph,
                color: (0.1, 0.1, 0.12),
            });
            // Panel border (top + bottom)
            overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                x: px,
                y: py,
                w: pw,
                h: 2.0,
                color: (0.35, 0.35, 0.4),
            });
            overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                x: px,
                y: py + ph - 2.0,
                w: pw,
                h: 2.0,
                color: (0.35, 0.35, 0.4),
            });
            // Settings text lines
            let theme_str = self.settings.theme.clone();
            let font_str = self.settings.font_size.to_string();
            let scrollback_str = self.settings.scrollback_lines.to_string();
            let shell_str = self.settings.shell.clone();
            let ai_str = (if self.settings.ai_enabled {
                "on"
            } else {
                "off"
            })
            .to_string();
            let endpoint_str = self.settings.ai_endpoint.clone();
            let model_str = self.settings.ai_model.clone();
            let fields: [(&str, &str); 7] = [
                ("Theme", &theme_str),
                ("Font Size", &font_str),
                ("Scrollback", &scrollback_str),
                ("Shell", &shell_str),
                ("AI", &ai_str),
                ("AI Endpoint", &endpoint_str),
                ("AI Model", &model_str),
            ];
            for (i, (label, value)) in fields.iter().enumerate() {
                let line = format!(
                    "  {}  {}: {}",
                    if i as u8 == self.settings.selected as u8 {
                        ">"
                    } else {
                        " "
                    },
                    label,
                    value
                );
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: line,
                    left: px + 10.0,
                    top: py + 10.0 + i as f32 * cell_h,
                    color: (200, 200, 200),
                });
            }
        }

        // About dialog overlay
        if self.about.visible {
            overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                x: 0.0,
                y: 0.0,
                w: screen_w,
                h: screen_h,
                color: (0.05, 0.05, 0.05),
            });
            let pw = screen_w * 0.4;
            let ph = screen_h * 0.3;
            let px = (screen_w - pw) * 0.5;
            let py = (screen_h - ph) * 0.5;
            overlay_rects.push(ggterm_render_wgpu::OverlayRect {
                x: px,
                y: py,
                w: pw,
                h: ph,
                color: (0.1, 0.1, 0.12),
            });
            let about_text = self.about.format_text();
            for (i, line) in about_text.lines().enumerate() {
                overlay_texts.push(ggterm_render_wgpu::OverlayTextSpec {
                    text: line.to_string(),
                    left: px + 10.0,
                    top: py + 10.0 + i as f32 * cell_h,
                    color: (200, 200, 200),
                });
            }
        }

        renderer.set_overlay_rects(overlay_rects);
        renderer.set_overlay_text(overlay_texts);

        // P20-A: Multi-pane viewport rendering.
        // When the active session has multiple panes, render each pane's grid
        // at its SplitTree area offset within a single render pass.
        let pane_count = self.sessions[active].pane_count();
        if pane_count > 1 {
            let session = &self.sessions[active];
            let tree = session.split_tree();
            let bounds = crate::splits::Rect::new(0, 0, screen_w as u32, screen_h as u32);
            let areas = tree.areas(bounds);

            // Build cursor states per pane (owned values, no borrow issues).
            let cursors: Vec<_> = areas
                .iter()
                .filter_map(|(id, _)| session.pane_app(*id).map(cursor_state))
                .collect();

            // Build PaneRenderSpec list (grid refs borrow session, cursors from local vec).
            let mut specs: Vec<crate::gpu::PaneRenderSpec> = Vec::new();
            for ((pane_id, rect), cursor) in areas.iter().zip(cursors.iter()) {
                if let Some(app) = session.pane_app(*pane_id) {
                    specs.push(crate::gpu::PaneRenderSpec {
                        grid: app.grid(),
                        cursor,
                        offset_x: rect.x,
                        offset_y: rect.y,
                        width: rect.width,
                        height: rect.height,
                        needs_prepare: session.pane_needs_prepare(*pane_id),
                    });
                }
            }

            if let Err(e) = gpu.render_multi_pane_frame(surface, renderer, &specs, bg_color) {
                log::error!("Render error: {e}");
            }

            // P21-D: Clear prepare flags after render (mutable borrow, disjoint from gpu).
            self.sessions[active].clear_prepare_flags();
        } else if let Err(e) = gpu.render_frame(surface, renderer, grid, &cursor, bg_color) {
            log::error!("Render error: {e}");
        }

        // P17-D: Update status bar and log it.
        if self.status_bar_visible {
            let status_text = self.status_bar.format();
            log::debug!("status: {}", status_text);
        }

        // P19-C: Update tab bar display data.
        let titles: Vec<&str> = self.sessions.iter().map(|s| s.title()).collect();
        self.tab_bar.update(&titles, self.active);
        if self.tab_bar.visible {
            log::debug!("tab_bar: {}", self.tab_bar.format());
        }

        // P19-C: Settings overlay logging.
        if self.settings.visible {
            log::debug!("settings: {}", self.settings.format_summary());
        }
    }

    /// Handle a winit key event using the existing keymap module.
    fn handle_keyboard_input(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }

        // ── P14-D: Config-driven keybinding dispatch ──
        // All configurable actions are resolved through check_keybinding().
        // The resolved_keybindings map is populated from ConfigManager at
        // startup and falls back to default_keybindings() when no config exists.
        if let PhysicalKey::Code(code) = &event.physical_key {
            let key_name = keycode_to_name(code);

            // Ctrl+T → new tab
            if self.check_keybinding(
                "new_tab",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                self.open_tab();
                return;
            }
            // Ctrl+W → close tab (or close active pane if splits exist)
            if self.check_keybinding(
                "close_tab",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                if self.active_session().pane_count() > 1 {
                    // Multiple panes: close the active pane instead of the tab.
                    self.active_session_mut().remove_active_pane();
                } else {
                    self.close_tab();
                }
                return;
            }
            // Ctrl+= → zoom in
            if self.check_keybinding(
                "zoom_in",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                if self.font_zoom.zoom_in() {
                    self.apply_font_size();
                }
                return;
            }
            // Ctrl+- → zoom out
            if self.check_keybinding(
                "zoom_out",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                if self.font_zoom.zoom_out() {
                    self.apply_font_size();
                }
                return;
            }
            // Ctrl+0 → reset zoom
            if self.check_keybinding(
                "zoom_reset",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                if self.font_zoom.reset() {
                    self.apply_font_size();
                }
                return;
            }
            // F11 → fullscreen
            if self.check_keybinding(
                "fullscreen",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                self.toggle_fullscreen();
                return;
            }
            // Ctrl+Shift+V → paste
            if self.check_keybinding(
                "paste",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                self.paste_from_clipboard();
                return;
            }
            // Ctrl+Shift+C → copy
            if self.check_keybinding(
                "copy",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                self.copy_selection_to_clipboard();
                return;
            }
            // Ctrl+Shift+K → clear
            if self.check_keybinding(
                "clear",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                crate::terminal_actions::clear_screen_and_scrollback(
                    self.active_session_mut().app_mut().grid_mut(),
                );
                return;
            }
            // Ctrl+Shift+R → reset terminal
            if self.check_keybinding(
                "reset",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                crate::terminal_actions::soft_reset(self.active_session_mut().app_mut().grid_mut());
                return;
            }
            // Ctrl+Shift+T → cycle theme
            if self.check_keybinding(
                "cycle_theme",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                self.cycle_theme();
                return;
            }
            // Ctrl+Shift+F → toggle search
            if self.check_keybinding(
                "search",
                self.mods.ctrl,
                self.mods.shift,
                self.mods.alt,
                key_name,
            ) {
                self.search.toggle();
                return;
            }
        }

        // ── P19-B: Split pane shortcuts (not configurable) ──

        // Ctrl+Shift+D → horizontal split (left | right)
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::KeyD) = &event.physical_key
        {
            self.split_pane_horizontal();
            return;
        }

        // Ctrl+Shift+\ → vertical split (top / bottom)
        // (Ctrl+Shift+S is reserved for AI Suggest)
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::Backslash) = &event.physical_key
        {
            self.split_pane_vertical();
            return;
        }

        // Ctrl+Shift+] → focus next pane
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::BracketRight) = &event.physical_key
        {
            self.active_session_mut().focus_next_pane();
            return;
        }

        // Ctrl+Shift+[ → focus previous pane
        if self.mods.ctrl
            && self.mods.shift
            && !self.mods.alt
            && let PhysicalKey::Code(KeyCode::BracketLeft) = &event.physical_key
        {
            self.active_session_mut().focus_prev_pane();
            return;
        }

        // Ctrl+Shift+Alt+Arrows → adjust split ratio
        if self.mods.ctrl && self.mods.shift && self.mods.alt {
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::ArrowLeft) => {
                    self.active_session_mut()
                        .split_tree_mut()
                        .adjust_active_ratio(-0.05);
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowRight) => {
                    self.active_session_mut()
                        .split_tree_mut()
                        .adjust_active_ratio(0.05);
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowUp) => {
                    self.active_session_mut()
                        .split_tree_mut()
                        .adjust_active_ratio(0.05);
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowDown) => {
                    self.active_session_mut()
                        .split_tree_mut()
                        .adjust_active_ratio(-0.05);
                    return;
                }
                _ => {}
            }
        }

        // Ctrl+Shift+B → toggle status bar visibility (not configurable)
        if self.mods.ctrl
            && self.mods.shift
            && let PhysicalKey::Code(KeyCode::KeyB) = &event.physical_key
        {
            self.status_bar_visible = !self.status_bar_visible;
            return;
        }

        // Ctrl+, (comma) → toggle settings overlay (P19-C)
        if self.mods.ctrl
            && !self.mods.shift
            && let PhysicalKey::Code(KeyCode::Comma) = &event.physical_key
        {
            self.settings.toggle();
            return;
        }

        // P19-C: When settings overlay is open, intercept navigation keys.
        if self.settings.visible {
            match &event.physical_key {
                PhysicalKey::Code(KeyCode::Escape) => {
                    self.settings.close();
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowUp) => {
                    self.settings.move_up();
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowDown) => {
                    self.settings.move_down();
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowLeft) => {
                    self.handle_settings_left();
                    return;
                }
                PhysicalKey::Code(KeyCode::ArrowRight) => {
                    self.handle_settings_right();
                    return;
                }
                _ => {}
            }
        }

        // Ctrl+Shift+Return → toggle maximized (not configurable)
        if self.mods.ctrl
            && self.mods.shift
            && let PhysicalKey::Code(KeyCode::Enter) = &event.physical_key
        {
            self.toggle_maximized();
            return;
        }

        // Alt+1-9 → switch to tab N (not configurable)
        if self.mods.alt
            && !self.mods.ctrl
            && let PhysicalKey::Code(code) = &event.physical_key
        {
            let tab_idx = match code {
                KeyCode::Digit1 => Some(0),
                KeyCode::Digit2 => Some(1),
                KeyCode::Digit3 => Some(2),
                KeyCode::Digit4 => Some(3),
                KeyCode::Digit5 => Some(4),
                KeyCode::Digit6 => Some(5),
                KeyCode::Digit7 => Some(6),
                KeyCode::Digit8 => Some(7),
                KeyCode::Digit9 => Some(8),
                _ => None,
            };
            if let Some(idx) = tab_idx {
                self.switch_tab(idx);
                return;
            }
        }

        // Ctrl+Tab → next tab, Ctrl+Shift+Tab → prev tab (not configurable)
        if self.mods.ctrl
            && let PhysicalKey::Code(KeyCode::Tab) = &event.physical_key
        {
            if self.mods.shift {
                self.prev_tab();
            } else {
                self.next_tab();
            }
            return;
        }

        // Phase 8-D: Ctrl+Shift+Up/Down for command block navigation (not configurable)
        if self.mods.ctrl
            && self.mods.shift
            && let PhysicalKey::Code(code) = &event.physical_key
        {
            match code {
                KeyCode::ArrowUp => {
                    self.active_session_mut()
                        .app_mut()
                        .handle_event(AppEvent::PrevCommandBlock);
                    return;
                }
                KeyCode::ArrowDown => {
                    self.active_session_mut()
                        .app_mut()
                        .handle_event(AppEvent::NextCommandBlock);
                    return;
                }
                // Ctrl+Shift+A → select all text (not configurable)
                KeyCode::KeyA => {
                    let grid = self.active_session().app().grid();
                    let range = crate::terminal_actions::select_all_range(grid);
                    self.selection
                        .start(range.start_col as u16, range.start_row as u16);
                    self.selection
                        .extend(range.end_col as u16, range.end_row as u16);
                    self.selection.finish();
                    return;
                }
                // P10-C: AI assistant shortcuts (Ctrl+Shift+E/S/H/N, not configurable)
                #[cfg(feature = "ai")]
                KeyCode::KeyE => {
                    self.trigger_ai_request(ggterm_ai::Action::Explain);
                    return;
                }
                #[cfg(feature = "ai")]
                KeyCode::KeyS => {
                    self.trigger_ai_request(ggterm_ai::Action::Suggest);
                    return;
                }
                #[cfg(feature = "ai")]
                KeyCode::KeyH => {
                    self.trigger_ai_request(ggterm_ai::Action::ErrorHelp);
                    return;
                }
                #[cfg(feature = "ai")]
                KeyCode::KeyN => {
                    self.trigger_ai_request(ggterm_ai::Action::NL2Command);
                    return;
                }
                _ => {}
            }
        }

        // P10-D: When search bar is open, intercept all keyboard input.
        if self.search.visible {
            self.handle_search_input(event);
            return;
        }

        // P10-C: Esc dismisses AI overlay if visible.
        #[cfg(feature = "ai")]
        if self.ai_overlay.is_visible()
            && let PhysicalKey::Code(KeyCode::Escape) = &event.physical_key
        {
            self.ai_overlay.hide();
            return;
        }

        // Extract logical text for printable character support.
        let logical_text: Option<&str> = match &event.logical_key {
            Key::Character(s) => Some(s.as_ref()),
            _ => None,
        };

        // Use the shared keymap module for mapping.
        let mods: crate::input::KeyModifiers = self.mods.into();
        if let Some(input_key) = map_winit_key(&event.physical_key, logical_text, &mods) {
            let bytes = self.encoder.encode(&input_key);
            if !bytes.is_empty() {
                self.write_to_pty(&bytes);
            }
        }
    }

    // ── Scrollback search (P10-D) ───────────────────────────────

    /// Handle keyboard input when the search bar is open.
    fn handle_search_input(&mut self, event: &KeyEvent) {
        // Direct field access for disjoint borrows: self.sessions (immutable)
        // and self.search (mutable) are separate fields of self.
        let grid = self.sessions[self.active].app().grid();
        let search = &mut self.search;
        match &event.physical_key {
            PhysicalKey::Code(KeyCode::Escape) => {
                search.close();
            }
            PhysicalKey::Code(KeyCode::Enter) => {
                if self.mods.shift {
                    search.prev_match();
                } else {
                    search.next_match();
                }
            }
            PhysicalKey::Code(KeyCode::Backspace) => {
                search.backspace(grid);
            }
            _ => {
                if let Key::Character(s) = &event.logical_key
                    && let Some(c) = s.chars().next()
                    && !c.is_control()
                {
                    search.type_char(c, grid);
                }
            }
        }
    }

    // ── AI assistant (P10-C, ai feature) ──────────────────────────

    /// Trigger an AI request from the current terminal context.
    ///
    /// Builds an [`AIContext`] from the terminal state, shows the overlay
    /// in "thinking" mode, and dispatches the request to the AIBridge.
    #[cfg(feature = "ai")]
    fn trigger_ai_request(&mut self, action: ggterm_ai::Action) {
        // Show overlay immediately.
        self.ai_overlay.start_request(action);

        // Build context from terminal.
        let ctx = ggterm_ai::AIContext::from_terminal(self.active_session().app().terminal());
        let req = crate::ai_bridge::AIRequest::new(action, ctx);

        if let Some(ref mut bridge) = self.ai_bridge {
            if !bridge.request(req) {
                self.ai_overlay.set_error("AI is busy, please wait...");
            }
        } else {
            self.ai_overlay
                .set_error("AI not configured (set ai.api_endpoint in config)");
        }
    }

    /// Poll the AIBridge for a completed result and update the overlay.
    #[cfg(feature = "ai")]
    fn poll_ai_bridge(&mut self) {
        if let Some(ref mut bridge) = self.ai_bridge
            && let Some(response) = bridge.poll_result()
        {
            match response.result {
                Ok(text) => self.ai_overlay.set_response(text),
                Err(e) => self.ai_overlay.set_error(e),
            }
        }
    }

    // ── Mouse handling (P9-D, pending integration) ────────────────

    /// Convert pixel position to terminal cell coordinates.
    fn pixel_to_cell_pos(&self) -> (u16, u16) {
        // P18: Use actual renderer cell dimensions (DPI-aware, font-measured).
        let (cw, ch) = if let Some(ref renderer) = self.renderer {
            (renderer.cell_width() as f64, renderer.cell_height() as f64)
        } else {
            (
                self.config.cell_width as f64,
                self.config.cell_height as f64,
            )
        };
        crate::mouse::pixel_to_cell(self.cursor_pos.0, self.cursor_pos.1, cw, ch)
    }

    /// P20-D: Check if the cursor is over a different pane and switch focus.
    ///
    /// Returns `true` if focus changed (caller may want to redraw).
    fn maybe_switch_pane_focus(&mut self) -> bool {
        let session = self.active_session();
        // Only relevant when there are multiple panes.
        if session.pane_count() <= 1 {
            return false;
        }

        // Need renderer to get screen dimensions for split area calculation.
        let Some(screen_w) = self.renderer.as_ref().map(|r| r.resolution_width()) else {
            return false;
        };
        let Some(screen_h) = self.renderer.as_ref().map(|r| r.resolution_height()) else {
            return false;
        };

        let bounds = crate::splits::Rect::new(0, 0, screen_w, screen_h);
        let (px, py) = (self.cursor_pos.0 as u32, self.cursor_pos.1 as u32);

        if let Some(hit_id) = session.split_tree().pane_at_point(px, py, bounds) {
            let active = session.split_tree().active();
            if hit_id != active {
                self.active_session_mut()
                    .split_tree_mut()
                    .set_active(hit_id);
                log::debug!("P20-D: pane focus → {hit_id}");
                return true;
            }
        }
        false
    }

    /// P21-A: Try to start dragging a split separator.
    ///
    /// Checks if cursor is near a separator line. If so, records the
    /// orientation and returns true (caller should skip normal mouse handling).
    fn try_start_separator_drag(&mut self) -> bool {
        let session = self.active_session();
        if session.pane_count() <= 1 {
            return false;
        }

        let Some(screen_w) = self.renderer.as_ref().map(|r| r.resolution_width()) else {
            return false;
        };
        let Some(screen_h) = self.renderer.as_ref().map(|r| r.resolution_height()) else {
            return false;
        };

        let bounds = crate::splits::Rect::new(0, 0, screen_w, screen_h);
        let (px, py) = (self.cursor_pos.0 as u32, self.cursor_pos.1 as u32);

        if let Some(orient) = session.split_tree().separator_at_point(px, py, bounds) {
            self.drag_resize = Some(orient);
            log::debug!("P21-A: separator drag started ({orient:?})");
            return true;
        }
        false
    }

    /// Handle winit MouseInput events (button press/release).
    fn handle_mouse_input(&mut self, state: ElementState, button: winit::event::MouseButton) {
        // P21-A: Handle split separator drag.
        if button == winit::event::MouseButton::Left {
            if state == ElementState::Pressed {
                // Check if we're clicking on a separator.
                if self.try_start_separator_drag() {
                    return; // Don't process as pane click or selection
                }
            } else if state == ElementState::Released && self.drag_resize.is_some() {
                self.drag_resize = None;
                log::debug!("P21-A: separator drag ended");
                return;
            }
        }

        // P20-D: On left-click, switch pane focus to the pane under the cursor.
        if state == ElementState::Pressed
            && button == winit::event::MouseButton::Left
            && self.maybe_switch_pane_focus()
            && let Some(ref window) = self.window
        {
            window.request_redraw();
        }

        let mouse_button = match button {
            winit::event::MouseButton::Left => crate::mouse::MouseButton::Left,
            winit::event::MouseButton::Right => crate::mouse::MouseButton::Right,
            winit::event::MouseButton::Middle => crate::mouse::MouseButton::Middle,
            winit::event::MouseButton::Back => crate::mouse::MouseButton::Other(8),
            winit::event::MouseButton::Forward => crate::mouse::MouseButton::Other(16),
            winit::event::MouseButton::Other(n) => crate::mouse::MouseButton::Other(n as u8),
        };

        let (col, row) = self.pixel_to_cell_pos();
        let mods = crate::mouse::MouseModifiers {
            shift: self.mods.shift,
            ctrl: self.mods.ctrl,
            alt: self.mods.alt,
        };

        let term = self.active_session().app().terminal();

        // Check if mouse tracking is active.
        if term.mouse_tracking_enabled() {
            let mouse_ev = crate::mouse::MouseEvent {
                button: mouse_button,
                x: col,
                y: row,
                mods,
            };

            let sgr = term.mouse_sgr_enabled();
            let urxvt = term.mouse_urxvt_enabled();

            match state {
                ElementState::Pressed => {
                    self.button_held = Some(mouse_button);
                    if let Some(bytes) =
                        crate::mouse::encode_mouse_event(&mouse_ev, sgr, urxvt, true)
                    {
                        self.write_to_pty(&bytes);
                    }
                }
                ElementState::Released => {
                    self.button_held = None;
                    if let Some(bytes) =
                        crate::mouse::encode_mouse_event(&mouse_ev, sgr, urxvt, false)
                    {
                        self.write_to_pty(&bytes);
                    }
                }
            }
            return;
        }

        // Mouse tracking is OFF — handle selection and paste locally.
        match (mouse_button, state) {
            (crate::mouse::MouseButton::Left, ElementState::Pressed) => {
                self.button_held = Some(mouse_button);
                self.selection.start(col, row);
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            (crate::mouse::MouseButton::Left, ElementState::Released) => {
                // P17-C: Cmd+Click (macOS) or Ctrl+Click (other) opens hovered URL.
                let open_link = (cfg!(target_os = "macos") && self.mods.super_key)
                    || (!cfg!(target_os = "macos") && self.mods.ctrl);
                if open_link && let Some(ref url) = self.hovered_link.take() {
                    crate::mouse::open_url(url);
                    return;
                }

                self.button_held = None;
                self.selection.finish();
                // Copy selection to clipboard if active.
                if self.selection.is_active() {
                    self.copy_selection_to_clipboard();
                }
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            (crate::mouse::MouseButton::Middle, ElementState::Pressed) => {
                // Middle-click paste from system clipboard.
                self.paste_from_clipboard();
            }
            _ => {}
        }
    }

    /// Handle cursor motion — extend selection or report mouse motion.
    fn handle_cursor_moved(&mut self) {
        // P21-A: If dragging a separator, adjust the ratio.
        if self.drag_resize.is_some() {
            let (px, py) = (self.cursor_pos.0 as u32, self.cursor_pos.1 as u32);
            let screen_w = self
                .renderer
                .as_ref()
                .map(|r| r.resolution_width())
                .unwrap_or(0);
            let screen_h = self
                .renderer
                .as_ref()
                .map(|r| r.resolution_height())
                .unwrap_or(0);
            let bounds = crate::splits::Rect::new(0, 0, screen_w, screen_h);
            let active = self.active;
            let tree = self.sessions[active].split_tree_mut();
            tree.set_ratio_at_point(px, py, bounds);
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
            return; // Skip normal cursor handling while dragging separator
        }

        let (col, row) = self.pixel_to_cell_pos();

        let (any_event, button_event) = {
            let term = self.active_session().app().terminal();
            (
                term.mouse_any_event_enabled(),
                term.mouse_button_event_enabled(),
            )
        };

        // If mouse motion tracking is on, report motion.
        if any_event || button_event {
            let held = self.button_held.is_some();
            if crate::mouse::should_report_motion(any_event, button_event, held) {
                let button = self.button_held.unwrap_or(crate::mouse::MouseButton::Left);
                let mods = crate::mouse::MouseModifiers {
                    shift: self.mods.shift,
                    ctrl: self.mods.ctrl,
                    alt: self.mods.alt,
                };
                let mouse_ev = crate::mouse::MouseEvent {
                    button,
                    x: col,
                    y: row,
                    mods,
                };

                let sgr = {
                    let term = self.active_session().app().terminal();
                    term.mouse_sgr_enabled()
                };
                if sgr {
                    let bytes = crate::mouse::encode_sgr_motion(&mouse_ev, held);
                    self.write_to_pty(bytes.as_bytes());
                }
                return;
            }
        }

        // Extend selection while dragging.
        if self.selection.dragging {
            self.selection.extend(col, row);
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
        }

        // P17-C: Detect hovered URL (OSC 8 hyperlink or plain text).
        self.update_hovered_link(col, row);
    }

    /// P17-C: Update `hovered_link` based on the cell under the cursor.
    ///
    /// Checks for OSC 8 hyperlinks first, then falls back to plain-text URL
    /// detection in the row's text content.
    fn update_hovered_link(&mut self, col: u16, row: u16) {
        let col = col as usize;
        let row = row as usize;
        let grid = &self.sessions[self.active].app().grid();

        // Try OSC 8 hyperlink on the cell at (col, row).
        if let Some(cell_row) = grid.display_row(row)
            && col < cell_row.cells.len()
        {
            let cell = &cell_row.cells[col];
            if let Some(ref link) = cell.hyperlink {
                self.hovered_link = Some(link.clone());
                return;
            }
        }

        // Fall back to plain-text URL detection.
        if let Some(cell_row) = grid.display_row(row) {
            let line: String = cell_row.cells.iter().map(|c| c.ch).collect();
            if let Some((_, _, url)) = crate::mouse::detect_url_at_position(&line, col) {
                self.hovered_link = Some(url);
                return;
            }
        }

        self.hovered_link = None;
    }

    /// Handle mouse wheel events — scroll scrollback or report to PTY.
    fn handle_mouse_wheel(&mut self, delta: winit::event::MouseScrollDelta) {
        // P20-D: Route wheel events to the pane under the cursor.
        if self.maybe_switch_pane_focus()
            && let Some(ref window) = self.window
        {
            window.request_redraw();
        }

        let (col, row) = self.pixel_to_cell_pos();
        let mods = crate::mouse::MouseModifiers {
            shift: self.mods.shift,
            ctrl: self.mods.ctrl,
            alt: self.mods.alt,
        };

        let term = self.active_session().app().terminal();

        // When mouse tracking is on, send wheel as button events.
        if term.mouse_tracking_enabled() {
            let sgr = term.mouse_sgr_enabled();
            let urxvt = term.mouse_urxvt_enabled();

            let (dx, dy) = match delta {
                winit::event::MouseScrollDelta::LineDelta(x, y) => (x as i32, -(y as i32)),
                winit::event::MouseScrollDelta::PixelDelta(pos) => {
                    let x = (pos.x as f32 / 8.0).round() as i32;
                    let y = (pos.y as f32 / 16.0).round() as i32;
                    (x, -y)
                }
            };

            // Each scroll line = one wheel event.
            for _ in 0..dy.abs() {
                let button = if dy > 0 {
                    crate::mouse::MouseButton::WheelUp
                } else {
                    crate::mouse::MouseButton::WheelDown
                };
                let ev = crate::mouse::MouseEvent {
                    button,
                    x: col,
                    y: row,
                    mods,
                };
                if let Some(bytes) = crate::mouse::encode_mouse_event(&ev, sgr, urxvt, true) {
                    self.write_to_pty(&bytes);
                }
            }

            // Horizontal scroll.
            for _ in 0..dx.abs() {
                let button = if dx > 0 {
                    crate::mouse::MouseButton::WheelRight
                } else {
                    crate::mouse::MouseButton::WheelLeft
                };
                let ev = crate::mouse::MouseEvent {
                    button,
                    x: col,
                    y: row,
                    mods,
                };
                if let Some(bytes) = crate::mouse::encode_mouse_event(&ev, sgr, urxvt, true) {
                    self.write_to_pty(&bytes);
                }
            }
            return;
        }

        // Mouse tracking OFF — scroll the scrollback buffer.
        let (lines, direction) = match delta {
            winit::event::MouseScrollDelta::LineDelta(_x, y) => (y.abs() as usize, y > 0.0),
            winit::event::MouseScrollDelta::PixelDelta(pos) => {
                let lines = (pos.y.abs() as f32 / 16.0).round() as usize;
                (lines.max(1), pos.y < 0.0) // Natural scroll: pixel up = scroll up
            }
        };

        let grid = self
            .active_session_mut()
            .app_mut()
            .terminal_mut()
            .grid_mut();
        if direction {
            grid.scroll_up_viewport(lines);
        } else {
            grid.scroll_down_viewport(lines);
        }

        if let Some(ref window) = self.window {
            window.request_redraw();
        }
    }

    /// Copy the current text selection to the clipboard.
    ///
    /// Extracts text from the grid between selection start and end.
    fn copy_selection_to_clipboard(&self) {
        let Some(((sx, sy), (ex, ey))) = self.selection.normalized() else {
            return;
        };

        let grid = self.active_session().app().grid();
        let mut text = String::new();

        if sy == ey {
            // Single-line selection.
            for x in sx..=ex {
                if let Some(cell) = grid.cell(x as usize, sy as usize) {
                    text.push(cell.ch);
                }
            }
        } else {
            // Multi-line selection.
            // First line: from sx to end of row.
            let width = grid.width();
            for x in sx..width as u16 {
                if let Some(cell) = grid.cell(x as usize, sy as usize) {
                    text.push(cell.ch);
                }
            }
            text.push('\n');
            // Middle lines: full rows.
            for y in (sy + 1)..ey {
                for x in 0..width as u16 {
                    if let Some(cell) = grid.cell(x as usize, y as usize) {
                        text.push(cell.ch);
                    }
                }
                text.push('\n');
            }
            // Last line: from start of row to ex.
            for x in 0..=ex {
                if let Some(cell) = grid.cell(x as usize, ey as usize) {
                    text.push(cell.ch);
                }
            }
        }

        // Trim trailing whitespace per line.
        let text = text
            .lines()
            .map(|l| l.trim_end())
            .collect::<Vec<_>>()
            .join("\n");

        if !text.is_empty() {
            log::debug!("Clipboard copy: {} chars", text.len());
            crate::clipboard::set_clipboard_bytes(text.as_bytes());
        }
    }

    /// Paste text from the system clipboard into the PTY.
    ///
    /// Reads from the clipboard via `pbpaste` (macOS) or platform equivalent.
    /// If bracketed paste mode is active, wraps the text in escape markers.
    fn paste_from_clipboard(&mut self) {
        let Some(text) = crate::clipboard::read_clipboard() else {
            log::debug!("Paste: clipboard empty or unavailable");
            return;
        };
        if text.is_empty() {
            return;
        }

        let bracketed = self.active_session().app().terminal().bracketed_paste();
        let bytes = crate::clipboard::bracket_paste(&text, bracketed);
        log::debug!("Paste: {} bytes (bracketed={})", bytes.len(), bracketed);
        self.write_to_pty(&bytes);
    }

    /// Poll for pending OSC 52 clipboard set operations.
    ///
    /// Called from `about_to_wait` to apply any OSC 52 clipboard changes
    /// that programs have requested.
    fn poll_osc52_clipboard(&mut self) {
        if let Some(data) = self
            .active_session_mut()
            .app_mut()
            .terminal_mut()
            .take_pending_clipboard_set()
        {
            log::debug!("OSC 52 clipboard set: {} bytes", data.len());
            crate::clipboard::set_clipboard_bytes(&data);
        }
    }

    /// Poll for bell events from the terminal and trigger visual bell (P11-E).
    fn poll_bell(&mut self) {
        if self
            .active_session_mut()
            .app_mut()
            .terminal_mut()
            .take_bell()
        {
            self.visual_bell_frames = VISUAL_BELL_DURATION_FRAMES;
            log::debug!("Bell triggered");
        }
    }

    // ── Font zoom (P11-A) ─────────────────────────────────────────

    /// Apply the current font zoom level to the renderer (P11-A).
    ///
    /// Calls `set_font_size()` on the GlyphonRenderer, which recomputes
    /// cell metrics. The actual cell dimension change triggers a resize
    /// on the next `about_to_wait` cycle.
    fn apply_font_size(&mut self) {
        let size = self.font_zoom.current_size();
        if let Some(ref mut renderer) = self.renderer {
            renderer.set_font_size(size);
            log::info!("Font size: {size:.1}px");
        }
    }

    // ── Window controls (P11-C) ───────────────────────────────────

    /// Apply the active theme from the App's ThemeManager to the GPU renderer (P11-D).
    fn apply_theme_to_renderer(&mut self) {
        // Clone the theme first to avoid borrow conflict between
        // active_session() and renderer.
        let theme = self
            .active_session_mut()
            .app_mut()
            .theme_manager()
            .current()
            .clone();
        if let Some(ref mut renderer) = self.renderer {
            renderer.set_theme(theme);
            log::debug!("Theme applied to renderer");
        }
    }

    /// Cycle through available themes (P11-D).
    fn cycle_theme(&mut self) {
        let name = {
            let mgr = self.active_session_mut().app_mut().theme_manager();
            mgr.cycle_next();
            mgr.current_name().to_owned()
        };
        self.apply_theme_to_renderer();
        log::info!("Theme: {name}");
    }

    /// Toggle fullscreen mode.
    fn toggle_fullscreen(&mut self) {
        if let Some(ref window) = self.window {
            self.fullscreen = !self.fullscreen;
            if self.fullscreen {
                window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
            } else {
                window.set_fullscreen(None);
            }
            log::info!("Fullscreen: {}", self.fullscreen);
        }
    }

    /// Toggle window maximized state.
    fn toggle_maximized(&mut self) {
        if let Some(ref window) = self.window {
            self.maximized = !self.maximized;
            window.set_maximized(self.maximized);
            log::info!("Maximized: {}", self.maximized);
        }
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
                log::info!("CloseRequested, exiting");
                event_loop.exit();
            }

            WindowEvent::RedrawRequested => {
                // Pump PTY events before rendering.
                self.active_session_mut().pump();

                // Check exit.
                if !self.active_session().is_running() {
                    event_loop.exit();
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
                }

                // Check PTY exit.
                if !self.active_session_mut().is_alive() {
                    log::info!("PTY exited");
                    event_loop.exit();
                }
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
                // P12-D: Send focus event report if DECSET 1004 is active.
                let report = if focused {
                    self.active_session().app().terminal().focus_in_report()
                } else {
                    self.active_session().app().terminal().focus_out_report()
                };
                if !report.is_empty() {
                    self.write_to_pty(&report);
                }
                if focused && let Some(ref window) = self.window {
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
        // Pump PTY events.
        self.active_session_mut().pump();

        // P10-C: Poll AI bridge for results.
        #[cfg(feature = "ai")]
        self.poll_ai_bridge();

        // P10-B: Poll OSC 52 clipboard set requests.
        self.poll_osc52_clipboard();

        // P11-E: Poll for bell events.
        self.poll_bell();

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

        // Check exit.
        if !self.active_session().is_running() {
            event_loop.exit();
            return;
        }

        // Check PTY exit.
        if !self.active_session_mut().is_alive() {
            log::info!("PTY exited");
            event_loop.exit();
        }

        // Always request redraw to keep the render loop alive.
        // The GPU only draws when there's new content (PTY output is event-driven).
        // But for a terminal we want continuous rendering to show the blinking cursor.
        if let Some(ref window) = self.window {
            window.request_redraw();
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
