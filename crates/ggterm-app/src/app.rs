//! The main application: connects PTY, Terminal, Parser, and Renderer.
//!
//! This is the core integration point. It owns the Terminal + Parser on the
//! main thread and processes events from the PTY reader thread.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use ggterm_core::{Parser, Terminal};
use ggterm_render::{ConsoleRenderer, CursorState, RenderTheme, Renderer};

use crate::command_nav::CommandNavState;
use crate::config::ConfigManager;
use crate::event::{AppEvent, EventReceiver, EventSender};
use crate::input::InputEncoder;
#[cfg(feature = "plugin")]
use crate::plugin_integration::{PluginBridge, build_context};
use crate::tabs::TabManager;
use crate::theme::AppTheme;
#[cfg(feature = "plugin")]
use ggterm_core::CommandMarkKind;
#[cfg(feature = "plugin")]
use ggterm_plugin::HookResult;

/// The terminal application.
///
/// Owns the terminal state, parser, renderer, and PTY communication.
/// In a full desktop build, this also manages the winit window and
/// wgpu surface (feature-gated behind `desktop`).
pub struct App {
    terminal: Terminal,
    parser: Parser,
    renderer: ConsoleRenderer,
    input_encoder: InputEncoder,
    event_rx: EventReceiver,
    pty_writer: Option<Box<dyn std::io::Write + Send>>,
    running: bool,
    /// Phase 5: theme manager
    theme: AppTheme,
    /// Phase 5: tab manager (metadata only — actual terminals managed at desktop level)
    tabs: TabManager,
    /// Phase 5: last AI response text (for display)
    #[cfg(feature = "ai")]
    last_ai_response: Option<String>,
    /// Phase 6: plugin manager (None when plugin feature disabled or not configured)
    #[cfg(feature = "plugin")]
    plugins: Option<PluginBridge>,
    /// Phase 8-B: configuration manager
    config: Option<ConfigManager>,
    /// Phase 8-D: command navigation overlay state
    command_nav: CommandNavState,
    /// PTY output tee buffer — captures PTY bytes for P2P forwarding.
    pty_tee: Vec<u8>,
}

impl App {
    /// Create a new application with the given terminal size.
    pub fn new(cols: usize, rows: usize) -> (Self, EventSender) {
        let (tx, rx) = mpsc::channel::<AppEvent>();

        let app = Self {
            terminal: Terminal::new(cols, rows),
            parser: Parser::new(),
            renderer: ConsoleRenderer::new(cols, rows),
            input_encoder: InputEncoder::new(),
            event_rx: rx,
            pty_writer: None,
            running: false,
            theme: AppTheme::new(),
            tabs: TabManager::new(cols, rows),
            #[cfg(feature = "ai")]
            last_ai_response: None,
            #[cfg(feature = "plugin")]
            plugins: None,
            config: None,
            command_nav: CommandNavState::new(),
            pty_tee: Vec::new(),
        };

        (app, tx)
    }

    /// Create a new application with a config manager.
    ///
    /// Applies the config values at construction time:
    /// - `appearance.theme` → sets the starting theme
    /// - `terminal.scrollback_lines` → sets Grid scrollback
    /// - `terminal.shell` → stored for PtySession to use
    /// - `ai.*` → AI engine settings
    pub fn with_config(cols: usize, rows: usize, mgr: ConfigManager) -> (Self, EventSender) {
        let (mut app, tx) = Self::new(cols, rows);
        app.apply_config(mgr.config());
        app.config = Some(mgr);
        (app, tx)
    }

    /// Apply config values to the running app state.
    fn apply_config(&mut self, cfg: &crate::config::Config) {
        // Theme: switch to the configured theme name.
        // set_by_name returns true if the theme was found and applied.
        self.theme.set_by_name(&cfg.appearance.theme);

        // Scrollback limit
        self.terminal
            .grid_mut()
            .set_scrollback(cfg.terminal.scrollback_lines);
    }

    /// Reload config from disk and apply any changes.
    ///
    /// Returns `Ok(true)` if the config changed and was applied.
    pub fn reload_config(&mut self) -> Result<bool, crate::config::ConfigError> {
        if let Some(ref mut mgr) = self.config {
            let changed = mgr.reload()?;
            if changed {
                let cfg = mgr.config().clone();
                self.apply_config(&cfg);
            }
            Ok(changed)
        } else {
            Ok(false)
        }
    }

