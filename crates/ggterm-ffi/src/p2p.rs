//! P2P FFI — C-ABI wrappers for Iroh-based QUIC terminal sharing.
//!
//! Exposes P2P host and client functions so Flutter/Dart can:
//!
//! 1. Generate a connection ticket (host side, for QR code display)
//! 2. Connect to a host using a ticket (client side, after scanning QR)
//! 3. Check connection status
//! 4. Free C-allocated strings
//!
//! When the `p2p` feature is disabled, all functions return 0/null/false.

// FFI unsafe functions have simple null-safety contracts documented inline.
#![allow(clippy::missing_safety_doc)]

use std::ffi::c_char;
use std::ffi::{CStr, CString};

// ── Free function (always available — no p2p dependency) ───────────────

/// Free a C string allocated by P2P FFI functions.
///
/// # Safety
/// `ptr` must be a valid pointer from a previous P2P FFI call, or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ggterm_p2p_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            drop(CString::from_raw(ptr));
        }
    }
}

/// Get the session_id of the host created by `ggterm_p2p_generate_ticket`.
///
/// Returns 0 if no host has been created or p2p is not compiled.
#[unsafe(no_mangle)]
pub extern "C" fn ggterm_p2p_host_session_id() -> u32 {
    #[cfg(feature = "p2p")]
    {
        host_session_id()
    }
    #[cfg(not(feature = "p2p"))]
    {
        0
    }
}

// ── Feature-gated implementation ───────────────────────────────────────

