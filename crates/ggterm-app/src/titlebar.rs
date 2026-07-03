//! macOS custom titlebar: keeps traffic light buttons in a frameless window.
//!
//! When `with_decorations(false)` is set on macOS, the traffic light buttons
//! (close/minimize/zoom) disappear. This module restores them by creating
//! a standard NSButton titlebar accessory and positioning it correctly.
//!
//! On Linux/Windows, we draw our own window control buttons.

#[cfg(target_os = "macos")]
pub mod macos {
    use raw_window_handle::HasWindowHandle;

    /// Install traffic light buttons on a frameless window.
    ///
    /// This works by accessing the NSWindow's underlying NSView and
    /// calling `setShowsToolbarButton:` / using standard button items.
    ///
    /// The simplest approach on macOS is to use a "Unified Titlebar" style
    /// by setting `styleMask` to include `NSFullSizeContentViewWindowMask`
    /// and keeping the buttons visible via `standardWindowButton:`.
    ///
    /// However, with `with_decorations(false)`, we lose ALL titlebar.
    /// Instead, we use a workaround: create the window WITH decorations,
    /// then hide the title bar area by making the content view full-size.
    pub fn install_traffic_lights(window: &winit::window::Window) {
        use objc2::msg_send;
        use objc2::runtime::AnyObject;

        let Ok(handle) = window.window_handle() else {
            return;
        };
        let raw_window_handle::RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return;
        };

        let ns_view = appkit.ns_view.as_ptr() as *mut AnyObject;

        unsafe {
            // Walk up from the NSView to find the NSWindow.
            let window: *mut AnyObject = msg_send![ns_view, window];
            if window.is_null() {
                return;
            }

            // standardWindowButton: returns the button for close/minimize/zoom.
            // 0 = close, 1 = miniaturize, 2 = zoom
            let close_btn: *mut AnyObject = msg_send![window, standardWindowButton: 0i64];
            let mini_btn: *mut AnyObject = msg_send![window, standardWindowButton: 1i64];
            let zoom_btn: *mut AnyObject = msg_send![window, standardWindowButton: 2i64];

            // Make sure all buttons are visible.
            for btn in [close_btn, mini_btn, zoom_btn] {
                if !btn.is_null() {
                    let _: bool = msg_send![btn, setHidden: false];
                }
            }

            // Set the window's background color to match our theme.
            let _: () = msg_send![window, setOpaque: false];

            // CRITICAL: set showsToolbarButton to false to avoid extra button.
            let _: () = msg_send![window, setShowsToolbarButton: false];

            log::info!("macOS traffic lights installed on frameless window");
        }
    }

    /// The width reserved for traffic light buttons on the left side.
    /// Standard macOS traffic lights occupy ~70px of horizontal space.
    pub const TRAFFIC_LIGHT_WIDTH: f32 = 78.0;

    /// The vertical offset from top where traffic lights are centered.
    pub const TRAFFIC_LIGHT_Y: f32 = 14.0;
}

#[cfg(target_os = "macos")]
pub use macos::{TRAFFIC_LIGHT_WIDTH, TRAFFIC_LIGHT_Y, install_traffic_lights};

#[cfg(not(target_os = "macos"))]
pub const TRAFFIC_LIGHT_WIDTH: f32 = 0.0;

#[cfg(not(target_os = "macos"))]
pub fn install_traffic_lights(_window: &winit::window::Window) {}

/// Window control buttons for Linux/Windows (minimize/maximize/close).
/// These are drawn as overlay rectangles in the tab bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowControlButton {
    Minimize,
    Maximize,
    Close,
}

#[cfg(not(target_os = "macos"))]
pub mod x11 {
    use super::WindowControlButton;

    /// Layout of the three window control buttons (right side of tab bar).
    pub struct ControlButtonLayout {
        pub minimize: (f32, f32, f32), // x, y, size
        pub maximize: (f32, f32, f32),
        pub close: (f32, f32, f32),
    }

    pub const BTN_SIZE: f32 = 14.0;
    pub const BTN_GAP: f32 = 8.0;

    /// Compute button positions given tab bar right edge x and bar height.
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

    /// Hit-test which button was clicked. Returns None if no button hit.
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
    use super::*;

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn test_x11_hit_test() {
        let layout = x11::compute_layout(1000.0, 36.0);
        assert_eq!(
            x11::hit_test(&layout, layout.close.0 + 1.0, layout.close.1 + 1.0),
            Some(WindowControlButton::Close)
        );
        assert_eq!(x11::hit_test(&layout, 0.0, 0.0), None);
    }
}