    /// Get the current config, if a ConfigManager is attached.
    pub fn config(&self) -> Option<&crate::config::Config> {
        self.config.as_ref().map(|m| m.config())
    }

    /// Attach a ConfigManager to an existing app.
    pub fn set_config(&mut self, mgr: ConfigManager) {
        self.apply_config(mgr.config());
        self.config = Some(mgr);
    }

    /// Attach a PTY writer for sending keyboard input to the child process.
    pub fn set_pty_writer(&mut self, writer: Box<dyn std::io::Write + Send>) {
        self.pty_writer = Some(writer);
    }

    /// Mark the app as running.
    ///
    /// Call this before using [`pump()`](Self::pump) in a non-blocking
    /// event loop (e.g. the desktop winit loop).  [`new()`](Self::new)
    /// leaves `running = false`; this flips it so [`is_running()`](Self::is_running)
    /// returns `true` until a `PtyExit` or `Quit` event arrives.
    pub fn start(&mut self) {
        self.running = true;
    }

    /// Process a single event. Returns `true` if the app should continue running.
    pub fn handle_event(&mut self, event: AppEvent) -> bool {
        match event {
            AppEvent::PtyBytes(bytes) => {
                // Capture bytes for P2P tee.
                self.pty_tee.extend_from_slice(&bytes);

                #[cfg(feature = "plugin")]
                let marks_before = self.terminal.command_marks().len();

                self.parser.feed(&bytes, &mut self.terminal);
                self.render();

                // Phase 6: dispatch OnOutput hook (read-only)
                #[cfg(feature = "plugin")]
                if let Some(ref mut bridge) = self.plugins {
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    let ctx = build_context(
                        self.renderer.cols(),
                        self.renderer.rows(),
                        self.theme.current_name(),
                    );
                    bridge.dispatch_output(&text, &ctx);

                    // P6-D3: OSC 133 command block dispatch.
                    // Detect new CommandStart (B) and CommandEnd (D) marks
                    // produced by this PtyBytes event and fire the
                    // corresponding hooks.
                    let marks = self.terminal.command_marks();
                    for (offset, mark) in marks[marks_before..].iter().enumerate() {
                        match mark.kind {
                            CommandMarkKind::CommandStart => {
                                let cmd = self.terminal.extract_row_text(mark.row);
                                bridge.dispatch_command_start(&cmd, &ctx);
                            }
                            CommandMarkKind::CommandEnd => {
                                let exit_code = mark.exit_code.unwrap_or(-1);
                                // Find nearest preceding CommandStart mark
                                // to extract the command text.
                                let global_idx = marks_before + offset;
                                let cmd = marks[..global_idx]
                                    .iter()
                                    .rev()
                                    .find(|m| m.kind == CommandMarkKind::CommandStart)
                                    .map(|m| self.terminal.extract_row_text(m.row))
                                    .unwrap_or_default();
                                bridge.dispatch_command_end(&cmd, exit_code, &ctx);
                            }
                            _ => {}
                        }
                    }
                }
            }

            AppEvent::Resize { cols, rows } => {
                self.terminal.resize(cols as usize, rows as usize);
                self.renderer.resize(cols as usize, rows as usize);
                self.tabs.resize_all(cols as usize, rows as usize);
                self.render();

                // Phase 6: dispatch OnResize hook
                #[cfg(feature = "plugin")]
                if let Some(ref mut bridge) = self.plugins {
                    let ctx =
                        build_context(cols as usize, rows as usize, self.theme.current_name());
                    bridge.dispatch_resize(cols as usize, rows as usize, &ctx);
                }
            }

            AppEvent::Keyboard(bytes) => {
                // Phase 6: dispatch OnInput hook before PTY write
                #[cfg(feature = "plugin")]
                let effective_bytes: Option<Vec<u8>> = {
                    if let Some(ref mut bridge) = self.plugins {
                        let text = String::from_utf8_lossy(&bytes).into_owned();
                        let ctx = build_context(
                            self.renderer.cols(),
                            self.renderer.rows(),
                            self.theme.current_name(),
                        );
                        match bridge.dispatch_input(&text, &ctx) {
                            HookResult::Deny => None,
                            HookResult::Transform(new_text) => Some(new_text.into_bytes()),
                            _ => Some(bytes.clone()),
                        }
                    } else {
                        Some(bytes.clone())
                    }
                };
                #[cfg(not(feature = "plugin"))]
                let effective_bytes: Option<Vec<u8>> = Some(bytes);

                if let Some(effective_bytes) = effective_bytes
                    && let Some(ref mut writer) = self.pty_writer
                {
                    let _ = writer.write_all(&effective_bytes);
                    let _ = writer.flush();
                }
            }

            AppEvent::PtyExit => {
                self.running = false;
            }

            AppEvent::Quit => {
                self.running = false;
            }

            // ── Tab management (Phase 5-B) ──
            AppEvent::NewTab => {
                self.tabs.open_tab();
            }
            AppEvent::CloseTab(index) => {
                let idx = index.unwrap_or(self.tabs.active_index());
                self.tabs.close_tab(idx);
            }
            AppEvent::SwitchTab(index) => {
                self.tabs.switch_tab(index);
            }
            AppEvent::NextTab => {
                self.tabs.next_tab();
            }
            AppEvent::PrevTab => {
                self.tabs.prev_tab();
            }

            // ── Theme management (Phase 5-A) ──
            AppEvent::SetTheme(name) => {
                #[cfg(feature = "plugin")]
                let old_name = self.theme.current_name().to_string();
                self.theme.set_by_name(&name);

                // Phase 6: dispatch OnThemeChange hook
                #[cfg(feature = "plugin")]
                if let Some(ref mut bridge) = self.plugins {
                    let ctx = build_context(
                        self.renderer.cols(),
                        self.renderer.rows(),
                        self.theme.current_name(),
                    );
                    bridge.dispatch_theme_change(&old_name, &name, &ctx);
                }
            }
            AppEvent::CycleTheme => {
                #[cfg(feature = "plugin")]
                let old_name = self.theme.current_name().to_string();
                self.theme.cycle_next();

                // Phase 6: dispatch OnThemeChange hook
                #[cfg(feature = "plugin")]
                if let Some(ref mut bridge) = self.plugins {
                    let new_name = self.theme.current_name().to_string();
                    let ctx = build_context(self.renderer.cols(), self.renderer.rows(), &new_name);
                    bridge.dispatch_theme_change(&old_name, &new_name, &ctx);
                }
            }

            // ── Config events (Phase 8-B) ──
            AppEvent::ReloadConfig => {
                let _ = self.reload_config();
            }

            // ── AI events (Phase 5-C) ──
            #[cfg(feature = "ai")]
            AppEvent::AIResponse(text) => {
                self.last_ai_response = Some(text);
            }
            #[cfg(feature = "ai")]
            AppEvent::AIError(msg) => {
                self.last_ai_response = Some(format!("AI Error: {msg}"));
            }
            #[cfg(feature = "ai")]
            AppEvent::AIRequest(_) => {
                // AI requests are handled by AIBridge at the desktop level.
            }

            // ── Command navigation events ──
            AppEvent::NextCommandBlock => {
                self.command_nav.jump_next(&self.terminal);
            }
            AppEvent::PrevCommandBlock => {
                self.command_nav.jump_prev(&self.terminal);
            }
            AppEvent::ToggleCommandNav => {
                self.command_nav.toggle();
            }
        }

        self.running
    }