#[cfg(feature = "p2p")]
mod imp {
    use super::*;
    use crate::transport;
    use ggterm_p2p::{P2pClient, P2pHost, P2pTransport};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};

    // ── Global registries ──────────────────────────────────────────────

    struct HostEntry {
        host: Option<P2pHost>,
        ticket: String,
        accepted: Arc<Mutex<Option<Result<P2pTransport, String>>>>,
        accept_started: AtomicBool,
    }

    /// Pending client connect result.
    struct PendingConnect {
        result: Arc<Mutex<Option<Result<(), String>>>>,
    }

    static P2P_HOSTS: OnceLock<Mutex<HashMap<u32, HostEntry>>> = OnceLock::new();
    static P2P_HOST_SESSION: OnceLock<Mutex<u32>> = OnceLock::new();
    static PENDING_CONNECTS: OnceLock<Mutex<HashMap<u32, PendingConnect>>> = OnceLock::new();

    fn p2p_hosts() -> &'static Mutex<HashMap<u32, HostEntry>> {
        P2P_HOSTS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn pending_connects() -> &'static Mutex<HashMap<u32, PendingConnect>> {
        PENDING_CONNECTS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    pub(super) fn host_session_id() -> u32 {
        let id = P2P_HOST_SESSION.get_or_init(|| Mutex::new(0));
        *id.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_host_session_id(id: u32) {
        let storage = P2P_HOST_SESSION.get_or_init(|| Mutex::new(0));
        *storage.lock().unwrap_or_else(|e| e.into_inner()) = id;
    }

    fn to_c_string(s: &str) -> *mut c_char {
        match CString::new(s) {
            Ok(cs) => cs.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    }

    // ── Host lifecycle ─────────────────────────────────────────────────

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn ggterm_p2p_host_start(session_id: u32) -> u32 {
        match P2pHost::start() {
            Ok(host) => {
                let ticket = host.ticket().to_string();
                let entry = HostEntry {
                    host: Some(host),
                    ticket,
                    accepted: Arc::new(Mutex::new(None)),
                    accept_started: AtomicBool::new(false),
                };
                let mut hosts = p2p_hosts().lock().unwrap_or_else(|e| e.into_inner());
                hosts.insert(session_id, entry);
                session_id
            }
            Err(e) => {
                transport::set_error(format!("P2P host start failed: {e}"));
                0
            }
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn ggterm_p2p_host_ticket(session_id: u32) -> *mut c_char {
        let hosts = p2p_hosts().lock().unwrap_or_else(|e| e.into_inner());
        let Some(entry) = hosts.get(&session_id) else {
            return std::ptr::null_mut();
        };
        to_c_string(&entry.ticket)
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn ggterm_p2p_host_accept(session_id: u32) -> i32 {
        let accept_arc = {
            let hosts = p2p_hosts().lock().unwrap_or_else(|e| e.into_inner());
            let Some(state) = hosts.get(&session_id) else {
                return -1;
            };
            if !state.accept_started.load(Ordering::Relaxed) {
                drop(hosts);
                let mut hosts = p2p_hosts().lock().unwrap_or_else(|e| e.into_inner());
                let Some(state) = hosts.get_mut(&session_id) else {
                    return -1;
                };
                let Some(mut host) = state.host.take() else {
                    transport::set_error("P2P host already consumed");
                    return -1;
                };
                let accepted = state.accepted.clone();
                state.accept_started.store(true, Ordering::Relaxed);
                drop(hosts);

                std::thread::spawn(move || {
                    let result = host.accept();
                    let mut guard = accepted.lock().unwrap_or_else(|e| e.into_inner());
                    *guard = Some(result.map_err(|e| e.to_string()));
                });
                return 0;
            }
            state.accepted.clone()
        };

        let mut guard = accept_arc.lock().unwrap_or_else(|e| e.into_inner());
        match guard.take() {
            None => 0,
            Some(Ok(transport)) => {
                let mut map = transport::sessions()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if let Some(s) = map.get_mut(&session_id) {
                    s.transport = Some(Box::new(transport));
                }
                1
            }
            Some(Err(e)) => {
                transport::set_error(format!("P2P accept failed: {e}"));
                -1
            }
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn ggterm_p2p_generate_ticket() -> *mut c_char {
        let session_id = transport::create_session(80, 24);
        set_host_session_id(session_id);
        let result = unsafe { ggterm_p2p_host_start(session_id) };
        if result == 0 {
            return std::ptr::null_mut();
        }
        unsafe { ggterm_p2p_host_ticket(session_id) }
    }

    // ── Client connect (NON-BLOCKING) ─────────────────────────────────

    /// NON-BLOCKING: Returns session_id immediately. Connection happens
    /// in background. Poll `ggterm_p2p_connect_status` to check.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn ggterm_p2p_connect(ticket: *const c_char) -> u32 {
        if ticket.is_null() {
            transport::set_error("null ticket");
            return 0;
        }

        let ticket_str = match unsafe { CStr::from_ptr(ticket) }.to_str() {
            Ok(s) => s.to_string(),
            Err(_) => {
                transport::set_error("invalid UTF-8 in ticket");
                return 0;
            }
        };

        // Create session immediately.
        let session_id = transport::create_session(80, 24);
        if session_id == 0 {
            transport::set_error("failed to create session");
            return 0;
        }

        // Set up pending result tracking.
        let result = Arc::new(Mutex::new(None::<Result<(), String>>));
        {
            let mut pending = pending_connects().lock().unwrap_or_else(|e| e.into_inner());
            pending.insert(
                session_id,
                PendingConnect {
                    result: result.clone(),
                },
            );
        }

        // Spawn background thread — does NOT block the caller.
        let _ = std::thread::Builder::new()
            .name("p2p-connect".into())
            .spawn(move || {
                match P2pClient::connect(&ticket_str) {
                    Ok(transport) => {
                        // Install transport into session.
                        let mut map = transport::sessions()
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        if let Some(s) = map.get_mut(&session_id) {
                            s.transport = Some(Box::new(transport));
                        }
                        if let Ok(mut g) = result.lock() {
                            *g = Some(Ok(()));
                        }
                    }
                    Err(e) => {
                        let msg = format!("{e}");
                        if let Ok(mut g) = result.lock() {
                            *g = Some(Err(msg));
                        }
                    }
                }
            });

        session_id
    }

    /// Check connection status: 0=connecting, 1=connected, -1=failed.
    #[unsafe(no_mangle)]
    pub extern "C" fn ggterm_p2p_connect_status(session_id: u32) -> i32 {
        let sid = if session_id == 0 {
            let host_sid = host_session_id();
            if host_sid == 0 {
                return -1;
            }
            host_sid
        } else {
            session_id
        };

        // Check pending connects first.
        {
            let pending = pending_connects().lock().unwrap_or_else(|e| e.into_inner());
            if let Some(pc) = pending.get(&sid) {
                let r = pc.result.lock().unwrap_or_else(|e| e.into_inner());
                return match &*r {
                    None => 0,
                    Some(Ok(())) => 1,
                    Some(Err(msg)) => {
                        transport::set_error(msg.clone());
                        -1
                    }
                };
            }
        }

        // No pending — check installed transport.
        let mut map = transport::sessions()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let connected = match map.get_mut(&sid) {
            Some(s) => s.transport.as_mut().map(|t| t.is_alive()).unwrap_or(false),
            None => return -1,
        };
        if connected { 1 } else { 0 }
    }

    // ── Status & cleanup ───────────────────────────────────────────────

    #[unsafe(no_mangle)]
    pub extern "C" fn ggterm_p2p_is_connected(session_id: u32) -> bool {
        let sid = if session_id == 0 {
            let host_sid = host_session_id();
            if host_sid == 0 {
                return false;
            }
            host_sid
        } else {
            session_id
        };

        let mut map = transport::sessions()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let Some(s) = map.get_mut(&sid) else {
            return false;
        };
        s.transport.as_mut().is_some_and(|t| t.is_alive())
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn ggterm_p2p_close(session_id: u32) {
        let mut hosts = p2p_hosts().lock().unwrap_or_else(|e| e.into_inner());
        hosts.remove(&session_id);
        drop(hosts);

        let mut pending = pending_connects().lock().unwrap_or_else(|e| e.into_inner());
        pending.remove(&session_id);
        drop(pending);

        let mut map = transport::sessions()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(s) = map.get_mut(&session_id) {
            s.transport = None;
        }
    }
}

// ── Non-p2p stubs ──────────────────────────────────────────────────────

#[cfg(not(feature = "p2p"))]
mod stub {
    use std::ffi::c_char;
    use std::ptr;

    #[unsafe(no_mangle)]
    pub extern "C" fn ggterm_p2p_host_start(_session_id: u32) -> u32 {
        0
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn ggterm_p2p_host_ticket(_session_id: u32) -> *mut c_char {
        ptr::null_mut()
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn ggterm_p2p_host_accept(_session_id: u32) -> i32 {
        -1
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn ggterm_p2p_generate_ticket() -> *mut c_char {
        ptr::null_mut()
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn ggterm_p2p_connect(_ticket: *const c_char) -> u32 {
        0
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn ggterm_p2p_is_connected(_session_id: u32) -> bool {
        false
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn ggterm_p2p_close(_session_id: u32) {}
}

// Re-export so symbols are at crate root
#[cfg(feature = "p2p")]
pub use imp::*;

#[cfg(not(feature = "p2p"))]
pub use stub::*;

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "p2p"))]
mod tests {
    use super::*;

    #[test]
    fn t_free_null_is_safe() {
        unsafe {
            ggterm_p2p_free_string(std::ptr::null_mut());
        }
    }

    #[test]
    fn t_connect_null_ticket() {
        let id = unsafe { ggterm_p2p_connect(std::ptr::null()) };
        assert_eq!(id, 0);
    }

    #[test]
    fn t_connect_invalid_ticket() {
        let ticket = CString::new("invalid-ticket").unwrap();
        let id = unsafe { ggterm_p2p_connect(ticket.as_ptr()) };
        assert_eq!(id, 0);
    }

    #[test]
    fn t_is_connected_invalid_session() {
        assert!(!ggterm_p2p_is_connected(99999));
    }

    #[test]
    fn t_host_ticket_invalid_session() {
        let ptr = unsafe { ggterm_p2p_host_ticket(99999) };
        assert!(ptr.is_null());
    }

    #[test]
    fn t_host_accept_invalid_session() {
        let result = unsafe { ggterm_p2p_host_accept(99999) };
        assert_eq!(result, -1);
    }

    #[test]
    fn t_close_invalid_session() {
        unsafe {
            ggterm_p2p_close(99999);
        }
    }
}
