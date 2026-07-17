//! Platform titlebar support.
//!
//! - **macOS**: Transparent titlebar with preserved traffic light buttons.
//! - **Linux/Windows**: Custom-drawn caption buttons (minimize/maximize/close)
//!   that match each platform's conventions.

#[cfg(target_os = "macos")]
pub mod macos {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use raw_window_handle::HasWindowHandle;

    /// Style mask bit for full-size content view (extends content under titlebar).
    const NS_FULL_SIZE_CONTENT_VIEW_MASK: u64 = 1 << 15;

    /// Make the window's titlebar transparent and extend content underneath.
    pub fn make_titlebar_transparent(window: &winit::window::Window) {
        let Ok(handle) = window.window_handle() else {
            log::warn!("Failed to get window handle for titlebar FFI");
            return;
        };
        let raw_window_handle::RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return;
        };

        let ns_view = appkit.ns_view.as_ptr() as *mut AnyObject;

        unsafe {
            let win: *mut AnyObject = msg_send![ns_view, window];
            if win.is_null() {
                log::warn!("NSWindow is null in make_titlebar_transparent");
                return;
            }

            let current_mask: u64 = msg_send![win, styleMask];
            let new_mask = current_mask | NS_FULL_SIZE_CONTENT_VIEW_MASK;
            let _: () = msg_send![win, setStyleMask: new_mask];
            let _: () = msg_send![win, setTitlebarAppearsTransparent: true];
            let _: () = msg_send![win, setTitleVisibility: 1i64];
            let _: () = msg_send![win, setOpaque: false];
            if let Some(cls) = objc2::runtime::AnyClass::get(c"NSColor") {
                let clear: *mut AnyObject = msg_send![cls, clearColor];
                if !clear.is_null() {
                    let _: () = msg_send![win, setBackgroundColor: clear];
                }
            }

            log::info!("macOS titlebar made transparent (traffic lights preserved)");
        }
    }

    /// Width reserved for traffic light buttons on the left side.
    pub const TRAFFIC_LIGHT_WIDTH: f32 = 92.0;
}

#[cfg(target_os = "macos")]
pub use macos::{TRAFFIC_LIGHT_WIDTH, make_titlebar_transparent as install_traffic_lights};

#[cfg(not(target_os = "macos"))]
pub const TRAFFIC_LIGHT_WIDTH: f32 = 0.0;

#[cfg(not(target_os = "macos"))]
pub fn install_traffic_lights(_window: &winit::window::Window) {}

// ═══════════════════════════════════════════════════════════════════════════
//  Linux / Windows caption buttons
// ═══════════════════════════════════════════════════════════════════════════

/// Window control buttons for Linux/Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowControlButton {
    Minimize,
    Maximize,
    Close,
}

/// Caption button layout for Linux/Windows.
///
/// On Windows, caption buttons follow the standard convention:
/// - Full-height buttons (no gap between them)
/// - Order left-to-right: Minimize, Maximize, Close
/// - Close button is rightmost, touching the right edge
/// - Each button is ~46px wide (CSS standard)
///
/// On Linux/X11, a similar layout is used for consistency.
#[derive(Debug, Clone, Copy)]
pub struct CaptionButton {
    /// X position of the button's left edge.
    pub x: f32,
    /// Y position of the button's top edge.
    pub y: f32,
    /// Button width.
    pub w: f32,
    /// Button height.
    pub h: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct CaptionLayout {
    pub minimize: CaptionButton,
    pub maximize: CaptionButton,
    pub close: CaptionButton,
}

/// Standard caption button width on Windows.
pub const CAPTION_BTN_W: f32 = 46.0;

/// Compute caption button layout.
///
/// Buttons are positioned flush against the right edge of the window,
/// stacked left-to-right: Minimize → Maximize → Close.
/// All three buttons share the same height (full bar height).
pub fn compute_caption_layout(screen_w: f32, bar_h: f32) -> CaptionLayout {
    let btn_w = CAPTION_BTN_W;
    let close_x = screen_w - btn_w;
    let maximize_x = close_x - btn_w;
    let minimize_x = maximize_x - btn_w;

    CaptionLayout {
        minimize: CaptionButton {
            x: minimize_x,
            y: 0.0,
            w: btn_w,
            h: bar_h,
        },
        maximize: CaptionButton {
            x: maximize_x,
            y: 0.0,
            w: btn_w,
            h: bar_h,
        },
        close: CaptionButton {
            x: close_x,
            y: 0.0,
            w: btn_w,
            h: bar_h,
        },
    }
}

/// Hit-test a pixel position against caption buttons.
pub fn caption_hit_test(layout: &CaptionLayout, px: f32, py: f32) -> Option<WindowControlButton> {
    for (btn, b) in [
        (WindowControlButton::Minimize, &layout.minimize),
        (WindowControlButton::Maximize, &layout.maximize),
        (WindowControlButton::Close, &layout.close),
    ] {
        if px >= b.x && px <= b.x + b.w && py >= b.y && py <= b.y + b.h {
            return Some(btn);
        }
    }
    None
}

/// Check if pixel is inside ANY caption button area.
pub fn is_in_caption_area(layout: &CaptionLayout, px: f32, py: f32) -> bool {
    px >= layout.minimize.x && py <= layout.close.h
}

// ── Legacy x11 module removed; all callers now use compute_caption_layout ─

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_traffic_light_width() {
        #[cfg(target_os = "macos")]
        assert_eq!(TRAFFIC_LIGHT_WIDTH, 92.0);
        #[cfg(not(target_os = "macos"))]
        assert_eq!(TRAFFIC_LIGHT_WIDTH, 0.0);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn test_caption_layout_positions() {
        let layout = compute_caption_layout(1000.0, 52.0);
        // Close is rightmost, touching right edge
        assert!((layout.close.x - (1000.0 - 46.0)).abs() < 0.01);
        assert!((layout.close.w - 46.0).abs() < 0.01);
        // Maximize is to the left of close
        assert!((layout.maximize.x - (1000.0 - 92.0)).abs() < 0.01);
        // Minimize is to the left of maximize
        assert!((layout.minimize.x - (1000.0 - 138.0)).abs() < 0.01);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn test_caption_hit_test() {
        let layout = compute_caption_layout(1000.0, 52.0);
        // Close button center
        assert_eq!(
            caption_hit_test(&layout, 977.0, 26.0),
            Some(WindowControlButton::Close)
        );
        // Maximize button center
        assert_eq!(
            caption_hit_test(&layout, 931.0, 26.0),
            Some(WindowControlButton::Maximize)
        );
        // Minimize button center
        assert_eq!(
            caption_hit_test(&layout, 885.0, 26.0),
            Some(WindowControlButton::Minimize)
        );
        // Outside all buttons
        assert_eq!(caption_hit_test(&layout, 800.0, 26.0), None);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn test_caption_is_in_area() {
        let layout = compute_caption_layout(1000.0, 52.0);
        assert!(is_in_caption_area(&layout, 977.0, 10.0));
        assert!(!is_in_caption_area(&layout, 800.0, 10.0));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn test_caption_full_bar_height() {
        let layout = compute_caption_layout(800.0, 48.0);
        // All buttons should span the full bar height
        assert!((layout.minimize.h - 48.0).abs() < 0.01);
        assert!((layout.maximize.h - 48.0).abs() < 0.01);
        assert!((layout.close.h - 48.0).abs() < 0.01);
    }
}