    /// Render the current terminal state.
    fn render(&mut self) {
        let (cx, cy) = self.terminal.cursor();
        let cursor = CursorState::new(cx, cy);
        self.renderer.render(self.terminal.grid(), &cursor, None);
    }

    /// Run the event loop (blocking).
    ///
    /// This is the headless/test mode — no winit window. Events arrive
    /// via the channel from the PTY reader thread or test code.
    pub fn run(&mut self) {
        self.running = true;

        while self.running {
            match self.event_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(event) => {
                    self.handle_event(event);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Idle — in desktop mode, this is where wgpu vsync would block.
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    // All senders dropped — quit.
                    self.running = false;
                }
            }
        }
    }

    /// Process all pending events without blocking.
    ///
    /// Useful for tests: feed events and check state without blocking.
    /// Useful for tests: feed events and check state without blocking.
    ///
    /// Returns `true` if any events were processed (P21-D: dirty rect).
    /// Take and clear the PTY tee buffer (for P2P output forwarding).
    pub fn take_pty_tee(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pty_tee)
    }

    /// Put data back into the pty_tee buffer (used when P2P is waiting).
    pub fn restore_pty_tee(&mut self, data: Vec<u8>) {
        self.pty_tee = data;
    }

    pub fn pump(&mut self) -> bool {
        let mut had_data = false;
        while let Ok(event) = self.event_rx.try_recv() {
            self.handle_event(event);
            had_data = true;
        }
        had_data
    }

    /// Get a reference to the terminal.
    pub fn terminal(&self) -> &Terminal {
        &self.terminal
    }

    /// Get a mutable reference to the terminal.
    pub fn terminal_mut(&mut self) -> &mut Terminal {
        &mut self.terminal
    }

    /// Get the current terminal grid (read-only).
    pub fn grid(&self) -> &ggterm_core::Grid {
        self.terminal.grid()
    }

    /// Get the current terminal grid (mutable).
    pub fn grid_mut(&mut self) -> &mut ggterm_core::Grid {
        self.terminal.grid_mut()
    }

    /// Get command navigation overlay state (Phase 8-D).
    pub fn command_nav(&self) -> &CommandNavState {
        &self.command_nav
    }

    /// Get mutable command navigation overlay state (Phase 8-D).
    pub fn command_nav_mut(&mut self) -> &mut CommandNavState {
        &mut self.command_nav
    }

    /// Get the cursor position (x, y) and visibility.
    pub fn cursor_state(&self) -> (usize, usize, bool) {
        let (cx, cy) = self.terminal.cursor();
        (cx, cy, self.terminal.cursor_visible())
    }

    /// Get the current cursor position (x, y).
    pub fn cursor(&self) -> (usize, usize) {
        self.terminal.cursor()
    }

    /// Whether the cursor is visible (DECSET 25).
    pub fn cursor_visible(&self) -> bool {
        self.terminal.cursor_visible()
    }

    /// Get the current rendered output (ConsoleRenderer).
    pub fn output(&self) -> &str {
        self.renderer.output()
    }

    /// Check if the app is still running.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Send typed text as keyboard input (convenience for testing).
    pub fn send_text(&mut self, text: &str) {
        for ch in text.chars() {
            let key = crate::input::InputKey::char(ch);
            let bytes = self.input_encoder.encode(&key);
            if let Some(ref mut writer) = self.pty_writer {
                let _ = writer.write_all(&bytes);
                let _ = writer.flush();
            }
        }
    }

    /// Encode a key press and send to PTY (if attached).
    /// Inject bytes directly into the terminal emulator (bypassing PTY).
    /// Used for hold-mode messages and other UI text that should appear
    /// as if the terminal program printed it.
    pub fn inject_bytes(&mut self, bytes: &[u8]) {
        self.parser.feed(bytes, &mut self.terminal);
        self.render();
    }

    pub fn send_key(&mut self, key: &crate::input::InputKey) {
        let bytes = self.input_encoder.encode(key);
        if let Some(ref mut writer) = self.pty_writer {
            let _ = writer.write_all(&bytes);
            let _ = writer.flush();
        }
    }

    /// Get a reference to the input encoder (for mode updates like DECCKM).
    pub fn input_encoder(&mut self) -> &mut InputEncoder {
        &mut self.input_encoder
    }

    // ── Phase 5: Theme accessors ──

    /// Get the current theme name.
    pub fn theme_name(&self) -> &str {
        self.theme.current_name()
    }

    /// Get the current render theme.
    pub fn theme(&self) -> &RenderTheme {
        self.theme.current()
    }

    /// Set theme by name. Returns `true` if found.
    pub fn set_theme(&mut self, name: &str) -> bool {
        self.theme.set_by_name(name)
    }

    /// Cycle to the next theme.
    pub fn cycle_theme(&mut self) {
        self.theme.cycle_next();
    }

    /// Get the app theme manager (mutable, for registering callbacks).
    pub fn theme_manager(&mut self) -> &mut AppTheme {
        &mut self.theme
    }

    // ── Phase 5: Tab accessors ──

    /// Get the number of tabs.
    pub fn tab_count(&self) -> usize {
        self.tabs.tab_count()
    }

    /// Get the active tab index.
    pub fn active_tab_index(&self) -> usize {
        self.tabs.active_index()
    }

    /// Get tab manager reference.
    pub fn tabs(&self) -> &TabManager {
        &self.tabs
    }

    /// Get tab manager (mutable).
    pub fn tabs_mut(&mut self) -> &mut TabManager {
        &mut self.tabs
    }

    // ── Phase 6: Plugin accessors ──

    /// Set the plugin bridge for hook dispatch.
    ///
    /// Pass `Some(bridge)` to enable plugin hooks, or `None` to disable.
    #[cfg(feature = "plugin")]
    pub fn set_plugins(&mut self, bridge: Option<PluginBridge>) {
        self.plugins = bridge;
    }

    /// Get a reference to the plugin bridge (if configured).
    #[cfg(feature = "plugin")]
    pub fn plugins(&self) -> Option<&PluginBridge> {
        self.plugins.as_ref()
    }

    /// Get a mutable reference to the plugin bridge (if configured).
    #[cfg(feature = "plugin")]
    pub fn plugins_mut(&mut self) -> Option<&mut PluginBridge> {
        self.plugins.as_mut()
    }

    // ── Phase 5: AI accessors ──

    /// Get the last AI response text (if any).
    #[cfg(feature = "ai")]
    pub fn last_ai_response(&self) -> Option<&str> {
        self.last_ai_response.as_deref()
    }

    /// Clear the last AI response.
    #[cfg(feature = "ai")]
    pub fn clear_ai_response(&mut self) {
        self.last_ai_response = None;
    }
}

