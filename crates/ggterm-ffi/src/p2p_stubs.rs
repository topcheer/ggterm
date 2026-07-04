//! P2P FFI stubs — compiled when the `p2p` feature is disabled.
//!
//! All functions return 0/null/false so the binary still links.
//! Dart side detects missing symbols via try/catch symbol lookup.

#![allow(clippy::missing_safety_doc)]

use std::ffi::c_char;

#[unsafe(no_mangle)]
pub extern "C" fn ggterm_p2p_host_start(_session_id: u32) -> u32 {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn ggterm_p2p_host_ticket(_session_id: u32) -> *mut c_char {
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn ggterm_p2p_host_accept(_session_id: u32) -> i32 {
    -1
}

#[unsafe(no_mangle)]
pub extern "C" fn ggterm_p2p_generate_ticket() -> *mut c_char {
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn ggterm_p2p_host_session_id() -> u32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_p2p_connect(_ticket: *const c_char) -> u32 {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn ggterm_p2p_is_connected(_session_id: u32) -> bool {
    false
}

#[unsafe(no_mangle)]
pub extern "C" fn ggterm_p2p_close(_session_id: u32) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_p2p_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            drop(std::ffi::CString::from_raw(ptr));
        }
    }
}
