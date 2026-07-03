//! macOS custom titlebar: transparent titlebar with traffic light buttons.
//!
//! Instead of removing decorations (which kills the traffic lights),
//! we keep decorations and make the titlebar transparent + full-size content.
//! This lets the tab bar extend underneath the titlebar while keeping
//! the close/minimize/zoom buttons fully functional.

#[cfg(target_os = "macos")]
pub mod macos {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use raw_window_handle::HasWindowHandle;

    /// Style mask bit for full-size content view (extends content under titlebar).
    const NS_FULL_SIZE_CONTENT_VIEW_MASK: u64 = 1 << 15;

    /// Make the window's titlebar transparent and extend content underneath.
    ///
    /// This is how Warp, WezTerm, and VS Code achieve unified titlebars
    /// on macOS: the native titlebar becomes invisible, content fills the
    /// entire window, and traffic light buttons float on top.
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
            // Get the NSWindow from the NSView.
            let win: *mut AnyObject = msg_send![ns_view, window];
            if win.is_null() {
                log::warn!("NSWindow is null in make_titlebar_transparent");
                return;
            }

            // 1. Add NSFullSizeContentViewWindowMask to styleMask.
            //    This makes the content view extend underneath the titlebar.
            let current_mask: u64 = msg_send![win, styleMask];
            let new_mask = current_mask | NS_FULL_SIZE_CONTENT_VIEW_MASK;
            let _: () = msg_send![win, setStyleMask: new_mask];

            // 2. Make the titlebar transparent (not hidden — buttons stay).
            let _: () = msg_send![win, setTitlebarAppearsTransparent: true];

            // 3. Hide the title text.
            let _: () = msg_send![win, setTitleVisibility: 1i64]; // NSWindowTitleHidden = 1

            // 4. Make the window non-opaque so our background shows through.
            let _: () = msg_send![win, setOpaque: false];

            // 5. Set the background color to clear (NSColor clearColor).
            //    NSColor clearColor = [NSColor colorWithDeviceWhite:0.0 alpha:0.0]
            //    We use the simpler: [NSColor clearColor]
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
    pub const TRAFFIC_LIGHT_WIDTH: f32 = 78.0;
}

#[cfg(target_os = "macos")]
pub use macos::{TRAFFIC_LIGHT_WIDTH, make_titlebar_transparent as install_traffic_lights};

#[cfg(not(target_os = "macos"))]
pub const TRAFFIC_LIGHT_WIDTH: f32 = 0.0;

#[cfg(not(target_os = "macos"))]
pub fn install_traffic_lights(_window: &winit::window::Window) {}

/// Window control buttons for Linux/Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowControlButton {
    Minimize,
    Maximize,
    Close,
}

#[cfg(not(target_os = "macos"))]
pub mod x11 {
    use super::WindowControlButton;

    pub struct ControlButtonLayout {
        pub minimize: (f32, f32, f32),
        pub maximize: (f32, f32, f32),
        pub close: (f32, f32, f32),
    }

    pub const BTN_SIZE: f32 = 14.0;
    pub const BTN_GAP: f32 = 8.0;

    pub fn compute_layout(right_edge: f32, bar_height: f32) -> ControlButtonLayout {
        let total_w = BTN_SIZE * 3.0 + BTN_GAP * 2.0;
        let start_x = right_edge - total_w - 12.0;
        let y = (bar_height - BTN_SIZE) / 2.0;

        ControlButtonLayout {
            close: (start_x, y, BTN_SIZE),
            minimize: (start_x + BTN_SIZE + BTN_GAP, y, BTN_SIZE),
            maximize: (start_x + (BTN_SIZE + BTN_GAP) * 2.0, y, BTN_SIZE),
        }
    }

    pub fn hit_test(layout: &ControlButtonLayout, px: f32, py: f32) -> Option<WindowControlButton> {
        for (btn, &(x, y, size)) in [
            (WindowControlButton::Close, &layout.close),
            (WindowControlButton::Minimize, &layout.minimize),
            (WindowControlButton::Maximize, &layout.maximize),
        ] {
            if px >= x && px <= x + size && py >= y && py <= y + size {
                return Some(btn);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_traffic_light_width() {
        #[cfg(target_os = "macos")]
        assert_eq!(super::TRAFFIC_LIGHT_WIDTH, 78.0);
        #[cfg(not(target_os = "macos"))]
        assert_eq!(super::TRAFFIC_LIGHT_WIDTH, 0.0);
    }
}