/// Spawn a PTY reader thread.
///
/// Reads from the PTY in a background thread and sends `PtyBytes` events
/// to the main event loop.
pub fn spawn_pty_reader(
    mut reader: Box<dyn std::io::Read + Send>,
    sender: EventSender,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    // EOF — child process exited
                    let _ = sender.send(AppEvent::PtyExit);
                    break;
                }
                Ok(n) => {
                    let bytes = buf[..n].to_vec();
                    if sender.send(AppEvent::PtyBytes(bytes)).is_err() {
                        // Main loop dropped receiver — quit.
                        break;
                    }
                }
                Err(_e) => {
                    // Read error — treat as exit.
                    let _ = sender.send(AppEvent::PtyExit);
                    break;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::AppEvent;

    #[test]
    fn test_app_creation() {
        let (app, _tx) = App::new(80, 24);
        assert_eq!(app.grid().width(), 80);
        assert_eq!(app.grid().height(), 24);
        assert!(!app.is_running());
    }

    #[test]
    fn test_app_process_pty_bytes() {
        let (mut app, tx) = App::new(40, 5);

        // Feed some bytes via the event channel
        tx.send(AppEvent::PtyBytes(b"Hello World".to_vec()))
            .unwrap();
        app.pump();

        // Check that the terminal processed the bytes
        let output = app.output().to_string();
        assert!(output.contains("Hello World"));
    }

    #[test]
    fn test_app_resize() {
        let (mut app, tx) = App::new(80, 24);

        tx.send(AppEvent::Resize {
            cols: 120,
            rows: 40,
        })
        .unwrap();
        app.pump();

        assert_eq!(app.grid().width(), 120);
        assert_eq!(app.grid().height(), 40);
    }

    #[test]
    fn test_app_quit_event() {
        let (mut app, tx) = App::new(80, 24);

        tx.send(AppEvent::Quit).unwrap();
        app.pump();

        assert!(!app.is_running());
    }

    #[test]
    fn test_app_pty_exit() {
        let (mut app, tx) = App::new(80, 24);

        tx.send(AppEvent::PtyExit).unwrap();
        app.pump();

        assert!(!app.is_running());
    }

    #[test]
    fn test_app_multiline_output() {
        let (mut app, tx) = App::new(20, 5);

        tx.send(AppEvent::PtyBytes(b"Line 1\r\nLine 2\r\nLine 3".to_vec()))
            .unwrap();
        app.pump();

        let output = app.output().to_string();
        assert!(output.contains("Line 1"));
        assert!(output.contains("Line 2"));
        assert!(output.contains("Line 3"));
    }

    #[test]
    fn test_app_with_colors() {
        let (mut app, tx) = App::new(40, 3);

        tx.send(AppEvent::PtyBytes(b"\x1b[1;31mError\x1b[0m".to_vec()))
            .unwrap();
        app.pump();

        let output = app.output();
        assert!(output.contains("1;31"));
    }

    #[test]
    fn test_app_multiple_events() {
        let (mut app, tx) = App::new(40, 5);

        tx.send(AppEvent::PtyBytes(b"First".to_vec())).unwrap();
        tx.send(AppEvent::PtyBytes(b"Second".to_vec())).unwrap();
        tx.send(AppEvent::PtyBytes(b"Third".to_vec())).unwrap();
        app.pump();

        let output = app.output();
        assert!(output.contains("FirstSecondThird"));
    }

    #[test]
    fn test_app_cjk_text() {
        let (mut app, tx) = App::new(40, 5);

        tx.send(AppEvent::PtyBytes("你好世界".as_bytes().to_vec()))
            .unwrap();
        app.pump();

        let output = app.output();
        assert!(output.contains("你好世界"));
    }

    // ── Phase 5: Theme tests ──

    #[test]
    fn t_app_theme_default_is_dark() {
        let (app, _tx) = App::new(80, 24);
        assert_eq!(app.theme_name(), "dark");
    }

    #[test]
    fn t_app_set_theme_via_event() {
        let (mut app, tx) = App::new(80, 24);
        tx.send(AppEvent::SetTheme("dracula".to_string())).unwrap();
        app.pump();
        assert_eq!(app.theme_name(), "dracula");
    }

    #[test]
    fn t_app_set_theme_directly() {
        let (mut app, _tx) = App::new(80, 24);
        assert!(app.set_theme("light"));
        assert_eq!(app.theme_name(), "light");
    }

    #[test]
    fn t_app_set_theme_unknown() {
        let (mut app, _tx) = App::new(80, 24);
        assert!(!app.set_theme("nonexistent"));
        assert_eq!(app.theme_name(), "dark");
    }

    #[test]
    fn t_app_cycle_theme() {
        let (mut app, _tx) = App::new(80, 24);
        assert_eq!(app.theme_name(), "dark");
        // Cycle through all themes: dark → light → dracula → solarized-dark → ...
        let names = ggterm_render::ThemeManager::available_themes();
        for &expected in &names[1..] {
            app.cycle_theme();
            assert_eq!(app.theme_name(), expected);
        }
        // After cycling all themes, should wrap back to "dark".
        app.cycle_theme();
        assert_eq!(app.theme_name(), "dark");
    }

    #[test]
    fn t_app_cycle_theme_via_event() {
        let (mut app, tx) = App::new(80, 24);
        tx.send(AppEvent::CycleTheme).unwrap();
        app.pump();
        assert_eq!(app.theme_name(), "light");
    }

    // ── Phase 5: Tab tests ──

    #[test]
    fn t_app_default_tab_count() {
        let (app, _tx) = App::new(80, 24);
        assert_eq!(app.tab_count(), 1);
    }

    #[test]
    fn t_app_new_tab_via_event() {
        let (mut app, tx) = App::new(80, 24);
        tx.send(AppEvent::NewTab).unwrap();
        tx.send(AppEvent::NewTab).unwrap();
        app.pump();
        assert_eq!(app.tab_count(), 3);
    }

    #[test]
    fn t_app_switch_tab_via_event() {
        let (mut app, tx) = App::new(80, 24);
        tx.send(AppEvent::NewTab).unwrap();
        tx.send(AppEvent::NewTab).unwrap();
        app.pump();
        assert_eq!(app.active_tab_index(), 2); // last opened is active

        tx.send(AppEvent::SwitchTab(0)).unwrap();
        app.pump();
        assert_eq!(app.active_tab_index(), 0);
    }

    #[test]
    fn t_app_next_tab_via_event() {
        let (mut app, tx) = App::new(80, 24);
        tx.send(AppEvent::NewTab).unwrap();
        tx.send(AppEvent::NewTab).unwrap();
        app.pump();
        tx.send(AppEvent::SwitchTab(0)).unwrap();
        app.pump();
        tx.send(AppEvent::NextTab).unwrap();
        app.pump();
        assert_eq!(app.active_tab_index(), 1);
    }

    #[test]
    fn t_app_prev_tab_via_event() {
        let (mut app, tx) = App::new(80, 24);
        tx.send(AppEvent::NewTab).unwrap();
        tx.send(AppEvent::NewTab).unwrap();
        app.pump();
        tx.send(AppEvent::SwitchTab(0)).unwrap();
        app.pump();
        tx.send(AppEvent::PrevTab).unwrap();
        app.pump();
        assert_eq!(app.active_tab_index(), 2); // wraps to last
    }

    #[test]
    fn t_app_close_tab_via_event() {
        let (mut app, tx) = App::new(80, 24);
        tx.send(AppEvent::NewTab).unwrap();
        tx.send(AppEvent::NewTab).unwrap();
        app.pump();
        assert_eq!(app.tab_count(), 3);

        tx.send(AppEvent::CloseTab(Some(2))).unwrap();
        app.pump();
        assert_eq!(app.tab_count(), 2);
    }

    #[test]
    fn t_app_close_active_tab_via_event() {
        let (mut app, tx) = App::new(80, 24);
        tx.send(AppEvent::NewTab).unwrap();
        app.pump();
        assert_eq!(app.tab_count(), 2);

        tx.send(AppEvent::CloseTab(None)).unwrap();
        app.pump();
        assert_eq!(app.tab_count(), 1);
    }

    #[test]
    fn t_app_resize_updates_tabs() {
        let (mut app, tx) = App::new(80, 24);
        tx.send(AppEvent::NewTab).unwrap();
        app.pump();

        tx.send(AppEvent::Resize {
            cols: 120,
            rows: 40,
        })
        .unwrap();
        app.pump();

        assert_eq!(app.tabs().tabs()[0].cols, 120);
        assert_eq!(app.tabs().tabs()[0].rows, 40);
    }

    // ── Phase 5: combined event tests ──

    #[test]
    fn t_app_theme_and_tab_events_combined() {
        let (mut app, tx) = App::new(80, 24);

        tx.send(AppEvent::SetTheme("light".to_string())).unwrap();
        tx.send(AppEvent::NewTab).unwrap();
        tx.send(AppEvent::NewTab).unwrap();
        tx.send(AppEvent::CycleTheme).unwrap();
        app.pump();

        assert_eq!(app.theme_name(), "dracula");
        assert_eq!(app.tab_count(), 3);
    }

    #[test]
    fn t_app_quit_still_works_with_new_events() {
        let (mut app, tx) = App::new(80, 24);

        tx.send(AppEvent::NewTab).unwrap();
        tx.send(AppEvent::SetTheme("dracula".to_string())).unwrap();
        tx.send(AppEvent::Quit).unwrap();
        app.pump();

        assert!(!app.is_running());
        assert_eq!(app.tab_count(), 2);
        assert_eq!(app.theme_name(), "dracula");
    }

    #[test]
    fn t_inject_bytes_appears_on_grid() {
        let (mut app, _tx) = App::new(80, 24);
        app.start();

        // Inject "Hi!" directly into the terminal emulator.
        app.inject_bytes(b"Hi!");

        // Verify the text appears on the grid.
        let grid = app.terminal().grid();
        let cell0 = grid.cell(0, 0).unwrap();
        assert_eq!(cell0.ch, 'H');
        let cell1 = grid.cell(1, 0).unwrap();
        assert_eq!(cell1.ch, 'i');
        let cell2 = grid.cell(2, 0).unwrap();
        assert_eq!(cell2.ch, '!');
    }

    #[test]
    fn t_inject_bytes_ansi_escape() {
        let (mut app, _tx) = App::new(80, 24);
        app.start();

        // Inject ANSI-colored text and verify escape sequences are processed.
        app.inject_bytes(b"\x1b[31mR\x1b[0m");

        let grid = app.terminal().grid();
        let cell = grid.cell(0, 0).unwrap();
        assert_eq!(cell.ch, 'R');
        // The cell should have red foreground (SGR 31).
        assert_eq!(cell.fg, ggterm_core::Color::Indexed(1));
    }

    #[test]
    fn t_inject_bytes_combining_char() {
        let (mut app, _tx) = App::new(80, 24);
        app.start();

        // Inject 'e' + U+0301 (combining acute accent → é).
        app.inject_bytes("e\u{0301}".as_bytes());

        let grid = app.terminal().grid();
        let cell = grid.cell(0, 0).unwrap();
        assert_eq!(cell.ch, 'e');
        assert_eq!(cell.combining.len(), 1);
        assert_eq!(cell.combining[0], '\u{0301}');
    }

    #[test]
    fn t_inject_bytes_newline_advances_cursor() {
        let (mut app, _tx) = App::new(80, 24);
        app.start();

        // Inject two lines separated by \r\n.
        app.inject_bytes(b"line1\r\nline2");

        let grid = app.terminal().grid();
        // Row 0 should have "line1".
        assert_eq!(grid.cell(0, 0).unwrap().ch, 'l');
        // Row 1 should have "line2".
        assert_eq!(grid.cell(0, 1).unwrap().ch, 'l');
        assert_eq!(grid.cell(1, 1).unwrap().ch, 'i');
        assert_eq!(grid.cell(4, 1).unwrap().ch, '2');
    }
}
