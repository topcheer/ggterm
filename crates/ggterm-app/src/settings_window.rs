//! Independent settings window — a separate OS window for GGTerm settings.
//!
//! When the user clicks the gear button or presses Ctrl+, a new winit window
//! is created with its own wgpu surface. The terminal window is not affected.
//!
//! The settings window renders a clean GUI-style interface:
//! - Section headers with accent color
//! - Field rows: label (left) + current value (right)
//! - Navigation: Up/Down to select, Left/Right to adjust, Esc to close
//! - Changes apply live to the terminal

use ggterm_render_wgpu::{GlyphonRenderer, OverlayTextSpec, UiRect};
use std::sync::Arc;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

/// The settings fields that can be edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    Theme,
    FontSize,
    CursorStyle,
    FontFamily,
    Scrollback,
    Shell,
    RestoreSession,
    AiEnabled,
}

impl SettingsField {
    /// All fields in display order.
    pub fn all() -> &'static [SettingsField] {
        &[
            SettingsField::Theme,
            SettingsField::FontSize,
            SettingsField::FontFamily,
            SettingsField::CursorStyle,
            SettingsField::Scrollback,
            SettingsField::Shell,
            SettingsField::RestoreSession,
            SettingsField::AiEnabled,
        ]
    }

    pub fn next(self) -> Self {
        let all = Self::all();
        let idx = all.iter().position(|&f| f == self).unwrap_or(0);
        all[(idx + 1) % all.len()]
    }

    pub fn prev(self) -> Self {
        let all = Self::all();
        let idx = all.iter().position(|&f| f == self).unwrap_or(0);
        all[(idx + all.len() - 1) % all.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            SettingsField::Theme => "Theme",
            SettingsField::FontSize => "Font Size",
            SettingsField::FontFamily => "Font Family",
            SettingsField::CursorStyle => "Cursor Style",
            SettingsField::Scrollback => "Scrollback Lines",
            SettingsField::Shell => "Shell",
            SettingsField::RestoreSession => "Restore Session",
            SettingsField::AiEnabled => "AI Assistant",
        }
    }
}

/// Draft settings values held in the settings window until applied.
#[derive(Debug, Clone)]
pub struct SettingsDraft {
    pub theme: String,
    pub font_size: u32,
    pub font_family: String,
    pub cursor_style: String,
    pub scrollback_lines: usize,
    pub shell: String,
    pub restore_session: bool,
    pub ai_enabled: bool,
}

/// The independent settings window state.
pub struct SettingsWindowState {
    /// The winit window for settings.
    pub window: Arc<Window>,
    /// wgpu surface for this window.
    surface: wgpu::Surface<'static>,
    /// GPU context (device/queue/config).
    gpu: crate::gpu::GpuContext,
    /// Renderer for drawing settings UI.
    renderer: GlyphonRenderer,
    /// Currently selected field.
    pub selected: SettingsField,
    /// Draft values being edited.
    pub draft: SettingsDraft,
    /// Whether settings is in the process of closing.
    closing: bool,
    /// Font size for the settings UI text.
    ui_font_size: f32,
    /// Pixel dimensions of the settings window.
    width: u32,
    height: u32,
    /// Scale factor (Retina = 2.0).
    scale_factor: f64,
    /// Available theme names.
    themes: Vec<String>,
    /// Available cursor styles.
    cursor_styles: Vec<String>,
    /// Last known mouse position (physical pixels).
    mouse_pos: (f32, f32),
}

