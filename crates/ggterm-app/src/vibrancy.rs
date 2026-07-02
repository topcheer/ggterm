//! P27-E: macOS window vibrancy (NSVisualEffectView) integration.
//!
//! Sets up a `NSVisualEffectView` as the window's background view to
//! provide the system-wide vibrancy/blur effect.
//!
//! # Platform
//! macOS only — this module compiles to nothing on other platforms.

#[cfg(target_os = "macos")]
/// # Safety
/// `ns_view` must be a valid NSView pointer (e.g. from `raw_window_handle`).
#[cfg(target_os = "macos")]
#[allow(unsafe_op_in_unsafe_fn)]
pub unsafe fn apply_vibrancy_to_view(ns_view: *mut std::ffi::c_void) {
    if ns_view.is_null() {
        return;
    }

    apply_vibrancy_inner(ns_view);
}

#[cfg(target_os = "macos")]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn apply_vibrancy_inner(ns_view: *mut std::ffi::c_void) {
    use std::ffi::CString;

    unsafe extern "C" {
        fn objc_msgSend();
        fn objc_getClass(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
        fn sel_registerName(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
    }

    // Helper: send a message that returns an object pointer.
    fn msg_obj(recv: *mut std::ffi::c_void, sel: *mut std::ffi::c_void) -> *mut std::ffi::c_void {
        let func: unsafe extern "C" fn(
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
        ) -> *mut std::ffi::c_void =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        unsafe { func(recv, sel) }
    }

    // Helper: send a message with one i64 argument.
    fn msg_void_i(recv: *mut std::ffi::c_void, sel: *mut std::ffi::c_void, arg: i64) {
        let func: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, i64) =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        unsafe { func(recv, sel, arg) }
    }

    // Helper: send a message with one object argument.
    fn msg_void_obj(
        recv: *mut std::ffi::c_void,
        sel: *mut std::ffi::c_void,
        arg: *mut std::ffi::c_void,
    ) {
        let func: unsafe extern "C" fn(
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
        ) = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        unsafe { func(recv, sel, arg) }
    }

    // Helper: send a message with one bool argument.
    fn msg_void_bool(recv: *mut std::ffi::c_void, sel: *mut std::ffi::c_void, arg: bool) {
        let func: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, bool) =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        unsafe { func(recv, sel, arg) }
    }

    // window = [ns_view window]
    let window_sel = sel_registerName(CString::new("window").unwrap().as_ptr());
    let window = msg_obj(ns_view, window_sel);
    if window.is_null() {
        log::warn!("vibrancy: NSView has no window");
        return;
    }

    // NSVisualEffectView *view = [[NSVisualEffectView alloc] init]
    let veff_class = objc_getClass(CString::new("NSVisualEffectView").unwrap().as_ptr());
    if veff_class.is_null() {
        log::warn!("vibrancy: NSVisualEffectView not found");
        return;
    }
    let alloc_sel = sel_registerName(CString::new("alloc").unwrap().as_ptr());
    let init_sel = sel_registerName(CString::new("init").unwrap().as_ptr());
    let view = msg_obj(veff_class, alloc_sel);
    let view = msg_obj(view, init_sel);

    // frame = [window frame] → [view setFrame: frame]
    {
        let read_frame: unsafe extern "C" fn(
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
        ) -> (f64, f64, f64, f64) =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        let frame_sel = sel_registerName(CString::new("frame").unwrap().as_ptr());
        let (x, y, w, h) = read_frame(window, frame_sel);

        let set_frame: unsafe extern "C" fn(
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
            f64,
            f64,
            f64,
            f64,
        ) = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        let set_frame_sel = sel_registerName(CString::new("setFrame:").unwrap().as_ptr());
        set_frame(view, set_frame_sel, x, y, w, h);
    }

    // [view setBlendingMode: 0] — BehindWindow
    msg_void_i(
        view,
        sel_registerName(CString::new("setBlendingMode:").unwrap().as_ptr()),
        0,
    );

    // [view setState: 1] — Active
    msg_void_i(
        view,
        sel_registerName(CString::new("setState:").unwrap().as_ptr()),
        1,
    );

    // [view setMaterial: 21] — UnderWindowBackground
    msg_void_i(
        view,
        sel_registerName(CString::new("setMaterial:").unwrap().as_ptr()),
        21,
    );

    // [view setAutoresizingMask: 18] — widthSizable(2) | heightSizable(16)
    msg_void_i(
        view,
        sel_registerName(CString::new("setAutoresizingMask:").unwrap().as_ptr()),
        18,
    );

    // contentView = [window contentView]
    let cv_sel = sel_registerName(CString::new("contentView").unwrap().as_ptr());
    let content_view = msg_obj(window, cv_sel);

    // [contentView addSubview:view positioned:NSWindowBelow relativeTo:nil]
    // NSWindowBelow = -1 (must not use 0 / NSWindowOut — AppKit asserts on macOS 26+)
    {
        let add_fn: unsafe extern "C" fn(
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
            i64,
            *mut std::ffi::c_void,
        ) = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        let add_sel = sel_registerName(
            CString::new("addSubview:positioned:relativeTo:")
                .unwrap()
                .as_ptr(),
        );
        // NSWindowBelow = -1, NSWindowAbove = 1, NSWindowOut = 0
        add_fn(content_view, add_sel, view, -1, std::ptr::null_mut());
    }

    // [window setOpaque: NO]
    msg_void_bool(
        window,
        sel_registerName(CString::new("setOpaque:").unwrap().as_ptr()),
        false,
    );

    // [NSColor clearColor] → [window setBackgroundColor: clear]
    let ns_color = objc_getClass(CString::new("NSColor").unwrap().as_ptr());
    let clear_sel = sel_registerName(CString::new("clearColor").unwrap().as_ptr());
    let clear_color = msg_obj(ns_color, clear_sel);
    msg_void_obj(
        window,
        sel_registerName(CString::new("setBackgroundColor:").unwrap().as_ptr()),
        clear_color,
    );

    log::info!("macOS vibrancy applied");
}

#[cfg(not(target_os = "macos"))]
pub fn apply_vibrancy_to_view(_ns_view: *mut std::ffi::c_void) {
    // No-op on non-macOS platforms.
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn t_noop_on_non_macos() {
        super::apply_vibrancy_to_view(std::ptr::null_mut());
    }
}