impl SettingsWindowState {
    /// Theme list for cycling.
    const THEMES: &'static [&'static str] = &[
        "dark",
        "light",
        "dracula",
        "solarized-dark",
        "solarized-light",
        "gruvbox",
        "nord",
        "tokyo-night",
        "catppuccin-mocha",
    ];

    /// Cursor style list for cycling.
    const CURSOR_STYLES: &'static [&'static str] = &["block", "underline", "bar"];

    /// Open a new settings window.
    ///
    /// Creates a winit window, wgpu surface, and renderer.
    /// Returns the state, or None if GPU init fails.
    pub fn open(event_loop: &ActiveEventLoop, draft: SettingsDraft) -> Option<Self> {
        let win_w = 560u32;
        let win_h = 480u32;

        let attrs = Window::default_attributes()
            .with_title("GGTerm Settings")
            .with_inner_size(LogicalSize::new(win_w as f64, win_h as f64))
            .with_resizable(false);

        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("Failed to create settings window: {e}");
                return None;
            }
        };

        let scale_factor = window.scale_factor();
        let phys_w = (win_w as f64 * scale_factor).round() as u32;
        let phys_h = (win_h as f64 * scale_factor).round() as u32;

        // Init wgpu for this window.
        let (_instance, surface, adapter) = match crate::gpu::init_wgpu(Arc::clone(&window)) {
            Ok(result) => result,
            Err(e) => {
                log::error!("Failed to init wgpu for settings window: {e}");
                return None;
            }
        };

        let gpu = match crate::gpu::GpuContext::from_surface(
            &surface,
            &adapter,
            phys_w.max(1),
            phys_h.max(1),
        ) {
            Ok(g) => g,
            Err(e) => {
                log::error!("Failed to create GPU context for settings: {e}");
                return None;
            }
        };

        let renderer = gpu.create_renderer(phys_w, phys_h, scale_factor);

        Some(Self {
            window,
            surface,
            gpu,
            renderer,
            selected: SettingsField::Theme,
            draft,
            closing: false,
            ui_font_size: 15.0,
            width: phys_w,
            height: phys_h,
            scale_factor,
            themes: Self::THEMES.iter().map(|s| s.to_string()).collect(),
            cursor_styles: Self::CURSOR_STYLES.iter().map(|s| s.to_string()).collect(),
            mouse_pos: (0.0, 0.0),
        })
    }

    /// Window ID for event routing.
    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    /// Returns true if the window should be closed.
    pub fn should_close(&self) -> bool {
        self.closing
    }

    /// Request close.
    pub fn close(&mut self) {
        self.closing = true;
    }

    /// Handle keyboard input in the settings window.
    pub fn handle_keyboard(&mut self, event: &KeyEvent) -> bool {
        use winit::keyboard::{KeyCode, PhysicalKey};

        if event.state != ElementState::Pressed {
            return false;
        }

        match &event.physical_key {
            PhysicalKey::Code(KeyCode::Escape) => {
                self.close();
                true
            }
            PhysicalKey::Code(KeyCode::ArrowUp) => {
                self.selected = self.selected.prev();
                self.window.request_redraw();
                true
            }
            PhysicalKey::Code(KeyCode::ArrowDown) => {
                self.selected = self.selected.next();
                self.window.request_redraw();
                true
            }
            PhysicalKey::Code(KeyCode::ArrowLeft) => {
                self.adjust_field(false);
                self.window.request_redraw();
                true
            }
            PhysicalKey::Code(KeyCode::ArrowRight) | PhysicalKey::Code(KeyCode::Enter) => {
                self.adjust_field(true);
                self.window.request_redraw();
                true
            }
            _ => false,
        }
    }

    /// Adjust the selected field. `increase` = true for right/enter, false for left.
    fn adjust_field(&mut self, increase: bool) {
        match self.selected {
            SettingsField::Theme => {
                let idx = self
                    .themes
                    .iter()
                    .position(|t| t == &self.draft.theme)
                    .unwrap_or(0);
                let new_idx = if increase {
                    (idx + 1) % self.themes.len()
                } else {
                    (idx + self.themes.len() - 1) % self.themes.len()
                };
                self.draft.theme = self.themes[new_idx].clone();
            }
            SettingsField::FontSize => {
                if increase {
                    self.draft.font_size = (self.draft.font_size + 1).min(32);
                } else {
                    self.draft.font_size = self.draft.font_size.saturating_sub(1).max(6);
                }
            }
            SettingsField::FontFamily => {
                // Cycle through common monospace fonts.
                let fonts = [
                    "monospace",
                    "Menlo",
                    "DejaVu Sans Mono",
                    "Cascadia Mono",
                    "Courier New",
                ];
                let idx = fonts
                    .iter()
                    .position(|f| *f == self.draft.font_family.as_str())
                    .unwrap_or(0);
                let new_idx = if increase {
                    (idx + 1) % fonts.len()
                } else {
                    (idx + fonts.len() - 1) % fonts.len()
                };
                self.draft.font_family = fonts[new_idx].to_string();
            }
            SettingsField::CursorStyle => {
                let idx = self
                    .cursor_styles
                    .iter()
                    .position(|c| c == &self.draft.cursor_style)
                    .unwrap_or(0);
                let new_idx = if increase {
                    (idx + 1) % self.cursor_styles.len()
                } else {
                    (idx + self.cursor_styles.len() - 1) % self.cursor_styles.len()
                };
                self.draft.cursor_style = self.cursor_styles[new_idx].clone();
            }
            SettingsField::Scrollback => {
                if increase {
                    self.draft.scrollback_lines = (self.draft.scrollback_lines + 1000).min(100000);
                } else {
                    self.draft.scrollback_lines = self.draft.scrollback_lines.saturating_sub(1000);
                }
            }
            SettingsField::Shell => {
                // Cycle through common shells.
                let shells = ["", "/bin/bash", "/bin/zsh", "/bin/fish", "/bin/sh"];
                let idx = shells
                    .iter()
                    .position(|s| *s == self.draft.shell.as_str())
                    .unwrap_or(0);
                let new_idx = if increase {
                    (idx + 1) % shells.len()
                } else {
                    (idx + shells.len() - 1) % shells.len()
                };
                self.draft.shell = shells[new_idx].to_string();
            }
            SettingsField::RestoreSession => {
                self.draft.restore_session = !self.draft.restore_session;
            }
            SettingsField::AiEnabled => {
                self.draft.ai_enabled = !self.draft.ai_enabled;
            }
        }
    }

    /// Handle window events for the settings window.
    pub fn handle_event(&mut self, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::KeyboardInput { event, .. } => self.handle_keyboard(event),
            WindowEvent::CloseRequested => {
                self.close();
                true
            }
            WindowEvent::Resized(size) => {
                let phys_w = size.width.max(1);
                let phys_h = size.height.max(1);
                self.gpu.resize(&self.surface, phys_w, phys_h);
                self.width = phys_w;
                self.height = phys_h;
                self.window.request_redraw();
                true
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let up = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y > 0.0,
                    MouseScrollDelta::PixelDelta(pos) => pos.y > 0.0,
                };
                self.adjust_field(!up);
                self.window.request_redraw();
                true
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_pos = (position.x as f32, position.y as f32);
                // Hit-test which field the cursor is over.
                if let Some(field) = self.field_at_pos(position.x as f32, position.y as f32)
                    && field != self.selected
                {
                    self.selected = field;
                    self.window.request_redraw();
                }
                true
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: winit::event::MouseButton::Left,
                ..
            } => {
                let (px, py) = self.mouse_pos;
                if let Some(field) = self.field_at_pos(px, py) {
                    self.selected = field;
                    // Click on the right half → adjust (next value).
                    // Click on the left half → just select.
                    let scale = self.scale_factor as f32;
                    let margin = 24.0 * scale;
                    let value_x = margin + (self.width as f32 - margin * 2.0) * 0.5;
                    if px > value_x {
                        self.adjust_field(true);
                    }
                    self.window.request_redraw();
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    /// Find which settings field is at a pixel position.
    fn field_at_pos(&self, px: f32, py: f32) -> Option<SettingsField> {
        let scale = self.scale_factor as f32;
        let margin = 24.0 * scale;
        let row_h = 36.0 * scale;
        let header_h = 48.0 * scale;

        // Check if x is within content area.
        if px < margin || px > self.width as f32 - margin {
            return None;
        }

        let start_y = margin + header_h;
        if py < start_y {
            return None;
        }

        let row_idx = ((py - start_y) / row_h) as usize;
        let fields = SettingsField::all();
        if row_idx < fields.len() {
            Some(fields[row_idx])
        } else {
            None
        }
    }

    /// Render one frame of the settings window.
    pub fn render(&mut self) {
        let surface = &self.surface;
        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            _ => return,
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("settings render encoder"),
            });

        // Build UI.
        let (bg_rects, texts) = self.build_ui();

        // Set UI rects and overlay texts on the renderer.
        self.renderer.set_ui_rects(bg_rects);
        self.renderer.set_overlay_text(texts);

        // Render: single pass — clear background + draw overlays.
        {
            let bg_color = wgpu::Color {
                r: 0.06,
                g: 0.06,
                b: 0.08,
                a: 1.0,
            };
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("settings render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(bg_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            let _ = self.renderer.render_overlays_to_pass(
                &self.gpu.device,
                &self.gpu.queue,
                &mut render_pass,
            );
        }

        self.gpu.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
    }

    /// Build the UI layout: background rects and text specs.
    fn build_ui(&self) -> (Vec<UiRect>, Vec<OverlayTextSpec>) {
        let mut rects = Vec::new();
        let mut texts = Vec::new();

        let scale = self.scale_factor as f32;
        let margin = 24.0 * scale;
        let row_h = 36.0 * scale;
        let header_h = 48.0 * scale;
        let footer_h = 32.0 * scale;

        let content_w = self.width as f32 - margin * 2.0;

        // Accent color (blue-ish).
        let accent = (0.35, 0.52, 0.85, 1.0);
        let selected_bg = (0.15, 0.17, 0.25, 0.8);
        let label_color = (200, 205, 220u8);
        let value_color = (240, 240, 250u8);
        let header_color = (255, 255, 255u8);
        let footer_color = (140, 145, 160u8);

        // ── Header ──
        texts.push(OverlayTextSpec {
            text: "Settings".to_string(),
            left: margin,
            top: margin * 0.6,
            color: header_color,
            ..Default::default()
        });

        // Accent bar under header.
        rects.push(UiRect {
            x: margin,
            y: margin + header_h * 0.5,
            w: content_w,
            h: 2.0 * scale,
            color: accent,
            radius: 1.0,
            stroke_width: 0.0,
        });

        // ── Field rows ──
        let fields = SettingsField::all();
        let start_y = margin + header_h;

        for (i, &field) in fields.iter().enumerate() {
            let y = start_y + i as f32 * row_h;
            let is_selected = field == self.selected;

            // Selected row background.
            if is_selected {
                rects.push(UiRect {
                    x: margin,
                    y,
                    w: content_w,
                    h: row_h - 4.0 * scale,
                    color: selected_bg,
                    radius: 6.0 * scale,
                    stroke_width: 0.0,
                });
            }

            // Label (left side).
            texts.push(OverlayTextSpec {
                text: field.label().to_string(),
                left: margin + 12.0 * scale,
                top: y + 8.0 * scale,
                color: if is_selected {
                    (255, 255, 255)
                } else {
                    label_color
                },
                ..Default::default()
            });

            // Value (right side).
            let value = self.field_value_str(field);
            let value_text = if matches!(
                field,
                SettingsField::RestoreSession | SettingsField::AiEnabled
            ) {
                if value == "On" { "[x] On" } else { "[ ] Off" }.to_string()
            } else {
                value.clone()
            };

            // Right-align value by estimating text width.
            let char_w = self.ui_font_size * scale * 0.6;
            let approx_w = value_text.len() as f32 * char_w;
            let value_x = margin + content_w - approx_w - 12.0 * scale;

            // Selected non-boolean field: show ◀ ▶ arrows to indicate adjustability.
            if is_selected
                && !matches!(
                    field,
                    SettingsField::RestoreSession | SettingsField::AiEnabled
                )
            {
                for (arrow, ax) in [
                    ("\u{25C0}", value_x - char_w * 2.0), // ◀ before value
                    ("\u{25B6}", value_x + approx_w + char_w * 0.5), // ▶ after value
                ] {
                    texts.push(OverlayTextSpec {
                        text: arrow.to_string(),
                        left: ax,
                        top: y + 8.0 * scale,
                        color: (100, 130, 180),
                        ..Default::default()
                    });
                }
            }

            texts.push(OverlayTextSpec {
                text: value_text,
                left: value_x,
                top: y + 8.0 * scale,
                color: if is_selected {
                    (130, 200, 255)
                } else {
                    value_color
                },
                ..Default::default()
            });
        }

        // ── Footer ──
        let footer_y = self.height as f32 - margin - footer_h;
        texts.push(OverlayTextSpec {
            text: "Up/Down: select  Left/Right: adjust  Esc: close".to_string(),
            left: margin,
            top: footer_y,
            color: footer_color,
            ..Default::default()
        });

        (rects, texts)
    }

    /// Get the display string for a field value.
    fn field_value_str(&self, field: SettingsField) -> String {
        match field {
            SettingsField::Theme => self.draft.theme.clone(),
            SettingsField::FontSize => format!("{} px", self.draft.font_size),
            SettingsField::FontFamily => self.draft.font_family.clone(),
            SettingsField::CursorStyle => self.draft.cursor_style.clone(),
            SettingsField::Scrollback => {
                if self.draft.scrollback_lines >= 1000 {
                    format!("{}k", self.draft.scrollback_lines / 1000)
                } else {
                    self.draft.scrollback_lines.to_string()
                }
            }
            SettingsField::Shell => {
                if self.draft.shell.is_empty() {
                    "(default)".to_string()
                } else {
                    self.draft.shell.clone()
                }
            }
            SettingsField::RestoreSession => if self.draft.restore_session {
                "On"
            } else {
                "Off"
            }
            .to_string(),
            SettingsField::AiEnabled => {
                if self.draft.ai_enabled { "On" } else { "Off" }.to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_field_navigation() {
        let f = SettingsField::Theme;
        assert_eq!(f.next(), SettingsField::FontSize);
        assert_eq!(f.prev(), SettingsField::AiEnabled); // wraps
    }

    #[test]
    fn test_settings_field_labels() {
        assert_eq!(SettingsField::Theme.label(), "Theme");
        assert_eq!(SettingsField::FontSize.label(), "Font Size");
        assert_eq!(SettingsField::AiEnabled.label(), "AI Assistant");
    }

    #[test]
    fn test_settings_field_all_count() {
        assert_eq!(SettingsField::all().len(), 8);
    }
}
